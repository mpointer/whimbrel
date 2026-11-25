package com.mpointer.whimbrel.mobile.reminders

import android.content.SharedPreferences
import com.mpointer.whimbrel.mobile.read.PhoneCalendarEvent
import com.mpointer.whimbrel.mobile.read.PhoneTopicSnapshot
import com.mpointer.whimbrel.mobile.sync.EnrollmentModel
import com.mpointer.whimbrel.mobile.sync.EnrollmentStatus
import com.mpointer.whimbrel.mobile.sync.SyncModel
import java.time.ZoneId
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

// Turns synced data into scheduled reminders: stores the phone-local settings and hands
// ReminderPlanner's output to the scheduler. Reminders run on the phone, so every plan has them.
// The system scheduler (AlarmReminderScheduler) is handed in by WhimbrelApplication. Ported from Reminders.swift.

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

/**
 * What the reminders remember about the phone across launches: today only whether the notification
 * permission was asked. Prefs file "reminders"; the key is unchanged from when this also cached the plan.
 */
class ReminderStateStore(private val prefs: SharedPreferences) {
    /** The notification-permission prompt is asked once ever, so a relaunch doesn't repeat it. */
    var permissionAsked: Boolean
        get() = prefs.getBoolean(ASKED, false)
        set(value) { prefs.edit().putBoolean(ASKED, value).apply() }

    private companion object {
        const val ASKED = "permissionAsked"
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
    private val state: ReminderStateStore,
    private val text: ReminderText,
    /** Whether this phone is signed in and approved, so it has synced data to remind from. */
    private val isApproved: () -> Boolean,
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

    /** False in demo runs: nothing is scheduled or asked. */
    var isEnabled = true

    /**
     * Set from the activity's start and stop. The permission prompt is only ever shown in the foreground;
     * if the phone was approved in the background, it is asked when this becomes true.
     */
    var isForeground = false
        set(value) {
            val was = field
            field = value
            if (value && !was) askPermissionIfNeeded()
        }

    /** The prompt was shown once (or was not needed); the Settings denied notice reads it. */
    val permissionAsked: Boolean get() = state.permissionAsked

    /** Bumped by every [reschedule] and wipe; a run that finds it changed after its awaits is stale. */
    private var generation = 0

    /**
     * Once ever, in the foreground, on an approved phone. Called on each return to the foreground and after
     * each sync, so a phone approved while the app is open is asked too.
     */
    fun askPermissionIfNeeded() {
        if (!isEnabled || !isForeground || !isApproved() || state.permissionAsked) return
        if (requestPermission()) state.permissionAsked = true
    }

    /**
     * Plans from the replica's next 48 hours and replaces every pending reminder. On a phone that is not
     * approved it plans nothing (a wipe clears the pending set). With the store not open it leaves the pending
     * set as it is. Overlapping runs: only the newest may replace.
     */
    suspend fun reschedule() {
        if (!isEnabled || !isApproved()) return
        generation += 1
        val mine = generation
        val from = clock()
        val read = readReplica(from, from + HORIZON_MS) ?: return
        val planned = ReminderPlanner.plan(read.events, read.snapshots, settings.settings, nowMs = from, zone = zone(), text = text)
        if (!mayReplace(mine, generation, isApproved())) return
        scheduler.replaceAll(planned)
    }

    /** The phone was wiped or unenrolled: nothing stays pending and no run in flight may land. */
    fun clearOnWipe() {
        generation += 1
        scheduler.replaceAll(emptyList())
    }

    companion object {
        const val HORIZON_MS = 48L * 3600 * 1000

        /** The generation guard: a run may replace only if no newer run started and the phone is still approved. */
        fun mayReplace(mine: Int, current: Int, isApproved: Boolean) = mine == current && isApproved

        /** The app's wiring: approval from [enrollment], the replica from [sync]'s next 48 hours (up to 200 events). */
        fun create(
            sync: () -> SyncModel,
            enrollment: () -> EnrollmentModel,
            scheduler: ReminderScheduling,
            settings: ReminderSettingsStore,
            state: ReminderStateStore,
            text: ReminderText,
            requestPermission: () -> Boolean,
            clock: () -> Long = System::currentTimeMillis,
        ) = ReminderCoordinator(
            scheduler = scheduler,
            settings = settings,
            state = state,
            text = text,
            isApproved = { enrollment().status.value == EnrollmentStatus.Approved },
            readReplica = { fromMs, untilMs ->
                sync().read { ReplicaRead(it.upcomingEvents(fromMs, untilMs, 200), it.topicSnapshots()) }
            },
            requestPermission = requestPermission,
            clock = clock,
        )
    }
}

/** One background refresh: sync once, reschedule. */
object ReminderRefresh {
    /**
     * True when the run finished (or had nothing to do); false when something failed and WorkManager should
     * retry later. Never throws, but a cancellation is rethrown so the worker stops at once.
     */
    suspend fun run(coordinator: ReminderCoordinator, syncNow: suspend () -> Unit): Boolean = try {
        syncNow()
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
    val showsMeetingToggle: Boolean,
    val showsLeadPicker: Boolean,
    val showsDigestToggle: Boolean,
    val showsDigestTime: Boolean,
    val showsDeniedNotice: Boolean,
    val showsFooter: Boolean,
) {
    companion object {
        /** [denied] is notification permission refused. */
        fun make(settings: ReminderSettings, denied: Boolean) = ReminderSettingsRows(
            showsMeetingToggle = true,
            showsLeadPicker = settings.meetingEnabled,
            showsDigestToggle = true,
            showsDigestTime = settings.digestEnabled,
            showsDeniedNotice = denied,
            showsFooter = true,
        )
    }
}
