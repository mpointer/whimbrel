import XCTest
import SwiftUI
@testable import Whimbrel

final class DesignSystemComponentTests: XCTestCase {
    func testWhimbrelPillButtonStyleHasThreeKinds() {
        XCTAssertNotNil(WhimbrelPillButtonStyle(kind: .primary))
        XCTAssertNotNil(WhimbrelPillButtonStyle(kind: .secondary))
        XCTAssertNotNil(WhimbrelPillButtonStyle(kind: .quiet))
    }

    func testWhimbrelSourceChipIsNonInteractiveWithoutOnTap() {
        let chip = WhimbrelSourceChip(systemImage: "calendar", text: "Local calendar", onTap: nil)
        XCTAssertNil(chip.onTap)
    }

    func testWhimbrelEmptyStateRendersWithoutAction() {
        let state = WhimbrelEmptyState(systemImage: "calendar.badge.exclamationmark", title: "No calendar connected", message: "Connect a calendar to see upcoming meetings here.")
        XCTAssertNil(state.action)
        XCTAssertNil(state.actionTitle)
    }

    func testBodyScaleSizes() {
        XCTAssertEqual(Font.whimbrelPointSize(for: .body), 14)
        XCTAssertEqual(Font.whimbrelPointSize(for: .callout), 13)
        XCTAssertEqual(Font.whimbrelPointSize(for: .subheadline), 12)
        XCTAssertEqual(Font.whimbrelPointSize(for: .footnote), 12)
        XCTAssertEqual(Font.whimbrelPointSize(for: .caption), 11)
        XCTAssertEqual(Font.whimbrelPointSize(for: .caption2), 11)
    }
}
