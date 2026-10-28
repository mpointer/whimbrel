import SwiftUI
import XCTest
@testable import Whimbrel

@MainActor
final class SkinDesignSystemTests: XCTestCase {
    /// Every test runs with Whimbrel and its default palette pinned, so the baselines never read the
    /// skin persisted in this machine's defaults (the test host is the app, on `.standard`).
    nonisolated override func invokeTest() {
        MainActor.assumeIsolated {
            SkinStore.shared.withOverride(skin: SkinDefaults.whimbrel, paletteId: "default") { super.invokeTest() }
        }
    }

    private func with<T>(_ skin: Skin, _ body: () -> T) -> T {
        SkinStore.shared.withOverride(skin: skin, paletteId: "default", body)
    }

    /// The body face now names its (regular) weight; the snapshots guard that it renders the same.
    func test_whimbrel_skin_fonts_equal_current_constants() {
        XCTAssertEqual(Font.whimbrelDisplay(.title2), Font.system(.title2, design: .serif).weight(.semibold))
        XCTAssertEqual(Font.whimbrelDisplay(.title), Font.system(.title, design: .serif).weight(.semibold))
        XCTAssertEqual(Font.whimbrelBody(.body), Font.system(size: 14, weight: .regular, design: .default))
        XCTAssertEqual(Font.whimbrelBody(.headline), Font.system(size: 13, weight: .regular, design: .default).weight(.semibold))
        XCTAssertEqual(Font.whimbrelPageTitle, Font.system(size: 28, weight: .regular, design: .serif))
    }

    func test_terminal_face_maps_to_monospaced() {
        XCTAssertEqual(SkinFace(.sfMono).font(size: 12), Font.system(size: 12, weight: .regular, design: .monospaced))
        XCTAssertEqual(SkinFace(.sfProRounded, .bold).font(size: 12), Font.system(size: 12, weight: .bold, design: .rounded))
        XCTAssertEqual(SkinFace(.newYork).font(size: 12), Font.system(size: 12, weight: .regular, design: .serif))
        var skin = SkinDefaults.whimbrel
        skin.typography.body = SkinFace(.sfMono)
        XCTAssertEqual(with(skin) { Font.whimbrelBody(.body) }, Font.system(size: 14, weight: .regular, design: .monospaced))
    }

    func test_radius_follows_skin() {
        XCTAssertEqual(Radius.card, 12)
        XCTAssertEqual(Radius.control, 8)
        var skin = SkinDefaults.whimbrel
        skin.shape.radius = 0
        XCTAssertEqual(with(skin) { Radius.card }, 0)
        XCTAssertEqual(with(skin) { Radius.control }, 0)
        skin.shape.radius = 24
        XCTAssertEqual(with(skin) { Radius.control }, 16)
    }

    func test_voice_greeting_fills_name() {
        XCTAssertEqual(SkinDefaults.whimbrel.voice.text(.greetingMorning, name: "Mike"), "Good morning, Mike")
        XCTAssertEqual(TodayLogic.greetingKey(now: Date(timeIntervalSince1970: 0), calendar: utc(hour: 9)), .greetingMorning)
    }

    func test_caps_transform() {
        XCTAssertEqual(SkinCaps.none.apply("Open loops"), "Open loops")
        XCTAssertEqual(SkinCaps.upper.apply("Open loops"), "OPEN LOOPS")
        XCTAssertEqual(SkinCaps.small.apply("Open loops"), "Open loops")
        XCTAssertEqual(SkinDefaults.whimbrel.typography.caps, .upper)
    }

    func test_icons_follow_skin() {
        XCTAssertEqual(SkinStore.shared.selected.icons[.today], "sun.max")
        var skin = SkinDefaults.whimbrel
        skin.icons.symbols[.today] = "moon"
        XCTAssertEqual(with(skin) { SkinStore.shared.selected.icons[.today] }, "moon")
    }

    /// The same source scan `ColorTokenUsageTests` does: no button label (or action) reads the skin's voice.
    func test_buttons_ignore_voice() throws {
        let ui = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI")
        let files = [ui.appendingPathComponent("Today/StandardTodayLayout.swift"), ui.appendingPathComponent("Today/Archetypes/TerminalTodayLayout.swift"), ui.appendingPathComponent("Today/Archetypes/GlassTodayLayout.swift"), ui.appendingPathComponent("Today/Archetypes/FieldTodayLayout.swift"), ui.appendingPathComponent("Today/Archetypes/AlmanacTodayLayout.swift"), ui.appendingPathComponent("Today/Archetypes/ReferenceTodayLayout.swift"), ui.appendingPathComponent("Today/Archetypes/SkinPrepLine.swift"), ui.appendingPathComponent("DesignSystem.swift")]
        var offenders: [String] = []
        var buttons = 0
        for url in files {
            let lines = try String(contentsOf: url, encoding: .utf8).components(separatedBy: "\n")
            for (i, line) in lines.enumerated() where line.contains("Button(") || line.contains("Button {") {
                buttons += 1
                // Known limit: a button's extent is approximated as its first line to its `.buttonStyle`, or twelve
                // lines; a label built further down than that (or via a helper) is not seen.
                var window = ""
                for next in lines[i...].prefix(12) {
                    window += next + "\n"
                    if next.contains(".buttonStyle") { break }
                }
                if window.contains("SkinVoice") || window.range(of: #"\bvoice\b"#, options: .regularExpression) != nil { offenders.append("\(url.lastPathComponent):\(i + 1)") }
            }
        }
        XCTAssertGreaterThan(buttons, 0, "the scan found no buttons")
        XCTAssertEqual(offenders, [])
    }

    private func utc(hour: Int) -> Calendar {
        // `greetingKey` reads the hour of `now`; a fixed clock-in-hour calendar keeps the test timezone-free.
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(secondsFromGMT: (hour * 3600))!
        return c
    }
}
