import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

private final class WidthBox { var width: CGFloat = 0 }
private struct WidthKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

@MainActor
final class ArchetypeReferenceTests: XCTestCase {
    /// <repo>/skins, found from this file's path (apps/macos/WhimbrelTests/ArchetypeReferenceTests.swift).
    private let skins = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("skins")

    private func skin() throws -> Skin {
        let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(try Data(contentsOf: skins.appendingPathComponent("reference.json")), base: SkinDefaults.whimbrel))
        XCTAssertEqual(repairs, [])
        return skin
    }

    func test_reference_skin_loads_without_repairs() throws {
        let skin = try skin()
        XCTAssertEqual(skin.id, "reference")
        XCTAssertEqual(skin.paletteIds, ["paper"])
        XCTAssertEqual(skin.archetype, .reference)
        XCTAssertEqual(skin.voice.text(.greetingMorning, name: "Ada"), "Good morning, Ada")
    }

    private func palette() throws -> Theme {
        let data = try Data(contentsOf: skins.appendingPathComponent("palettes/paper.json"))
        return try XCTUnwrap(ThemeLoader.parse(data), "paper")
    }

    func test_reference_palettes_pass_contrast() throws {
        let theme = try palette()
        XCTAssertNotEqual(theme.light, theme.dark, "paper needs its own light and dark (night) variants")
        XCTAssertNotEqual(theme.dark, ThemeDefaults.theme.dark, "paper must not have fallen back to Default")
        for (mode, p) in [("light", theme.light), ("dark", theme.dark)] {
            XCTAssertEqual(Set(p.keys), Set(ThemeToken.allCases), mode)
            XCTAssertEqual(ThemeContrast.failures(p), [], mode)
            XCTAssertTrue(ThemeContrast.passes(p), mode)
            // The layout also draws accent, error and link text on the page and on the card.
            XCTAssertEqual(ArchetypeContrast.failures(p, ArchetypeContrast.pairs(for: .reference)), [], mode)
        }
    }

    /// `[Start]` is the primary action, so it takes the primary role colour; other links keep the link colour.
    /// accentPrimary on page and card is in the contrast loop above.
    func test_reference_start_uses_the_primary_role_colour() {
        XCTAssertEqual(ReferenceLink.Role.primary.token, .accentPrimary)
        XCTAssertEqual(ReferenceLink.Role.link.token, .link)
    }

    func test_reference_infobox_placement_threshold() {
        for w in [0, 660, 672, 692, 699.5] { XCTAssertEqual(ReferenceTodayLayout.infoboxPlacement(width: CGFloat(w)), .stacked, "\(w)") }
        for w in [700, 712, 760] { XCTAssertEqual(ReferenceTodayLayout.infoboxPlacement(width: CGFloat(w)), .floated, "\(w)") }
    }

    func test_reference_places_expected_sections() {
        XCTAssertEqual(ReferenceTodayLayout.placedSections, Set(TodaySection.allCases).subtracting([.followups]))
        // Honest placement: only follow-ups (deliberately TodayChrome's) are left for the leftovers list.
        for content in [TodayContent.sample, .worstCase, .empty] {
            XCTAssertTrue(Set(content.nonEmptySections()).subtracting([.followups]).isSubset(of: ReferenceTodayLayout.placedSections))
            XCTAssertEqual(TodayChrome.leftovers(of: content, placed: ReferenceTodayLayout.placedSections), content.followups.isEmpty ? [] : [.followups])
        }
    }

    /// Runs `body` with Reference and `palette` in force. The palettes are installed in the
    /// theme store for the duration, since the test host does not load `skins/palettes`.
    private func withReference<T>(_ body: () throws -> T) throws -> T {
        let store = ThemeStore.shared
        let before = store.themes
        store.install(before + ThemeLoader.loadDirectory(skins.appendingPathComponent("palettes")))
        defer { store.install(before) }
        return try SkinStore.shared.withOverride(skin: try skin(), paletteId: "paper", body)
    }

    private func layout(_ content: TodayContent) -> ReferenceTodayLayout {
        ReferenceTodayLayout(content: content, actions: .noop, composer: TodayFixtures.composer)
    }

    /// Today's content width at the 900pt minimum window: 672 at the ideal sidebar, 692 at its
    /// minimum; 660 is the brief's stricter figure; 760 is the widest `TodayChrome` allows.
    private let widths: [CGFloat] = [660, 672, 692, 700, 712, 760]

    func test_reference_no_horizontal_overflow() throws {
        try withReference {
            for content in [TodayContent.worstCase, .sample, .empty] {
                for width in widths {
                    let measured = renderedWidth(layout(content), proposing: width)
                    XCTAssertLessThanOrEqual(measured, width + 0.5, "content wider than \(width)")
                }
            }
        }
    }

    /// The width the layout takes when offered `width`: a layout that cannot shrink to it reports more.
    private func renderedWidth<V: View>(_ view: V, proposing width: CGFloat) -> CGFloat {
        let box = WidthBox()
        let probe = view
            .fixedSize(horizontal: false, vertical: true)
            .background(GeometryReader { Color.clear.preference(key: WidthKey.self, value: $0.size.width) })
            .frame(width: width, alignment: .leading)
            .onPreferenceChange(WidthKey.self) { box.width = $0 }
        let host = NSHostingView(rootView: probe)
        host.frame = CGRect(x: 0, y: 0, width: width, height: 6000)
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        host.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(box.width, 0, "the probe measured nothing")
        return box.width
    }

    /// The probe must see an overflow, or the test above proves nothing.
    func test_overflow_probe_detects_a_rigid_wide_view() {
        XCTAssertGreaterThan(renderedWidth(Text(String(repeating: "x", count: 200)).fixedSize(), proposing: 660), 660)
        XCTAssertLessThanOrEqual(renderedWidth(Text(String(repeating: "x ", count: 200)), proposing: 660), 660.5)
    }

    // MARK: snapshots (local only)

    private func page(_ content: TodayContent) -> some View {
        ScrollView { layout(content).padding(Spacing.xl).frame(maxWidth: 760, alignment: .leading).frame(maxWidth: .infinity) }
            .background(Color.whimbrelBackgroundPage)
    }

    func test_reference_sample_light() throws {
        try withReference { try assertSnapshot(page(.sample), named: "reference-sample-light", size: CGSize(width: 900, height: 1800), colorScheme: .light) }
    }

    func test_reference_sample_dark() throws {
        try withReference { try assertSnapshot(page(.sample), named: "reference-sample-dark", size: CGSize(width: 900, height: 1800), colorScheme: .dark) }
    }

    func test_reference_worstcase_light() throws {
        try withReference { try assertSnapshot(page(.worstCase), named: "reference-worstcase-light", size: CGSize(width: 900, height: 2800), colorScheme: .light) }
    }

    func test_reference_sample_stacked_light() throws {
        try withReference { try assertSnapshot(page(.sample), named: "reference-sample-stacked-light", size: CGSize(width: 680, height: 1800), colorScheme: .light) }
    }

    /// With no width proposed (an ideal-size query) the layout used 0, which collapsed it.
    func test_intro_layout_with_no_width_uses_children_ideal_widths() {
        let host = NSHostingView(rootView: ReferenceIntroLayout {
            Color.red.frame(width: 100, height: 20)
            Color.blue.frame(width: 150, height: 30)
        })
        let size = host.fittingSize
        XCTAssertEqual(size.width, 150, accuracy: 0.5)
        XCTAssertEqual(size.height, 20 + Spacing.lg + 30, accuracy: 0.5)
    }
}
