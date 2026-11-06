package com.mpointer.whimbrel.mobile.read

import java.time.ZoneId
import java.time.ZoneOffset
import java.time.ZonedDateTime
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TodayModelTest {
    private val zone: ZoneId = ZoneOffset.UTC
    private val now = ZonedDateTime.of(2026, 10, 19, 9, 0, 0, 0, zone).toInstant().toEpochMilli()
    private fun ago(seconds: Long) = now - seconds * 1000

    private fun run(id: Long, name: String, status: String, at: Long) = PhoneRoutineRun(id, name, at, status, "out $id")

    @Test
    fun briefRunPicksTheNewestSuccessfulDailyBriefOfToday() {
        val runs = listOf(
            run(1, "Daily Brief", "success", ago(3600)),
            run(2, "Daily Brief", "success", ago(600)),
            run(3, "Daily Brief", "failed", ago(60)),
            run(4, "Meeting Prep", "success", ago(30)),
            run(5, "Daily Brief", "success", ago(86_400)),
        )
        assertEquals(2L, TodayModel.briefRun(runs, now, zone)?.id)
        assertNull(TodayModel.briefRun(listOf(runs[4]), now, zone))
    }

    @Test
    fun reloadFillsUpNextPrepAndRecentFromTheStore() = runTest {
        val store = ScriptedStore().apply {
            meetings = (1..12).map { PhoneMeeting(it.toLong(), "d", now - it * 3_600_000L, title = "M$it") }
            events = listOf(
                PhoneCalendarEvent("later", "google_calendar", "Later", now + 7_200_000, emptyList()),
                PhoneCalendarEvent("soon", "google_calendar", "Soon", now + 1_800_000, listOf("a@x.com")),
                PhoneCalendarEvent("tomorrow", "google_calendar", "Tomorrow", now + 86_400_000, emptyList()),
            )
            preps["soon"] = PhoneMeetingPrep("soon", "Soon", 1, "{}")
            runs = listOf(run(7, "Daily Brief", "success", ago(60)))
        }
        val model = TodayModel()
        assertTrue(!model.loaded.value)
        model.reload(syncWith(store, backgroundScope), now, zone)
        assertEquals("the soonest event today; tomorrow's is not Up next", "soon", model.upNext.value?.externalId)
        assertEquals("soon", model.prep.value?.eventId)
        assertEquals(10, model.recent.value.size)
        assertEquals(7L, model.brief.value?.id)
        assertTrue(model.loaded.value)
    }

    @Test
    fun anEventThatStartedFifteenMinutesAgoStillCountsAsNext() = runTest {
        val store = ScriptedStore().apply {
            events = listOf(
                PhoneCalendarEvent("old", "google_calendar", "Old", now - 16 * 60_000, emptyList()),
                PhoneCalendarEvent("just", "google_calendar", "Just started", now - 14 * 60_000, emptyList()),
            )
        }
        val model = TodayModel()
        model.reload(syncWith(store, backgroundScope), now, zone)
        assertEquals("just", model.upNext.value?.externalId)
    }

    @Test
    fun reloadWithNoStoreLeavesEverythingEmpty() = runTest {
        val model = TodayModel()
        model.reload(syncWith(null, backgroundScope), now, zone)
        assertNull(model.upNext.value)
        assertNull(model.brief.value)
        assertNull(model.prep.value)
        assertTrue(model.recent.value.isEmpty())
        assertTrue(model.loaded.value)
    }
}
