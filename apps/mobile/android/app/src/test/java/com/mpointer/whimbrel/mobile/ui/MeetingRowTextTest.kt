package com.mpointer.whimbrel.mobile.ui

import com.mpointer.whimbrel.mobile.read.PhoneMeeting
import com.mpointer.whimbrel.mobile.read.PhoneMeetingCopy
import com.mpointer.whimbrel.mobile.read.ReadStrings
import com.mpointer.whimbrel.mobile.sync.DeviceLabel
import com.mpointer.whimbrel.mobile.sync.DeviceLabels
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MeetingRowTextTest {
    private val zone = ZoneId.of("UTC")
    private val strings = object : ReadStrings {
        override val now = "Now"
        override fun inMinutes(n: Int) = "In ${n}m"
        override fun inHours(n: Int) = "In ${n}h"
        override val syncing = "Syncing…"
        override val notSyncedYet = "Not synced yet"
        override val syncedJustNow = "Synced just now"
        override fun syncedMinutesAgo(n: Int) = "Synced $n min ago"
        override fun syncedHoursAgo(n: Int) = "Synced $n h ago"
        override fun timeRange(startMs: Long, endMs: Long?, zone: ZoneId, withDate: Boolean): String {
            val f = DateTimeFormatter.ofPattern("HH:mm").withZone(zone)
            val start = f.format(Instant.ofEpochMilli(startMs))
            return (if (withDate) "Nov 4, " else "") + if (endMs == null) start else "$start–${f.format(Instant.ofEpochMilli(endMs))}"
        }
        override fun meetings(n: Int) = if (n == 1) "1 meeting" else "$n meetings"
        override fun copies(n: Int) = if (n == 1) "+1 copy" else "+$n copies"
        override fun device(label: DeviceLabel) = when (label) {
            is DeviceLabel.Named -> label.name
            DeviceLabel.ThisPhone -> "This phone"
            is DeviceLabel.ShortId -> "Device ${label.prefix}"
        }
    }
    private val nineAm = 1_700_000_000_000L - 1_700_000_000_000L % 86_400_000L + 9 * 3_600_000L
    private fun meeting(title: String = "Design review", attendees: List<String> = listOf("Ann", "Sam", "Tom", "Lee"), device: String = "mac-a", copies: List<PhoneMeetingCopy>? = null) =
        PhoneMeeting(1, device, nineAm, nineAm + 30 * 60_000, title, attendees, alsoRecorded = copies)
    private val labels = DeviceLabels(names = mapOf("mac-a" to "Work Mac"))

    @Test
    fun titleTimeAndAttendeeSummary() {
        val row = meetingRowText(meeting(), labels, strings, zone)
        assertEquals("Design review", row.title)
        assertEquals("09:00–09:30", row.timeRange)
        assertEquals("Ann, Sam +2", row.attendees)
        assertEquals("Work Mac", row.device)
        assertNull(row.alsoRecorded)
        assertEquals("Nov 4, 09:00–09:30", meetingRowText(meeting(), labels, strings, zone, withDate = true).timeRange)
    }

    @Test
    fun alsoRecordedShowsTheCopyCount() {
        val one = listOf(PhoneMeetingCopy(2, "mac-b", "Design review"))
        assertEquals("+1 copy", meetingRowText(meeting(copies = one), labels, strings, zone).alsoRecorded)
        assertEquals("+2 copies", meetingRowText(meeting(copies = one + one), labels, strings, zone).alsoRecorded)
        assertNull(meetingRowText(meeting(copies = emptyList()), labels, strings, zone).alsoRecorded)
    }

    @Test
    fun unknownDeviceShowsAShortId() {
        assertEquals("Device abcdef", meetingRowText(meeting(device = "abcdef0123456789"), labels, strings, zone).device)
    }

    @Test
    fun longTitleIsKeptWhole() {
        val long = "Partner sync with a very long title that has to wrap across several lines on a small phone screen ".repeat(3).trim()
        assertEquals(long, meetingRowText(meeting(title = long), labels, strings, zone).title)
    }
}
