package com.mpointer.whimbrel.mobile.core

import com.mpointer.whimbrel.mobile.sync.SyncCredentials
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CoreMemoryStoreTest {
    private val native = FakeSyncNative().apply { }
    private val creds = SyncCredentials("tok", "libsql://db", "key", "dev")

    private fun store(): CoreMemoryStore {
        val opener = object : FakeSyncNative() { override fun memoryOpen(path: String) = 9L }
        return CoreMemoryStore(CoreStore.open(File("x"), opener)!!, native)
    }

    @Test
    fun groupedMeetingCountIsTheCoreCount() {
        native.groupedCountReply = 3
        assertEquals(3, store().groupedMeetingCount())
    }

    @Test
    fun groupedMeetingCountIsNilOnFailure() {
        native.groupedCountReply = -1
        assertNull(store().groupedMeetingCount())
    }

    @Test
    fun pullOnlySetsTheDeviceIdThenPulls() {
        assertEquals(true, store().pullOnly(creds, 14))
        assertEquals(listOf("setDeviceId", "pull"), native.calls)
        assertEquals(listOf<Any>(9L, "libsql://db", "tok", "key", "dev", 14), native.pullOnlyArgs)
    }

    @Test
    fun moreToPullAsksTheCore() {
        assertEquals(false, store().moreToPull())
        native.moreToPullReply = true
        assertEquals(true, store().moreToPull())
    }

    @Test
    fun olderHistoryLoadingAsksTheCoreForThisStore() {
        assertEquals(false, store().olderHistoryLoading())
        native.olderHistoryReply = true
        assertEquals(true, store().olderHistoryLoading())
        assertEquals("the store's own handle, not a global", listOf(9L, 9L), native.olderHistoryHandles)
    }

    @Test
    fun topicSnapshotsDecodeAndFailQuietly() {
        val s = store()
        native.topicSnapshotsReply = """[{"topic_key":"acme","name":"Acme","you_owe":[{"text":"Send","due_date":"2026-11-12"}]}]"""
        assertEquals(listOf("acme"), s.topicSnapshots().map { it.topicKey })
        native.topicSnapshotsReply = "nope"
        assertEquals(emptyList<Any>(), s.topicSnapshots())
        native.topicSnapshotsReply = null
        assertEquals(emptyList<Any>(), s.topicSnapshots())
    }

    @Test
    fun malformedReadJsonGivesEmptyListOrNull() {
        val s = store()
        native.groupedReply = "not json"
        native.memoryGetMeetingReply = "{"
        native.transcriptReply = "[]"
        native.actionItemsReply = "nope"
        native.runsReply = "nope"
        native.prepReply = "nope"
        native.upcomingReply = "nope"
        assertEquals(emptyList<Any>(), s.recentMeetings(10))
        assertNull(s.meeting(1))
        assertNull(s.transcriptPage(1, 0, 10))
        assertEquals(emptyList<Any>(), s.actionItems(1))
        assertEquals(emptyList<Any>(), s.routineRuns(10))
        assertNull(s.prep("e"))
        assertEquals(emptyList<Any>(), s.upcomingEvents(0, 1, 10))
        // The native layer's own null (no handle, DB error) is the same quiet answer.
        native.groupedReply = null
        native.memoryGetMeetingReply = null
        assertEquals(emptyList<Any>(), s.recentMeetings(10))
        assertNull(s.meeting(1))
    }

    @Test
    fun wellFormedReadJsonDecodes() {
        val s = store()
        native.groupedReply = """[{"id":1,"device_id":"d","start_unix_ms":5,"title":"T","status":"completed"}]"""
        native.runsReply = """[{"id":4,"routine_name":"R","ran_at_unix_ms":9,"status":"success","output_summary":"ok"}]"""
        assertEquals(listOf("T"), s.recentMeetings(10).map { it.title })
        assertEquals(true, s.routineRuns(10).single().succeeded)
    }
}
