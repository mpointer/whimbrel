import XCTest
@testable import Whimbrel

final class EpisodeOptionsMenuTests: XCTestCase {
    func testMenuLabelsMatchControlledVocabulary() {
        // Guards against copy drift from the UX Spec's exact required labels.
        XCTAssertEqual(EpisodeOptionsMenu.regenerateLabel, "Regenerate summary")
        XCTAssertEqual(EpisodeOptionsMenu.hideLabel, "Hide episode")
        XCTAssertEqual(EpisodeOptionsMenu.unhideLabel, "Unhide episode")
        XCTAssertEqual(EpisodeOptionsMenu.forgetLabel, "Forget this activity range…")
    }
}
