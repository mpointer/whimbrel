import XCTest
@testable import Whimbrel

/// `AXUIElement` is a live, system-owned opaque type with no public initializer, so
/// (matching the existing precedent set by `AXTextWalker`/`ZoomParticipantCountReader`/
/// `GoogleMeetParticipantCountReader`) a synthetic AX tree can't reasonably be
/// constructed here. What CAN be — and is — tested directly is the string-parsing
/// logic that interprets Teams's real `"In this meeting (N)"` /
/// `"In this meeting, N total"` formats, found via live inspection of an actual Teams
/// meeting (see this task's report). The tree-walk itself was verified against that
/// real, running meeting rather than via a unit test — see the report for the
/// significant finding there about Teams's WebView2 renderer requiring an
/// `AXEnhancedUserInterface` activation, which this reader now actively triggers
/// itself (on the pid-derived application element, before each read) per an
/// explicit decision from the human partner.
final class TeamsParticipantCountReaderTests: XCTestCase {
    func testParsesTheRealSoloCallValueFormat() {
        XCTAssertEqual(TeamsParticipantCountReader.parseInThisMeetingValue(from: "In this meeting (1)"), 1)
    }

    func testParsesAMultiDigitValueFormat() {
        XCTAssertEqual(TeamsParticipantCountReader.parseInThisMeetingValue(from: "In this meeting (42)"), 42)
    }

    func testValueFormatReturnsNilForTextWithNoCount() {
        XCTAssertNil(TeamsParticipantCountReader.parseInThisMeetingValue(from: "In this meeting"))
    }

    func testValueFormatReturnsNilForUnrelatedText() {
        XCTAssertNil(TeamsParticipantCountReader.parseInThisMeetingValue(from: "Leave"))
    }

    func testValueFormatReturnsNilForEmptyParentheses() {
        XCTAssertNil(TeamsParticipantCountReader.parseInThisMeetingValue(from: "In this meeting ()"))
    }

    func testValueFormatReturnsNilForNonNumericContents() {
        XCTAssertNil(TeamsParticipantCountReader.parseInThisMeetingValue(from: "In this meeting (waiting)"))
    }

    func testParsesTheRealSoloCallTotalFormat() {
        XCTAssertEqual(TeamsParticipantCountReader.parseInThisMeetingTotal(from: "In this meeting, 1 total"), 1)
    }

    func testParsesAMultiDigitTotalFormat() {
        XCTAssertEqual(TeamsParticipantCountReader.parseInThisMeetingTotal(from: "In this meeting, 42 total"), 42)
    }

    func testTotalFormatReturnsNilForUnrelatedText() {
        XCTAssertNil(TeamsParticipantCountReader.parseInThisMeetingTotal(from: "Michael Pointer, Organizer, Unmuted"))
    }

    func testTotalFormatReturnsNilForNonNumericContents() {
        XCTAssertNil(TeamsParticipantCountReader.parseInThisMeetingTotal(from: "In this meeting, several total"))
    }
}
