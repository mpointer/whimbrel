import XCTest
@testable import Whimbrel

/// The decisions behind removing a Google account: when the grant is revoked, that the local
/// sign-out never depends on the revoke, and what the confirmation says. The revoke request
/// itself (form body, endpoint) is tested in `core/api/src/oauth.rs`; the data deletion in
/// `core/memory`.
final class GoogleDisconnectTests: XCTestCase {
    func testGrantIsSharedOnlyWhileAnotherConnectionUsesTheSameAddress() {
        // Removing Gmail while Calendar signs in as the same address (case-insensitive).
        XCTAssertTrue(GoogleDisconnect.grantIsShared(
            removing: .gmail, email: "Mike@Example.com", gmailEmails: ["mike@example.com"],
            calendarEmails: ["mike@example.com"], syncEmail: nil))
        // Another account is no reason to wait.
        XCTAssertFalse(GoogleDisconnect.grantIsShared(
            removing: .gmail, email: "mike@example.com", gmailEmails: ["mike@example.com"],
            calendarEmails: ["other@example.com"], syncEmail: nil))
        // The same service's own list never counts: it is the one being removed.
        XCTAssertFalse(GoogleDisconnect.grantIsShared(
            removing: .calendar, email: "mike@example.com", gmailEmails: [],
            calendarEmails: ["mike@example.com"], syncEmail: nil))
        // The sync sign-in holds a token for the same grant.
        XCTAssertTrue(GoogleDisconnect.grantIsShared(
            removing: .calendar, email: "mike@example.com", gmailEmails: [],
            calendarEmails: ["mike@example.com"], syncEmail: "MIKE@example.com"))
    }

    func testLocalSignOutHappensFirstAndSurvivesAFailedRevoke() {
        var events: [String] = []
        var revoked: [String] = []
        let reason = GoogleDisconnect.disconnect(
            refreshToken: "refresh-1", revoke: true,
            deleteLocal: { events.append("delete") },
            startRevoke: { token in
                events.append("revoke")
                // Offline: the core answers "unreachable".
                revoked.append(GoogleDisconnect.revokeNow(token, revoker: { _ in "unreachable" }))
            })
        XCTAssertNil(reason)
        XCTAssertEqual(events, ["delete", "revoke"])
        XCTAssertEqual(revoked, ["unreachable"])
    }

    func testAnUnavailableRevokeIsALogCodeNotAFailure() {
        XCTAssertEqual(GoogleDisconnect.revokeNow("t", revoker: { _ in nil }), "revoke_unavailable")
    }

    func testNoRevokeIsStartedWhenTheGrantIsSharedOrThereIsNoToken() {
        var deleted = 0
        var started = 0
        XCTAssertEqual(
            GoogleDisconnect.disconnect(refreshToken: "t", revoke: false, deleteLocal: { deleted += 1 }, startRevoke: { _ in started += 1 }),
            "grant_shared")
        XCTAssertEqual(
            GoogleDisconnect.disconnect(refreshToken: nil, revoke: true, deleteLocal: { deleted += 1 }, startRevoke: { _ in started += 1 }),
            "nothing_to_revoke")
        XCTAssertEqual(
            GoogleDisconnect.disconnect(refreshToken: "", revoke: true, deleteLocal: { deleted += 1 }, startRevoke: { _ in started += 1 }),
            "nothing_to_revoke")
        XCTAssertEqual(deleted, 3, "the local sign-out runs every time")
        XCTAssertEqual(started, 0)
    }

    func testConfirmationSaysWhatIsRemovedAndWhetherAccessIsRevoked() {
        let all = [
            GoogleDisconnect.confirmationMessage(for: .gmail, revokes: true),
            GoogleDisconnect.confirmationMessage(for: .gmail, revokes: false),
            GoogleDisconnect.confirmationMessage(for: .calendar, revokes: true),
            GoogleDisconnect.confirmationMessage(for: .calendar, revokes: false),
        ]
        XCTAssertEqual(Set(all).count, 4)
        for message in all {
            XCTAssertTrue(message.contains("from this Mac"), message)
            XCTAssertFalse(message.contains("\u{2014}") || message.contains("\u{2013}"), "no dashes: \(message)")
        }
        XCTAssertTrue(all[0].contains("Gmail data") && all[0].contains("revokes Whimbrel's access"))
        XCTAssertTrue(all[2].contains("Calendar events") && all[2].contains("revokes Whimbrel's access"))
        XCTAssertFalse(all[1].contains("revokes") || all[3].contains("revokes"), "a shared grant is not revoked, so the text must not say it is")
        XCTAssertTrue(all[2].contains("Meetings you recorded are kept"))
    }
}
