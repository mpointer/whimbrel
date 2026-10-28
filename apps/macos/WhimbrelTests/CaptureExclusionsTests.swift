import XCTest
@testable import Whimbrel

@MainActor
final class CaptureExclusionsTests: XCTestCase {
    func test_lock_screen_bundle_ids_are_always_excluded_even_without_user_opt_in() {
        XCTAssertTrue(CaptureExclusions.shared.isExcluded("com.apple.loginwindow"))
        XCTAssertTrue(CaptureExclusions.shared.isExcluded("com.apple.ScreenSaver.Engine"))
    }

    func test_lock_screen_bundle_ids_cannot_be_removed_from_exclusion() {
        CaptureExclusions.shared.setExcluded(false, for: "com.apple.loginwindow")
        XCTAssertTrue(CaptureExclusions.shared.isExcluded("com.apple.loginwindow"))
    }

    func test_whimbrel_never_captures_its_own_windows() {
        XCTAssertTrue(CaptureExclusions.shared.isExcluded("com.mpointer.kestrel"))
        CaptureExclusions.shared.setExcluded(false, for: "com.mpointer.kestrel")
        XCTAssertTrue(CaptureExclusions.shared.isExcluded("com.mpointer.kestrel"))
    }

    func test_ordinary_apps_are_not_excluded_by_default() {
        XCTAssertFalse(CaptureExclusions.shared.isExcluded("com.example.SomeOrdinaryApp"))
    }
}
