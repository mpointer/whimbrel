import SwiftUI
import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the onboarding adapters (`UI/Redesign`): what state a permission shows, which buttons a
/// blocking screen has, and how a skin becomes a `WTheme`. Nothing here touches the app's defaults or launches it.
final class RedesignOnboardingAdapterTests: XCTestCase {
    func testPermissionState() {
        XCTAssertEqual(PermissionRedesign.state(granted: true, systemDenied: true, askedHere: true), .allowed)
        XCTAssertEqual(PermissionRedesign.state(granted: false, systemDenied: false, askedHere: false), .notYet)
        XCTAssertEqual(PermissionRedesign.state(granted: false, systemDenied: true, askedHere: false), .denied)
        XCTAssertEqual(PermissionRedesign.state(granted: false, systemDenied: false, askedHere: true), .denied)
    }

    func testRequiredAndOptionalCoverEveryKindOnce() {
        XCTAssertEqual(Set(PermissionRedesign.required + PermissionRedesign.optional), Set(PermissionKind.allCases))
        XCTAssertEqual(PermissionRedesign.required, PermissionKind.firstRun)
    }

    func testRedesignReasonsHaveNoDashesOrDeveloperTerms() {
        for kind in PermissionKind.allCases {
            let text = PermissionRedesign.reason(kind)
            XCTAssertFalse(text.contains("\u{2014}") || text.contains("\u{2013}"), text)
            XCTAssertFalse(text.contains("FR-"), text)
        }
    }

    func testBlockingScreenButtonsQuitFirstPrimaryLast() {
        let missing = LaunchScreenContent.of(.encryptedFileMissing(backupPresent: true, encryptedCopyPresent: true, lastAction: nil))
        var m = LaunchBlockingRedesignView.model(missing, busy: false, quit: {}, restore: {}, reset: {}, continueWithout: {}, retry: {})
        XCTAssertEqual(m.actions.map(\.id), ["quit", "restore"])
        XCTAssertEqual(m.actions.last?.kind, .primary)
        let failed = LaunchScreenContent.of(.error(.plainOpenFailed))
        m = LaunchBlockingRedesignView.model(failed, busy: false, quit: {}, restore: {}, reset: {}, continueWithout: {}, retry: {})
        XCTAssertEqual(m.actions.map(\.id), ["quit", "continueWithoutStore", "retry"])
        m = LaunchBlockingRedesignView.model(failed, busy: true, quit: {}, restore: {}, reset: {}, continueWithout: {}, retry: {})
        XCTAssertTrue(m.actions.allSatisfy { !$0.isEnabled })
        XCTAssertEqual(m.actions.last?.title, EncryptionCopy.trying)
    }

    /// Richer skins row 9: a skin's texture reaches the theme; Thread and the High Contrast skin draw none, and high contrast turns it off.
    func testSkinTextureReachesTheThemeAndThreadDrawsNone() {
        var skin = SkinDefaults.whimbrel
        skin.id = "almanac"
        for (asked, expected) in [(SkinTexture.none, WTexture.none), (.paper, .paper), (.grain, .grain), (.grid, .grid)] {
            skin.decoration.texture = asked
            XCTAssertEqual(RedesignBridge.texture(skin), expected)
        }
        for id in ["thread", "high-contrast"] {
            var plain = skin
            plain.id = id
            plain.decoration.texture = .grain
            XCTAssertEqual(RedesignBridge.texture(plain), .none, id)
        }
        skin.decoration.texture = .grid
        let on = WTheme(palette: [:], colorScheme: .light, contrast: .standard, texture: RedesignBridge.texture(skin))
        let increased = WTheme(palette: [:], colorScheme: .light, contrast: .increased, texture: RedesignBridge.texture(skin))
        XCTAssertTrue(on.drawsTexture)
        XCTAssertFalse(increased.drawsTexture)
    }

    /// Almanac as `skins/almanac.json` shapes it: a double border of one point, radius 2, airy.
    func testAlmanacShapeDrawsADoubleBorder() {
        var almanac = SkinDefaults.whimbrel
        almanac.id = "almanac"
        almanac.shape = SkinShape(radius: 2, border: .double, borderWidth: 1, density: .airy)
        let shape = RedesignBridge.shape(almanac)
        XCTAssertEqual(shape.borderStyle, .double)
        XCTAssertEqual(shape.borderWidth, 1)
        XCTAssertEqual(shape.cardRadius, 2)
        XCTAssertEqual(shape.surfaceRadius, 3)
        XCTAssertEqual(shape.density, .roomy)
        let theme = WTheme(palette: [:], variant: .light, shape: shape)
        XCTAssertTrue(theme.drawsDoubleBorder(width: theme.borderWidth))
        XCTAssertEqual(WBorderGeometry.insets(width: theme.borderWidth, double: true), [0, 1, 2, 3])
        // the other styles keep their own
        for (border, expected) in [(SkinBorder.solid, WBorderStyle.solid), (.dashed, .dashed), (.none, .none)] {
            var skin = almanac
            skin.shape.border = border
            XCTAssertEqual(RedesignBridge.shape(skin).borderStyle, expected)
        }
        // Thread keeps its own shape whatever the manifest says
        var thread = almanac
        thread.id = "thread"
        XCTAssertEqual(RedesignBridge.shape(thread), .thread)
    }

    /// Richer skins, row 10: a skin's motion reaches the theme. Thread keeps what it has, whatever its file says.
    func testMotionReachesTheThemeAndThreadKeepsItsOwn() {
        var almanac = SkinDefaults.whimbrel
        almanac.id = "almanac"
        almanac.motion = .subtle
        XCTAssertEqual(RedesignBridge.motion(almanac), .subtle)
        almanac.motion = .none
        XCTAssertEqual(RedesignBridge.motion(almanac), .none)
        var thread = almanac
        thread.id = "thread"
        thread.motion = .subtle
        XCTAssertEqual(RedesignBridge.motion(thread), .thread)
        XCTAssertEqual(WMotion.thread, WMotion.none)
    }

    /// Richer skins row 6: the skin's divider style reaches the shape the Home views read, and Thread has none.
    func testSkinDividersReachTheShape() {
        var skin = SkinDefaults.whimbrel
        skin.id = "almanac"
        for (divider, expected) in [(SkinDivider.none, WDividerStyle.none), (.hairline, .hairline), (.chalk, .chalk), (.double, .double), (.dotted, .dotted)] {
            skin.decoration.dividers = divider
            XCTAssertEqual(RedesignBridge.shape(skin).dividers, expected, "\(divider)")
        }
        skin.id = "thread"
        skin.decoration.dividers = .double
        XCTAssertEqual(RedesignBridge.shape(skin), .thread)
        XCTAssertEqual(WShape.thread.dividers, .none)
    }

    func testThreadSkinKeepsThreadShape() {
        XCTAssertEqual(RedesignBridge.shape(SkinDefaults.whimbrel).cardRadius, SkinDefaults.whimbrel.shape.radius)
        XCTAssertEqual(RedesignBridge.family(.newYork), .serif)
        XCTAssertEqual(RedesignBridge.family(.avenirNext), .named("Avenir Next"))
    }
}

/// Richer skins, row 3: `RedesignBridge.type` turns each real built-in skin (`skins/*.json`) (spelled as `skins/*.json` spells them) into the type style the
/// WhimbrelUI roles read. Thread keeps the package's own style, so it draws as it always did.
@MainActor
final class RedesignSkinTypeTests: XCTestCase {
    /// A skin with the typography `skins/<id>.json` gives it, spelled as the manifests spell it. It is built here and not read from
    /// disk: listing the skins folder from the test host blocks on this Mac.
    private func skin(_ id: String, display: String, body: String, numerals: String, label: String, caps: SkinCaps) -> Skin {
        var skin = SkinDefaults.whimbrel
        skin.id = id
        skin.typography = SkinTypography(
            display: SkinFace(display)!, body: SkinFace(body)!, numerals: SkinFace(numerals)!, label: SkinFace(label)!, caps: caps)
        return skin
    }

    private func skin(_ id: String) throws -> Skin {
        switch id {
        case "thread": return skin(id, display: "SF Pro/semibold", body: "SF Pro/regular", numerals: "SF Pro/semibold", label: "SF Pro/regular", caps: .none)
        case "whimbrel": return skin(id, display: "New York/semibold", body: "SF Pro/regular", numerals: "SF Pro/regular", label: "SF Pro/regular", caps: .upper)
        case "hypermodern": return skin(id, display: "SF Pro/thin", body: "SF Pro/regular", numerals: "SF Pro/light", label: "SF Pro/medium", caps: .none)
        case "game-day": return skin(id, display: "Avenir Next/heavy", body: "Avenir Next/regular", numerals: "Helvetica Neue/bold", label: "Avenir Next/semibold", caps: .upper)
        case "almanac": return skin(id, display: "Georgia/bold", body: "Georgia/regular", numerals: "Georgia/regular", label: "New York/semibold", caps: .small)
        case "italic": return skin(id, display: "Georgia/bold/italic", body: "Georgia/regular/italic", numerals: "Georgia/regular/italic", label: "Georgia/medium/italic", caps: .none)
        default: throw XCTSkip("no skin \(id)")
        }
    }

    func testThreadKeepsThePackagesOwnTypeStyle() throws {
        let style = RedesignBridge.type(try skin("thread"))
        XCTAssertEqual(style, .thread)
        for role in WTextRole.allCases {
            XCTAssertEqual(role.font(style: style), role.font(style: .thread), "\(role)")
            XCTAssertEqual(role.spec(style: style), role.spec(style: .thread), "\(role)")
        }
        XCTAssertEqual(WTextRole.sectionLabelSpec(style: style).weight, .semibold)
    }

    func testClassicSetsTitlesInNewYork() throws {
        let style = RedesignBridge.type(try skin("whimbrel"))
        XCTAssertEqual(WTextRole.title.spec(style: style).family, .serif)
        XCTAssertEqual(WTextRole.title.spec(style: style).weight, .semibold)
        XCTAssertEqual(WTextRole.body.spec(style: style).family, .system)
        XCTAssertEqual(WTextRole.time.spec(style: style).weight, .regular)
        XCTAssertEqual(style.labelCaps, .upper)
        XCTAssertEqual(WTextRole.reading.spec(style: style), WTextRole.reading.spec(style: .thread))
    }

    func testHypermodernIsThin() throws {
        let style = RedesignBridge.type(try skin("hypermodern"))
        for role in [WTextRole.subtitle, .title, .pageTitle] { XCTAssertEqual(role.spec(style: style).weight, .thin, "\(role)") }
        XCTAssertEqual(WTextRole.time.spec(style: style).weight, .light)
        XCTAssertEqual(WTextRole.caption.spec(style: style).weight, .medium)
        XCTAssertEqual(style.labelCaps, WLabelCaps.none)
    }

    func testGameDayIsHeavyInAvenirWithBoldNumerals() throws {
        let style = RedesignBridge.type(try skin("game-day"))
        for role in [WTextRole.subtitle, .title, .pageTitle] {
            XCTAssertEqual(role.weight(in: style), .heavy, "\(role)")
            XCTAssertEqual(role.family(in: style), .named("Avenir Next"), "\(role)")
        }
        XCTAssertEqual(WTextRole.time.weight(in: style), .bold)
        XCTAssertEqual(WTextRole.time.family(in: style), .named("Helvetica Neue"))
        XCTAssertEqual(WTextRole.caption.weight(in: style), .semibold)
        XCTAssertEqual(style.labelCaps, .upper)
    }

    func testAlmanacUsesSmallCaps() throws {
        let style = RedesignBridge.type(try skin("almanac"))
        XCTAssertEqual(style.labelCaps, .small)
        XCTAssertEqual(WTextRole.title.weight(in: style), .bold)
        XCTAssertEqual(WTextRole.sectionLabelSpec(style: style).weight, .semibold)
    }

    func testItalicReachesDisplayAndLabelOnly() throws {
        let style = RedesignBridge.type(try skin("italic"))
        XCTAssertTrue(WTextRole.title.spec(style: style).italic)
        XCTAssertTrue(WTextRole.caption.spec(style: style).italic)
        XCTAssertTrue(WTextRole.sectionLabelSpec(style: style).italic)
        for role in [WTextRole.body, .bodyStrong, .secondary, .time, .reading, .readingTitle] {
            XCTAssertFalse(role.spec(style: style).italic, "\(role)")
        }
        XCTAssertEqual(WTextRole.reading.spec(style: style), WTextRole.reading.spec(style: .thread))
    }

    func testAFamilyThisMacLacksDrawsAsTheSystemFace() {
        let style = WTypeStyle(displayFamily: .named("No Such Typeface 9000"), displayWeight: .heavy)
        XCTAssertEqual(WTextRole.title.spec(style: style).family, .system)
        XCTAssertEqual(WTextRole.title.spec(style: style).weight, .heavy)
    }
}
