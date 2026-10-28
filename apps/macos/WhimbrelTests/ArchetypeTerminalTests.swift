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
final class ArchetypeTerminalTests: XCTestCase {
    /// <repo>/skins, found from this file's path (apps/macos/WhimbrelTests/ArchetypeTerminalTests.swift).
    private let skins = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("skins")

    private func skin() throws -> Skin {
        let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(try Data(contentsOf: skins.appendingPathComponent("retro-terminal.json")), base: SkinDefaults.whimbrel))
        XCTAssertEqual(repairs, [])
        return skin
    }

    func test_retro_terminal_skin_loads_without_repairs() throws {
        let skin = try skin()
        XCTAssertEqual(skin.id, "retro-terminal")
        XCTAssertEqual(skin.paletteIds, ["phosphor", "amber"])
        XCTAssertEqual(skin.archetype, .terminal)
        XCTAssertEqual(skin.motion, .subtle)
        XCTAssertEqual(skin.typography.body.family, .sfMono)
    }

    func test_retro_terminal_palettes_pass_contrast() throws {
        for id in try skin().paletteIds {
            let data = try Data(contentsOf: skins.appendingPathComponent("palettes/\(id).json"))
            let raw = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertNil(raw["light"], "\(id) is dark-only")
            let theme = try XCTUnwrap(ThemeLoader.parse(data), id)
            XCTAssertNotEqual(theme.dark, ThemeDefaults.theme.dark, "\(id) must not have fallen back to Default")
            XCTAssertEqual(theme.light, theme.dark, "\(id): the dark variant serves both appearances")
            XCTAssertEqual(Set(theme.dark.keys), Set(ThemeToken.allCases), id)
            XCTAssertEqual(ThemeContrast.failures(theme.dark), [], id)
            XCTAssertTrue(ThemeContrast.passes(theme.dark), id)
            XCTAssertEqual(ArchetypeContrast.failures(theme.dark, ArchetypeContrast.pairs(for: .terminal)), [], id)
        }
    }

    func test_terminal_places_expected_sections() {
        XCTAssertEqual(TerminalTodayLayout.placedSections, Set(TodaySection.allCases).subtracting([.followups]))
    }

    func test_cursor_blinks_only_with_subtle_motion_and_reduce_motion_off() {
        XCTAssertTrue(TerminalCursor.animates(motion: .subtle, reduceMotion: false))
        XCTAssertFalse(TerminalCursor.animates(motion: .subtle, reduceMotion: true))
        XCTAssertFalse(TerminalCursor.animates(motion: .none, reduceMotion: false))
        XCTAssertFalse(TerminalCursor.animates(motion: .none, reduceMotion: true))
    }

    /// Runs `body` with Retro Terminal and `palette` in force. The palettes are installed in the
    /// theme store for the duration, since the test host does not load `skins/palettes`.
    private func withTerminal<T>(palette: String = "phosphor", _ body: () throws -> T) throws -> T {
        let store = ThemeStore.shared
        let before = store.themes
        store.install(before + ThemeLoader.loadDirectory(skins.appendingPathComponent("palettes")))
        defer { store.install(before) }
        return try SkinStore.shared.withOverride(skin: try skin(), paletteId: palette, body)
    }

    private func layout(_ content: TodayContent) -> TerminalTodayLayout {
        TerminalTodayLayout(content: content, actions: .noop, composer: TodayFixtures.composer)
    }

    /// Today's content width at the 900pt minimum window: the shell's sidebar is 160...180 wide
    /// (ideal 180) and `TodayChrome` pads 24 each side, so 672 at the ideal sidebar, 692 at its
    /// minimum. 660 is the brief's stricter figure; 760 is the widest `TodayChrome` allows.
    private let widths: [CGFloat] = [660, 672, 692, 760]

    func test_terminal_no_horizontal_overflow() throws {
        try withTerminal {
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

    func test_terminal_sample_light() throws {
        try withTerminal { try assertSnapshot(page(.sample), named: "terminal-sample-light", size: CGSize(width: 900, height: 1400), colorScheme: .light) }
    }

    func test_terminal_sample_dark() throws {
        try withTerminal { try assertSnapshot(page(.sample), named: "terminal-sample-dark", size: CGSize(width: 900, height: 1400), colorScheme: .dark) }
    }

    func test_terminal_worstcase_dark() throws {
        try withTerminal(palette: "amber") { try assertSnapshot(page(.worstCase), named: "terminal-worstcase-dark", size: CGSize(width: 900, height: 2400), colorScheme: .dark) }
    }
}
