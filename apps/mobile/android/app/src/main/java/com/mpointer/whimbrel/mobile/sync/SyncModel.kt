package com.mpointer.whimbrel.mobile.sync

import android.util.Log
import androidx.annotation.StringRes
import com.mpointer.whimbrel.mobile.BuildConfig
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.read.PhoneReadStore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Why the last round failed; the UI shows [resId]. The core's own cause is never shown. */
enum class SyncError(@StringRes val resId: Int) {
    SyncFailed(R.string.sync_error_failed),
    Unreachable(R.string.sync_error_unreachable),
    StorageUnavailable(R.string.sync_error_storage_unavailable),
}

/**
 * The phone's pull-only content sync, ported from `SyncModel.swift`: once approved and in the foreground,
 * pulls now and then every minute, backing off on failures, and shows the synced meeting count. The shell
 * calls [stop] when the app leaves the foreground.
 *
 * Not thread-safe: call it from the main thread, with [scope] on the main dispatcher. Every store call runs
 * on [storeDispatcher], which must be a single thread: it serializes the handle, and orders a reset's
 * close and directory removal before any re-open.
 */
class SyncModel(
    openStore: () -> MeetingSyncStore?,
    private val credentials: SyncCredentialSource,
    private val scope: CoroutineScope,
    private val storeDispatcher: CoroutineDispatcher,
    private val sleep: suspend (Long) -> Unit,
    private val clock: () -> Long,
    private val settingsPull: ((SyncCredentials, Int) -> Unit)? = null,
    private val wipeEpoch: () -> Int = { 0 },
    /** The core's failure cause can name the database host or carry a token: debug builds log it whole;
     *  release builds log it with URLs and long token-like strings removed, so a pull failure on a real
     *  phone is still visible (A3 gate, 2026-11-04). Never shown in the UI. */
    private val log: (String) -> Unit = { Log.w("content-sync", if (BuildConfig.DEBUG) it else redactForLog(it)) },
) {
    private val _meetingCount = MutableStateFlow<Int?>(null)
    val meetingCount: StateFlow<Int?> = _meetingCount.asStateFlow()
    private val _isSyncing = MutableStateFlow(false)
    val isSyncing: StateFlow<Boolean> = _isSyncing.asStateFlow()
    private val _lastSyncedAtMs = MutableStateFlow<Long?>(null)
    val lastSyncedAtMs: StateFlow<Long?> = _lastSyncedAtMs.asStateFlow()
    private val _lastError = MutableStateFlow<SyncError?>(null)
    val lastError: StateFlow<SyncError?> = _lastError.asStateFlow()
    private val _readRevision = MutableStateFlow(0)
    private val _catchingUp = MutableStateFlow(false)

    /** The last round succeeded and left rows on the relay: the phone is behind and pulling at the catch-up pace. */
    val catchingUp: StateFlow<Boolean> = _catchingUp.asStateFlow()

    /** Bumped when the store opens, after every successful pull and on reset, so screens reload what they read. */
    val readRevision: StateFlow<Int> = _readRevision.asStateFlow()

    companion object {
        /** Screen captures older than this aren't pulled. */
        const val captureWindowDays = 30
        /** Good rounds between pull-stats log lines. */
        const val STATS_LOG_EVERY = 10
    }

    /** Set once opened (null too if opening failed); [openStore] runs once, on the store thread. */
    private var store: MeetingSyncStore? = null
    private var openStore: (() -> MeetingSyncStore?)? = openStore
    private val reopenStore = openStore

    /** A reset asked for while a round was in flight: runs when that round ends. */
    private var pendingReset: (() -> Unit)? = null
    private var roundsLogged = 0
    private var consecutiveFailures = 0

    /** The last good round left rows on the relay (the core caps a round's page and time per peer). */
    private var moreToPull = false

    /** The running cadence loop, if any. Exposed so tests can join it. */
    var loopJob: Job? = null
        private set

    fun start() {
        if (loopJob != null) return
        loopJob = scope.launch {
            while (isActive) {
                try {
                    syncNow()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    // A bug in one round must not end syncing for the session.
                    log("sync round threw: $e")
                }
                try {
                    sleep(SyncTiming.nextDelaySeconds(consecutiveFailures, moreToPull))
                } catch (_: CancellationException) {
                    break
                }
            }
            // A cancelled loop was cleared by stop(), and a new one may have started since.
            if (isActive) loopJob = null
        }
    }

    fun stop() {
        loopJob?.cancel()
        loopJob = null
    }

    /**
     * The phone was removed from the account: stop, close the store and forget what was shown; the next
     * sync opens a fresh store. [then] runs on the store thread once the handle is closed (delete the
     * directory there). Never waits: a revoke is usually found by a round's own `renewIfDue`, so during a
     * round the reset is deferred to that round's end, and the round writes nothing more.
     */
    fun reset(then: () -> Unit) {
        stop()
        if (_isSyncing.value) {
            pendingReset = then
            return
        }
        val old = store
        store = null
        openStore = reopenStore
        _meetingCount.value = null
        _lastSyncedAtMs.value = null
        _lastError.value = null
        consecutiveFailures = 0
        moreToPull = false
        _catchingUp.value = false
        _readRevision.value += 1
        scope.launch(storeDispatcher) {
            runCatching { old?.close() }
            then()
        }
    }

    /** One pull round. [manual] (the Sync now button) may re-authenticate inside the automatic retry floor. */
    suspend fun syncNow(manual: Boolean = false) {
        if (_isSyncing.value) return
        _isSyncing.value = true
        try {
            round(manual)
        } finally {
            _isSyncing.value = false
            _catchingUp.value = moreToPull && consecutiveFailures == 0
            pendingReset?.let {
                pendingReset = null
                reset(it)
            }
        }
    }

    private suspend fun round(manual: Boolean) {
        openStore?.let { open ->
            openStore = null
            // Committed inside NonCancellable: a stop() or reset() during the open must not drop the handle
            // (it would leak, and with openStore spent every later round would say "unavailable").
            withContext(NonCancellable) {
                store = withContext(storeDispatcher) { open() }
                // Screens that asked before the store existed got nothing: have them read again.
                _readRevision.value += 1
            }
            currentCoroutineContext().ensureActive()
        }
        if (pendingReset != null) return
        val store = store ?: run {
            _lastError.value = SyncError.StorageUnavailable
            return
        }
        if (_meetingCount.value == null) {
            val count = onStore { store.groupedMeetingCount() }
            if (pendingReset != null) return
            _meetingCount.value = count
        }

        credentials.renewIfDue(afterFailure = false, force = manual)
        if (pendingReset != null) return
        // Read together: the settings pull must use the epoch these credentials belong to.
        val current = credentials.syncCredentials()
        val epoch = wipeEpoch()
        if (current == null) {
            consecutiveFailures += 1
            _lastError.value = SyncError.Unreachable
            return
        }
        val (ok, stats) = onStore { store.pullOnly(current, captureWindowDays) to store.lastPullStats() }
        // #572 evidence: per-peer cursor and row counts, release builds too. Every 10th round and every
        // failed one, so a catch-up (a round a second) doesn't flood logcat.
        if (!ok || roundsLogged++ % STATS_LOG_EVERY == 0) stats?.let { log("pull stats: $it") }
        if (pendingReset != null) return
        if (ok) {
            consecutiveFailures = 0
            moreToPull = onStore { store.moreToPull() }
            if (pendingReset != null) return
            _lastError.value = null
            _lastSyncedAtMs.value = clock()
            _readRevision.value += 1
            val count = onStore { store.groupedMeetingCount() }
            if (pendingReset != null) return
            _meetingCount.value = count ?: _meetingCount.value
            // Keys and routing for Ask: a network round trip off the store thread. Never fails the round.
            settingsPull?.let { pull ->
                try {
                    withContext(Dispatchers.IO) { pull(current, epoch) }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                }
            }
        } else {
            consecutiveFailures += 1
            moreToPull = false
            val cause = onStore { store.lastSyncError() } ?: "unknown"
            log("pull failed ($consecutiveFailures in a row): $cause")
            if (pendingReset != null) return
            _lastError.value = SyncError.SyncFailed
            credentials.renewIfDue(afterFailure = true, force = false)
        }
    }

    /** Reads the replica for a screen on the store thread. Null when the store is not open (not yet synced, or wiped). */
    suspend fun <T> read(body: (PhoneReadStore) -> T): T? {
        val store = store ?: return null
        return withContext(storeDispatcher) { body(store) }
    }

    /**
     * A blocking store call on the store thread; the caller waits it out even when cancelled (the call can't be
     * interrupted), then a cancelled caller (stop) gets CancellationException and the result is dropped.
     */
    private suspend fun <T> onStore(body: () -> T): T =
        withContext(NonCancellable) { withContext(storeDispatcher) { body() } }.also { currentCoroutineContext().ensureActive() }
}

/** Removes URLs (hosts name the account's database) and long token-like runs from a log line. */
internal fun redactForLog(message: String): String =
    message.replace(Regex("""[a-zA-Z][a-zA-Z0-9+.-]*://\S+"""), "<url>")
        .replace(Regex("""[A-Za-z0-9_\-.]{32,}"""), "<redacted>")
