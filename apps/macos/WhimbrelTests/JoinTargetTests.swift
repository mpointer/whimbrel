import XCTest
@testable import Whimbrel

final class JoinTargetTests: XCTestCase {
    private func target(_ provider: MeetingProvider, _ link: String) -> JoinTarget? {
        JoinTarget.make(from: DetectedConferenceLink(provider: provider, url: URL(string: link)!))
    }

    func test_zoom_with_password_opens_the_zoom_app() {
        let t = target(.zoom, "https://us02web.zoom.us/j/81234567890?pwd=AbC.1")
        XCTAssertEqual(t?.provider, .zoom)
        XCTAssertEqual(t?.appURL?.absoluteString, "zoommtg://zoom.us/join?action=join&confno=81234567890&pwd=AbC.1")
        XCTAssertEqual(t?.webURL.absoluteString, "https://us02web.zoom.us/j/81234567890?pwd=AbC.1")
    }

    func test_teams_meetup_join_opens_the_teams_app() {
        let link = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22x%22%7d"
        let t = target(.teams, link)
        XCTAssertEqual(t?.provider, .teams)
        XCTAssertEqual(t?.appURL?.absoluteString,
                       "msteams://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22x%22%7d")
        XCTAssertEqual(t?.webURL.absoluteString, link)
    }

    func test_a_non_meeting_teams_link_never_gets_the_app_scheme() {
        let t = target(.teams, "https://teams.microsoft.com/l/chat/0/0?users=someone@example.com")
        XCTAssertNil(t?.appURL)
    }

    func test_teams_live_opens_in_the_browser() {
        let t = target(.teams, "https://teams.live.com/meet/9876543210?p=abc")
        XCTAssertEqual(t?.provider, .teams)
        XCTAssertNil(t?.appURL)
        XCTAssertEqual(t?.webURL.absoluteString, "https://teams.live.com/meet/9876543210?p=abc")
    }

    func test_meet_code_opens_in_the_browser() {
        let t = target(.googleMeet, "https://meet.google.com/abc-defg-hij")
        XCTAssertEqual(t?.provider, .googleMeet)
        XCTAssertNil(t?.appURL)
        XCTAssertEqual(t?.webURL.absoluteString, "https://meet.google.com/abc-defg-hij")
    }

    func test_webex_links_open_in_the_browser() {
        for link in ["https://acme.webex.com/acme/j.php?MTID=m0123456789abcdef",
                     "https://acme.webex.com/meet/jdoe"] {
            let t = target(.webex, link)
            XCTAssertEqual(t?.provider, .webex, link)
            XCTAssertNil(t?.appURL, link)
            XCTAssertEqual(t?.webURL.absoluteString, link)
        }
    }

    func test_missing_app_falls_back_to_the_web_link() {
        let t = target(.zoom, "https://zoom.us/j/81234567890")!
        XCTAssertEqual(t.urlToOpen(appInstalled: { _ in false }).absoluteString, "https://zoom.us/j/81234567890")
        XCTAssertEqual(t.urlToOpen(appInstalled: { _ in true }).scheme, "zoommtg")
    }

    func test_web_only_target_never_asks_about_an_app() {
        let t = target(.googleMeet, "https://meet.google.com/abc-defg-hij")!
        XCTAssertEqual(t.urlToOpen(appInstalled: { _ in XCTFail("no app to check"); return true }).absoluteString,
                       "https://meet.google.com/abc-defg-hij")
    }

    func test_unrecognized_or_unsafe_links_give_no_target() {
        XCTAssertNil(JoinTarget.make(from: nil))
        let cases: [(MeetingProvider, String)] = [
            (.webex, "https://evilwebex.com/meet/jdoe"),
            (.zoom, "https://evilzoom.us/j/1"),
            (.zoom, "https://zoom.us.evil.com/j/1"),
            (.zoom, "https://zoom.us@evil.com/j/1"),
            (.zoom, "https://meet.google.com/abc-defg-hij"),
            (.googleMeet, "http://meet.google.com/abc-defg-hij"),
            (.googleMeet, "javascript://meet.google.com/%0aalert(1)"),
            (.teams, "https://teams.microsoft.com.evil.com/l/meetup-join/x"),
        ]
        for (provider, link) in cases {
            XCTAssertNil(target(provider, link), link)
        }
    }
}
