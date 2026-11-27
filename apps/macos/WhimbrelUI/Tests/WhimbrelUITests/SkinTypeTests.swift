import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

/// Richer skins, row 3 (docs/superpowers/plans/2026-11-25-richer-skins.md): every Thread text role belongs to a face, and a skin's
/// family, weight, italic and caps reach it. Thread must draw exactly as before; reading text never changes.
@Suite struct SkinTypeTests {
    // The four built-in skins the row names, as `skins/*.json` write them (the app tests read the real files).
    private let classic = WTypeStyle(
        displayFamily: .serif, bodyFamily: .system, numeralsFamily: .system, labelFamily: .system,
        displayWeight: .semibold, bodyWeight: .regular, numeralsWeight: .regular, labelWeight: .regular, labelCaps: .upper)
    private let hypermodern = WTypeStyle(
        displayFamily: .system, bodyFamily: .system, numeralsFamily: .system, labelFamily: .system,
        displayWeight: .thin, bodyWeight: .regular, numeralsWeight: .light, labelWeight: .medium, labelCaps: .none)
    private let gameDay = WTypeStyle(
        displayFamily: .named("Avenir Next"), bodyFamily: .named("Avenir Next"), numeralsFamily: .named("Helvetica Neue"),
        labelFamily: .named("Avenir Next"), displayWeight: .heavy, bodyWeight: .regular, numeralsWeight: .bold,
        labelWeight: .semibold, labelCaps: .upper)

    // MARK: Thread draws as before

    /// The font each role was built from before skins could set weight: family design, the Thread scale's weight, monospaced digits
    /// on the time.
    private func formerFont(_ role: WTextRole, scale: CGFloat) -> Font {
        let size = role.token.size * scale
        let weight = role.token.weight
        var font: Font = role.isReading
            ? .system(size: size, weight: weight, design: .serif)
            : .system(size: size, weight: weight, design: .default)
        if role == .time { font = font.monospacedDigit() }
        return font
    }

    @Test func threadKeepsTheExactCurrentFonts() {
        for scale: CGFloat in [1, 1.3] {
            for role in WTextRole.allCases {
                #expect(role.font(scale: scale, style: .thread) == formerFont(role, scale: scale), "\(role) at \(scale)")
            }
        }
    }

    @Test func threadWeightsAreTheScaleWeights() {
        let expected: [WTextRole: WWeight] = [
            .caption: .regular, .secondary: .regular, .body: .regular, .bodyStrong: .semibold, .time: .semibold,
            .subtitle: .semibold, .title: .semibold, .pageTitle: .semibold, .reading: .regular, .readingTitle: .regular,
        ]
        for role in WTextRole.allCases {
            #expect(role.weight(in: .thread) == expected[role], "\(role)")
            #expect(!role.italic(in: .thread))
        }
    }

    @Test func threadSectionLabelsStaySemiboldAndUpper() {
        let theme = WTheme.thread(variant: .light)
        let spec = WTextRole.sectionLabelSpec(style: .thread)
        #expect(spec.weight == .semibold)
        #expect(spec.size == 12)
        #expect(!spec.smallCaps && !spec.italic)
        #expect(spec.font == .system(size: 12, weight: .semibold, design: .default))
        #expect(theme.sectionLabel("Risks") == "RISKS")
    }

    @Test func threadScoreboardDigitsStayBold() {
        #expect(WTextRole.scoreboardSpec(scale: 1.2, style: .thread).font
            == .system(size: 44 * 1.2, weight: .bold, design: .default).monospacedDigit())
    }

    // MARK: The emphasis rule

    @Test func emphasisIsTwoStepsUpCappedAtEightHundred() {
        #expect(WWeight.regular.emphasis == .semibold)
        #expect(WWeight.medium.emphasis == .bold)
        #expect(WWeight.semibold.emphasis == .heavy)
        #expect(WWeight.bold.emphasis == .heavy)
        #expect(WWeight.heavy.emphasis == .heavy)
        #expect(WWeight.light.emphasis == .medium)
        // a body already past the cap keeps its own weight, so emphasis is never lighter than the text
        #expect(WWeight.black.emphasis == .black)
    }

    @Test func strongWeightIsBoldInThreadAndBodyPlusTwoSteps() {
        #expect(WTypeStyle.thread.strongWeight == .bold)
        #expect(WTypeStyle(bodyWeight: .regular).strongWeight == .semibold)
        #expect(WTypeStyle(bodyWeight: .medium).strongWeight == .bold)
        #expect(WTypeStyle(bodyWeight: .semibold).strongWeight == .heavy)
    }

    // MARK: Each role resolves per skin

    @Test func rolesBelongToFaces() {
        #expect(WTextRole.pageTitle.faceKind == .display)
        #expect(WTextRole.title.faceKind == .display)
        #expect(WTextRole.subtitle.faceKind == .display)
        #expect(WTextRole.time.faceKind == .numerals)
        #expect(WTextRole.caption.faceKind == .label)
        for role in [WTextRole.body, .bodyStrong, .secondary] { #expect(role.faceKind == .body) }
        #expect(WTextRole.reading.faceKind == .reading)
        #expect(WTextRole.readingTitle.faceKind == .reading)
    }

    @Test func classicSetsTitlesInTheSerif() {
        let title = WTextRole.title.spec(style: classic)
        #expect(title.family == .serif)
        #expect(title.weight == .semibold)
        #expect(WTextRole.pageTitle.spec(style: classic).family == .serif)
        #expect(WTextRole.body.spec(style: classic).family == .system)
        #expect(WTextRole.body.spec(style: classic).weight == .regular)
        #expect(WTextRole.bodyStrong.spec(style: classic).weight == .semibold)
        // numerals regular in Classic, where Thread's are semibold
        #expect(WTextRole.time.spec(style: classic).weight == .regular)
        #expect(WTextRole.time.spec(style: classic).monospacedDigits)
        #expect(classic.labelCaps == .upper)
    }

    @Test func hypermodernIsThin() {
        for role in [WTextRole.subtitle, .title, .pageTitle] { #expect(role.spec(style: hypermodern).weight == .thin, "\(role)") }
        #expect(WTextRole.time.spec(style: hypermodern).weight == .light)
        #expect(WTextRole.caption.spec(style: hypermodern).weight == .medium)
        #expect(WTextRole.body.spec(style: hypermodern).weight == .regular)
        #expect(WTextRole.bodyStrong.spec(style: hypermodern).weight == .semibold)
        #expect(WTextRole.sectionLabelSpec(style: hypermodern).weight == .medium)
    }

    @Test func gameDayIsHeavy() {
        for role in [WTextRole.subtitle, .title, .pageTitle] {
            #expect(role.spec(style: gameDay).weight == .heavy, "\(role)")
            #expect(role.family(in: gameDay) == .named("Avenir Next"))
        }
        #expect(WTextRole.time.weight(in: gameDay) == .bold)
        #expect(WTextRole.time.family(in: gameDay) == .named("Helvetica Neue"))
        #expect(WTextRole.caption.weight(in: gameDay) == .semibold)
        #expect(WTextRole.body.weight(in: gameDay) == .regular)
        // body emphasis stays two steps over the body weight, not over the display weight
        #expect(WTextRole.bodyStrong.weight(in: gameDay) == .semibold)
        #expect(WTextRole.sectionLabelSpec(style: gameDay).weight == .semibold)
        #expect(WTextRole.scoreboardSpec(style: gameDay).weight == .bold)
    }

    @Test func aSemiboldBodyStillShowsEmphasis() {
        let style = WTypeStyle(bodyWeight: .semibold)
        #expect(WTextRole.body.weight(in: style) == .semibold)
        #expect(WTextRole.bodyStrong.weight(in: style) == .heavy)
    }

    // MARK: Italic and reading text

    @Test func italicAppliesToDisplayAndLabelOnly() {
        let style = WTypeStyle(displayItalic: true, labelItalic: true)
        for role in [WTextRole.subtitle, .title, .pageTitle, .caption] { #expect(role.spec(style: style).italic, "\(role)") }
        for role in [WTextRole.body, .bodyStrong, .secondary, .time, .reading, .readingTitle] {
            #expect(!role.spec(style: style).italic, "\(role)")
        }
        #expect(WTextRole.sectionLabelSpec(style: style).italic)
        #expect(!WTextRole.scoreboardSpec(style: style).italic)
    }

    @Test func readingTextNeverTakesASkinFace() {
        let wild = WTypeStyle(
            displayFamily: .monospaced, bodyFamily: .rounded, readingFamily: .serif, numeralsFamily: .rounded, labelFamily: .monospaced,
            displayWeight: .black, bodyWeight: .black, numeralsWeight: .black, labelWeight: .black, displayItalic: true, labelItalic: true)
        for role in [WTextRole.reading, .readingTitle] {
            #expect(role.font(style: wild) == role.font(style: .thread), "\(role)")
        }
    }

    // MARK: Missing families

    @Test func aMissingFamilyFallsBackToTheSystemFace() {
        let missing = WFontFamily.named("No Such Typeface 9000")
        #expect(!WFontSupport.isInstalled("No Such Typeface 9000"))
        #expect(missing.resolved == .system)
        let style = WTypeStyle(displayFamily: missing, bodyFamily: missing, numeralsFamily: missing, labelFamily: missing, displayWeight: .heavy)
        #expect(WTextRole.title.spec(style: style).family == .system)
        #expect(WTextRole.time.spec(style: style).family == .system)
        #expect(WTextRole.caption.spec(style: style).family == .system)
        #expect(WTextRole.sectionLabelSpec(style: style).family == .system)
        // the fallback keeps the skin's weight
        #expect(WTextRole.title.spec(style: style).weight == .heavy)
        #expect(WTextRole.title.font(style: style) == WTextRole.title.font(style: WTypeStyle(displayWeight: .heavy)))
    }

    @Test func aSystemFamilyAndAnInstalledNameResolveToThemselves() {
        #expect(WFontFamily.serif.resolved == .serif)
        #expect(WFontFamily.named("Helvetica Neue").resolved == .named("Helvetica Neue"))
    }

    // MARK: Caps

    @Test func upperAndNoneCaseTheText() {
        #expect(WTheme(palette: [:], variant: .light, type: WTypeStyle(labelCaps: .upper)).sectionLabel("Risks") == "RISKS")
        #expect(WTheme(palette: [:], variant: .light, type: WTypeStyle(labelCaps: WLabelCaps.none)).sectionLabel("Risks") == "Risks")
        #expect(WTypeStyle(sectionLabelCaps: false).labelCaps == WLabelCaps.none)
        #expect(WTypeStyle(sectionLabelCaps: true).labelCaps == .upper)
    }

    @Test func smallCapsUseTheFacesOwnOrFallBackToSmallerCapitals() {
        for family: WFontFamily in [.system, .serif, .rounded, .monospaced, .named("Georgia"), .named("Courier New"), .named("Menlo")] {
            let style = WTypeStyle(labelFamily: family, labelCaps: .small)
            let theme = WTheme(palette: [:], variant: .light, type: style)
            let spec = WTextRole.sectionLabelSpec(style: style)
            if WFontSupport.hasSmallCaps(family) {
                #expect(theme.sectionLabel("Risks") == "Risks", "\(family)")
                #expect(spec.smallCaps && spec.size == 12, "\(family)")
            } else {
                // never shrunk lower case: capitals, at a smaller size
                #expect(theme.sectionLabel("Risks") == "RISKS", "\(family)")
                #expect(!spec.smallCaps && spec.size < 12, "\(family)")
            }
        }
    }

    @Test func aMissingFamilyAsksTheSystemFaceAboutSmallCaps() {
        #expect(WFontSupport.hasSmallCaps(.named("No Such Typeface 9000")) == WFontSupport.hasSmallCaps(.system))
    }

    @Test func aBracketedLabelCanKeepCapitalsWhateverTheSkinSays() {
        let style = WTypeStyle(labelCaps: .small)
        let upper = WTextRole.sectionLabelSpec(style: style, caps: .upper)
        #expect(!upper.smallCaps)
        #expect(upper.size == 12)
    }

    @Test func labelSizeFollowsTheTextScale() {
        #expect(WTextRole.sectionLabelSpec(scale: 1.5, style: .thread).size == 18)
    }
}
