import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The Home element contrast pairs and catalogue entry are opt-in: with the redesign flag off, repair and the
/// skin maker's catalogue are what they were. Reads the bundled skins and palettes from the repository; nothing
/// touches the app's defaults.
final class HomeElementPairsTests: XCTestCase {
    private let repo = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()

    private func palettes() -> [Theme] {
        ThemeLoader.loadDirectory(repo.appendingPathComponent("themes")) + ThemeLoader.loadDirectory(repo.appendingPathComponent("skins/palettes")) + [ThemeDefaults.theme]
    }

    func testRepairWithTheRedesignOffEqualsToday() {
        let all = palettes()
        XCTAssertGreaterThan(all.count, 3)
        for theme in all {
            for palette in [theme.light, theme.dark] {
                let a = PaletteRepair.repair(palette)
                let b = PaletteRepair.repair(palette, redesign: nil)
                XCTAssertEqual(a.palette, b.palette, theme.id)
                XCTAssertEqual(a.changed, b.changed, theme.id)
            }
        }
    }

    func testTodaysPairTablesAreNotTouched() {
        // Adding the opt-in entry point must not add to the tables every archetype test and repair read:
        // `all` is exactly the archetypes' own pairs, once each, and an element-only surface is not in it.
        func key(_ p: ArchetypeContrast.Pair) -> String { "\(p.text.rawValue) on \(p.name)" }
        let fromArchetypes = Set(SkinArchetype.allCases.flatMap(ArchetypeContrast.pairs(for:)).map(key))
        let all = ArchetypeContrast.all.map(key)
        XCTAssertEqual(Set(all), fromArchetypes)
        XCTAssertEqual(all.count, fromArchetypes.count, "each pair once")
        // `textOnAccent on accentPrimary` has always been required of every palette (`ThemeContrast.requiredPairs`),
        // so the scoreboard's pair adds nothing new; the text link on the page is the standard archetype's only
        // element-only pair.
        XCTAssertTrue(all.contains("textOnAccent on accentPrimary"))
        XCTAssertFalse(Set(ArchetypeContrast.pairs(for: .standard).map(key)).contains("link on backgroundPage"))
    }

    func testElementPairsFollowTheChoices() {
        func keys(_ e: WHomeElements) -> Set<String> { Set(ArchetypeContrast.elementPairs(e).map { "\($0.text.rawValue) on \($0.name)" }) }
        XCTAssertEqual(keys(WHomeElements()), [])
        XCTAssertEqual(keys(WHomeElements(nextMeeting: .scoreboard)), ["textOnAccent on accentPrimary"])
        XCTAssertEqual(keys(WHomeElements(actions: .links)), ["link on backgroundPage"])
        let boxed: Set<String> = ["textPrimary on backgroundCard", "textPrimary on backgroundPage", "textSecondary on backgroundCard", "textSecondary on backgroundPage"]
        XCTAssertEqual(keys(WHomeElements(nextMeeting: .infobox)), boxed)
        XCTAssertEqual(keys(WHomeElements(labels: .bracketed)), boxed)
        XCTAssertEqual(keys(WHomeElements(labels: .smallCaps)), boxed)
        XCTAssertEqual(keys(WHomeElements(ask: .prompt)), [])
        XCTAssertEqual(keys(WHomeElements(timeline: .ruled)), [])
    }

    func testRepairWithTheRedesignOnCoversEveryElementPair() {
        var palette = ThemeDefaults.theme.light
        palette[.textOnAccent] = 0xCCCCCC
        palette[.accentPrimary] = 0xDDDDDD
        palette[.link] = 0xEEEEEE
        palette[.backgroundPage] = 0xFFFFFF
        let repaired = PaletteRepair.repair(palette, redesign: WHomeElements.everyVariant).palette
        let pairs = ArchetypeContrast.withElementPairs(ArchetypeContrast.all, for: WHomeElements.everyVariant)
        XCTAssertEqual(ArchetypeContrast.failures(repaired, pairs), [])
    }

    func testCatalogueGainsTheTodayKeyOnlyWithTheRedesignOn() throws {
        let off = SkinCatalogue.object()
        XCTAssertNil(off["today"])
        XCTAssertNil(off["archetypes"])
        XCTAssertNil(SkinCatalogue.object(redesign: true)["archetypes"])
        let keys = try XCTUnwrap(((SkinCatalogue.object(redesign: true)["today"] as? [String: Any])?["options"]) as? [String: [String]]).keys
        XCTAssertEqual(Set(keys), ["nextMeeting", "timeline", "labels", "ask", "actions"])
        XCTAssertEqual(NSDictionary(dictionary: off), NSDictionary(dictionary: SkinCatalogue.object(redesign: false)))
        let on = SkinCatalogue.object(redesign: true)
        let today = try XCTUnwrap(on["today"] as? [String: Any])
        let options = try XCTUnwrap(today["options"] as? [String: [String]])
        XCTAssertEqual(options["nextMeeting"], ["card", "scoreboard", "infobox"])
        XCTAssertEqual(options["timeline"], ["line", "ruled"])
        XCTAssertEqual(options["labels"], ["plain", "bracketed", "smallCaps"])
        XCTAssertEqual(options["ask"], ["field", "prompt"])
        XCTAssertEqual(options["actions"], ["buttons", "links"])
        XCTAssertEqual(today["rule"] as? String, "set today.options to any of these keys; a key left out keeps the old archetype's choice, an unknown value is ignored")
        var rest = on
        rest["today"] = nil
        XCTAssertEqual(NSDictionary(dictionary: rest), NSDictionary(dictionary: off))
    }

    // MARK: richer skins row 12: the built-in skins' Home elements

    private var bundledSkins: [Skin] { SkinLoader.loadDirectory(repo.appendingPathComponent("skins")) }

    private func bundled(_ id: String) throws -> Skin { try XCTUnwrap(bundledSkins.first { $0.id == id }, id) }

    private func elements(_ skin: Skin) -> WHomeElements {
        WHomeElements.resolve(options: skin.archetypeOptions, archetype: skin.archetype.rawValue)
    }

    private var planned: [(id: String, elements: WHomeElements)] {
        [("whimbrel", WHomeElements(nextMeeting: .infobox, timeline: .line, labels: .smallCaps, ask: .field, actions: .buttons)),
         ("high-contrast", WHomeElements(nextMeeting: .infobox, timeline: .ruled, labels: .plain, ask: .field, actions: .buttons)),
         ("hypermodern", WHomeElements(nextMeeting: .scoreboard, timeline: .line, labels: .plain, ask: .prompt, actions: .buttons))]
    }

    func testClassicHighContrastAndHypermodernResolveToTheirOwnElements() throws {
        for (id, want) in planned {
            let skin = try bundled(id)
            XCTAssertEqual(elements(skin), want, id)
            XCTAssertNotEqual(elements(skin), WHomeElements(), "\(id) still looks like the default")
            XCTAssertEqual(Set(skin.archetypeOptions.keys), ["nextMeeting", "timeline", "labels", "ask", "actions"], id)
        }
        XCTAssertEqual(try bundled("hypermodern").archetype, .glass, "the archetype stays, so an older app draws its old layout")
        XCTAssertEqual(try bundled("high-contrast").archetype, .standard)
        XCTAssertEqual(SkinDefaults.whimbrel.archetypeOptions, try bundled("whimbrel").archetypeOptions)
    }

    func testTheElementPairsPassOnEveryPaletteOfTheThreeAsShipped() throws {
        let pool = palettes()
        for (id, want) in planned {
            let skin = try bundled(id)
            let pairs = ArchetypeContrast.withElementPairs(ArchetypeContrast.pairs(for: .standard), for: [want])
            XCTAssertFalse(skin.paletteIds.isEmpty, id)
            for pid in skin.paletteIds {
                let theme = try XCTUnwrap(pool.first { $0.id == pid }, "\(id): palette \(pid)")
                XCTAssertEqual(ArchetypeContrast.failures(theme.light, pairs), [], "\(id) on \(pid) light")
                XCTAssertEqual(ArchetypeContrast.failures(theme.dark, pairs), [], "\(id) on \(pid) dark")
            }
        }
    }

    func testThreadIsUnchangedAndOldSyncedCopiesStillLoad() throws {
        let thread = try bundled("thread")
        XCTAssertEqual(elements(thread), WHomeElements())
        XCTAssertEqual(thread.archetypeOptions, ["actions": "buttons", "ask": "field", "labels": "plain", "nextMeeting": "card", "timeline": "line"])
        // An older Whimbrel wrote these skins with no element keys; sync can still hand that copy to this app.
        for (id, _) in planned {
            for today in [#""today": {"archetype": "standard", "options": {}}"#, #""today": {"archetype": "standard"}"#, ""] {
                let json = #"{"id": "\#(id)", "name": "Old", "palettes": ["default"], \#(today)}"#.replacingOccurrences(of: ", }", with: "}")
                let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(Data(json.utf8), base: SkinDefaults.whimbrel), id)
                XCTAssertEqual(repairs, [], id)
                XCTAssertEqual(elements(skin), WHomeElements(), "\(id): an old copy draws its old Home")
            }
        }
    }
}
