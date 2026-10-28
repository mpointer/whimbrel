import EventKit
import XCTest
@testable import Whimbrel

final class CalendarSelfResponseTests: XCTestCase {
    func test_status_mapping() {
        XCTAssertEqual(CalendarService.selfResponse(from: .accepted), .accepted)
        XCTAssertEqual(CalendarService.selfResponse(from: .tentative), .tentative)
        XCTAssertEqual(CalendarService.selfResponse(from: .pending), .needsAction)
        XCTAssertEqual(CalendarService.selfResponse(from: .unknown), .needsAction)
        XCTAssertEqual(CalendarService.selfResponse(from: .declined), .declined)
        XCTAssertNil(CalendarService.selfResponse(from: .delegated))
    }

    func test_codable_round_trip_with_new_fields() throws {
        let base = #"{"id":"e1","title":"Sync","start_unix_ms":1000,"end_unix_ms":2000,"attendees":["Ana"],"conference_link":null,"source":"calendar","app_bundle_id":null"#
        // Older payloads lack the new keys.
        let old = try JSONDecoder().decode(QualifyingMeeting.self, from: Data((base + "}").utf8))
        XCTAssertNil(old.organizerEmail)
        XCTAssertNil(old.organizerName)
        XCTAssertFalse(old.selfIsOrganizer)
        XCTAssertNil(old.selfResponse)

        let new = try JSONDecoder().decode(QualifyingMeeting.self, from: Data((base + #","organizer_email":"a@x.com","organizer_name":"Ana Ruiz","self_is_organizer":true,"self_response":"needs_action"}"#).utf8))
        XCTAssertEqual(new.organizerEmail, "a@x.com")
        XCTAssertEqual(new.organizerName, "Ana Ruiz")
        XCTAssertTrue(new.selfIsOrganizer)
        XCTAssertEqual(new.selfResponse, .needsAction)
        XCTAssertEqual(try JSONDecoder().decode(QualifyingMeeting.self, from: JSONEncoder().encode(new)), new)
    }
}
