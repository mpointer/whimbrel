import XCTest
@testable import Whimbrel

final class MeetingHeuristicDetectorTests: XCTestCase {
    private func provider(_ url: String?) -> MeetingProvider? {
        MeetingHeuristicDetector.meetingURLProvider(url)
    }

    func testMatchesARealMeetURL() {
        XCTAssertEqual(provider("https://meet.google.com/abc-defg-hij"), .googleMeet)
    }

    func testMatchesWithQueryParamsAndFragment() {
        XCTAssertEqual(provider("https://meet.google.com/abc-defg-hij?authuser=0#fragment"), .googleMeet)
    }

    func testMatchesTeamsAndZoomWebClient() {
        XCTAssertEqual(provider("https://teams.microsoft.com/l/meetup-join/19%3ameeting"), .teams)
        XCTAssertEqual(provider("https://teams.live.com/meet/9876"), .teams)
        XCTAssertEqual(provider("https://app.zoom.us/wc/123456789/join"), .zoom)
    }

    func testZoomOutsideTheWebClientIsNotAMeeting() {
        // zoom.us pages that aren't the /wc/ web client (the marketing site, a join
        // page that hands off to the native app) aren't a call in this tab.
        XCTAssertNil(provider("https://zoom.us/pricing"))
    }

    func testRejectsNil() {
        XCTAssertNil(provider(nil))
    }

    func testRejectsPlainNonURLString() {
        XCTAssertNil(provider("not a url at all"))
    }

    func testRejectsUnrelatedDomainThatMentionsMeetGoogleComInAPath() {
        // The whole point of host-based matching, not substring matching: this string
        // literally contains "meet.google.com" but is not that host.
        XCTAssertNil(provider("https://evil.example.com/?redirect=meet.google.com"))
    }

    func testRejectsSubdomainSpoof() {
        // "meet.google.com.evil.example.com" is a different host entirely.
        XCTAssertNil(provider("https://meet.google.com.evil.example.com/abc"))
    }

    func testRejectsOtherGoogleDomains() {
        XCTAssertNil(provider("https://docs.google.com/document/d/abc"))
    }
}
