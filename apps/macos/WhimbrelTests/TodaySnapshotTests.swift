import SwiftUI
import XCTest
@testable import Whimbrel

@MainActor
final class TodaySnapshotTests: XCTestCase {
    private let size = CGSize(width: 900, height: 1400)

    /// Whimbrel and its default palette pinned, so the baselines never read the skin persisted in this
    /// machine's defaults (the test host is the app, on `.standard`).
    nonisolated override func invokeTest() {
        MainActor.assumeIsolated {
            SkinStore.shared.withOverride(skin: SkinDefaults.whimbrel, paletteId: "default") { super.invokeTest() }
        }
    }

    private func standard(_ content: TodayContent) -> some View {
        StandardTodayLayout(
            content: content, actions: .noop,
            slots: TodaySlots(live: nil, decisionCount: content.pendingApprovalCount, actions: .noop, composer: TodayFixtures.composer)
        )
    }

    func test_standard_sample_light() throws {
        try assertSnapshot(standard(.sample), named: "standard-sample-light", size: size, colorScheme: .light, tolerance: 0.0001)
    }

    func test_standard_sample_dark() throws {
        try assertSnapshot(standard(.sample), named: "standard-sample-dark", size: size, colorScheme: .dark, tolerance: 0.0001)
    }
}
