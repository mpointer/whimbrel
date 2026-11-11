package com.mpointer.whimbrel.mobile.reminders

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.net.Uri
import android.os.Build

/** One alarm per reminder; the system side of [AlarmReminderScheduler], faked in tests. */
interface AlarmBackend {
    fun set(reminder: PlannedReminder)
    fun cancel(id: String)
}

object ReminderRequestCodes {
    /** The PendingIntent request code for a reminder: stable for the id, so a later call can cancel what an earlier one set. */
    fun codeFor(id: String): Int = id.hashCode()
}

/**
 * Keeps the reminders as AlarmManager alarms. [replaceAll] is serialized: it cancels the alarms whose ids
 * the last call saved, sets the new ones, then saves their ids, so two overlapping calls can never leave a
 * mix of both sets. The ids live in the "reminders" prefs because a reboot or an app update clears the
 * alarms but not the memory of them, and a stale id only cancels an alarm that is not there.
 */
class AlarmReminderScheduler(private val prefs: SharedPreferences, private val alarms: AlarmBackend) : ReminderScheduling {
    @Synchronized
    override fun replaceAll(reminders: List<PlannedReminder>) {
        scheduledIds().forEach { runCatching { alarms.cancel(it) } }
        val set = mutableSetOf<String>()
        for (reminder in reminders) {
            if (runCatching { alarms.set(reminder) }.isSuccess) set += reminder.id
        }
        prefs.edit().putStringSet(IDS, set).commit()
    }

    @Synchronized
    fun scheduledIds(): Set<String> = prefs.getStringSet(IDS, emptySet()).orEmpty().toSet()

    private companion object {
        const val IDS = "scheduledIds"
    }
}

/** How a reminder alarm is set: at the exact time, or inside a window that ends at it. */
enum class AlarmMode { Exact, Window }

/** How early a window alarm may fire. It ends at the fire time, so a reminder is never late; at worst it is this early. */
const val ALARM_WINDOW_MS = 10 * 60_000L

fun windowStart(fireAtMs: Long): Long = fireAtMs - ALARM_WINDOW_MS

/** Below API 31 exact alarms need no permission; from 31 they need SCHEDULE_EXACT_ALARM, which the user can turn off. */
fun alarmMode(sdk: Int, canScheduleExact: Boolean): AlarmMode =
    if (sdk < Build.VERSION_CODES.S || canScheduleExact) AlarmMode.Exact else AlarmMode.Window

/**
 * The real alarms. Exact (and Doze-proof) when the system allows it, because an inexact alarm set hours ahead
 * can be deferred by up to an hour on Android 12+, which would put a reminder after its meeting. Otherwise a
 * window that ends at the fire time. The app never asks the user for the exact-alarm permission.
 */
class SystemAlarmBackend(private val context: Context) : AlarmBackend {
    private val manager get() = context.getSystemService(AlarmManager::class.java)

    override fun set(reminder: PlannedReminder) {
        val intent = intentFor(context, reminder.id)
            .putExtra(ReminderReceiver.EXTRA_TITLE, reminder.title)
            .putExtra(ReminderReceiver.EXTRA_BODY, reminder.body)
        val pending = pending(context, reminder.id, intent)
        val canExact = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && manager.canScheduleExactAlarms()
        if (alarmMode(Build.VERSION.SDK_INT, canExact) == AlarmMode.Exact) {
            try {
                manager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, reminder.fireAtMs, pending)
                return
            } catch (_: SecurityException) {
                // The permission was revoked between the check and the call: fall back to the window.
            }
        }
        manager.setWindow(AlarmManager.RTC_WAKEUP, windowStart(reminder.fireAtMs), ALARM_WINDOW_MS, pending)
    }

    override fun cancel(id: String) {
        manager.cancel(pending(context, id, intentFor(context, id)))
    }

    private companion object {
        /** Intents match on action, data and component, not extras, so the id goes in the data to keep each alarm its own. */
        fun intentFor(context: Context, id: String) =
            Intent(context, ReminderReceiver::class.java).setAction(ReminderReceiver.ACTION).setData(Uri.fromParts("whimbrel-reminder", id, null))

        fun pending(context: Context, id: String, intent: Intent): PendingIntent =
            PendingIntent.getBroadcast(context, ReminderRequestCodes.codeFor(id), intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }
}
