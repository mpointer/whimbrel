import SwiftUI
import XCTest
@testable import Whimbrel

final class ThemeStoreTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "theme-store-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    private func night() -> Theme {
        var light = ThemeDefaults.light
        light[.backgroundPage] = 0xEEEEFF
        var dark = ThemeDefaults.dark
        dark[.backgroundPage] = 0x000011
        return Theme(id: "night", name: "Night", author: "t", light: light, dark: dark)
    }

    /// The sRGB value a SwiftUI `Color` resolves to under a given appearance.
    private func hex(_ color: Color, dark: Bool) -> UInt32 {
        var out: UInt32 = 0
        NSAppearance(named: dark ? .darkAqua : .aqua)!.performAsCurrentDrawingAppearance {
            let c = NSColor(color).usingColorSpace(.sRGB)!
            out = (UInt32((c.redComponent * 255).rounded()) << 16)
                | (UInt32((c.greenComponent * 255).rounded()) << 8)
                | UInt32((c.blueComponent * 255).rounded())
        }
        return out
    }

    // MARK: store

    func testStartsOnDefaultAndKeepsDefaultFirstWhenThemesAreInstalled() {
        let store = ThemeStore(defaults: defaults())
        XCTAssertEqual(store.selectedId, "default")
        store.install([night()])
        XCTAssertEqual(store.themes.map(\.id), ["default", "night"])
    }

    func testSelectingAThemePersistsBumpsTheRevisionAndAnUnknownIdIsIgnored() {
        let d = defaults()
        let store = ThemeStore(defaults: d)
        store.install([night()])
        let before = store.revision
        store.select("night")
        XCTAssertEqual(store.selectedId, "night")
        XCTAssertGreaterThan(store.revision, before)
        XCTAssertEqual(d.string(forKey: ThemeStore.selectedKey), "night")
        let after = store.revision
        store.select("no-such-theme")
        XCTAssertEqual(store.selectedId, "night", "an unknown id is ignored")
        XCTAssertEqual(store.revision, after)
    }

    func testAStoredIdThatIsNoLongerInstalledFallsBackToDefault() {
        let d = defaults()
        d.set("gone", forKey: ThemeStore.selectedKey)
        let store = ThemeStore(defaults: d)
        store.install([night()])
        XCTAssertEqual(store.selectedId, "default")
        XCTAssertEqual(store.selected, ThemeDefaults.theme)
    }

    func testHexFollowsTheSelectedThemeAndAppearanceKind() {
        let store = ThemeStore(defaults: defaults())
        store.install([night()])
        XCTAssertEqual(store.hex(.backgroundPage, isDark: false), 0xF7F2E7)
        store.select("night")
        XCTAssertEqual(store.hex(.backgroundPage, isDark: false), 0xEEEEFF)
        XCTAssertEqual(store.hex(.backgroundPage, isDark: true), 0x000011)
    }

    // MARK: dynamic colors: Default reproduces today's look

    /// Today's `DesignSystem.swift` values, written out so a drift in the code or in
    /// `themes/default.json` fails here. (The three newer tokens are the opaque equivalents of
    /// today's fills; dark `textOnAccent` is the deliberate change described in the plan.)
    private let today: [(Color, light: UInt32, dark: UInt32)] = [
        (.whimbrelBackgroundPage, 0xF7F2E7, 0x1C1712), (.whimbrelBackgroundCard, 0xFFFFFF, 0x262019),
        (.whimbrelBorder, 0xE4D9C2, 0x3A322B), (.whimbrelTextPrimary, 0x241E19, 0xEDE6D9),
        (.whimbrelTextSecondary, 0x6B5F4F, 0xB7AB9A), (.whimbrelAccentPrimary, 0xA8502E, 0xC97B54),
        (.whimbrelAccentSecondary, 0x55606E, 0x7C8794), (.whimbrelAccentHighlight, 0xD9A441, 0xE8B968),
        (.whimbrelError, 0xB3261E, 0xF2877E), (.whimbrelWarning, 0x9A5B00, 0xE0A04A),
        (.whimbrelSuccess, 0x3B7A3B, 0x7CC47C), (.whimbrelLink, 0x2E5A87, 0x8FB4DC),
        (.whimbrelSelection, 0xEAE1CD, 0x312A24), (.whimbrelHighlight, 0xF6E8D0, 0x57462D),
        (.whimbrelTextOnAccent, 0xFFFFFF, 0x1C1712),
    ]

    func testTheSharedStoreOnDefaultResolvesEveryColorToTodaysValues() {
        let shared = ThemeStore.shared
        let previous = shared.selectedId
        shared.select("default")
        defer { shared.select(previous) }
        for (index, entry) in today.enumerated() {
            XCTAssertEqual(hex(entry.0, dark: false), entry.light, "light #\(index)")
            XCTAssertEqual(hex(entry.0, dark: true), entry.dark, "dark #\(index)")
        }
    }

    func testDefaultInCodeAndInTheJsonFileAreTheSameAsTheTable() throws {
        let file = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("themes/default.json")
        let fromFile = try XCTUnwrap(ThemeLoader.parse(try Data(contentsOf: file)))
        XCTAssertEqual(fromFile, ThemeDefaults.theme)
        for token in ThemeToken.allCases {
            XCTAssertNotNil(ThemeDefaults.light[token], "\(token)")
            XCTAssertNotNil(ThemeDefaults.dark[token], "\(token)")
        }
    }

    func testTheNewDefaultTokensAreTheOpaqueEquivalentsOfTodaysFills() {
        func blend(_ fg: UInt32, over bg: UInt32, alpha: Double) -> UInt32 {
            func ch(_ shift: UInt32) -> UInt32 {
                let f = Double((fg >> shift) & 0xFF), b = Double((bg >> shift) & 0xFF)
                return UInt32((alpha * f + (1 - alpha) * b).rounded())
            }
            return (ch(16) << 16) | (ch(8) << 8) | ch(0)
        }
        let l = ThemeDefaults.light, d = ThemeDefaults.dark
        XCTAssertEqual(l[.selection], blend(l[.border]!, over: l[.backgroundPage]!, alpha: 0.7))
        XCTAssertEqual(d[.selection], blend(d[.border]!, over: d[.backgroundPage]!, alpha: 0.7))
        XCTAssertEqual(l[.highlight], blend(l[.accentHighlight]!, over: l[.backgroundCard]!, alpha: 0.25))
        XCTAssertEqual(d[.highlight], blend(d[.accentHighlight]!, over: d[.backgroundCard]!, alpha: 0.25))
    }

    func testSelectingAThemeChangesWhatAColorResolvesToWithoutRebuildingIt() {
        let store = ThemeStore.shared
        let previous = store.selectedId
        store.install(ThemeLoader.loadDirectory(URL(fileURLWithPath: "/nonexistent")) + [night()])
        defer { store.select(previous) }
        store.select("default")
        let page = Color.whimbrelBackgroundPage
        XCTAssertEqual(hex(page, dark: false), 0xF7F2E7)
        store.select("night")
        XCTAssertEqual(hex(page, dark: false), 0xEEEEFF, "the same Color value re-reads the store at render time")
        XCTAssertEqual(hex(page, dark: true), 0x000011)
    }

    // MARK: appearance

    func testAppearanceDefaultsToSystemPersistsAndIgnoresAnUnknownStoredValue() {
        let d = defaults()
        XCTAssertEqual(AppearanceSettings(defaults: d).choice, .system)
        let settings = AppearanceSettings(defaults: d)
        settings.choice = .dark
        XCTAssertEqual(AppearanceSettings(defaults: d).choice, .dark)
        d.set("sepia-mode", forKey: AppearanceSettings.key)
        XCTAssertEqual(AppearanceSettings(defaults: d).choice, .system)
    }

    func testAppearanceChoicesMapToAppKit() {
        XCTAssertNil(AppearanceChoice.system.nsAppearance)
        XCTAssertEqual(AppearanceChoice.light.nsAppearance?.name, .aqua)
        XCTAssertEqual(AppearanceChoice.dark.nsAppearance?.name, .darkAqua)
    }
}
