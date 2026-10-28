import XCTest
@testable import Whimbrel

/// `AXUIElement` is a live, system-owned opaque type with no public initializer, so
/// (matching the existing precedent set by `AXTextWalker`/`ZoomParticipantCountReader`,
/// which also have no unit tests for this reason) a synthetic AX tree can't reasonably
/// be constructed here. What CAN be — and is — tested directly is the string-parsing
/// logic for both real formats found via live inspection of an actual Google Meet
/// call, in both Chrome and Safari (see this task's report). The tree-walk itself was
/// verified against that real, running meeting rather than via a unit test.
final class GoogleMeetParticipantCountReaderTests: XCTestCase {
    // MARK: - parseContributorsCount (the People panel's "Contributors N" header,
    // confirmed in both Chrome and Safari once the panel has been opened)

    func testParsesTheRealSoloCallContributorsFormat() {
        XCTAssertEqual(GoogleMeetParticipantCountReader.parseContributorsCount(from: "Contributors 1"), 1)
    }

    func testParsesAMultiDigitContributorsCount() {
        XCTAssertEqual(GoogleMeetParticipantCountReader.parseContributorsCount(from: "Contributors 42"), 42)
    }

    func testContributorsReturnsNilForTextWithNoCount() {
        XCTAssertNil(GoogleMeetParticipantCountReader.parseContributorsCount(from: "Contributors"))
    }

    func testContributorsReturnsNilForUnrelatedText() {
        XCTAssertNil(GoogleMeetParticipantCountReader.parseContributorsCount(from: "People"))
    }

    func testContributorsReturnsNilForNonNumericContents() {
        XCTAssertNil(GoogleMeetParticipantCountReader.parseContributorsCount(from: "Contributors many"))
    }

    // MARK: - parseBareDigitCount (Chrome's passive toolbar badge, opportunistic
    // fallback only — confirmed absent in Safari, the browser this reader actually
    // needs to work in)

    func testParsesTheRealSoloCallBareDigitFormat() {
        XCTAssertEqual(GoogleMeetParticipantCountReader.parseBareDigitCount(from: "1"), 1)
    }

    func testParsesAMultiDigitBareCount() {
        XCTAssertEqual(GoogleMeetParticipantCountReader.parseBareDigitCount(from: "42"), 42)
    }

    func testBareDigitReturnsNilForEmptyString() {
        XCTAssertNil(GoogleMeetParticipantCountReader.parseBareDigitCount(from: ""))
    }

    func testBareDigitReturnsNilForOverflowBadge() {
        // Meet is known to show a truncated overflow badge (e.g. "9+") at high
        // participant counts — not directly observed, only a solo call was
        // available, but this must not silently parse as 9.
        XCTAssertNil(GoogleMeetParticipantCountReader.parseBareDigitCount(from: "9+"))
    }

    func testBareDigitReturnsNilForUnrelatedText() {
        XCTAssertNil(GoogleMeetParticipantCountReader.parseBareDigitCount(from: "People"))
    }

    func testBareDigitReturnsNilForWhitespace() {
        XCTAssertNil(GoogleMeetParticipantCountReader.parseBareDigitCount(from: " 1 "))
    }
}
