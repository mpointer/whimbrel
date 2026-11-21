package com.mpointer.whimbrel.mobile.read

import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import java.time.LocalDate
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonArray
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Fixed words and dates, so the tests read the composition and not the phone's formats. */
private object TestTopicStrings : TopicStrings {
    override val overdue = "overdue"
    override fun due(day: String) = "due $day"
    override fun asked(day: String) = "asked $day"
    override fun you(n: Long) = "$n you"
    override fun builtOn(computer: String, whenText: String) = "Built on $computer · $whenText"
    override fun day(date: LocalDate) = "D$date"
    override fun monthDay(ms: Long) = "M$ms"
    override fun dayTime(ms: Long) = "T$ms"
    override fun ago(thenMs: Long, nowMs: Long) = "${nowMs - thenMs}ms ago"
}

class TopicsModelTest {
    private fun topic(key: String, name: String, nextAt: Long? = null) =
        PhoneTopicSnapshot(key, name, nextAt?.let { PhoneTopicSnapshot.NextMeeting("M", it) })

    @Test
    fun sortsByNextMeetingThenName() {
        val list = listOf(topic("c", "Zed"), topic("a", "beta", 300), topic("b", "Alpha", 200), topic("d", "apple"), topic("e", "Gone", 50))
        assertEquals("a next meeting that has passed counts as none", listOf("b", "a", "d", "e", "c"), TopicsModel.sorted(list, nowMs = 100).map { it.topicKey })
    }

    @Test
    fun listLoadsSortedAndSaysWhenItHasLoaded() = runTest {
        val store = ScriptedStore().apply { topics = listOf(topic("b", "B"), topic("a", "A")) }
        val model = TopicsModel { 0 }
        assertFalse(model.loaded.value)
        model.reload(syncWith(store, backgroundScope))
        assertEquals(listOf("a", "b"), model.topics.value.map { it.topicKey })
        assertTrue(model.loaded.value)
    }

    @Test
    fun noOpenStoreReadsAsLoadedAndEmpty() = runTest {
        val sync = syncWith(null, backgroundScope)
        val list = TopicsModel().also { it.reload(sync) }
        assertTrue(list.loaded.value)
        assertTrue(list.topics.value.isEmpty())
        val page = TopicPageModel("acme").also { it.reload(sync) }
        assertTrue("a missing page, not loading", page.loaded.value)
        assertNull(page.snapshot.value)
    }

    /** A replica that has one of the meetings a page names. */
    private class LinkStore(private val inner: ScriptedStore) : MeetingSyncStore by inner {
        val asked = mutableListOf<PhoneMeetingRef>()
        override fun meetingId(deviceId: String, sourceMeetingId: Long): Long? {
            asked += PhoneMeetingRef(deviceId, sourceMeetingId)
            return if (deviceId == "mac-a" && sourceMeetingId == 7L) 42L else null
        }
    }

    @Test
    fun pageResolvesItsMeetingRefsThroughTheStore() = runTest {
        val here = PhoneMeetingRef("mac-a", 7)
        val gone = PhoneMeetingRef("mac-b", 7)
        val snap = PhoneTopicSnapshot(
            "acme", "Acme",
            youOwe = listOf(PhoneTopicSnapshot.Step("A", meeting = here), PhoneTopicSnapshot.Step("B")),
            owedToYou = listOf(PhoneTopicSnapshot.Step("C", meeting = gone)),
            done = listOf(PhoneTopicSnapshot.Step("D", meeting = here)),
            recent = listOf(PhoneTopicSnapshot.Recent("meeting", "Call", meeting = here)),
        )
        val store = LinkStore(ScriptedStore().apply { topics = listOf(topic("other", "Other"), snap) })
        val model = TopicPageModel("acme")
        model.reload(syncWith(store, backgroundScope))
        assertEquals(snap, model.snapshot.value)
        assertEquals("a ref goes through the store, never compared with a local id", mapOf(here to 42L), model.meetingIds.value)
        assertEquals("each ref is asked once", listOf(here, gone), store.asked)

        val missing = TopicPageModel("nope").also { it.reload(syncWith(store, backgroundScope)) }
        assertNull(missing.snapshot.value)
        assertTrue(missing.loaded.value)
    }

    private val today = LocalDate.of(2026, 11, 12)

    @Test
    fun dueReadsOverdueOnlyOnceTheDayHasPassed() {
        assertEquals("due D2026-11-12", TopicText.dueText("2026-11-12", today, TestTopicStrings))
        assertEquals("overdue", TopicText.dueText("2026-11-11", today, TestTopicStrings))
        assertFalse(TopicText.isOverdue("2026-11-12", today))
        assertEquals("a date that does not parse is shown as it is", "due soon", TopicText.dueText("soon", today, TestTopicStrings))
    }

    @Test
    fun leadIsADatedFactOrTheNextStep() {
        assertEquals("Quote due D2026-11-14", TopicText.lead(PhoneTopicSnapshot.Lead.Dated("Quote due", "2026-11-14"), today, TestTopicStrings))
        assertEquals("Send feedback due D2026-11-13", TopicText.lead(PhoneTopicSnapshot.Lead.NextStep("Send feedback", "2026-11-13"), today, TestTopicStrings))
        assertEquals("Send feedback overdue", TopicText.lead(PhoneTopicSnapshot.Lead.NextStep("Send feedback", "2026-11-01"), today, TestTopicStrings))
        assertEquals("Send feedback", TopicText.lead(PhoneTopicSnapshot.Lead.NextStep("Send feedback", null), today, TestTopicStrings))
    }

    @Test
    fun attendeesPutTheUserFirstThenEachOrganisation() {
        val orgs = buildJsonArray {
            add(buildJsonArray { add("acme.com"); add(3) })
            add(buildJsonArray { add("x.org"); add(1) })
        }
        assertEquals("1 you · 3 acme.com · 1 x.org", TopicText.attendees(PhoneTopicSnapshot.NextMeeting("M", 0, orgs, me = 1), TestTopicStrings))
        assertEquals("3 acme.com · 1 x.org", TopicText.attendees(PhoneTopicSnapshot.NextMeeting("M", 0, orgs), TestTopicStrings))
        assertEquals("", TopicText.attendees(PhoneTopicSnapshot.NextMeeting("M", 0), TestTopicStrings))
    }

    @Test
    fun footerNamesTheComputerAndNeverReadsAsTheFuture() {
        val snap = PhoneTopicSnapshot("k", "N", builtAtUnixMs = 1_000, builtBy = "Mike's MacBook")
        assertEquals("Built on Mike's MacBook · 500ms ago", TopicText.footer(snap, 1_500, TestTopicStrings))
        assertEquals("a computer clock ahead of the phone's", "Built on Mike's MacBook · 0ms ago", TopicText.footer(snap, 900, TestTopicStrings))
    }
}
