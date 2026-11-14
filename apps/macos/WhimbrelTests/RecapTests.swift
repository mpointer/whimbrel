import XCTest
@testable import Whimbrel

final class RecapTests: XCTestCase {
    func testToHeaderKeepsUsableAddressesOnceInOrder() {
        XCTAssertEqual(RecapSender.toHeader(["ana@x.com", " ANA@x.com ", "bo@y.org"]), "ana@x.com, bo@y.org")
        XCTAssertNil(RecapSender.toHeader([]))
        XCTAssertNil(RecapSender.toHeader(["no at sign", "a@b@c"]))
    }

    func testAnAddressThatCouldEndTheHeaderIsRefused() {
        XCTAssertFalse(RecapSender.isSendable("eve@x.com\r\nBcc: z@z.com"))
        XCTAssertFalse(RecapSender.isSendable("a@x.com, b@y.com"))
        XCTAssertFalse(RecapSender.isSendable("Ana <ana@x.com>"))
        XCTAssertFalse(RecapSender.isSendable("@x.com"))
        XCTAssertTrue(RecapSender.isSendable("b+c@y.org"))
    }

    func testAddressesAreTheTickedInviteesThenTheAddedOnesOnce() {
        let invitees = [CoreAttendee(name: "Ana", email: "ana@x.com"), CoreAttendee(name: "Bo", email: "bo@y.org"), CoreAttendee(name: "Cy", email: nil)]
        XCTAssertEqual(
            RecapDraft.addresses(invitees: invitees, ticked: ["bo@y.org"], added: ["dee@z.com", "BO@y.org"]),
            ["bo@y.org", "dee@z.com"]
        )
    }

    func testTheSubjectNamesTheMeetingAndItsDay() {
        // 2026-11-14 12:00 UTC
        let subject = RecapDraft.subject(title: "  Q3 planning ", startUnixMs: 1_794_571_200_000, locale: Locale(identifier: "en_US"))
        XCTAssertTrue(subject.hasPrefix("Meeting recap: Q3 planning ("), subject)
        XCTAssertTrue(subject.contains("2026"), subject)
    }

    func testTheEmailBodyComesFromCoreWithTheFooter() {
        let summary = CoreRecapSummary(title: "Sync", tldr: "Done.", decisions: [], actionItems: ["Ana: ship it"], openQuestions: [])
        let labels = CoreRecapLabels(decisions: "Decisions", actionItems: "Action items", openQuestions: "Open questions", footer: "Sent with Whimbrel")
        XCTAssertEqual(
            CoreMemoryStore.recapRender(summary, labels: labels, format: "email"),
            "Done.\n\nAction items:\n• Ana: ship it\n\n--\nSent with Whimbrel: https://whimbrelapp.com"
        )
        XCTAssertEqual(CoreMemoryStore.recapRender(summary, labels: labels, format: "slack"), "*Sync*\n\nDone.\n\n*Action items*\n• Ana: ship it")
        XCTAssertNil(CoreMemoryStore.recapRender(summary, labels: labels, format: "html"))
    }

    func testTheMailtoDraftFallsBackToTheClipboardWhenLong() {
        let short = CoreMemoryStore.recapMailto(to: ["ana@x.com"], subject: "Hi", body: "Body", pasteHint: "Paste")
        XCTAssertEqual(short, CoreMailtoDraft(url: "mailto:ana%40x.com?subject=Hi&body=Body", bodyInUrl: true))
        let long = CoreMemoryStore.recapMailto(to: ["ana@x.com"], subject: "Hi", body: String(repeating: "word ", count: 600), pasteHint: "Paste")
        XCTAssertEqual(long?.bodyInUrl, false)
        XCTAssertEqual(long?.url, "mailto:ana%40x.com?subject=Hi&body=Paste")
    }
}
