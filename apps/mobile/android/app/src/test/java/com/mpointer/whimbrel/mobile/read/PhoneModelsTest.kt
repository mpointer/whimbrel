package com.mpointer.whimbrel.mobile.read

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PhoneModelsTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun groupedMeetingWithCopiesAndNoEnd() {
        val m = json.decodeFromString<PhoneMeeting>(
            """
            {"id":259,"device_id":"Hp7AHkXx","start_unix_ms":1000,"end_unix_ms":null,"title":"Rock Arkie's Zoom Meeting",
             "attendees":["a@x.com"],"trigger":"calendar","audio_path":null,"status":"completed","transcript_path":null,
             "notes":null,"summary_tldr":"We agreed.","summary_decisions":["Ship it"],"summary_action_items":[],
             "summary_open_questions":["When?"],"summary_provider":null,"summary_model":null,"estimated_speaker_count":null,
             "also_recorded":[{"id":258,"device_id":"ZmpQhobe","title":"Rock Arkie's Zoom Meeting"}]}
            """,
        )
        assertEquals(259L, m.id)
        assertEquals("Hp7AHkXx", m.deviceId)
        assertNull(m.endUnixMs)
        assertEquals("We agreed.", m.summaryTldr)
        assertEquals(listOf("Ship it"), m.summaryDecisions)
        assertEquals(listOf(258L), m.alsoRecorded?.map { it.id })
    }

    @Test
    fun meetingWithoutSummaryFieldsStillDecodes() {
        val m = json.decodeFromString<PhoneMeeting>("""{"id":1,"device_id":"d","start_unix_ms":5,"title":"T","status":"completed"}""")
        assertEquals(emptyList<String>(), m.attendees)
        assertEquals(emptyList<String>(), m.summaryDecisions)
        assertNull(m.summaryTldr)
        assertNull(m.alsoRecorded)
    }

    @Test
    fun routineRunWithOriginDevice() {
        val run = json.decodeFromString<PhoneRoutineRun>(
            """
            {"id":4,"routine_id":2,"routine_name":"Daily Brief","ran_at_unix_ms":900,"status":"success",
             "output_summary":"Today: 3 meetings.","output_json":"{}","origin_device_id":"ZmpQhobe"}
            """,
        )
        assertEquals("Daily Brief", run.routineName)
        assertEquals("Today: 3 meetings.", run.outputSummary)
        assertEquals("ZmpQhobe", run.originDeviceId)
        assertTrue(run.succeeded)
    }

    @Test
    fun prepBriefIsDecodedFromItsJsonString() {
        val prep = json.decodeFromString<PhoneMeetingPrep>(
            """
            {"event_id":"evt-1","title":"Standup","start_unix_ms":2000,"generated_at_unix_ms":1500,
             "brief_json":"{\"summary\":{\"text\":\"Review the launch.\",\"sources\":[]},\"attendees\":[{\"who\":\"Ann\",\"note\":\"Owns QA\",\"sources\":[]}],\"open_items\":[{\"text\":\"Sign-off\",\"sources\":[]}],\"open_questions\":[],\"sources\":[]}",
             "provider":"anthropic","model":"m"}
            """,
        )
        assertEquals("evt-1", prep.eventId)
        assertEquals("Review the launch.", prep.brief?.summary?.text)
        assertEquals("Ann", prep.brief?.attendees?.first()?.who)
        assertEquals(listOf("Sign-off"), prep.brief?.openItems?.map { it.text })
    }

    @Test
    fun prepWithUnparseableBriefHasNoBrief() {
        val prep = json.decodeFromString<PhoneMeetingPrep>("""{"event_id":"e","title":"T","start_unix_ms":1,"brief_json":"nope"}""")
        assertNull(prep.brief)
    }

    @Test
    fun transcriptPageActionItemAndCalendarEvent() {
        val page = json.decodeFromString<PhoneTranscriptPage>(
            """{"segments":[{"id":1,"meeting_id":9,"meeting_title":"T","start_cs":150,"speaker_label":"Speaker 1","text":"Hello"}],"total":42}""",
        )
        assertEquals(42, page.total)
        assertEquals("Speaker 1", page.segments.first().speakerLabel)
        assertEquals(150L, page.segments.first().startCs)
        val item = json.decodeFromString<PhoneActionItem>(
            """{"id":3,"meeting_id":9,"text":"Send notes","completed":true,"order_index":0,"source":"generated","created_unix_ms":1,"updated_unix_ms":2}""",
        )
        assertEquals("Send notes", item.text)
        assertTrue(item.completed)
        val event = json.decodeFromString<PhoneCalendarEvent>(
            """{"external_id":"e1","source":"google_calendar","title":"Standup","start_unix_ms":2000,"attendees":["a@x.com"]}""",
        )
        assertEquals("e1", event.externalId)
        assertEquals(listOf("a@x.com"), event.attendees)
    }

    @Test
    fun emojiSurvivesTitleAndSpeakerLabel() {
        val m = json.decodeFromString<PhoneMeeting>("""{"id":1,"device_id":"d","start_unix_ms":5,"title":"Launch 🚀 sync","status":"completed"}""")
        assertEquals("Launch 🚀 sync", m.title)
        val page = json.decodeFromString<PhoneTranscriptPage>(
            """{"segments":[{"id":1,"start_cs":0,"speaker_label":"Sam 🎤","text":"Hi 👋"}],"total":1}""",
        )
        assertEquals("Sam 🎤", page.segments.first().speakerLabel)
        assertEquals("Hi 👋", page.segments.first().text)
    }

    @Test
    fun decodesCalendarRowId() {
        val e = json.decodeFromString<PhoneCalendarEvent>(
            """{"row_id":42,"external_id":"x","source":"google_calendar","title":"Sync","start_unix_ms":9,"attendees":[],"end_unix_ms":10}""",
        )
        assertEquals(42L, e.rowId)
        val old = json.decodeFromString<PhoneCalendarEvent>("""{"external_id":"x","source":"s","title":"T","start_unix_ms":9,"attendees":[]}""")
        assertNull(old.rowId)
    }

    @Test
    fun decodesTopicSnapshotStepsAndNextMeeting() {
        val s = readJson.decodeFromString<List<PhoneTopicSnapshot>>(
            """
            [{"topic_key":"acme","name":"Acme","kind":"customer","lead":{"kind":"step","text":"Send quote","due_date":"2026-11-12"},
              "role":null,"next_meeting":{"title":"Acme call","start_unix_ms":1000,"attendees_by_org":[["acme.com",3]],"me":1},
              "you_owe":[{"text":"Send quote","source_kind":"meeting","source_title":null,"asked_at_unix_ms":5,"due_date":"2026-11-12","meeting":null},
                         {"text":"Call back","source_kind":"email"}],
              "owed_to_you":[],"done":[],"recent":[],"recent_total":0,"truncated":false,"built_at_unix_ms":7,"built_by":"Mac"}]
            """,
        ).single()
        assertEquals("acme", s.topicKey)
        assertEquals("Acme", s.name)
        assertEquals("Acme call" to 1000L, s.nextMeeting?.let { it.title to it.startUnixMs })
        assertEquals(listOf("Send quote" to "2026-11-12", "Call back" to null), s.youOwe.map { it.text to it.dueDate })
        assertEquals(listOf("meeting", "email"), s.youOwe.map { it.sourceKind })
        val bare = readJson.decodeFromString<PhoneTopicSnapshot>("""{"topic_key":"k","name":"N"}""")
        assertNull(bare.nextMeeting)
        assertEquals(emptyList<PhoneTopicSnapshot.Step>(), bare.youOwe)
    }

    private val fullTopic = """
        {"topic_key":"acme","name":"Acme","kind":"customer","lead":{"kind":"date","label":"Quote due","date":"2026-11-14"},
         "role":{"text":"You own the quote.","edited":true},
         "next_meeting":{"title":"Acme call","start_unix_ms":1000,"attendees_by_org":[["acme.com",3],["bad"],["x.org",1]],"me":1},
         "you_owe":[{"text":"Send quote","source_kind":"meeting","source_title":"Acme call","asked_at_unix_ms":5,"due_date":"2026-11-12",
                     "meeting":{"device_id":"mac-a","source_meeting_id":7}}],
         "owed_to_you":[{"text":"Order form","source_kind":"email"}],"done":[{"text":"Deck","source_kind":"meeting"}],
         "recent":[{"kind":"email","title":"Re: terms","sender":"Priya","ts_unix_ms":9,"meeting":null}],
         "recent_total":4,"truncated":true,"built_at_unix_ms":7,"built_by":"Mac"}
    """

    @Test
    fun decodesTheWholeTopicPage() {
        val s = readJson.decodeFromString<PhoneTopicSnapshot>(fullTopic)
        assertEquals(PhoneTopicSnapshot.Lead.Dated("Quote due", "2026-11-14"), s.lead)
        assertEquals(PhoneTopicSnapshot.Role("You own the quote.", true), s.role)
        assertEquals("a pair that is not [domain, count] is left out", listOf(PhoneTopicSnapshot.OrgCount("acme.com", 3), PhoneTopicSnapshot.OrgCount("x.org", 1)), s.nextMeeting?.attendeesByOrg)
        assertEquals(1L, s.nextMeeting?.me)
        val step = s.youOwe.single()
        assertEquals("meeting", step.sourceKind)
        assertEquals("Acme call", step.sourceTitle)
        assertEquals(5L, step.askedAtUnixMs)
        assertEquals(PhoneMeetingRef("mac-a", 7), step.meeting)
        assertEquals("Order form", s.owedToYou.single().text)
        assertEquals("Deck", s.done.single().text)
        assertEquals(PhoneTopicSnapshot.Recent("email", "Re: terms", "Priya", 9), s.recent.single())
        assertEquals(4L, s.recentTotal)
        assertEquals(true, s.truncated)
        assertEquals("Mac", s.builtBy)
        assertNull("a meeting that has started is no next meeting", s.upcomingMeeting(1000))
        assertEquals("Acme call", s.upcomingMeeting(999)?.title)
    }

    @Test
    fun aLeadKindThisBuildDoesNotKnowLosesTheLeadNotThePage() {
        val s = readJson.decodeFromString<PhoneTopicSnapshot>("""{"topic_key":"k","name":"N","lead":{"kind":"mood","emoji":"x"},"you_owe":[{"text":"A"}]}""")
        assertNull(s.lead)
        assertEquals("A", s.youOwe.single().text)
        assertEquals(
            PhoneTopicSnapshot.Lead.NextStep("Send", null),
            readJson.decodeFromString<PhoneTopicSnapshot>("""{"topic_key":"k","name":"N","lead":{"kind":"step","text":"Send"}}""").lead,
        )
    }

    @Test
    fun aPageThatNoLongerParsesInFullStillReminds() {
        // `role` and `recent` have shapes this build cannot read; reminders only need the name, next meeting and steps.
        val list = PhoneTopicSnapshot.decodeList(
            """
            [{"topic_key":"new","name":"Newer core","role":"owner","recent":"none",
              "next_meeting":{"title":"Call","start_unix_ms":5},"you_owe":[{"text":"Pay","due_date":"2026-11-12"}]},
             {"name":"no key"}, 7,
             $fullTopic]
            """,
        )!!
        assertEquals(listOf("new", "acme"), list.map { it.topicKey })
        assertEquals(PhoneTopicSnapshot.NextMeeting("Call", 5), list[0].nextMeeting)
        assertEquals(listOf(PhoneTopicSnapshot.Step("Pay", "2026-11-12")), list[0].youOwe)
        assertEquals("the page that parses is whole", "Mac", list[1].builtBy)
        assertNull(PhoneTopicSnapshot.decodeList("""{"not":"a list"}"""))
        assertNull(PhoneTopicSnapshot.decodeList(null))
    }

    @Test
    fun decodesCaptureItem() {
        val c = readJson.decodeFromString<PhoneCaptureItem>("""{"window_title":"","app_bundle_id":"com.app","url":null,"text":"Hello","extra":1}""")
        assertEquals("the app stands in for an empty window title", "com.app", c.title)
        assertEquals("Hello", c.text)
    }
}
