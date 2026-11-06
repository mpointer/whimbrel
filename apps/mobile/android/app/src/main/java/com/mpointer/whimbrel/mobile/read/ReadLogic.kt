package com.mpointer.whimbrel.mobile.read

import android.content.Context
import android.text.format.DateUtils
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.sync.DeviceLabel
import java.time.ZoneId
import java.util.Formatter
import java.util.Locale

/** The texts [ReadLogic] needs: strings.xml in the app, a map in tests. */
interface ReadStrings {
    val now: String
    fun inMinutes(n: Int): String
    fun inHours(n: Int): String
    val syncing: String
    val notSyncedYet: String
    val syncedJustNow: String
    fun syncedMinutesAgo(n: Int): String
    fun syncedHoursAgo(n: Int): String

    /** "9:00 – 9:30 AM" (the phone's clock style), with the date in front when [withDate]; just the start with no end. */
    fun timeRange(startMs: Long, endMs: Long?, zone: ZoneId, withDate: Boolean): String

    /** "+2 copies": how many other recordings of the same call exist. */
    fun copies(n: Int): String

    /** What to call a device, its fallbacks resolved. */
    fun device(label: DeviceLabel): String
    /** "3 meetings". */
    fun meetings(n: Int): String
}

/** [ReadStrings] backed by the app's strings.xml. */
class ResourceReadStrings(private val context: Context) : ReadStrings {
    override val now get() = context.getString(R.string.read_now)
    override fun inMinutes(n: Int) = context.getString(R.string.read_in_minutes, n)
    override fun inHours(n: Int) = context.getString(R.string.read_in_hours, n)
    override val syncing get() = context.getString(R.string.read_syncing)
    override val notSyncedYet get() = context.getString(R.string.read_not_synced_yet)
    override val syncedJustNow get() = context.getString(R.string.read_synced_just_now)
    override fun syncedMinutesAgo(n: Int) = context.getString(R.string.read_synced_minutes_ago, n)
    override fun syncedHoursAgo(n: Int) = context.getString(R.string.read_synced_hours_ago, n)
    override fun timeRange(startMs: Long, endMs: Long?, zone: ZoneId, withDate: Boolean): String {
        val flags = DateUtils.FORMAT_SHOW_TIME or if (withDate) DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH else 0
        return DateUtils.formatDateRange(context, Formatter(StringBuilder(), Locale.getDefault()), startMs, endMs ?: startMs, flags, zone.id).toString()
    }
    override fun copies(n: Int) = context.resources.getQuantityString(R.plurals.meeting_copies, n, n)
    override fun meetings(n: Int) = context.resources.getQuantityString(R.plurals.settings_meetings_synced, n, n)
    override fun device(label: DeviceLabel) = label.localized { id, args -> context.getString(id, *args) }
}

/** Pure read-side text logic, a port of the Mac's ReadLogic.swift. */
object ReadLogic {
    /** "morning", "afternoon" or "evening" for a local hour (0-23); a key, not display text. */
    fun daypart(hour: Int): String = if (hour < 12) "morning" else if (hour < 17) "afternoon" else "evening"

    /** "A, B +3": the first two names, then how many more. */
    fun attendeeSummary(names: List<String>): String {
        val shown = names.take(2).joinToString(", ")
        return if (names.size > 2) "$shown +${names.size - 2}" else shown
    }

    const val COLLAPSED_ATTENDEES = 3

    /** How many names the collapsed meeting-page list hides. */
    fun hiddenAttendeeCount(names: List<String>): Int = maxOf(0, names.size - COLLAPSED_ATTENDEES)

    /** The meeting-page names: the first three when collapsed, all of them when [expanded]. */
    fun attendeeLine(names: List<String>, expanded: Boolean): String =
        (if (expanded) names else names.take(COLLAPSED_ATTENDEES)).joinToString(", ")

    /** The synced meeting count under the sync line; null (hidden) until it is known. */
    fun meetingCountLine(count: Int?, strings: ReadStrings): String? = count?.let(strings::meetings)

    /** The sync status line: an error wins, then Syncing, then how long ago the last pull was. */
    fun syncLine(lastSyncedAtMs: Long?, nowMs: Long, isSyncing: Boolean, error: String?, strings: ReadStrings): String {
        if (error != null) return error
        if (isSyncing) return strings.syncing
        if (lastSyncedAtMs == null) return strings.notSyncedYet
        val seconds = maxOf(0L, (nowMs - lastSyncedAtMs) / 1000)
        return when {
            seconds < 60 -> strings.syncedJustNow
            seconds < 3600 -> strings.syncedMinutesAgo((seconds / 60).toInt())
            else -> strings.syncedHoursAgo((seconds / 3600).toInt())
        }
    }

    /** "Now" while running, "In 9m" / "In 2h" up to 12 hours ahead, null otherwise (or with no known end). */
    fun relativeStart(startMs: Long, endMs: Long?, nowMs: Long, strings: ReadStrings): String? {
        if (startMs <= nowMs) return if (endMs != null && nowMs < endMs) strings.now else null
        val diff = startMs - nowMs
        if (diff > 12 * 3_600_000L) return null
        val minutes = ((diff + 59_999) / 60_000).toInt()
        return if (minutes >= 60) strings.inHours(minutes / 60) else strings.inMinutes(minutes)
    }
}
