package com.mpointer.whimbrel.mobile.reminders

import com.mpointer.whimbrel.mobile.read.PhoneCalendarEvent
import com.mpointer.whimbrel.mobile.read.PhoneTopicSnapshot
import java.time.LocalDateTime
import java.time.ZoneId
import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull

class ReminderPlannerTest {
    private val zone = ZoneId.of("America/New_York")

    private object PlainText : ReminderText {
        override fun meetingIn(title: String, minutes: Int) = "$title in $minutes minutes"
        override fun withTopic(line: String, topic: String) = "$line · $topic"
        override fun stepsDue(count: Int) = if (count == 1) "1 step due today" else "$count steps due today"
    }

    /** A local time in the fixed zone, as epoch ms; `day` is the day of November 2026. */
    private fun at(hour: Int, minute: Int = 0, day: Int = 11): Long =
        LocalDateTime.of(2026, 11, day, hour, minute).atZone(zone).toInstant().toEpochMilli()

    private fun event(id: String, title: String, start: Long, rowId: Long? = null) =
        PhoneCalendarEvent(externalId = id, source = "google_calendar", title = title, startUnixMs = start, attendees = emptyList(), rowId = rowId)

    private fun snapshot(
        key: String = "topic-1",
        name: String = "Northwind",
        next: PhoneTopicSnapshot.NextMeeting? = null,
        owe: List<PhoneTopicSnapshot.Step> = emptyList(),
    ) = PhoneTopicSnapshot(topicKey = key, name = name, nextMeeting = next, youOwe = owe)

    private fun step(text: String, due: String?) = PhoneTopicSnapshot.Step(text = text, dueDate = due)

    private fun plan(
        events: List<PhoneCalendarEvent> = emptyList(),
        snapshots: List<PhoneTopicSnapshot> = emptyList(),
        settings: ReminderSettings = ReminderSettings(),
        isCloud: Boolean = true,
        now: Long = at(8, 30),
    ) = ReminderPlanner.plan(events, snapshots, settings, isCloud, now, zone, PlainText)

    @Test fun meetingReminderFiresLeadMinutesBefore() {
        val planned = plan(events = listOf(event("e1", "Northwind weekly", at(10))))
        assertEquals(1, planned.size)
        assertEquals("meeting:e1", planned[0].id)
        assertEquals(at(9, 50), planned[0].fireAtMs)
        assertEquals("Northwind weekly in 10 minutes", planned[0].title)
        assertNull(planned[0].body)
    }

    @Test fun meetingReminderAddsTopicName() {
        val next = PhoneTopicSnapshot.NextMeeting(title = "Northwind weekly", startUnixMs = at(10))
        val planned = plan(events = listOf(event("e1", "Northwind weekly", at(10))), snapshots = listOf(snapshot(next = next)))
        assertEquals("Northwind weekly in 10 minutes · Northwind", planned[0].title)
    }

    @Test fun meetingReminderIgnoresTopicWithDifferentStart() {
        val next = PhoneTopicSnapshot.NextMeeting(title = "Northwind weekly", startUnixMs = at(11))
        val planned = plan(events = listOf(event("e1", "Northwind weekly", at(10))), snapshots = listOf(snapshot(next = next)))
        assertEquals("Northwind weekly in 10 minutes", planned[0].title)
    }

    @Test fun dedupeKeepsNewestRowWithin24h() {
        val kept = ReminderPlanner.dedupe(listOf(
            event("a", "Standup", at(10), rowId = 1),
            event("b", "Standup", at(14), rowId = 2),
        ))
        assertEquals(listOf("b"), kept.map { it.externalId })
    }

    @Test fun dedupeKeepsDailyRepeats() {
        val kept = ReminderPlanner.dedupe(listOf(
            event("a", "Standup", at(10), rowId = 1),
            event("b", "Standup", at(10, day = 12), rowId = 2),
        ))
        assertEquals(setOf("a", "b"), kept.map { it.externalId }.toSet())
    }

    @Test fun dedupeTreatsMissingRowIdAsZeroAndKeepsInputOrderOnTies() {
        val kept = ReminderPlanner.dedupe(listOf(
            event("a", "Standup", at(10)),
            event("b", "Standup", at(11)),
            event("c", "Other", at(11), rowId = null),
        ))
        assertEquals(setOf("a", "c"), kept.map { it.externalId }.toSet())
    }

    @Test fun skipsPastFireTimes() {
        assertEquals(emptyList<PlannedReminder>(), plan(events = listOf(event("e1", "Soon", at(8, 35)))))
    }

    @Test fun capsMeetingReminders() {
        val events = (0 until 70).map { event("e$it", "Meeting $it", at(10) + it * 60_000L) }
        val planned = plan(events = events.reversed(), settings = ReminderSettings(digestEnabled = false))
        assertEquals(60, planned.size)
        assertEquals("meeting:e0", planned.first().id)
        assertEquals("meeting:e59", planned.last().id)
    }

    @Test fun digestListsDueAndOverdueSteps() {
        val snap = snapshot(owe = listOf(step("Send deck", "2026-11-11"), step("Call Sam", "2026-11-09"), step("Later", "2026-11-12")))
        val planned = plan(snapshots = listOf(snap), now = at(7))
        assertEquals(1, planned.size)
        assertEquals("digest", planned[0].id)
        assertEquals(at(8), planned[0].fireAtMs)
        assertEquals("2 steps due today", planned[0].title)
        assertEquals("Send deck\nCall Sam", planned[0].body)
    }

    @Test fun digestAfterTimeTargetsTomorrow() {
        val planned = plan(snapshots = listOf(snapshot(owe = listOf(step("Send deck", "2026-11-12")))))
        assertEquals(1, planned.size)
        assertEquals(at(8, day = 12), planned[0].fireAtMs)
    }

    @Test fun ignoresMalformedDueDates() {
        val snap = snapshot(owe = listOf(step("a", ""), step("b", "soon"), step("c", "2026-11"), step("d", null)))
        assertEquals(emptyList<PlannedReminder>(), plan(snapshots = listOf(snap)))
    }

    @Test fun singleStepUsesSingular() {
        val planned = plan(snapshots = listOf(snapshot(owe = listOf(step("Send deck", "2026-11-11")))), now = at(7))
        assertEquals("1 step due today", planned[0].title)
    }

    @Test fun nothingOffCloud() {
        val snap = snapshot(owe = listOf(step("Send deck", "2026-11-11")))
        assertEquals(emptyList<PlannedReminder>(), plan(events = listOf(event("e1", "Soon", at(10))), snapshots = listOf(snap), isCloud = false, now = at(7)))
    }

    @Test fun disabledKindsAreSkipped() {
        val events = listOf(event("e1", "Soon", at(10)))
        val snap = snapshot(owe = listOf(step("Send deck", "2026-11-11")))
        val noMeetings = plan(events = events, snapshots = listOf(snap), settings = ReminderSettings(meetingEnabled = false), now = at(7))
        assertEquals(listOf("digest"), noMeetings.map { it.id })
        val noDigest = plan(events = events, snapshots = listOf(snap), settings = ReminderSettings(digestEnabled = false), now = at(7))
        assertEquals(listOf("meeting:e1"), noDigest.map { it.id })
    }

    @Test fun meetingsComeBeforeDigest() {
        val snap = snapshot(owe = listOf(step("Send deck", "2026-11-12")))
        val planned = plan(events = listOf(event("e1", "Soon", at(10))), snapshots = listOf(snap))
        assertEquals(listOf("meeting:e1", "digest"), planned.map { it.id })
    }
}
