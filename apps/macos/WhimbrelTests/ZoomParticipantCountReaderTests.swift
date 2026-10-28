import XCTest
@testable import Whimbrel

/// `AXUIElement` is a live, system-owned opaque type with no public initializer, so
/// (matching the existing precedent set by `AXTextWalker`/`ScreenTextCaptureService`,
/// which also have no unit tests) a synthetic AX tree can't reasonably be constructed
/// here. What CAN be — and is — tested directly is the string-parsing logic that
/// interprets Zoom's real `"Participants (N)"` format, found via live inspection of
/// an actual Zoom meeting (see this task's report). The tree-walk itself was verified
/// against that real, running meeting rather than via a unit test.
final class ZoomParticipantCountReaderTests: XCTestCase {
    func testParsesTheRealSoloCallFormat() {
        XCTAssertEqual(ZoomParticipantCountReader.parseParticipantCount(from: "Participants (1)"), 1)
    }

    func testParsesAMultiDigitCount() {
        XCTAssertEqual(ZoomParticipantCountReader.parseParticipantCount(from: "Participants (42)"), 42)
    }

    func testReturnsNilForTextWithNoCount() {
        XCTAssertNil(ZoomParticipantCountReader.parseParticipantCount(from: "Participants"))
    }

    func testReturnsNilForUnrelatedText() {
        XCTAssertNil(ZoomParticipantCountReader.parseParticipantCount(from: "Mute all"))
    }

    func testReturnsNilForEmptyParentheses() {
        XCTAssertNil(ZoomParticipantCountReader.parseParticipantCount(from: "Participants ()"))
    }

    func testReturnsNilForNonNumericContents() {
        XCTAssertNil(ZoomParticipantCountReader.parseParticipantCount(from: "Participants (waiting)"))
    }
}
