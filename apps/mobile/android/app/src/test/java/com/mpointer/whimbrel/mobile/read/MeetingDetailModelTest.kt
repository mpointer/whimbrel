package com.mpointer.whimbrel.mobile.read

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MeetingDetailModelTest {
    private fun store() = ScriptedStore().apply {
        meetings = listOf(PhoneMeeting(1, "a", 1_000, title = "Call"), PhoneMeeting(2, "a", 500, title = "No transcript"))
        transcripts[1] = (0 until 150).map { PhoneTranscriptSegment(it.toLong(), it.toLong(), "S", "t$it") }
        items[1] = listOf(PhoneActionItem(1, "Send notes", false, 0))
    }

    @Test
    fun detailLoadsTranscriptInPagesAndHandlesNone() = runTest {
        val sync = syncWith(store(), backgroundScope)
        val model = MeetingDetailModel(1)
        model.reload(sync)
        assertEquals("Call", model.meeting.value?.title)
        assertEquals(listOf("Send notes"), model.actionItems.value.map { it.text })
        assertEquals(100, model.transcript.value.size)
        assertEquals(150, model.transcriptTotal.value)
        assertTrue(model.hasMoreTranscript)
        model.loadMoreTranscript(sync)
        assertEquals(150, model.transcript.value.size)
        assertFalse(model.hasMoreTranscript)

        val none = MeetingDetailModel(2)
        none.reload(sync)
        assertEquals(0, none.transcriptTotal.value)
        assertTrue(none.loaded.value)
        assertFalse(none.hasMoreTranscript)

        val unopened = MeetingDetailModel(1)
        unopened.reload(syncWith(null, backgroundScope))
        assertFalse("no store yet: still loading, not missing", unopened.loaded.value)
    }

    @Test
    fun reloadKeepsAlreadyShownTranscriptPages() = runTest {
        val sync = syncWith(store(), backgroundScope)
        val model = MeetingDetailModel(1)
        model.reload(sync)
        model.loadMoreTranscript(sync)
        model.reload(sync)
        assertEquals(150, model.transcript.value.size)
        assertEquals("no segment twice", 150, model.transcript.value.map { it.id }.toSet().size)
    }

    @Test
    fun missingMeetingLoadsAsNull() = runTest {
        val model = MeetingDetailModel(99)
        model.reload(syncWith(store(), backgroundScope))
        assertNull(model.meeting.value)
        assertTrue(model.loaded.value)
    }
}
