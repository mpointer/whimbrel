import XCTest
@testable import Whimbrel

final class LiveNotesMergeTests: XCTestCase {
    func testOnlyLibraryKeepsLibrary() {
        XCTAssertEqual(LiveNotesMerge.combined(library: "a\nb", draft: nil), "a\nb")
        XCTAssertEqual(LiveNotesMerge.combined(library: "a\nb", draft: "  \n"), "a\nb")
    }

    func testOnlyTodayDraftIsUsed() {
        XCTAssertEqual(LiveNotesMerge.combined(library: nil, draft: "typed on Today"), "typed on Today")
        XCTAssertEqual(LiveNotesMerge.combined(library: "", draft: "typed on Today"), "typed on Today")
        XCTAssertEqual(LiveNotesMerge.combined(library: "  \n", draft: "typed on Today"), "typed on Today")
    }

    func testBothEmptyIsEmpty() {
        XCTAssertEqual(LiveNotesMerge.combined(library: nil, draft: nil), "")
        XCTAssertEqual(LiveNotesMerge.combined(library: "", draft: ""), "")
    }

    func testSameTextIsNotDuplicated() {
        XCTAssertEqual(LiveNotesMerge.combined(library: "a\nb", draft: "a\nb"), "a\nb")
        XCTAssertEqual(LiveNotesMerge.combined(library: "a\nb", draft: "a\nb\n"), "a\nb")
        XCTAssertEqual(LiveNotesMerge.combined(library: "a\nb", draft: " b \na"), "a\nb")
    }

    func testDifferentTextKeepsBothLibraryFirst() {
        XCTAssertEqual(LiveNotesMerge.combined(library: "from library", draft: "from today"), "from library\nfrom today")
    }

    func testOverlapKeepsOnlyTheNewDraftLines() {
        XCTAssertEqual(LiveNotesMerge.combined(library: "L1\nT1", draft: "T1\nT2"), "L1\nT1\nT2")
    }

    func testALineThatDiffersByAWordIsKept() {
        XCTAssertEqual(LiveNotesMerge.combined(library: "ship friday", draft: "ship monday"), "ship friday\nship monday")
    }

    func testRepeatedDraftLineKeepsExtraCopies() {
        XCTAssertEqual(LiveNotesMerge.combined(library: "x", draft: "x\nx"), "x\nx")
    }

    func testBlankLinesBetweenKeptLinesSurvive() {
        XCTAssertEqual(LiveNotesMerge.combined(library: "a", draft: "b\n\nc"), "a\nb\n\nc")
        XCTAssertEqual(LiveNotesMerge.combined(library: "a", draft: "a\n\nc"), "a\nc")
    }

    func testCombiningTheResultAgainChangesNothing() {
        let cases: [(String, String)] = [("L1\nT1", "T1\nT2"), ("a", "b\n\nc"), ("x", "x\nx"), ("a\nb", "b\na\nz")]
        for (lib, draft) in cases {
            let once = LiveNotesMerge.combined(library: lib, draft: draft)
            XCTAssertEqual(LiveNotesMerge.combined(library: once, draft: draft), once, "\(lib) | \(draft)")
        }
    }
}
