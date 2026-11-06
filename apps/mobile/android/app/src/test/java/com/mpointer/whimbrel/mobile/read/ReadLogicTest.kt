package com.mpointer.whimbrel.mobile.read

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ReadLogicTest {
    private val strings = object : ReadStrings {
        override val now = "Now"
        override fun inMinutes(n: Int) = "In ${n}m"
        override fun inHours(n: Int) = "In ${n}h"
        override val syncing = "Syncing…"
        override val notSyncedYet = "Not synced yet"
        override val syncedJustNow = "Synced just now"
        override fun syncedMinutesAgo(n: Int) = "Synced $n min ago"
        override fun syncedHoursAgo(n: Int) = "Synced $n h ago"
        override fun timeRange(startMs: Long, endMs: Long?, zone: java.time.ZoneId, withDate: Boolean) = ""
        override fun meetings(n: Int) = if (n == 1) "1 meeting" else "$n meetings"
        override fun copies(n: Int) = ""
        override fun device(label: com.mpointer.whimbrel.mobile.sync.DeviceLabel) = ""
    }
    private val now = 10_000_000_000L
    private fun sync(agoSec: Long?, syncing: Boolean = false, error: String? = null) =
        ReadLogic.syncLine(agoSec?.let { now - it * 1000 }, now, syncing, error, strings)

    @Test
    fun meetingCountLineIsHiddenUntilKnown() {
        assertNull(ReadLogic.meetingCountLine(null, strings))
        assertEquals("1 meeting", ReadLogic.meetingCountLine(1, strings))
        assertEquals("0 meetings", ReadLogic.meetingCountLine(0, strings))
        assertEquals("42 meetings", ReadLogic.meetingCountLine(42, strings))
    }

    @Test
    fun daypartBoundaries() {
        assertEquals("morning", ReadLogic.daypart(5))
        assertEquals("morning", ReadLogic.daypart(11))
        assertEquals("afternoon", ReadLogic.daypart(12))
        assertEquals("afternoon", ReadLogic.daypart(16))
        assertEquals("evening", ReadLogic.daypart(17))
    }

    @Test
    fun attendeeSummary() {
        assertEquals("A, B +2", ReadLogic.attendeeSummary(listOf("A", "B", "C", "D")))
        assertEquals("A, B", ReadLogic.attendeeSummary(listOf("A", "B")))
        assertEquals("", ReadLogic.attendeeSummary(emptyList()))
    }

    @Test
    fun meetingPageAttendees() {
        fun names(n: Int) = (1..n).map { "P$it" }
        assertEquals("", ReadLogic.attendeeLine(names(0), false))
        assertEquals("P1", ReadLogic.attendeeLine(names(1), false))
        assertEquals("P1, P2, P3", ReadLogic.attendeeLine(names(3), false))
        assertEquals(0, ReadLogic.hiddenAttendeeCount(names(3)))
        assertEquals("P1, P2, P3", ReadLogic.attendeeLine(names(4), false))
        assertEquals(1, ReadLogic.hiddenAttendeeCount(names(4)))
        assertEquals("P1, P2, P3, P4", ReadLogic.attendeeLine(names(4), true))
        assertEquals("P1, P2, P3", ReadLogic.attendeeLine(names(52), false))
        assertEquals(49, ReadLogic.hiddenAttendeeCount(names(52)))
        assertEquals(52, ReadLogic.attendeeLine(names(52), true).split(", ").size)
    }

    @Test
    fun syncLineBuckets() {
        assertEquals("Not synced yet", sync(null))
        assertEquals("Synced just now", sync(0))
        assertEquals("Synced just now", sync(59))
        assertEquals("Synced 1 min ago", sync(60))
        assertEquals("Synced 59 min ago", sync(3599))
        assertEquals("Synced 1 h ago", sync(3600))
        assertEquals("Synced 3 h ago", sync(3 * 3600))
        assertEquals("Synced just now", sync(-30)) // a clock behind the last sync clamps to zero
    }

    @Test
    fun syncLinePrecedence() {
        assertEquals("Syncing…", sync(20, syncing = true))
        assertEquals("Syncing…", sync(null, syncing = true))
        assertEquals("Couldn't sync.", sync(20, syncing = true, error = "Couldn't sync."))
        assertEquals("Couldn't sync.", sync(null, error = "Couldn't sync."))
    }

    private fun rel(startMin: Int, endMin: Int?) =
        ReadLogic.relativeStart(now + startMin * 60_000L, endMin?.let { now + it * 60_000L }, now, strings)

    @Test
    fun relativeStart() {
        assertEquals("In 9m", rel(9, 60))
        assertEquals("Now", rel(-30, 60))
        assertNull(rel(-120, -60))
        assertEquals("In 1m", ReadLogic.relativeStart(now + 1, now + 3_600_000, now, strings)) // rounds up
        assertEquals("In 1h", rel(60, 90))
        assertEquals("In 2h", rel(120, 150))
        assertEquals("In 12h", rel(720, 750))
        assertNull(rel(721, 750))
        assertNull(rel(-10, null))
    }
}
