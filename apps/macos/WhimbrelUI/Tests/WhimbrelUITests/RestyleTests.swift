import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

/// Step 10, the restyle pass (docs/ui-redesign/screens/restyle.md). The old screens are drawn by the app, so the package proves what
/// it can without a view: each rule hands back the old value with the switch off and the Thread value with it on, and the screens'
/// sources hold no hard-coded colors or font sizes outside the old branch. Nothing here reads real defaults, a store or the clock.
@Suite struct RestyleTests {
    private let thread = WTheme.thread(variant: .light)

    // MARK: Text

    @Test func textWithTheSwitchOffKeepsTheOldFontAndOnTakesTheThreadRole() {
        for text in WRestyleText.allCases {
            #expect(WRestyle.text(text, look: .classic) == WTextSpec())
            #expect(WRestyle.text(text, look: .classic).isClassic)
            #expect(WRestyle.text(text, look: .thread).role == text.role)
        }
    }

    @Test func everyOldStyleMapsToAThreadRoleAtLeastAsLargeAsItsSmallestReadableSize() {
        for text in WRestyleText.allCases {
            #expect(text.role.size() >= WTokens.TypeScale.minimumReadable, "\(text)")
        }
        #expect(WRestyleText.caption.role == .secondary)
        #expect(WRestyleText.callout.role == .body && WRestyleText.body.role == .body)
        #expect(WRestyleText.calloutStrong.role == .bodyStrong)
        #expect(WRestyleText.display.role == .subtitle && WRestyleText.pageTitle.role == .pageTitle)
    }

    // MARK: Card

    @Test func aCardWithTheSwitchOffIsExactlyTheOldOne() {
        let old = WCardSpec(padding: 19, cornerRadius: 11, borderWidth: 2, dashed: true, doubled: true)
        #expect(WRestyle.card(look: .classic, classic: old, theme: thread) == old)
    }

    @Test func aCardWithTheSwitchOnTakesTheThemesShape() {
        let old = WCardSpec(padding: 19, cornerRadius: 11, borderWidth: 2, dashed: true, doubled: true)
        let card = WRestyle.card(look: .thread, classic: old, theme: thread)
        #expect(card.cornerRadius == WTokens.Radius.card)
        #expect(card.padding == 16)
        #expect(card.borderWidth == thread.borderWidth)
        #expect(!card.dashed && !card.doubled)
        // A skin with its own shape keeps it, and a roomy one gets more padding.
        let skin = WTheme(palette: [:], variant: .light, shape: WShape(cardRadius: 4, borderWidth: 1, borderStyle: .dashed, density: .roomy))
        let skinned = WRestyle.card(look: .thread, classic: old, theme: skin)
        #expect(skinned.cornerRadius == 4 && skinned.dashed && skinned.padding == 20)
        // High contrast draws a solid, thicker line.
        let high = WRestyle.card(look: .thread, classic: old, theme: WTheme.thread(variant: .highContrastLight))
        #expect(!high.dashed && high.borderWidth == WShape.thread.highContrastBorderWidth)
    }

    // MARK: Rows

    @Test func aSelectedRowShowsFillAndBarTogetherOnlyInTheThreadLook() {
        #expect(WRestyle.row(look: .classic, selected: true) == WRowSpec(fill: .none, leadingBar: false))
        #expect(WRestyle.row(look: .classic, selected: false) == WRowSpec(fill: .none, leadingBar: false))
        #expect(WRestyle.row(look: .thread, selected: true) == WRowSpec(fill: .selection, leadingBar: true))
        #expect(WRestyle.row(look: .thread, selected: false) == WRowSpec(fill: .none, leadingBar: false))
    }

    // MARK: Buttons

    @Test func buttonsKeepTheOldPillWithTheSwitchOffAndTakeTheThreadKindWithItOn() {
        for kind in WRestyleButtonKind.allCases {
            #expect(WRestyle.button(look: .classic, kind: kind) == .classic(kind))
            #expect(WRestyle.button(look: .thread, kind: kind) == .thread(kind))
        }
        #expect(WButtonSpec.thread(.primary).thread == .primary)
        #expect(WButtonSpec.thread(.secondary).thread == .secondary)
        #expect(WButtonSpec.thread(.quiet).thread == .quiet)
    }

    // MARK: Status and glyphs

    @Test func aRunsStatusColorChangesOnlyInTheThreadLookAndNeverForAnUnknownStatus() {
        for tone in [WRunTone.success, .failure, .other] {
            #expect(!WRestyle.runToneUsesStatusColor(look: .classic, tone: tone))
        }
        #expect(WRestyle.runToneUsesStatusColor(look: .thread, tone: .success))
        #expect(WRestyle.runToneUsesStatusColor(look: .thread, tone: .failure))
        #expect(!WRestyle.runToneUsesStatusColor(look: .thread, tone: .other))
    }

    @Test func aGlyphKeepsItsOldSizeOffAndFollowsTheTextSizeOn() {
        #expect(WRestyle.glyphSize(look: .classic, classic: 20, role: .title, scale: 2) == 20)
        #expect(WRestyle.glyphSize(look: .thread, classic: 20, role: .title, scale: 1) == 20)
        #expect(WRestyle.glyphSize(look: .thread, classic: 12, role: .secondary, scale: 2) == 24)
    }

    @Test func withTheSwitchOffEveryPureRuleHandsBackTheOldValueWhateverItIs() {
        for size in [9, 12, 17.5, 28] as [CGFloat] {
            for scale in [1, 1.5, 2] as [CGFloat] {
                for role in [WTextRole.title, .secondary, .body] {
                    #expect(WRestyle.glyphSize(look: .classic, classic: size, role: role, scale: scale) == size)
                }
            }
        }
        for card in [WCardSpec(padding: 0, cornerRadius: 0, borderWidth: 0), WCardSpec(padding: 13, cornerRadius: 7, borderWidth: 1.5, dashed: true)] {
            for variant in WVariant.allCases {
                #expect(WRestyle.card(look: .classic, classic: card, theme: WTheme.thread(variant: variant)) == card)
            }
        }
        for tone in [WRunTone.success, .failure, .other] { #expect(!WRestyle.runToneUsesStatusColor(look: .classic, tone: tone)) }
        for selected in [true, false] { #expect(WRestyle.row(look: .classic, selected: selected) == WRowSpec(fill: .none, leadingBar: false)) }
    }

    @Test func theLookFollowsTheSwitch() {
        #expect(WLook(redesign: false) == .classic)
        #expect(WLook(redesign: true) == .thread)
        #expect(EnvironmentValues().wTheme == WTheme.thread(variant: .light))
    }

    // MARK: The control border (3:1)

    private func luminance(_ hex: UInt32) -> Double {
        func channel(_ v: UInt32) -> Double {
            let c = Double(v) / 255
            return c <= 0.03928 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel((hex >> 16) & 0xFF) + 0.7152 * channel((hex >> 8) & 0xFF) + 0.0722 * channel(hex & 0xFF)
    }

    private func ratio(_ a: UInt32, _ b: UInt32) -> Double {
        let (x, y) = (luminance(a), luminance(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }

    /// What `firmEdge` drew before: the secondary ink at 70% over the card (the ink itself in high contrast, where it is the border).
    private func oldFirmEdge(_ variant: WVariant) -> UInt32 {
        let palette = variant.threadPalette
        if variant.isHighContrast { return palette["border"] ?? 0 }
        let fg = palette["textSecondary"] ?? 0, bg = palette["backgroundCard"] ?? 0
        func mix(_ shift: UInt32) -> UInt32 {
            let f = Double((fg >> shift) & 0xFF), b = Double((bg >> shift) & 0xFF)
            return UInt32((0.7 * f + 0.3 * b).rounded())
        }
        return mix(16) << 16 | mix(8) << 8 | mix(0)
    }

    @Test func noControlGetsAWeakerEdgeThanBeforeAndEveryVariantStaysThreeToOne() throws {
        for variant in WVariant.allCases {
            let palette = variant.threadPalette
            let edge = try #require(WTheme.thread(variant: variant).controlBorderValue)
            let card = palette["backgroundCard"] ?? 0, page = palette["backgroundPage"] ?? 0
            #expect(ratio(edge, card) >= 3, "\(variant) on card")
            #expect(ratio(edge, page) >= 3, "\(variant) on page")
            #expect(ratio(edge, card) >= ratio(oldFirmEdge(variant), card), "\(variant) not weaker than before")
        }
    }

    @Test func theAccentIsUsedOnTheSelectionFillOnlyWhereItIsReadable() {
        // Thread's accent passes on its selection color in every variant, so the chosen chip keeps the accent.
        for variant in WVariant.allCases {
            let palette = variant.threadPalette
            #expect(WControlEdge.readable(palette["accentPrimary"] ?? 0, on: palette["selection"] ?? 0), "\(variant)")
        }
        // Classic's Default palette does not (4.2 and 4.3 to 1), so its chosen chip is written in the text color.
        #expect(!WControlEdge.readable(0xA8502E, on: 0xEAE1CD))
        #expect(!WControlEdge.readable(0xC97B54, on: 0x312A24))
    }

    @Test func theControlBorderReachesThreeToOneOnTheCardAndThePageInEveryVariant() {
        for variant in WVariant.allCases {
            let palette = variant.threadPalette
            let edge = WThreadOnly.controlBorder.value(for: variant)
            #expect(ratio(edge, palette["backgroundCard"] ?? 0) >= 3, "\(variant) on card")
            #expect(ratio(edge, palette["backgroundPage"] ?? 0) >= 3, "\(variant) on page")
        }
    }

    @Test func theThreadThemeUsesTheTokenAndAnotherPaletteKeepsItsOwnEdge() throws {
        // The token, unless the old firm edge is stronger on the card (light and dark): then the old edge, so no control gets a weaker one. In high contrast the old edge is the token.
        for variant in WVariant.allCases {
            let theme = WTheme.thread(variant: variant)
            let value = try #require(theme.controlBorderValue)
            #expect(value == WThreadOnly.controlBorder.value(for: variant) || value == oldFirmEdge(variant))
        }
        #expect(WTheme.thread(variant: .highContrastLight).controlBorderValue == WThreadOnly.controlBorder.highContrastLight)
        #expect(WTheme.thread(variant: .highContrastDark).controlBorderValue == WThreadOnly.controlBorder.highContrastDark)
        // High contrast is unchanged: the token is the border itself.
        #expect(WThreadOnly.controlBorder.highContrastLight == WVariant.highContrastLight.threadPalette["border"])
        #expect(WThreadOnly.controlBorder.highContrastDark == WVariant.highContrastDark.threadPalette["border"])
        // A user palette (a different card and hairline) gets its own softened edge, not the Thread gray.
        var palette = WVariant.light.threadPalette
        palette["border"] = 0x112233
        palette["backgroundCard"] = 0xFFEEDD
        #expect(WTheme(palette: palette, variant: .light).controlBorderValue == nil)
    }

    // MARK: Source scan

    private func repoRoot() -> URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root = root.deletingLastPathComponent() }
        return root
    }

    /// The restyled screens, by file. Each is scanned after its comments are removed.
    private static let screens = [
        "TopicsView.swift", "TopicChip.swift", "AutomationsView.swift", "RoutinesInboxView.swift", "PendingApprovalsView.swift",
        "MeetingsListView.swift", "MeetingCard.swift", "MeetingRowView.swift", "CaptureDetailView.swift", "EpisodeDetailView.swift",
        "ThreadDetailView.swift", "ClientControlsView.swift",
    ]

    /// Leftovers the scan allows, each with its reason. A new entry needs a reason; an empty list is the goal.
    private static let allowed: [String: String] = [:]

    private func stripped(_ text: String) -> String {
        text.split(separator: "\n", omittingEmptySubsequences: false).map { line -> String in
            if let range = line.range(of: "//") { return String(line[..<range.lowerBound]) }
            return String(line)
        }.joined(separator: "\n")
    }

    @Test func theRestyledScreensHoldNoHardCodedColorOrFontSize() throws {
        let ui = repoRoot().appendingPathComponent("apps/macos/Whimbrel/UI")
        try #require(FileManager.default.fileExists(atPath: ui.path), "apps/macos/Whimbrel/UI not found from the test file")
        for name in Self.screens {
            let text = stripped(try String(contentsOf: ui.appendingPathComponent(name), encoding: .utf8))
            for pattern in ["Color(red:", "Color(hex", ".font(.system(size:", "NSColor(", "Color(white:"] where text.contains(pattern) {
                #expect(Self.allowed["\(name):\(pattern)"] != nil, "\(name) has \(pattern)")
            }
        }
    }

    @Test func theRestyledScreensGoThroughTheRestyleModifiersNotTheOldFontsAndButtons() throws {
        let ui = repoRoot().appendingPathComponent("apps/macos/Whimbrel/UI")
        try #require(FileManager.default.fileExists(atPath: ui.path), "apps/macos/Whimbrel/UI not found from the test file")
        // These files draw their text and buttons through `restyledFont` and `restyledButton`; the old calls remain only in the
        // flag-off branch of a `RedesignSwitch`, and there are none left in these files.
        for name in Self.screens {
            let text = stripped(try String(contentsOf: ui.appendingPathComponent(name), encoding: .utf8))
            for pattern in [".font(.whimbrelBody(", ".font(.whimbrelDisplay(", ".font(.whimbrelPageTitle", ".buttonStyle(.whimbrelPrimary)", ".buttonStyle(.whimbrelSecondary)", ".buttonStyle(.whimbrelQuiet)", ".textFieldStyle(.roundedBorder)"] {
                #expect(!text.contains(pattern), "\(name) still has \(pattern)")
            }
        }
    }

    @Test func noNewFileNamesASettingsAnchorLiteral() throws {
        let ui = repoRoot().appendingPathComponent("apps/macos")
        try #require(FileManager.default.fileExists(atPath: ui.path), "apps/macos not found from the test file")
        for path in ["Whimbrel/UI/Redesign/Restyle.swift", "WhimbrelUI/Sources/WhimbrelUI/Restyle.swift", "WhimbrelUI/Sources/WhimbrelUI/ControlBorder.swift"] {
            let text = try String(contentsOf: ui.appendingPathComponent(path), encoding: .utf8)
            #expect(!text.contains(".settingsAnchor(" + "\""), Comment(rawValue: path))
        }
    }
}

@Suite struct WContrastPaletteTests {
    private let cream: [String: UInt32] = ["backgroundPage": 0xFAF3E8, "textPrimary": 0x3A2E24, "accentPrimary": 0xA8502E]

    @Test func standardContrastDrawsThePickedSkin() {
        for scheme in [ColorScheme.light, .dark] {
            #expect(WContrastPalette.palette(skin: cream, colorScheme: scheme, contrast: .standard) == cream)
        }
    }

    @Test func increasedContrastDrawsThreadHighContrastWhateverSkinIsPicked() {
        #expect(WContrastPalette.palette(skin: cream, colorScheme: .light, contrast: .increased) == WThreadPalette.highContrastLight)
        #expect(WContrastPalette.palette(skin: cream, colorScheme: .dark, contrast: .increased) == WThreadPalette.highContrastDark)
        #expect(WContrastPalette.palette(skin: [:], colorScheme: .light, contrast: .increased) == WThreadPalette.highContrastLight)
    }

    @Test func oneColorFollowsTheSameRule() {
        #expect(WContrastPalette.hex(token: "textPrimary", skin: 0x123456, isDark: false, increased: false) == 0x123456)
        #expect(WContrastPalette.hex(token: "textPrimary", skin: 0x123456, isDark: false, increased: true) == WThreadPalette.highContrastLight["textPrimary"])
        #expect(WContrastPalette.hex(token: "textPrimary", skin: 0x123456, isDark: true, increased: true) == WThreadPalette.highContrastDark["textPrimary"])
        #expect(WContrastPalette.hex(token: "notAToken", skin: 0x123456, isDark: true, increased: true) == 0x123456)
    }

    @Test func aThemeBuiltFromTheChosenPaletteHasTheThreadTextAndBackgroundInHighContrast() {
        let chosen = WContrastPalette.palette(skin: cream, colorScheme: .light, contrast: .increased)
        let theme = WTheme(palette: chosen, colorScheme: .light, contrast: .increased)
        #expect(theme.palette["backgroundPage"] == WThreadPalette.highContrastLight["backgroundPage"])
        #expect(theme.palette["textPrimary"] == WThreadPalette.highContrastLight["textPrimary"])
        #expect(theme.usesThreadPalette)
    }
}
