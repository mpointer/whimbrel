package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.read.PhoneActionItem
import com.mpointer.whimbrel.mobile.read.PhoneCalendarEvent
import com.mpointer.whimbrel.mobile.read.PhoneMeeting
import com.mpointer.whimbrel.mobile.read.PhoneMeetingPrep
import com.mpointer.whimbrel.mobile.read.PhoneRoutineRun
import com.mpointer.whimbrel.mobile.read.PhoneTopicSnapshot
import com.mpointer.whimbrel.mobile.read.PhoneTranscriptPage
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

private const val STORE_THREAD = "test-store"

/** The thread's name without the " @coroutine#n" suffix debug mode appends. */
private fun threadName() = Thread.currentThread().name.substringBefore(" @")

/** Stands in for the local store: scripted pull results, a settable count, and the thread each call ran on. */
private class FakeStore : MeetingSyncStore {
    val pullResults: MutableList<Boolean> = Collections.synchronizedList(mutableListOf())
    @Volatile var count = 0
    @Volatile var countAfterPull: Int? = null
    val pulls: MutableList<SyncCredentials> = Collections.synchronizedList(mutableListOf())
    val threads: MutableList<String> = Collections.synchronizedList(mutableListOf())
    @Volatile var closed = false
    @Volatile var more = false

    /** Called on the store thread after each pull is recorded. */
    @Volatile var onPull: (() -> Unit)? = null

    private fun note() { threads += threadName() }

    override fun pullOnly(c: SyncCredentials, captureWindowDays: Int): Boolean {
        note()
        pulls += c
        try {
            val ok = synchronized(pullResults) { if (pullResults.isEmpty()) true else pullResults.removeAt(0) }
            countAfterPull?.let { if (ok) count = it }
            return ok
        } finally {
            onPull?.invoke()
        }
    }

    override fun groupedMeetingCount(): Int? = count.also { note() }
    override fun moreToPull(): Boolean = more.also { note() }
    override fun lastSyncError(): String = "connect failed: secret detail"
    override fun lastPullStats(): String = """[{"peer":"abc123","fetched":5}]"""
    override fun close() { note(); closed = true }

    override fun recentMeetings(limit: Int): List<PhoneMeeting> = emptyList<PhoneMeeting>().also { note() }
    override fun meeting(id: Long): PhoneMeeting? = null
    override fun transcriptPage(meetingId: Long, offset: Int, limit: Int): PhoneTranscriptPage? = null
    override fun actionItems(meetingId: Long): List<PhoneActionItem> = emptyList()
    override fun routineRuns(limit: Int): List<PhoneRoutineRun> = emptyList()
    override fun prep(eventId: String): PhoneMeetingPrep? = null
    override fun upcomingEvents(fromUnixMs: Long, untilUnixMs: Long, limit: Int): List<PhoneCalendarEvent> = emptyList()
    override fun topicSnapshots() = emptyList<PhoneTopicSnapshot>()
}

private class FakeCredentials : SyncCredentialSource {
    var credentials: SyncCredentials? = SyncCredentials("t", "u", "k", "phone")
    val renewals = mutableListOf<Pair<Boolean, Boolean>>()

    /** Runs inside every renewIfDue, like a revoke found while renewing. */
    var onRenew: (() -> Unit)? = null
    override fun syncCredentials() = credentials
    override suspend fun renewIfDue(afterFailure: Boolean, force: Boolean) {
        renewals += afterFailure to force
        onRenew?.invoke()
    }
}

class SyncModelTest {
    private val storeExecutor = Executors.newSingleThreadExecutor { Thread(it, STORE_THREAD) }
    private val storeDispatcher = storeExecutor.asCoroutineDispatcher()
    private val store = FakeStore()
    private val credentials = FakeCredentials()
    private val sleeper = FakeSleeper()
    private val logged = Collections.synchronizedList(mutableListOf<String>())
    private var now = 1_000L

    @After
    fun tearDown() = storeExecutor.shutdownNow().let { }

    private fun TestScope.model(
        open: () -> MeetingSyncStore? = { store },
        scope: CoroutineScope = backgroundScope,
        sleep: suspend (Long) -> Unit = { sleeper.sleep(it) },
        settingsPull: ((SyncCredentials, Int) -> Unit)? = null,
        wipeEpoch: () -> Int = { 0 },
    ) = SyncModel(open, credentials, scope, storeDispatcher, sleep, { now }, settingsPull, wipeEpoch, log = { logged += it })

    /** Waits for every call already posted to the store thread. */
    private suspend fun drainStore() = withContext(storeDispatcher) {}

    @Test
    fun readRunsOnTheStoreQueueAndReturnsNilWithoutAStore() = runTest {
        val model = model()
        model.syncNow()
        store.threads.clear()
        assertEquals(0, model.read { it.recentMeetings(5).size })
        assertEquals(listOf(STORE_THREAD), store.threads.toList())
        val none = model(open = { null }).run { syncNow(); read { it.recentMeetings(5).size } }
        assertNull("no store, no read", none)
        assertNull("not opened yet, no read", model().read { it.recentMeetings(5).size })
    }

    @Test
    fun aCatchUpLogsThePullStatsEveryTenthRound() = runTest {
        store.pullResults += List(12) { true }
        val model = model()
        repeat(12) { model.syncNow() }
        assertEquals("rounds 1 and 11", 2, logged.count { it.startsWith("pull stats:") })
    }

    @Test
    fun theFirstAndEveryFailedRoundLogThePullStats() = runTest {
        store.pullResults += listOf(true, false)
        val model = model()
        model.syncNow()
        model.syncNow()
        assertEquals(2, logged.count { it == """pull stats: [{"peer":"abc123","fetched":5}]""" })
    }

    @Test
    fun readRevisionIncrementsAfterASuccessfulPullOnly() = runTest {
        val model = model()
        model.syncNow() // opens (+1) and pulls (+1)
        val afterFirst = model.readRevision.value
        assertEquals(2, afterFirst)
        store.pullResults += listOf(true, false)
        model.syncNow()
        assertEquals(afterFirst + 1, model.readRevision.value)
        model.syncNow()
        assertEquals("a failed pull changes nothing to reload", afterFirst + 1, model.readRevision.value)
    }

    @Test
    fun readRevisionIncrementsWhenTheStoreOpensEvenIfThePullFails() = runTest {
        store.pullResults += false
        val model = model()
        assertEquals(0, model.readRevision.value)
        model.syncNow()
        assertTrue("screens that read before the store opened must read again, offline too", model.readRevision.value > 0)
    }

    @Test
    fun resetClosesTheStoreAndReopensLazily() = runTest {
        val opened = mutableListOf<FakeStore>()
        val model = model(open = { FakeStore().also { opened += it } })
        model.syncNow()
        assertNotNull(model.meetingCount.value)
        val cleaned = CompletableDeferred<Boolean>()
        model.reset { cleaned.complete(opened[0].closed) }
        assertNull(model.meetingCount.value)
        assertNull(model.loopJob)
        assertNull("no store, no read", model.read { it.recentMeetings(5) })
        assertTrue("cleanup runs after the handle is closed", cleaned.await())
        assertEquals(STORE_THREAD, opened[0].threads.last())
        model.syncNow()
        assertEquals("re-enrolling opens a fresh store", 2, opened.size)
        assertFalse(opened[1].closed)
    }

    /** A revoke is usually found while a round is in flight (renewIfDue inside syncNow): the reset completes when the round ends. */
    @Test
    fun resetRequestedDuringARoundCompletesAfterTheRound() = runTest {
        val opened = mutableListOf<FakeStore>()
        val model = model(open = { FakeStore().also { opened += it } })
        val cleaned = CompletableDeferred<Unit>()
        var cleanups = 0
        credentials.onRenew = { model.reset { cleanups += 1; cleaned.complete(Unit) } }
        model.syncNow()
        cleaned.await()
        assertEquals("cleanup ran once the round ended", 1, cleanups)
        assertTrue("the store is closed", opened[0].closed)
        assertEquals("a revoked round pulls nothing", 0, opened[0].pulls.size)
        assertNull(model.meetingCount.value)
        assertFalse(model.isSyncing.value)
    }

    /** Review focus 4: a pull that returns after the reset writes nothing (no reload bump, no "synced" time, no settings pull). */
    @Test
    fun revokedMidPullDoesNotWriteAfterReset() = runTest {
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        store.onPull = {
            entered.complete(Unit)
            release.await(10, TimeUnit.SECONDS)
        }
        store.countAfterPull = 9
        var settingsPulls = 0
        val model = model(settingsPull = { _, _ -> settingsPulls += 1 })
        val round = launch { model.syncNow() }
        entered.await()
        val beforeReset = model.readRevision.value // the open's bump
        val cleaned = CompletableDeferred<Unit>()
        model.reset { cleaned.complete(Unit) }
        assertTrue("deferred while the round runs", model.isSyncing.value)
        release.countDown()
        round.join()
        cleaned.await()
        assertEquals("only the reset's own bump", beforeReset + 1, model.readRevision.value)
        assertNull(model.lastSyncedAtMs.value)
        assertNull(model.meetingCount.value)
        assertNull(model.lastError.value)
        assertEquals(0, settingsPulls)
        assertTrue(store.closed)
    }

    @Test
    fun storeRemovalMarkerIsSetFirstAndClearedOnlyAfterSuccess() {
        // Ported here as in SyncModelTests.swift; the edge cases live in StoreRemovalTest.
        val prefs = FakePrefs()
        StoreRemoval.markPending(prefs)
        assertTrue(prefs.getBoolean(StoreRemoval.pendingKey, false))
        StoreRemoval.finish(prefs) { throw java.io.IOException("permission denied") }
        assertTrue("failed removal stays pending", prefs.getBoolean(StoreRemoval.pendingKey, false))
        StoreRemoval.finishIfPending(prefs) { throw java.io.FileNotFoundException() }
        assertFalse("already gone counts as done", prefs.getBoolean(StoreRemoval.pendingKey, false))
        var removals = 0
        StoreRemoval.finishIfPending(prefs) { removals += 1 }
        assertEquals("nothing pending, nothing removed", 0, removals)
    }

    @Test
    fun startPullsAtOnceThenEveryMinuteAndShowsTheGroupedCount() = runTest {
        sleeper.limit = 2
        store.countAfterPull = 7
        now = 42_000
        val model = model()
        model.start()
        model.loopJob!!.join()
        assertEquals(3, store.pulls.size)
        assertEquals("phone", store.pulls.first().deviceId)
        assertEquals(listOf(60L, 60L, 60L), sleeper.intervals)
        assertEquals(7, model.meetingCount.value)
        assertEquals(42_000L, model.lastSyncedAtMs.value)
        assertNull(model.lastError.value)
        assertFalse(model.isSyncing.value)
        assertEquals(false, credentials.renewals.first().first)
        assertNull("a loop that ended on its own clears itself", model.loopJob)
    }

    @Test
    fun aBacklogBringsTheNextRoundBackAtTheCatchUpInterval() = runTest {
        sleeper.limit = 0
        store.more = true
        val model = model()
        model.start()
        model.loopJob!!.join()
        assertEquals(listOf(SyncTiming.catchUpIntervalSeconds), sleeper.intervals)
        assertTrue("moreToPull is read on the store thread", store.threads.all { it == STORE_THREAD })
        assertTrue("behind with no failure: catching up", model.catchingUp.value)
        model.reset {}
        assertFalse("a reset (sign-out, revoke) ends catch-up", model.catchingUp.value)
    }

    @Test
    fun aFailedRoundBacksOffEvenWhenTheRelayHasMore() = runTest {
        sleeper.limit = 0
        store.more = true
        store.pullResults += false
        val model = model()
        model.start()
        model.loopJob!!.join()
        assertEquals(listOf(120L), sleeper.intervals)
        assertFalse("a failure ends catch-up", model.catchingUp.value)
    }

    @Test
    fun failuresBackOffShowAGenericMessageAndRenewTheToken() = runTest {
        sleeper.limit = 2
        store.pullResults += listOf(false, false, true)
        val model = model()
        model.start()
        model.loopJob!!.join()
        assertEquals(listOf(120L, 240L, 60L), sleeper.intervals)
        assertNull("cleared by the good round", model.lastError.value)
        assertTrue("a failed round re-authenticates (token may be refused)", credentials.renewals.any { it.first })
    }

    @Test
    fun failureMessageIsGenericAndKeepsTheLastCount() = runTest {
        store.count = 3
        store.pullResults += false
        val model = model()
        model.syncNow()
        assertEquals(SyncError.SyncFailed, model.lastError.value)
        assertEquals("Couldn't sync. Will retry automatically.", StringCatalog.resolve(model.lastError.value!!.resId, emptyArray()))
        assertTrue("the core's cause goes to the log only", logged.any { "secret detail" in it })
        assertEquals("what's already on the phone still shows", 3, model.meetingCount.value)
        assertNull(model.lastSyncedAtMs.value)
    }

    @Test
    fun noTokenYetIsAFailureWithoutPulling() = runTest {
        credentials.credentials = null
        val model = model()
        model.syncNow()
        assertEquals(0, store.pulls.size)
        assertEquals(SyncError.Unreachable, model.lastError.value)
        assertEquals("Couldn't reach the server. Will retry automatically.", StringCatalog.resolve(SyncError.Unreachable.resId, emptyArray()))
    }

    @Test
    fun manualSyncForcesARenewal() = runTest {
        val model = model()
        model.syncNow(manual = true)
        assertEquals(true, credentials.renewals.first().second)
    }

    @Test
    fun noLocalStoreSaysSo() = runTest {
        val model = model(open = { null })
        model.syncNow()
        assertEquals(SyncError.StorageUnavailable, model.lastError.value)
        assertEquals("This phone's local storage is unavailable.", StringCatalog.resolve(SyncError.StorageUnavailable.resId, emptyArray()))
        assertNull(model.meetingCount.value)
    }

    @Test
    fun stopEndsTheLoopAndNoMorePulls() = runTest {
        val firstPull = CompletableDeferred<Unit>()
        store.onPull = { firstPull.complete(Unit) }
        val model = model(sleep = { delay(it * 3_600_000) })
        model.start()
        val loop = model.loopJob!!
        firstPull.await()
        model.stop()
        assertNull(model.loopJob)
        assertTrue("the loop must be cancelled", loop.isCancelled)
        loop.join()
        drainStore()
        assertEquals(1, store.pulls.size)
    }

    @Test
    fun startTwiceRunsOneLoop() = runTest {
        val model = model(sleep = { delay(it * 3_600_000) })
        model.start()
        val first = model.loopJob
        model.start()
        assertSame(first, model.loopJob)
        model.stop()
    }

    /** Opening the store (migrations) happens on the store thread, and the model shows syncing while it opens. */
    @Test
    fun storeOpensOffTheMainThreadOnFirstSync() = runTest {
        var openedOn: String? = null
        var model: SyncModel? = null
        var syncingWhileOpening: Boolean? = null
        model = model(open = {
            openedOn = threadName()
            syncingWhileOpening = model?.isSyncing?.value
            store
        })
        model.syncNow()
        assertEquals(STORE_THREAD, openedOn)
        assertEquals(true, syncingWhileOpening)
        assertEquals(1, store.pulls.size)
        assertEquals(STORE_THREAD, store.threads.first())
        assertNull(model.lastError.value)
    }

    /** Opens on the store thread, signalling [entered] and blocking until [release]. */
    private class BlockingOpen(val store: FakeStore) : () -> MeetingSyncStore? {
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val opens = java.util.concurrent.atomic.AtomicInteger()
        override fun invoke(): MeetingSyncStore {
            opens.incrementAndGet()
            entered.complete(Unit)
            release.await(10, TimeUnit.SECONDS)
            return store
        }
    }

    /** A stop() while the first open runs must not drop the opened handle (it would leak, and every later round say "unavailable"). */
    @Test
    fun stopDuringTheFirstOpenKeepsTheHandle() = runTest {
        val open = BlockingOpen(store)
        val model = model(open = open, sleep = { delay(it * 3_600_000) })
        model.start()
        val loop = model.loopJob!!
        open.entered.await()
        model.stop()
        open.release.countDown()
        loop.join()
        model.syncNow()
        assertEquals(1, open.opens.get())
        assertNull(model.lastError.value)
        assertEquals(1, store.pulls.size)
        assertFalse(store.closed)
    }

    /** A revoke while the first open runs: the opened handle is closed before `then` deletes the directory. */
    @Test
    fun resetDuringTheFirstOpenClosesTheHandleBeforeCleanup() = runTest {
        val open = BlockingOpen(store)
        val model = model(open = open, sleep = { delay(it * 3_600_000) })
        model.start()
        open.entered.await()
        val closedAtCleanup = CompletableDeferred<Boolean>()
        model.reset { closedAtCleanup.complete(store.closed) }
        open.release.countDown()
        assertTrue("closed before cleanup", closedAtCleanup.await())
        assertEquals(0, store.pulls.size)
        assertNull(model.meetingCount.value)
    }

    @Test
    fun storeThatFailsToOpenSaysSo() = runTest {
        val model = model(open = { null })
        model.syncNow()
        assertEquals(SyncError.StorageUnavailable, model.lastError.value)
        assertFalse(model.isSyncing.value)
    }

    /** The iPhone reads the wipe epoch with the credentials, and pulls settings off the store queue. */
    @Test
    fun settingsPullGetsTheEpochCapturedWithTheCredentialsOffTheStoreThread() = runTest {
        var epoch = 3
        val got = mutableListOf<Triple<SyncCredentials, Int, String>>()
        credentials.onRenew = { epoch = 4 } // a wipe bumped it before this round read the credentials
        val model = model(
            settingsPull = { c, e ->
                synchronized(got) { got += Triple(c, e, threadName()) }
                epoch = 5 // later bumps don't change what this round passed
            },
            wipeEpoch = { epoch },
        )
        model.syncNow()
        assertEquals(1, got.size)
        val (c, e, thread) = got[0]
        assertEquals("phone", c.deviceId)
        assertEquals(4, e)
        assertNotEquals(STORE_THREAD, thread)
    }

    @Test
    fun settingsPullIsSkippedWhenThePullFails() = runTest {
        store.pullResults += false
        var settingsPulls = 0
        model(settingsPull = { _, _ -> settingsPulls += 1 }).syncNow()
        assertEquals(0, settingsPulls)
    }
}
