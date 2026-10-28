import XCTest
@testable import Whimbrel

final class MeetingPrepDecodeTests: XCTestCase {
    func testAStoredBriefDecodesIncludingItsNestedBriefJSON() throws {
        let brief = #"{"summary":{"text":"Follow up on pricing.","sources":["M1"]},"attendees":[{"who":"Ben Lawrick","note":"Sent the deck.","sources":["E1"],"last_interaction_unix_ms":5000}],"open_items":[{"text":"Ben sends the deck","sources":["M1"]}],"open_questions":[],"sources":[{"id":"M1","kind":"meeting","title":"Acme kickoff","ts_unix_ms":1000,"meeting_id":7,"thread_id":null},{"id":"E1","kind":"email","title":"Re: pricing","ts_unix_ms":2000,"meeting_id":null,"thread_id":9}]}"#
        let row: [String: Any] = [
            "event_id": "evt-1", "title": "Acme renewal", "start_unix_ms": 100, "generated_at_unix_ms": 90,
            "brief_json": brief, "provider": "ollama", "model": "m",
        ]
        let data = try JSONSerialization.data(withJSONObject: row)

        let prep = try JSONDecoder().decode(CoreMeetingPrep.self, from: data)

        XCTAssertEqual(prep.eventId, "evt-1")
        let decoded = try XCTUnwrap(prep.brief)
        XCTAssertEqual(decoded.summary.text, "Follow up on pricing.")
        XCTAssertEqual(decoded.attendees.first?.who, "Ben Lawrick")
        XCTAssertEqual(decoded.attendees.first?.lastInteractionUnixMs, 5000)
        XCTAssertEqual(decoded.sources.map(\.id), ["M1", "E1"])
        XCTAssertEqual(decoded.sources[0].meetingId, 7)
        XCTAssertEqual(decoded.sources[1].threadId, 9)
    }

    func testABriefWithBrokenNestedJSONHasNoBriefInsteadOfFailing() throws {
        let row: [String: Any] = ["event_id": "e", "title": "t", "start_unix_ms": 1, "generated_at_unix_ms": 1, "brief_json": "not json", "provider": "p", "model": "m"]
        let prep = try JSONDecoder().decode(CoreMeetingPrep.self, from: JSONSerialization.data(withJSONObject: row))
        XCTAssertNil(prep.brief)
    }
}
