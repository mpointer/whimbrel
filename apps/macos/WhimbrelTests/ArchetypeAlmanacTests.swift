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
final class ArchetypeAlmanacTests: XCTestCase {
    /// <repo>/skins, found from this file's path (apps/macos/WhimbrelTests/ArchetypeAlmanacTests.swift).
    private let skins = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("skins")

    private func skin() throws -> Skin {
        let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(try Data(contentsOf: skins.appendingPathComponent("almanac.json")), base: SkinDefaults.whimbrel))
        XCTAssertEqual(repairs, [])
        return skin
    }

    func test_almanac_skin_loads_without_repairs() throws {
        let skin = try skin()
        XCTAssertEqual(skin.id, "almanac")
        XCTAssertEqual(skin.paletteIds, ["parchment"])
        XCTAssertEqual(skin.archetype, .almanac)
        XCTAssertEqual(skin.typography.caps, .small)
        XCTAssertEqual(skin.shape.border, .double)
        XCTAssertEqual(skin.voice.text(.greetingMorning, name: "Ada"), "Good morrow, Ada")
    }

    private func palette() throws -> Theme {
        let data = try Data(contentsOf: skins.appendingPathComponent("palettes/parchment.json"))
        return try XCTUnwrap(ThemeLoader.parse(data), "parchment")
    }

    func test_almanac_palettes_pass_contrast() throws {
        let theme = try palette()
        XCTAssertNotEqual(theme.light, theme.dark, "parchment needs its own light and dark variants")
        XCTAssertNotEqual(theme.dark, ThemeDefaults.theme.dark, "parchment must not have fallen back to Default")
        for (mode, p) in [("light", theme.light), ("dark", theme.dark)] {
            XCTAssertEqual(Set(p.keys), Set(ThemeToken.allCases), mode)
            XCTAssertEqual(ThemeContrast.failures(p), [], mode)
            XCTAssertTrue(ThemeContrast.passes(p), mode)
            // The layout also draws accent and error text on the page and on the card.
            XCTAssertEqual(ArchetypeContrast.failures(p, ArchetypeContrast.pairs(for: .almanac)), [], mode)
        }
    }

    func test_almanac_places_expected_sections() {
        XCTAssertEqual(AlmanacTodayLayout.placedSections, Set(TodaySection.allCases).subtracting([.followups]))
        // Honest placement: only follow-ups (deliberately TodayChrome's) are left for the leftovers list.
        for content in [TodayContent.sample, .worstCase, .empty] {
            XCTAssertTrue(Set(content.nonEmptySections()).subtracting([.followups]).isSubset(of: AlmanacTodayLayout.placedSections))
            XCTAssertEqual(TodayChrome.leftovers(of: content, placed: AlmanacTodayLayout.placedSections), content.followups.isEmpty ? [] : [.followups])
        }
    }

    /// Runs `body` with Almanac and `palette` in force. The palettes are installed in the
    /// theme store for the duration, since the test host does not load `skins/palettes`.
    private func withAlmanac<T>(_ body: () throws -> T) throws -> T {
        let store = ThemeStore.shared
        let before = store.themes
        store.install(before + ThemeLoader.loadDirectory(skins.appendingPathComponent("palettes")))
        defer { store.install(before) }
        return try SkinStore.shared.withOverride(skin: try skin(), paletteId: "parchment", body)
    }

    private func layout(_ content: TodayContent) -> AlmanacTodayLayout {
        AlmanacTodayLayout(content: content, actions: .noop, composer: TodayFixtures.composer)
    }

    /// Today's content width at the 900pt minimum window: 672 at the ideal sidebar, 692 at its
    /// minimum; 660 is the brief's stricter figure; 760 is the widest `TodayChrome` allows.
    private let widths: [CGFloat] = [660, 672, 692, 760]

    func test_almanac_no_horizontal_overflow() throws {
        try withAlmanac {
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

    func test_almanac_sample_light() throws {
        try withAlmanac { try assertSnapshot(page(.sample), named: "almanac-sample-light", size: CGSize(width: 900, height: 1800), colorScheme: .light) }
    }

    func test_almanac_sample_dark() throws {
        try withAlmanac { try assertSnapshot(page(.sample), named: "almanac-sample-dark", size: CGSize(width: 900, height: 1800), colorScheme: .dark) }
    }

    func test_almanac_worstcase_light() throws {
        try withAlmanac { try assertSnapshot(page(.worstCase), named: "almanac-worstcase-light", size: CGSize(width: 900, height: 2800), colorScheme: .light) }
    }
}
