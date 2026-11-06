package com.mpointer.whimbrel.mobile.read

import java.time.ZoneOffset
import java.time.ZonedDateTime
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DemoFixturesTest {
    /** Every screen can be shown from the demo store: Today, Meetings, detail and Routines. */
    @Test
    fun demoStoreServesEveryScreen() = runTest {
        val zone = ZoneOffset.UTC
        val now = ZonedDateTime.of(2026, 11, 4, 14, 0, 0, 0, zone).toInstant().toEpochMilli()
        val sync = syncWith(DemoSyncStore(clock = { now }, zone = zone), backgroundScope)

        val today = TodayModel()
        today.reload(sync, now, zone)
        assertNotNull("a Daily Brief dated today", today.brief.value)
        assertNotNull(today.upNext.value)
        assertNotNull(today.prep.value?.brief)
        assertEquals(10, today.recent.value.size)

        val meetings = MeetingsModel()
        meetings.reload(sync)
        assertEquals(12, meetings.meetings.value.size)
        assertEquals(2, meetings.devices.size)

        val detail = MeetingDetailModel(1)
        detail.reload(sync)
        assertNotNull(detail.meeting.value?.summaryTldr)
        assertEquals(150, detail.transcriptTotal.value)
        val noSummary = MeetingDetailModel(3)
        noSummary.reload(sync)
        assertNull(noSummary.meeting.value?.summaryTldr)

        val routines = RoutinesModel()
        routines.reload(sync)
        assertEquals(4, routines.runs.value.size)
        assertTrue(routines.runs.value.any { !it.succeeded })
    }

    @Test
    fun demoBriefAndNextMeetingStayInsideTodayJustAfterMidnightAndLateAtNight() = runTest {
        val zone = ZoneOffset.UTC
        for (hour in listOf(0, 23)) {
            val now = ZonedDateTime.of(2026, 11, 4, hour, if (hour == 0) 5 else 50, 0, 0, zone).toInstant().toEpochMilli()
            val today = TodayModel()
            today.reload(syncWith(DemoSyncStore(clock = { now }, zone = zone), backgroundScope), now, zone)
            assertNotNull("brief at hour $hour", today.brief.value)
            assertNotNull("next meeting at hour $hour", today.upNext.value)
        }
    }
}
