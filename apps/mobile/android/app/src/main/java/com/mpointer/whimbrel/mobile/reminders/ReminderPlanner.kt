package com.mpointer.whimbrel.mobile.reminders

import com.mpointer.whimbrel.mobile.read.PhoneCalendarEvent
import com.mpointer.whimbrel.mobile.read.PhoneTopicSnapshot
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime
import kotlin.math.abs

// Decides which local notifications the phone should have pending: a reminder before each
// upcoming meeting and one morning digest of due steps. Pure: the caller hands in what it read
// from the replica and schedules the result. Every reminder opens Today until the Android
// Topics tab lands (#644), so there is no route.

data class ReminderSettings(
    val meetingEnabled: Boolean = true,
    /** Minutes before the start; 5, 10 or 15. */
    val leadMinutes: Int = 10,
    val digestEnabled: Boolean = true,
    /** Minutes after local midnight; 480 is 8:00. */
    val digestMinuteOfDay: Int = 480,
)

data class PlannedReminder(
    val id: String,
    val fireAtMs: Long,
    val title: String,
    val body: String?,
)

/** The copy, so the app reads it from strings.xml and tests use plain English. */
interface ReminderText {
    fun meetingIn(title: String, minutes: Int): String
    fun withTopic(line: String, topic: String): String
    fun stepsDue(count: Int): String
}

object ReminderPlanner {
    /** The system keeps a limited number of pending alarms; one is the digest. */
    const val MAX_MEETING_REMINDERS = 60

    private const val DEDUPE_WINDOW_MS = 24L * 60 * 60 * 1000
    private val DAY_STRING = Regex("[0-9]{4}-[0-9]{2}-[0-9]{2}")

    fun plan(
        events: List<PhoneCalendarEvent>,
        snapshots: List<PhoneTopicSnapshot>,
        settings: ReminderSettings,
        isCloud: Boolean,
        nowMs: Long,
        zone: ZoneId,
        text: ReminderText,
    ): List<PlannedReminder> {
        if (!isCloud) return emptyList()
        val planned = mutableListOf<PlannedReminder>()
        if (settings.meetingEnabled) planned += meetingReminders(events, snapshots, settings, nowMs, text)
        if (settings.digestEnabled) digestReminder(snapshots, settings, nowMs, zone, text)?.let { planned += it }
        return planned
    }

    /**
     * One event per title within a 24-hour window: the newest row wins (null row id counts as 0;
     * ties keep input order). The result keeps the input order.
     */
    fun dedupe(events: List<PhoneCalendarEvent>): List<PhoneCalendarEvent> {
        val newestFirst = events.withIndex().sortedByDescending { it.value.rowId ?: 0L }
        val kept = mutableListOf<IndexedValue<PhoneCalendarEvent>>()
        for (candidate in newestFirst) {
            val duplicate = kept.any {
                it.value.title == candidate.value.title &&
                    abs(it.value.startUnixMs - candidate.value.startUnixMs) < DEDUPE_WINDOW_MS
            }
            if (!duplicate) kept += candidate
        }
        return kept.sortedBy { it.index }.map { it.value }
    }

    private fun meetingReminders(
        events: List<PhoneCalendarEvent>,
        snapshots: List<PhoneTopicSnapshot>,
        settings: ReminderSettings,
        nowMs: Long,
        text: ReminderText,
    ): List<PlannedReminder> {
        val reminders = dedupe(events).mapNotNull { event ->
            val fireAt = event.startUnixMs - settings.leadMinutes * 60_000L
            if (fireAt <= nowMs) return@mapNotNull null
            val topic = snapshots.firstOrNull {
                it.nextMeeting?.title == event.title && it.nextMeeting.startUnixMs == event.startUnixMs
            }
            var title = text.meetingIn(event.title, settings.leadMinutes)
            if (topic != null) title = text.withTopic(title, topic.name)
            PlannedReminder(id = "meeting:${event.externalId}", fireAtMs = fireAt, title = title, body = null)
        }
        return reminders.sortedBy { it.fireAtMs }.take(MAX_MEETING_REMINDERS)
    }

    private fun digestReminder(
        snapshots: List<PhoneTopicSnapshot>,
        settings: ReminderSettings,
        nowMs: Long,
        zone: ZoneId,
        text: ReminderText,
    ): PlannedReminder? {
        val fireAt = nextOccurrence(settings.digestMinuteOfDay, nowMs, zone)
        val today = fireAt.toLocalDate().toString()
        val due = snapshots.flatMap { it.youOwe }.filter { step ->
            val dueDate = step.dueDate
            dueDate != null && DAY_STRING.matches(dueDate) && dueDate <= today
        }
        if (due.isEmpty()) return null
        return PlannedReminder(
            id = "digest",
            fireAtMs = fireAt.toInstant().toEpochMilli(),
            title = text.stepsDue(due.size),
            body = due.joinToString("\n") { it.text },
        )
    }

    /** The first local `minuteOfDay` strictly after now. A time skipped by DST moves forward. */
    private fun nextOccurrence(minuteOfDay: Int, nowMs: Long, zone: ZoneId): ZonedDateTime {
        val now = Instant.ofEpochMilli(nowMs).atZone(zone)
        val time = LocalTime.of(minuteOfDay / 60, minuteOfDay % 60)
        var date: LocalDate = now.toLocalDate()
        var candidate = ZonedDateTime.of(date, time, zone)
        while (!candidate.toInstant().isAfter(now.toInstant())) {
            date = date.plusDays(1)
            candidate = ZonedDateTime.of(date, time, zone)
        }
        return candidate
    }
}
