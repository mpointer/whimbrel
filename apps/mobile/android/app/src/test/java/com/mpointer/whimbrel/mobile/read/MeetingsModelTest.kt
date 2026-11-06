package com.mpointer.whimbrel.mobile.read

import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.ZonedDateTime
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MeetingsModelTest {
    private val zone: ZoneId = ZoneOffset.UTC
    private fun ms(day: Int, hour: Int, minute: Int = 0) =
        ZonedDateTime.of(2026, 10, day, hour, minute, 0, 0, zone).toInstant().toEpochMilli()

    private fun meeting(id: Long, title: String, start: Long, device: String = "a") = PhoneMeeting(id, device, start, title = title)

    @Test
    fun sectionsGroupByLocalDayNewestFirst() {
        val meetings = listOf(meeting(3, "Late", ms(19, 0, 1)), meeting(2, "Night", ms(18, 23, 59)), meeting(1, "Morning", ms(18, 9)))
        val sections = MeetingsModel.sections(meetings, null, "", zone)
        assertEquals("23:59 and 00:01 are different days", listOf(listOf(3L), listOf(2L, 1L)), sections.map { s -> s.meetings.map { it.id } })
        assertEquals(listOf(LocalDate.of(2026, 10, 19), LocalDate.of(2026, 10, 18)), sections.map { it.day })
    }

    @Test
    fun deviceFilterAndSearch() {
        val meetings = listOf(
            meeting(1, "Acme pricing", ms(19, 9), "a"),
            meeting(2, "Standup", ms(19, 8), "b"),
            meeting(3, "ACME renewal", ms(18, 9), "b"),
        )
        fun ids(device: String?, query: String) = MeetingsModel.sections(meetings, device, query, zone).flatMap { it.meetings }.map { it.id }
        assertEquals(listOf(2L, 3L), ids("b", ""))
        assertEquals(listOf(1L, 3L), ids(null, "acme"))
        assertEquals(listOf(1L), ids("a", "acme"))
        assertEquals("blank search shows everything", 3, ids(null, "   ").size)
    }

    @Test
    fun searchMatchesEmojiTitles() {
        val meetings = listOf(meeting(1, "🎉 Launch party", ms(19, 9)), meeting(2, "Standup", ms(19, 8)))
        assertEquals(listOf(1L), MeetingsModel.sections(meetings, null, "🎉", zone).flatMap { it.meetings }.map { it.id })
        assertEquals(listOf(1L), MeetingsModel.sections(meetings, null, "LAUNCH", zone).flatMap { it.meetings }.map { it.id })
    }

    @Test
    fun listSaysWhenItIsCappedAtTwoHundred() = runTest {
        val store = ScriptedStore().apply { meetings = (1..250).map { meeting(it.toLong(), "M$it", ms(19, 9)) } }
        val sync = syncWith(store, backgroundScope)
        val model = MeetingsModel()
        model.reload(sync)
        assertEquals(200, model.meetings.value.size)
        assertTrue(model.isCapped)
        store.meetings = (1..5).map { meeting(it.toLong(), "M$it", ms(19, 9)) }
        model.reload(sync)
        assertFalse(model.isCapped)
        assertEquals(listOf("a"), model.devices)
        assertTrue(model.loaded.value)
    }
}
