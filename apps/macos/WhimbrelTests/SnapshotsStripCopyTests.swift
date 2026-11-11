import XCTest
@testable import Whimbrel

final class SnapshotsStripCopyTests: XCTestCase {
    func testToggleLabel() {
        XCTAssertEqual(SnapshotsStripCopy.keepLabel, "Keep slides at full size")
    }

    func testExplanationWhenShrinkIsOn() {
        XCTAssertEqual(
            SnapshotsStripCopy.explanation(shrinkAfterDays: 30, keepFullImagesForever: false),
            "Otherwise slides are shrunk after 30 days (Settings → Retention). Kept slides stay even if the meeting is deleted."
        )
    }

    func testExplanationWhenKeepForeverIsOn() {
        XCTAssertEqual(
            SnapshotsStripCopy.explanation(shrinkAfterDays: 30, keepFullImagesForever: true),
            "Kept slides stay even if the meeting is deleted."
        )
    }
}
