import XCTest
@testable import Whimbrel

final class PhoneModelsTests: XCTestCase {
    private func decode<T: Decodable>(_ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    func testGroupedMeetingWithCopiesAndNoEnd() throws {
        let meeting: PhoneMeeting = try decode("""
        {"id":259,"device_id":"Hp7AHkXx","start_unix_ms":1000,"end_unix_ms":null,"title":"Rock Arkie's Zoom Meeting",
         "attendees":["a@x.com"],"trigger":"calendar","audio_path":null,"status":"completed","transcript_path":null,
         "notes":null,"summary_tldr":"We agreed.","summary_decisions":["Ship it"],"summary_action_items":[],
         "summary_open_questions":["When?"],"summary_provider":null,"summary_model":null,"estimated_speaker_count":null,
         "also_recorded":[{"id":258,"device_id":"ZmpQhobe","title":"Rock Arkie's Zoom Meeting"}]}
        """)
        XCTAssertEqual(meeting.id, 259)
        XCTAssertEqual(meeting.deviceId, "Hp7AHkXx")
        XCTAssertNil(meeting.endUnixMs)
        XCTAssertEqual(meeting.summaryTldr, "We agreed.")
        XCTAssertEqual(meeting.summaryDecisions, ["Ship it"])
        XCTAssertEqual(meeting.alsoRecorded?.map(\.id), [258])
    }

    func testMeetingWithoutSummaryFieldsStillDecodes() throws {
        let meeting: PhoneMeeting = try decode(#"{"id":1,"device_id":"d","start_unix_ms":5,"title":"T","status":"completed"}"#)
        XCTAssertEqual(meeting.attendees, [])
        XCTAssertEqual(meeting.summaryDecisions, [])
        XCTAssertNil(meeting.summaryTldr)
        XCTAssertNil(meeting.alsoRecorded)
    }

    func testRoutineRunWithOriginDevice() throws {
        let run: PhoneRoutineRun = try decode("""
        {"id":4,"routine_id":2,"routine_name":"Daily Brief","ran_at_unix_ms":900,"status":"success",
         "output_summary":"Today: 3 meetings.","output_json":"{}","origin_device_id":"ZmpQhobe"}
        """)
        XCTAssertEqual(run.routineName, "Daily Brief")
        XCTAssertEqual(run.outputSummary, "Today: 3 meetings.")
        XCTAssertEqual(run.originDeviceId, "ZmpQhobe")
    }

    func testPrepBriefIsDecodedFromItsJsonString() throws {
        let prep: PhoneMeetingPrep = try decode("""
        {"event_id":"evt-1","title":"Standup","start_unix_ms":2000,"generated_at_unix_ms":1500,
         "brief_json":"{\\"summary\\":{\\"text\\":\\"Review the launch.\\",\\"sources\\":[]},\\"attendees\\":[{\\"who\\":\\"Ann\\",\\"note\\":\\"Owns QA\\",\\"sources\\":[]}],\\"open_items\\":[{\\"text\\":\\"Sign-off\\",\\"sources\\":[]}],\\"open_questions\\":[],\\"sources\\":[]}",
         "provider":"anthropic","model":"m"}
        """)
        XCTAssertEqual(prep.eventId, "evt-1")
        XCTAssertEqual(prep.brief?.summary.text, "Review the launch.")
        XCTAssertEqual(prep.brief?.attendees.first?.who, "Ann")
        XCTAssertEqual(prep.brief?.openItems.map(\.text), ["Sign-off"])
    }

    func testTranscriptPageActionItemAndCalendarEvent() throws {
        let page: PhoneTranscriptPage = try decode("""
        {"segments":[{"id":1,"meeting_id":9,"meeting_title":"T","start_cs":150,"speaker_label":"Speaker 1","text":"Hello"}],"total":42}
        """)
        XCTAssertEqual(page.total, 42)
        XCTAssertEqual(page.segments.first?.speakerLabel, "Speaker 1")
        XCTAssertEqual(page.segments.first?.startCs, 150)
        let item: PhoneActionItem = try decode(#"{"id":3,"meeting_id":9,"text":"Send notes","completed":true,"order_index":0,"source":"generated","created_unix_ms":1,"updated_unix_ms":2}"#)
        XCTAssertEqual(item.text, "Send notes")
        XCTAssertTrue(item.completed)
        let event: PhoneCalendarEvent = try decode(#"{"external_id":"e1","source":"google_calendar","title":"Standup","start_unix_ms":2000,"attendees":["a@x.com"]}"#)
        XCTAssertEqual(event.externalId, "e1")
        XCTAssertEqual(event.attendees, ["a@x.com"])
    }
}
