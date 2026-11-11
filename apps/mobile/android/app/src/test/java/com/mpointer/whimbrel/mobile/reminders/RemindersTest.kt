package com.mpointer.whimbrel.mobile.reminders

import com.mpointer.whimbrel.mobile.core.AccountPlan
import com.mpointer.whimbrel.mobile.read.PhoneCalendarEvent
import com.mpointer.whimbrel.mobile.sync.FakePrefs
import java.time.ZoneId
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

private val cloud = AccountPlan(plan = "cloud", billing = true)
private val free = AccountPlan(plan = "free", billing = true)
private const val NOW = 1_800_000_000_000L

private class FakeScheduler : ReminderScheduling {
    val replaced = mutableListOf<List<PlannedReminder>>()
    override fun replaceAll(reminders: List<PlannedReminder>) { replaced += reminders }
}

private object PlainText : ReminderText {
    override fun meetingIn(title: String, minutes: Int) = "$title in $minutes minutes"
    override fun withTopic(line: String, topic: String) = "$line · $topic"
    override fun stepsDue(count: Int) = if (count == 1) "1 step due today" else "$count steps due today"
}

private class Rig(
    cloudAtStart: Boolean,
    val prefs: FakePrefs = FakePrefs(),
    val scheduler: FakeScheduler = FakeScheduler(),
    fetchPlan: suspend () -> AccountPlan? = { null },
    readReplica: suspend (Long, Long) -> ReminderCoordinator.ReplicaRead? = { _, _ -> null },
    var permissionAsks: Int = 0,
    var permissionHandled: Boolean = true,
) {
    val cache = PlanCache(prefs).also { if (cloudAtStart) it.update(cloud) }
    val coordinator = ReminderCoordinator(
        scheduler = scheduler,
        settings = ReminderSettingsStore(prefs),
        plan = cache,
        text = PlainText,
        fetchPlan = fetchPlan,
        readReplica = readReplica,
        requestPermission = { permissionAsks += 1; permissionHandled },
        clock = { NOW },
        zone = { ZoneId.of("UTC") },
    )
}

private val emptyRead = ReminderCoordinator.ReplicaRead(emptyList(), emptyList())

@OptIn(ExperimentalCoroutinesApi::class)
class RemindersTest {
    @Test
    fun planCacheKeepsValueWhenFetchFails() {
        val cache = PlanCache(FakePrefs())
        cache.update(cloud)
        cache.update(null)
        assertTrue(cache.isCloud)
    }

    @Test
    fun planCacheClearsWhenPlanIsFree() {
        val cache = PlanCache(FakePrefs())
        cache.update(cloud)
        cache.update(free)
        assertFalse(cache.isCloud)
    }

    @Test
    fun settingsDefaultsAreOn() {
        val s = ReminderSettingsStore(FakePrefs()).settings
        assertTrue(s.meetingEnabled)
        assertEquals(10, s.leadMinutes)
        assertTrue(s.digestEnabled)
        assertEquals(480, s.digestMinuteOfDay)
    }

    @Test
    fun settingsRoundTrip() {
        val prefs = FakePrefs()
        val changed = ReminderSettings(meetingEnabled = false, leadMinutes = 15, digestEnabled = false, digestMinuteOfDay = 500)
        ReminderSettingsStore(prefs).settings = changed
        assertEquals(changed, ReminderSettingsStore(prefs).settings)
    }

    @Test
    fun coordinatorClearsRemindersOffCloud() = runTest {
        val rig = Rig(cloudAtStart = false)
        rig.coordinator.reschedule()
        assertEquals(listOf(emptyList<PlannedReminder>()), rig.scheduler.replaced)
        assertEquals(0, rig.permissionAsks)
    }

    @Test
    fun coordinatorLeavesRemindersAloneWhenStoreIsNotOpen() = runTest {
        val rig = Rig(cloudAtStart = true)
        rig.coordinator.reschedule()
        assertTrue(rig.scheduler.replaced.isEmpty())
    }

    @Test
    fun coordinatorSchedulesPlannedRemindersOnCloud() = runTest {
        val event = PhoneCalendarEvent(externalId = "e1", source = "google_calendar", title = "Standup", startUnixMs = NOW + 3_600_000, attendees = emptyList(), rowId = 1)
        val rig = Rig(cloudAtStart = true, readReplica = { _, _ -> ReminderCoordinator.ReplicaRead(listOf(event), emptyList()) })
        rig.coordinator.reschedule()
        assertEquals(listOf("meeting:e1"), rig.scheduler.replaced.single().map { it.id })
        assertEquals("Standup in 10 minutes", rig.scheduler.replaced.single().single().title)
    }

    @Test
    fun failedPlanCheckKeepsCachedPlanAndSkipsThrottleStamp() = runTest {
        val rig = Rig(cloudAtStart = true, fetchPlan = { null })
        rig.coordinator.refreshPlanIfDue(force = true)
        assertTrue(rig.coordinator.isCloud.value)
        assertTrue(rig.cache.isCloud)
        assertNull(rig.cache.lastCheckedAtMs)
    }

    @Test
    fun recentCheckThrottlesNonForcedRefresh() = runTest {
        var fetches = 0
        val rig = Rig(cloudAtStart = false, fetchPlan = { fetches += 1; cloud })
        rig.cache.lastCheckedAtMs = NOW - 60_000
        rig.coordinator.refreshPlanIfDue(force = false)
        assertEquals(0, fetches)
        assertEquals(NOW - 60_000, rig.cache.lastCheckedAtMs)
        rig.coordinator.refreshPlanIfDue(force = true)
        assertEquals(1, fetches)
        assertEquals(NOW, rig.cache.lastCheckedAtMs)
    }

    @Test
    fun oldCheckIsDueAgain() = runTest {
        var fetches = 0
        val rig = Rig(cloudAtStart = false, fetchPlan = { fetches += 1; free })
        rig.cache.lastCheckedAtMs = NOW - 31 * 60_000
        rig.coordinator.refreshPlanIfDue(force = false)
        assertEquals(1, fetches)
    }

    @Test
    fun permissionAskedOnceWhenCloudInForeground() = runTest {
        val prefs = FakePrefs()
        val rig = Rig(cloudAtStart = false, prefs = prefs, fetchPlan = { cloud }, readReplica = { _, _ -> emptyRead })
        rig.coordinator.isForeground = true
        rig.coordinator.refreshPlanIfDue(force = true)
        rig.coordinator.refreshPlanIfDue(force = true)
        assertEquals(1, rig.permissionAsks)
        // A relaunch (new coordinator, same prefs) does not ask again.
        val again = Rig(cloudAtStart = true, prefs = prefs, fetchPlan = { cloud }, readReplica = { _, _ -> emptyRead })
        again.coordinator.isForeground = true
        again.coordinator.refreshPlanIfDue(force = true)
        assertEquals(0, again.permissionAsks)
    }

    @Test
    fun permissionDeferredUntilForeground() = runTest {
        val rig = Rig(cloudAtStart = false, fetchPlan = { cloud }, readReplica = { _, _ -> emptyRead })
        rig.coordinator.refreshPlanIfDue(force = true)
        assertEquals(0, rig.permissionAsks)
        rig.coordinator.isForeground = true
        assertEquals(1, rig.permissionAsks)
    }

    @Test
    fun permissionNotConsumedWhenNothingCouldAsk() = runTest {
        val rig = Rig(cloudAtStart = true, permissionHandled = false)
        rig.coordinator.isForeground = true
        assertEquals(1, rig.permissionAsks)
        assertFalse(rig.cache.permissionAsked)
        rig.permissionHandled = true
        rig.coordinator.isForeground = false
        rig.coordinator.isForeground = true
        assertEquals(2, rig.permissionAsks)
        assertTrue(rig.cache.permissionAsked)
    }

    @Test
    fun suppressedPromptLeavesFlagUnsetSoLaterForegroundAsksAgain() = runTest {
        // A free user's catch-up prompt was dismissed this process; the requester reports the ask as suppressed.
        val rig = Rig(cloudAtStart = false, fetchPlan = { cloud }, readReplica = { _, _ -> emptyRead }, permissionHandled = false)
        rig.coordinator.isForeground = true
        rig.coordinator.refreshPlanIfDue(force = true)
        assertEquals(1, rig.permissionAsks)
        assertFalse(rig.cache.permissionAsked)
        rig.permissionHandled = true
        rig.coordinator.isForeground = false
        rig.coordinator.isForeground = true
        assertEquals(2, rig.permissionAsks)
        assertTrue(rig.cache.permissionAsked)
    }

    @Test
    fun cloudToFreeClearsReminders() = runTest {
        val rig = Rig(cloudAtStart = true, fetchPlan = { free }, readReplica = { _, _ -> emptyRead })
        rig.coordinator.refreshPlanIfDue(force = true)
        assertFalse(rig.cache.isCloud)
        assertEquals(emptyList<PlannedReminder>(), rig.scheduler.replaced.last())
    }

    @Test
    fun overlappingPlanChecksMakeOneRequest() = runTest {
        var fetches = 0
        val gate = CompletableDeferred<Unit>()
        val rig = Rig(cloudAtStart = false, fetchPlan = { fetches += 1; gate.await(); free }, readReplica = { _, _ -> emptyRead })
        val first = launch { rig.coordinator.refreshPlanIfDue(force = false) }
        runCurrent()
        rig.coordinator.refreshPlanIfDue(force = false)
        rig.coordinator.refreshPlanIfDue(force = true)
        gate.complete(Unit)
        first.join()
        assertEquals(1, fetches)
    }

    @Test
    fun cancelledPlanCheckDoesNotBlockTheNext() = runTest {
        val gate = CompletableDeferred<Unit>()
        var fetches = 0
        val rig = Rig(cloudAtStart = false, fetchPlan = { fetches += 1; if (fetches == 1) gate.await(); free })
        val first = launch { rig.coordinator.refreshPlanIfDue(force = true) }
        runCurrent()
        first.cancel()
        runCurrent()
        rig.coordinator.refreshPlanIfDue(force = true)
        assertEquals(2, fetches)
    }

    @Test
    fun staleRescheduleDoesNotOverwriteNewer() = runTest {
        val gate = CompletableDeferred<Unit>()
        var calls = 0
        val rig = Rig(cloudAtStart = true, readReplica = { _, _ ->
            calls += 1
            if (calls == 1) gate.await()
            emptyRead
        })
        val older = launch { rig.coordinator.reschedule() }
        runCurrent()
        rig.coordinator.reschedule()
        assertEquals(1, rig.scheduler.replaced.size)
        gate.complete(Unit)
        older.join()
        assertEquals("the older run must not land after the newer one", 1, rig.scheduler.replaced.size)
    }

    @Test
    fun rescheduleDroppedWhenPlanFlipsToFreeMeanwhile() = runTest {
        val gate = CompletableDeferred<Unit>()
        val rig = Rig(cloudAtStart = true, fetchPlan = { free }, readReplica = { _, _ -> gate.await(); emptyRead })
        val older = launch { rig.coordinator.reschedule() }
        runCurrent()
        rig.coordinator.refreshPlanIfDue(force = true)
        assertEquals(listOf(emptyList<PlannedReminder>()), rig.scheduler.replaced)
        gate.complete(Unit)
        older.join()
        assertEquals("the free clear wins", listOf(emptyList<PlannedReminder>()), rig.scheduler.replaced)
    }

    @Test
    fun wipeClearsRemindersAndCancelsInFlightRuns() = runTest {
        val gate = CompletableDeferred<Unit>()
        val rig = Rig(cloudAtStart = true, readReplica = { _, _ -> gate.await(); emptyRead })
        val older = launch { rig.coordinator.reschedule() }
        runCurrent()
        rig.coordinator.clearOnWipe()
        assertEquals(listOf(emptyList<PlannedReminder>()), rig.scheduler.replaced)
        assertFalse(rig.cache.isCloud)
        gate.complete(Unit)
        older.join()
        assertEquals(1, rig.scheduler.replaced.size)
    }

    @Test
    fun demoCoordinatorNeverSchedulesOrAsks() = runTest {
        var fetches = 0
        val rig = Rig(cloudAtStart = true, fetchPlan = { fetches += 1; cloud }, readReplica = { _, _ -> emptyRead })
        rig.coordinator.isEnabled = false
        rig.coordinator.isForeground = true
        rig.coordinator.refreshPlanIfDue(force = true)
        rig.coordinator.reschedule()
        assertTrue(rig.scheduler.replaced.isEmpty())
        assertEquals(0, rig.permissionAsks)
        assertEquals(0, fetches)
    }

    @Test
    fun requestCodesAreStablePerReminderId() {
        assertEquals(ReminderRequestCodes.codeFor("digest"), ReminderRequestCodes.codeFor("digest"))
        assertEquals("digest".hashCode(), ReminderRequestCodes.codeFor("digest"))
        assertTrue(ReminderRequestCodes.codeFor("meeting:a") != ReminderRequestCodes.codeFor("meeting:b"))
    }

    @Test
    fun refreshRunsSyncThenPlanCheckThenReschedule() = runTest {
        val order = mutableListOf<String>()
        val rig = Rig(cloudAtStart = true, fetchPlan = { order += "plan"; cloud }, readReplica = { _, _ -> order += "read"; emptyRead })
        assertTrue(ReminderRefresh.run(rig.coordinator) { order += "sync" })
        assertEquals(listOf("sync", "plan", "read"), order)
    }

    @Test
    fun refreshReportsFailureInsteadOfThrowing() = runTest {
        val rig = Rig(cloudAtStart = true)
        assertFalse(ReminderRefresh.run(rig.coordinator) { throw IllegalStateException("boom") })
    }

    @Test
    fun refreshRespectsCancellation() = runTest {
        val rig = Rig(cloudAtStart = true)
        assertThrows(CancellationException::class.java) {
            kotlinx.coroutines.runBlocking { ReminderRefresh.run(rig.coordinator) { throw CancellationException("stopped") } }
        }
    }
}

class AlarmReminderSchedulerTest {
    private class RecordingAlarms(val delayMs: Long = 0) : AlarmBackend {
        val events = java.util.Collections.synchronizedList(mutableListOf<String>())
        override fun cancel(id: String) { events += "cancel:$id" }
        override fun set(reminder: PlannedReminder) { if (delayMs > 0) Thread.sleep(delayMs); events += "set:${reminder.id}" }
    }

    private fun reminder(id: String) = PlannedReminder(id, fireAtMs = 1, title = "t", body = null)

    @Test
    fun replaceAllCancelsPreviousIdsThenSetsThenSavesNewIds() {
        val prefs = FakePrefs()
        val alarms = RecordingAlarms()
        val scheduler = AlarmReminderScheduler(prefs, alarms)
        scheduler.replaceAll(listOf(reminder("a"), reminder("b")))
        scheduler.replaceAll(listOf(reminder("c")))
        assertEquals(listOf("set:a", "set:b", "cancel:a", "cancel:b", "set:c"), alarms.events)
        assertEquals(setOf("c"), AlarmReminderScheduler(prefs, RecordingAlarms()).scheduledIds())
    }

    @Test
    fun emptyListClearsEverything() {
        val prefs = FakePrefs()
        val alarms = RecordingAlarms()
        val scheduler = AlarmReminderScheduler(prefs, alarms)
        scheduler.replaceAll(listOf(reminder("a")))
        scheduler.replaceAll(emptyList())
        assertEquals(listOf("set:a", "cancel:a"), alarms.events)
        assertTrue(scheduler.scheduledIds().isEmpty())
    }

    @Test
    fun overlappingCallsDoNotInterleave() {
        val alarms = RecordingAlarms(delayMs = 5)
        val scheduler = AlarmReminderScheduler(FakePrefs(), alarms)
        val xs = listOf("x1", "x2", "x3").map(::reminder)
        val ys = listOf("y1", "y2", "y3").map(::reminder)
        val threads = listOf(Thread { scheduler.replaceAll(xs) }, Thread { scheduler.replaceAll(ys) })
        threads.forEach { it.start() }
        threads.forEach { it.join() }
        val sets = alarms.events.filter { it.startsWith("set:") }.map { it.removePrefix("set:") }
        assertTrue(sets.toString(), sets == listOf("x1", "x2", "x3", "y1", "y2", "y3") || sets == listOf("y1", "y2", "y3", "x1", "x2", "x3"))
        val saved = scheduler.scheduledIds()
        assertTrue(saved == setOf("x1", "x2", "x3") || saved == setOf("y1", "y2", "y3"))
        // The second call cancelled exactly what the first one set.
        val cancels = alarms.events.filter { it.startsWith("cancel:") }.map { it.removePrefix("cancel:") }.toSet()
        assertEquals(if (saved.first().startsWith("x")) setOf("y1", "y2", "y3") else setOf("x1", "x2", "x3"), cancels)
    }
}

class ReminderSettingsRowsTest {
    @Test
    fun offCloudShowsUpgradeLinesOnly() {
        val rows = ReminderSettingsRows.make(isCloud = false, settings = ReminderSettings(), denied = true)
        assertTrue(rows.showsUpgradeLines)
        assertFalse(rows.showsMeetingToggle || rows.showsLeadPicker || rows.showsDigestToggle || rows.showsDigestTime || rows.showsDeniedNotice || rows.showsFooter)
    }

    @Test
    fun leadPickerHiddenWhenMeetingRemindersOffAndTimeWhenDigestOff() {
        val rows = ReminderSettingsRows.make(true, ReminderSettings(meetingEnabled = false, digestEnabled = false), denied = false)
        assertFalse(rows.showsLeadPicker)
        assertFalse(rows.showsDigestTime)
        assertTrue(rows.showsMeetingToggle && rows.showsDigestToggle && rows.showsFooter)
        assertFalse(rows.showsUpgradeLines)
    }

    @Test
    fun deniedNoticeOnlyOnCloud() {
        assertTrue(ReminderSettingsRows.make(true, ReminderSettings(), denied = true).showsDeniedNotice)
    }
}
