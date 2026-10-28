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
final class ArchetypeGlassTests: XCTestCase {
    /// <repo>/skins, found from this file's path (apps/macos/WhimbrelTests/ArchetypeGlassTests.swift).
    private let skins = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("skins")

    private func skin() throws -> Skin {
        let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(try Data(contentsOf: skins.appendingPathComponent("hypermodern.json")), base: SkinDefaults.whimbrel))
        XCTAssertEqual(repairs, [])
        return skin
    }

    func test_hypermodern_skin_loads_without_repairs() throws {
        let skin = try skin()
        XCTAssertEqual(skin.id, "hypermodern")
        XCTAssertEqual(skin.paletteIds, ["violet-night", "ice"])
        XCTAssertEqual(skin.archetype, .glass)
        XCTAssertEqual(skin.typography.display.weight, .thin)
    }

    private func palette(_ id: String) throws -> (theme: Theme, dark: ThemePalette) {
        let data = try Data(contentsOf: skins.appendingPathComponent("palettes/\(id).json"))
        let raw = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertNil(raw["light"], "\(id) is dark-only")
        let theme = try XCTUnwrap(ThemeLoader.parse(data), id)
        return (theme, theme.dark)
    }

    func test_hypermodern_palettes_pass_contrast() throws {
        for id in try skin().paletteIds {
            let (theme, dark) = try palette(id)
            XCTAssertNotEqual(dark, ThemeDefaults.theme.dark, "\(id) must not have fallen back to Default")
            XCTAssertEqual(theme.light, dark, "\(id): the dark variant serves both appearances")
            XCTAssertEqual(Set(dark.keys), Set(ThemeToken.allCases), id)
            XCTAssertEqual(ThemeContrast.failures(dark), [], id)
        }
    }

    /// The card is `.ultraThinMaterial` under a palette tint, so what shows through varies: text must pass 4.5:1
    /// on the tint over every backdrop it can have (the glow, the accent, and white and black as bounds for the
    /// material), and the greeting and date on the glow itself. The surfaces are `ArchetypeContrast`'s.
    func test_glass_text_contrast_holds_on_every_backdrop() throws {
        for id in try skin().paletteIds {
            let p = try palette(id).dark
            XCTAssertEqual(ArchetypeContrast.failures(p, ArchetypeContrast.pairs(for: .glass)), [], id)
        }
    }

    func test_glass_places_expected_sections() {
        XCTAssertEqual(GlassTodayLayout.placedSections, [.greeting, .composer, .upNext, .schedule, .recap])
    }

    /// Runs `body` with Hypermodern and `palette` in force. The palettes are installed in the
    /// theme store for the duration, since the test host does not load `skins/palettes`.
    private func withGlass<T>(palette: String = "violet-night", _ body: () throws -> T) throws -> T {
        let store = ThemeStore.shared
        let before = store.themes
        store.install(before + ThemeLoader.loadDirectory(skins.appendingPathComponent("palettes")))
        defer { store.install(before) }
        return try SkinStore.shared.withOverride(skin: try skin(), paletteId: palette, body)
    }

    private func layout(_ content: TodayContent) -> GlassTodayLayout {
        GlassTodayLayout(content: content, actions: .noop, composer: TodayFixtures.composer)
    }

    /// Content widths: 672 / 692 at the 900pt minimum window (sidebar 180 / 160 plus 24 padding each
    /// side), 660 the brief's stricter figure, 760 the widest `TodayChrome` allows.
    private let widths: [CGFloat] = [660, 672, 692, 760]

    func test_glass_no_horizontal_overflow() throws {
        try withGlass {
            for content in [TodayContent.worstCase, .sample, .empty] {
                for width in widths {
                    let measured = renderedWidth(layout(content), proposing: width)
                    XCTAssertLessThanOrEqual(measured, width + 0.5, "content wider than \(width)")
                }
            }
        }
    }

    /// The width the layout takes when offered `width`: a layout that cannot shrink to it reports more.
    /// (A plain `fittingSize` is no use: wrapping text's ideal width is its unwrapped width.)
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

    func test_glass_sample_light() throws {
        try withGlass { try assertSnapshot(page(.sample), named: "glass-sample-light", size: CGSize(width: 900, height: 1100), colorScheme: .light) }
    }

    func test_glass_sample_dark() throws {
        try withGlass(palette: "ice") { try assertSnapshot(page(.sample), named: "glass-sample-dark", size: CGSize(width: 900, height: 1100), colorScheme: .dark) }
    }

    func test_glass_worstcase_dark() throws {
        try withGlass { try assertSnapshot(page(.worstCase), named: "glass-worstcase-dark", size: CGSize(width: 900, height: 2400), colorScheme: .dark) }
    }
}
