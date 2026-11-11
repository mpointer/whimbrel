package com.mpointer.whimbrel.mobile.reminders

import android.content.SharedPreferences
import com.mpointer.whimbrel.mobile.core.AccountPlan
import com.mpointer.whimbrel.mobile.read.PhoneCalendarEvent
import com.mpointer.whimbrel.mobile.read.PhoneTopicSnapshot
import com.mpointer.whimbrel.mobile.sync.EnrollmentModel
import com.mpointer.whimbrel.mobile.sync.SyncModel
import java.time.ZoneId
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

// Turns synced data into scheduled reminders: stores the phone-local settings, remembers whether
// the account is on Cloud, and hands ReminderPlanner's output to the scheduler. The system
// scheduler (AlarmReminderScheduler) is handed in by WhimbrelApplication. Ported from Reminders.swift.

/** The reminder settings, phone-local (never part of PhoneSettingsSync). Prefs file "reminders". */
class ReminderSettingsStore(private val prefs: SharedPreferences) {
    var settings: ReminderSettings
        get() {
            val fallback = ReminderSettings()
            return ReminderSettings(
                meetingEnabled = prefs.getBoolean(MEETING, fallback.meetingEnabled),
                leadMinutes = prefs.getInt(LEAD, fallback.leadMinutes),
                digestEnabled = prefs.getBoolean(DIGEST, fallback.digestEnabled),
                digestMinuteOfDay = prefs.getInt(DIGEST_MINUTE, fallback.digestMinuteOfDay),
            )
        }
        set(value) {
            prefs.edit()
                .putBoolean(MEETING, value.meetingEnabled)
                .putInt(LEAD, value.leadMinutes)
                .putBoolean(DIGEST, value.digestEnabled)
                .putInt(DIGEST_MINUTE, value.digestMinuteOfDay)
                .apply()
        }

    private companion object {
        const val MEETING = "meetingEnabled"
        const val LEAD = "leadMinutes"
        const val DIGEST = "digestEnabled"
        const val DIGEST_MINUTE = "digestMinuteOfDay"
    }
}

/** The last plan answer the Worker gave, kept so reminders still work offline. Prefs file "reminders". */
class PlanCache(private val prefs: SharedPreferences) {
    val isCloud: Boolean get() = prefs.getBoolean(CLOUD, false)

    /** The notification-permission prompt is asked once ever, so a relaunch doesn't repeat it. */
    var permissionAsked: Boolean
        get() = prefs.getBoolean(ASKED, false)
        set(value) { prefs.edit().putBoolean(ASKED, value).apply() }

    /** Null (a failed check) keeps the cached value; a plan replaces it. */
    fun update(plan: AccountPlan?) {
        if (plan == null) return
        prefs.edit().putBoolean(CLOUD, plan.isCloud).apply()
    }

    /** When the last successful check ran (epoch ms); the 30-minute throttle reads it. */
    var lastCheckedAtMs: Long?
        get() = if (prefs.contains(CHECKED)) prefs.getLong(CHECKED, 0) else null
        set(value) {
            val e = prefs.edit()
            if (value == null) e.remove(CHECKED) else e.putLong(CHECKED, value)
            e.apply()
        }

    /** After a wipe: not Cloud, never checked. The permission flag stays (it is about the phone, not the account). */
    fun reset() {
        prefs.edit().putBoolean(CLOUD, false).remove(CHECKED).apply()
    }

    private companion object {
        const val CLOUD = "isCloud"
        const val ASKED = "permissionAsked"
        const val CHECKED = "planCheckedAt"
    }
}

/** The system's alarm store, faked in tests. */
interface ReminderScheduling {
    /** Makes exactly [reminders] the pending set (an empty list clears it). Safe to call from any thread. */
    fun replaceAll(reminders: List<PlannedReminder>)
}

/**
 * Not thread-safe except [AlarmReminderScheduler]: call from the main thread (the app scope, or the
 * refresh worker switched to Main), as for [SyncModel].
 */
class ReminderCoordinator(
    private val scheduler: ReminderScheduling,
    /** Shared with the Settings screen, which writes it and then calls [reschedule]. */
    val settings: ReminderSettingsStore,
    private val plan: PlanCache,
    private val text: ReminderText,
    private val fetchPlan: suspend () -> AccountPlan?,
    private val readReplica: suspend (fromMs: Long, untilMs: Long) -> ReplicaRead?,
    /**
     * Shows the notification-permission prompt. True when the question is settled (asked now, already
     * granted, or not needed on this Android version); false when nothing could ask (no screen), so the
     * next foreground tries again.
     */
    private val requestPermission: () -> Boolean,
    private val clock: () -> Long = System::currentTimeMillis,
    private val zone: () -> ZoneId = { ZoneId.systemDefault() },
) {
    /** What one read of the replica returns. */
    data class ReplicaRead(val events: List<PhoneCalendarEvent>, val snapshots: List<PhoneTopicSnapshot>)

    private val _isCloud = MutableStateFlow(plan.isCloud)

    /** Cloud or not, as last known (the cache until a check answers). */
    val isCloud: StateFlow<Boolean> = _isCloud.asStateFlow()

    /** False in demo runs: nothing is checked, scheduled or asked, whatever the cached plan says. */
    var isEnabled = true

    /**
     * Set from the activity's start and stop. The permission prompt is only ever shown in the foreground;
     * if Cloud turned on in the background, it is asked when this becomes true.
     */
    var isForeground = false
        set(value) {
            val was = field
            field = value
            if (value && !was) askPermissionIfNeeded()
        }

    /** The prompt was shown once (or was not needed); the Settings denied notice reads it. */
    val permissionAsked: Boolean get() = plan.permissionAsked

    private var checkingPlan = false

    /** Bumped by every [reschedule] and wipe; a run that finds it changed after its awaits is stale. */
    private var generation = 0

    /**
     * Asks the Worker for the plan: always with [force] (launch), otherwise at most every 30 minutes. A check
     * already in flight absorbs overlapping calls. A failed check changes nothing and is not counted, so the
     * next sync retries. Reschedules when the answer flips Cloud on or off.
     */
    suspend fun refreshPlanIfDue(force: Boolean) {
        if (!isEnabled || checkingPlan) return
        val current = clock()
        val last = plan.lastCheckedAtMs
        if (!force && last != null && current >= last && current - last < PLAN_CHECK_INTERVAL_MS) return
        checkingPlan = true
        val answer = try {
            fetchPlan()
        } finally {
            checkingPlan = false
        }
        if (answer == null) return
        plan.update(answer)
        plan.lastCheckedAtMs = clock()
        val changed = plan.isCloud != _isCloud.value
        _isCloud.value = plan.isCloud
        if (changed) reschedule()
        askPermissionIfNeeded()
    }

    /** Once ever, in the foreground, on Cloud. */
    private fun askPermissionIfNeeded() {
        if (!isEnabled || !isForeground || !_isCloud.value || plan.permissionAsked) return
        if (requestPermission()) plan.permissionAsked = true
    }

    /**
     * Plans from the replica's next 48 hours and replaces every pending reminder. Off Cloud it clears them.
     * With the store not open it leaves the pending set as it is. Overlapping runs: only the newest may
     * replace, and a run whose plan changed meanwhile drops out.
     */
    suspend fun reschedule() {
        if (!isEnabled) return
        generation += 1
        val mine = generation
        val cloud = _isCloud.value
        if (!cloud) {
            scheduler.replaceAll(emptyList())
            return
        }
        val from = clock()
        val read = readReplica(from, from + HORIZON_MS) ?: return
        val planned = ReminderPlanner.plan(read.events, read.snapshots, settings.settings, isCloud = true, nowMs = from, zone = zone(), text = text)
        if (!mayReplace(mine, generation, cloud, _isCloud.value)) return
        scheduler.replaceAll(planned)
    }

    /** The phone was wiped or unenrolled: nothing stays pending, no run in flight may land, and the plan is forgotten. */
    fun clearOnWipe() {
        generation += 1
        plan.reset()
        _isCloud.value = false
        scheduler.replaceAll(emptyList())
    }

    companion object {
        /** Plan checks after a sync wait at least this long since the last good one. */
        const val PLAN_CHECK_INTERVAL_MS = 30L * 60 * 1000
        const val HORIZON_MS = 48L * 3600 * 1000

        /** The generation guard: a run may replace only if no newer run started and the plan did not change. */
        fun mayReplace(mine: Int, current: Int, plannedAsCloud: Boolean, isCloud: Boolean) = mine == current && plannedAsCloud == isCloud

        /** The app's wiring: the plan from [enrollment], the replica from [sync]'s next 48 hours (up to 200 events). */
        fun create(
            sync: () -> SyncModel,
            enrollment: () -> EnrollmentModel,
            scheduler: ReminderScheduling,
            settings: ReminderSettingsStore,
            plan: PlanCache,
            text: ReminderText,
            requestPermission: () -> Boolean,
            clock: () -> Long = System::currentTimeMillis,
        ) = ReminderCoordinator(
            scheduler = scheduler,
            settings = settings,
            plan = plan,
            text = text,
            fetchPlan = { enrollment().fetchAccountPlan() },
            readReplica = { fromMs, untilMs ->
                sync().read { ReplicaRead(it.upcomingEvents(fromMs, untilMs, 200), it.topicSnapshots()) }
            },
            requestPermission = requestPermission,
            clock = clock,
        )
    }
}

/** One background refresh: sync once, check the plan if due, reschedule. */
object ReminderRefresh {
    /**
     * True when the run finished (or had nothing to do); false when something failed and WorkManager should
     * retry later. Never throws, but a cancellation is rethrown so the worker stops at once.
     */
    suspend fun run(coordinator: ReminderCoordinator, syncNow: suspend () -> Unit): Boolean = try {
        syncNow()
        currentCoroutineContext().ensureActive()
        coordinator.refreshPlanIfDue(force = false)
        currentCoroutineContext().ensureActive()
        coordinator.reschedule()
        true
    } catch (e: CancellationException) {
        throw e
    } catch (_: Exception) {
        false
    }
}

/** Which rows the Settings Reminders section shows. */
data class ReminderSettingsRows(
    val showsUpgradeLines: Boolean,
    val showsMeetingToggle: Boolean,
    val showsLeadPicker: Boolean,
    val showsDigestToggle: Boolean,
    val showsDigestTime: Boolean,
    val showsDeniedNotice: Boolean,
    val showsFooter: Boolean,
) {
    companion object {
        /** Off Cloud only the two upgrade lines show. [denied] is notification permission refused. */
        fun make(isCloud: Boolean, settings: ReminderSettings, denied: Boolean) = ReminderSettingsRows(
            showsUpgradeLines = !isCloud,
            showsMeetingToggle = isCloud,
            showsLeadPicker = isCloud && settings.meetingEnabled,
            showsDigestToggle = isCloud,
            showsDigestTime = isCloud && settings.digestEnabled,
            showsDeniedNotice = isCloud && denied,
            showsFooter = isCloud,
        )
    }
}
