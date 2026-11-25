import SwiftUI
import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the redesigned Settings window (`UI/Redesign/Settings`): the section list and titles, search, the
/// anchor ids, and the General, Profile and Plan & billing models built from app values. Nothing here opens a window,
/// reads the app's defaults or depends on the machine's language, time zone or date: words are compared with the same
/// `String(localized:)` lookups the code uses, and dates and prices go through the functions the classic pane calls.
final class SettingsRedesignTests: XCTestCase {
    // MARK: Sections and titles

    func testSectionListIsTheClassicListWithTheSameOrderAndVisibility() {
        let items = SettingsRedesign.navItems(pendingSuggestions: 0)
        XCTAssertEqual(items.map(\.id), SettingsSection.visibleCases.map(\.rawValue))
        XCTAssertEqual(items.map(\.icon), SettingsSection.visibleCases.map(\.systemImage))
        #if DEBUG
        XCTAssertTrue(items.contains { $0.id == SettingsSection.diagnostics.rawValue })
        #else
        XCTAssertFalse(items.contains { $0.id == SettingsSection.diagnostics.rawValue })
        #endif
    }

    func testOnlyTheCaptureSectionIsRenamed() {
        let memory = String(localized: "Memory & Privacy", comment: "Name of the Settings section for what Whimbrel remembers and keeps private. Settings search still finds it by the word capture.")
        for section in SettingsSection.allCases {
            XCTAssertEqual(SettingsRedesign.title(section), section == .capture ? memory : section.title, "\(section)")
        }
    }

    func testTheProfileBadgeShowsOnlyWhileSuggestionsWait() {
        XCTAssertNil(SettingsRedesign.navItems(pendingSuggestions: 0).first { $0.id == "profile" }?.badge)
        let withBadge = SettingsRedesign.navItems(pendingSuggestions: 3)
        XCTAssertEqual(withBadge.first { $0.id == "profile" }?.badge, "3")
        XCTAssertTrue(withBadge.filter { $0.id != "profile" }.allSatisfy { $0.badge == nil })
    }

    // MARK: Search

    func testSearchFindsWhatTheClassicSearchFindsInTheSameOrder() {
        for query in ["capture", "privacy", "update", "language", "refund", "cancel", "microphone", "key", "skin", "sync", "mcp", "  Plan  ", "zzzz", "", "   "] {
            XCTAssertEqual(SettingsRedesign.results(for: query).map(\.id), SettingsSearch.results(for: query).map(\.id), query)
        }
    }

    func testResultsNameTheSectionAsTheRedesignedWindowShowsIt() {
        for query in ["capture", "language", "refund", "mcp"] {
            let classic = SettingsSearch.results(for: query)
            let redesigned = SettingsRedesign.results(for: query)
            XCTAssertEqual(redesigned.map(\.section), classic.map { SettingsRedesign.title($0.section) }, query)
            XCTAssertEqual(redesigned.map(\.title), classic.map(\.title), query)
        }
    }

    func testCaptureStillFindsMemoryAndPrivacy() {
        let hits = SettingsRedesign.results(for: "capture")
        XCTAssertTrue(hits.contains { $0.id == SettingsAnchorID.captureYourData.rawValue })
        XCTAssertEqual(hits.first { $0.id == SettingsAnchorID.captureYourData.rawValue }?.section, SettingsRedesign.title(.capture))
    }

    /// The "memory" keyword matches whole words of the query, with a fixed section name so no language or catalog is involved.
    func testTheMemoryKeywordMatchesWholeWordsOnly() {
        let name = "Memory & Privacy"
        for query in ["memory", "Memory", "  MEMORY ", "privacy", "memory privacy", "memory & privacy", "Memory and"] {
            XCTAssertEqual(SettingsRedesign.matchesSectionName(query, name: name), query != "Memory and", query)
        }
        for query in ["m", "me", "&", "&&", "mem", "y", "pri", "memor y", "memories", "", "   ", "memory x"] {
            XCTAssertFalse(SettingsRedesign.matchesSectionName(query, name: name), query)
        }
        // Accents and case do not matter.
        XCTAssertTrue(SettingsRedesign.matchesSectionName("MEMÓRIA", name: "Memória e privacidade", extraKeywords: []))
    }

    func testTypingOneLetterOrAnAmpersandAddsNoCaptureEntry() {
        // The section title is passed in, so the test does not depend on the machine's language: in Portuguese it holds the word "e".
        for title in ["Memory & Privacy", "Memória e privacidade", "Speicher und Datenschutz"] {
            for query in ["m", "&", "e", "de", "un"] {
                let extra = Set(SettingsRedesign.entries(for: query, captureTitle: title).map(\.id)).subtracting(SettingsSearch.results(for: query).map(\.id))
                XCTAssertTrue(extra.isEmpty, "\(query) in \(title)")
            }
        }
        XCTAssertFalse(SettingsRedesign.matchesSectionName("e", name: "Memória e privacidade", extraKeywords: []))
        XCTAssertTrue(SettingsRedesign.matchesSectionName("privacidade", name: "Memória e privacidade"))
    }

    func testEveryResultOpensItsRegisteredSection() {
        for entry in SettingsSearch.entries {
            XCTAssertEqual(SettingsRedesign.section(forResult: entry.id), entry.section, entry.id)
        }
        XCTAssertNil(SettingsRedesign.section(forResult: "no.such.id"))
    }

    // MARK: Anchors

    func testTheTypedAnchorsAreTheRegistryIds() {
        XCTAssertEqual(Set(SettingsAnchorID.allCases.map(\.rawValue)), Set(SettingsSearch.entries.map(\.id)))
        XCTAssertEqual(SettingsAnchorID.allCases.count, SettingsSearch.entries.count)
    }

    /// A redrawn section declares every anchor id its classic section has, so search keeps landing on it. A section that
    /// still hosts its classic panes declares none: those panes carry the ids themselves.
    func testEveryRedrawnSectionDeclaresEveryAnchorTheClassicSectionHas() {
        for section in SettingsSection.allCases {
            let registry = Set(SettingsSearch.entries.filter { $0.section == section }.map(\.id))
            if let declared = SettingsRedesign.declaredAnchorIDs(for: section) {
                XCTAssertEqual(Set(declared), registry, "\(section)")
                XCTAssertEqual(declared.count, Set(declared).count, "\(section) declares an id twice")
            } else {
                XCTAssertTrue(SettingsRedesign.hostsClassicPanes(section), "\(section)")
            }
        }
        XCTAssertEqual(Set(SettingsSection.allCases.filter { SettingsRedesign.declaredAnchorIDs(for: $0) != nil }), [.general, .profile, .plan, .capture, .meetings, .ai, .topics, .devices, .connectors, .automations])
    }

    func testTheModelsCarryTheDeclaredAnchors() {
        XCTAssertEqual(generalModel().anchors.all, SettingsAnchorID.general.all)
        XCTAssertEqual(profileModel().anchors.all, SettingsAnchorID.profile.all)
        XCTAssertEqual(planModel(status: .notSignedIn).anchors.all, SettingsAnchorID.plan.all)
    }

    /// `SettingsSearchTests` wants each classic literal exactly once under `Whimbrel/UI`, so nothing in the redesign folder
    /// may write one. The redrawn views take their ids from `SettingsAnchorID` as data.
    func testNoFileInTheRedesignFolderWritesTheClassicAnchorCall() throws {
        let folder = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI/Redesign/Settings")
        let files = try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil).filter { $0.pathExtension == "swift" }
        XCTAssertGreaterThanOrEqual(files.count, 15)
        let needle = ".settingsAnchor" + "(\""
        for file in files {
            XCTAssertFalse(try String(contentsOf: file, encoding: .utf8).contains(needle), file.lastPathComponent)
        }
    }

    // MARK: Text scale

    func testTextScaleGrowsWithTheTextSize() {
        XCTAssertEqual(SettingsRedesign.textScale(for: .large), 1)
        XCTAssertEqual(SettingsRedesign.textScale(for: .accessibility3), 2)
        let scales = DynamicTypeSize.allCases.map(SettingsRedesign.textScale(for:))
        XCTAssertEqual(scales, scales.sorted())
    }

    // MARK: General

    private func skin(_ id: String, _ name: String) -> Skin {
        var s = SkinDefaults.whimbrel
        s.id = id
        s.name = name
        return s
    }

    private func generalInput(
        update: UpdateChecker.UpdateStatus = .idle, staged: Bool = false, blocked: Bool = false, languageChanged: Bool = false,
        locked: String? = nil, skins: WSkinGalleryModel = WSkinGalleryModel()
    ) -> GeneralSettingsAdapter.Input {
        GeneralSettingsAdapter.Input(
            appVersion: "9.9.9", coreVersion: "8.8.8", update: update, hasStagedUpdate: staged, restartBlockedByRecording: blocked,
            languageOverride: "", languageChanged: languageChanged, languageTags: ["es", "fr"], writeIn: "", writeInTags: ["en-US", "es-US"],
            localModelNote: nil, appearance: .dark, appearanceLockedCaption: locked, skins: skins)
    }

    private func generalModel(_ input: GeneralSettingsAdapter.Input? = nil) -> WGeneralSettingsModel {
        GeneralSettingsAdapter.model(input ?? generalInput(), languageName: { "name:\($0)" })
    }

    func testGeneralVersionsAndAppearance() {
        let m = generalModel()
        XCTAssertEqual(m.appVersion, "9.9.9")
        XCTAssertEqual(m.coreVersion, "8.8.8")
        XCTAssertEqual(m.appearanceOptions.map(\.id), AppearanceChoice.allCases.map(\.rawValue))
        XCTAssertEqual(m.appearanceSelection, AppearanceChoice.dark.rawValue)
        XCTAssertNil(m.appearanceLockedCaption)
        XCTAssertEqual(generalModel(generalInput(locked: "why")).appearanceLockedCaption, "why")
    }

    func testGeneralLeavesVersionUnknownWhenTheBundleHasNone() {
        var input = generalInput()
        input.appVersion = nil
        XCTAssertEqual(generalModel(input).appVersion, "unknown")
    }

    func testGeneralIdleHasNoUpdateLineAndNoRestart() {
        let m = generalModel()
        XCTAssertNil(m.updateStatus)
        XCTAssertFalse(m.updateBusy)
        XCTAssertFalse(m.restartAvailable)
        XCTAssertNil(m.restartBlockedNote)
    }

    func testGeneralWhileCheckingTheButtonWaits() {
        let m = generalModel(generalInput(update: .checking))
        XCTAssertTrue(m.updateBusy)
        XCTAssertEqual(m.updateStatus, "Checking…")
        XCTAssertTrue(generalModel(generalInput(update: .downloading(version: "1.2.3"))).updateBusy)
    }

    func testGeneralWithAnUpdateReadyOffersRestartAndSaysWhyItWasRefused() {
        let ready = generalModel(generalInput(update: .ready(version: "1.2.3"), staged: true))
        XCTAssertEqual(ready.updateStatus, "v1.2.3 is ready.")
        XCTAssertTrue(ready.restartAvailable)
        XCTAssertFalse(ready.updateBusy)
        let blocked = generalModel(generalInput(update: .ready(version: "1.2.3"), staged: true, blocked: true))
        XCTAssertEqual(blocked.restartBlockedNote, String(localized: "Finish or stop the current recording before restarting to install the update."))
    }

    func testGeneralKeepsTheRestartButtonForAStagedUpdateAfterAFailedCheck() {
        let m = generalModel(generalInput(update: .failed, staged: true))
        XCTAssertTrue(m.restartAvailable)
        XCTAssertEqual(m.updateStatus, "Couldn't check for updates. Try again later.")
    }

    func testGeneralUpdateLinesMatchTheClassicPaneWords() {
        XCTAssertEqual(GeneralSettingsAdapter.updateStatusLine(.upToDate(version: "2.0")), "Whimbrel is up to date (v2.0).")
        XCTAssertEqual(GeneralSettingsAdapter.updateStatusLine(.downloading(version: "2.0")), "Downloading v2.0…")
        XCTAssertNil(GeneralSettingsAdapter.updateStatusLine(.idle))
    }

    func testGeneralLanguageChoicesStartWithTheDefaultAndNoteAChange() {
        let m = generalModel(generalInput(languageChanged: true))
        XCTAssertEqual(m.languageOptions.map(\.id), ["", "es", "fr"])
        XCTAssertEqual(m.languageOptions.first?.title, String(localized: "Automatic"))
        XCTAssertEqual(m.languageOptions.last?.title, "name:fr")
        XCTAssertEqual(m.writeInOptions.map(\.id), ["", "en-US", "es-US"])
        XCTAssertEqual(m.writeInOptions.first?.title, String(localized: "Same as app language"))
        XCTAssertEqual(m.languageChangedNote, String(localized: "Quit and reopen Whimbrel to use the new language."))
        XCTAssertNil(generalModel().languageChangedNote)
    }

    func testGeneralLocalModelNoteOnlyForANonEnglishOutputWithOllama() {
        func name(_ tag: String) -> String { "name:\(tag)" }
        XCTAssertNotNil(GeneralSettingsAdapter.localModelNote(outputTag: "es-US", usesOllama: true, languageName: name))
        XCTAssertNil(GeneralSettingsAdapter.localModelNote(outputTag: "es-US", usesOllama: false, languageName: name))
        XCTAssertNil(GeneralSettingsAdapter.localModelNote(outputTag: "en-GB", usesOllama: true, languageName: name))
        XCTAssertNil(GeneralSettingsAdapter.localModelNote(outputTag: "", usesOllama: true, languageName: name))
    }

    func testSkinGalleryWithNoSkinsHasNoCards() {
        let m = SkinGalleryAdapter.model(skins: [], redesign: false, selectedSkinId: "x", selectedPaletteId: "y", palettes: { _ in [] }, dark: false, isBusy: false)
        XCTAssertTrue(m.cards.isEmpty)
        XCTAssertFalse(m.isBusy)
        XCTAssertEqual(m.labels.importTitle, String(localized: "Import Skin…"))
        XCTAssertTrue(generalModel(generalInput(skins: m)).skins.cards.isEmpty)
    }

    func testSkinGalleryListsBuiltInsThenYoursAndMarksTheSelectedOne() {
        let builtIn = skin("alpha", "Alpha")
        let yoursB = skin("user-b", "Bravo")
        let yoursA = skin("user-a", "Able")
        let theme = ThemeDefaults.theme
        let m = SkinGalleryAdapter.model(
            skins: [yoursB, builtIn, yoursA], redesign: false, selectedSkinId: "alpha", selectedPaletteId: theme.id,
            palettes: { _ in [theme] }, dark: false, isBusy: true)
        XCTAssertEqual(m.cards.map(\.id), ["alpha", "user-a", "user-b"])
        XCTAssertEqual(m.cards.map(\.isYours), [false, true, true])
        XCTAssertEqual(m.cards.map(\.isSelected), [true, false, false])
        XCTAssertEqual(m.cards.map(\.anchorID), ["alpha", "user-a", "user-b"].map(SkinStore.galleryCardId))
        XCTAssertEqual(m.cards[0].selectedPalette, theme.id)
        XCTAssertEqual(m.cards[0].palettes.map(\.id), [theme.id])
        XCTAssertTrue(m.cards[1].palettes.isEmpty)
        XCTAssertNil(m.cards[1].selectedPalette)
        XCTAssertTrue(m.isBusy)
    }

    func testTheThreadSkinIsListedOnlyWithTheSwitchOn() {
        let thread = skin("thread", "Thread")
        let off = SkinGalleryAdapter.model(skins: [thread], redesign: false, selectedSkinId: "", selectedPaletteId: "", palettes: { _ in [] }, dark: false, isBusy: false)
        let on = SkinGalleryAdapter.model(skins: [thread], redesign: true, selectedSkinId: "", selectedPaletteId: "", palettes: { _ in [ThemeDefaults.theme] }, dark: false, isBusy: false)
        XCTAssertTrue(off.cards.isEmpty)
        XCTAssertEqual(on.cards.map(\.id), ["thread"])
    }

    func testPaletteDotsFollowTheAppearance() {
        let theme = ThemeDefaults.theme
        XCTAssertEqual(SkinGalleryAdapter.paletteDot(theme, dark: false).colors, [theme.light[.accentPrimary] ?? 0, theme.light[.backgroundPage] ?? 0])
        XCTAssertEqual(SkinGalleryAdapter.paletteDot(theme, dark: true).colors, [theme.dark[.accentPrimary] ?? 0, theme.dark[.backgroundPage] ?? 0])
    }

    // MARK: Profile

    private func profileModel(
        facts: [ProfileFact] = [], suggestions: [CoreProfileSuggestion] = [], routing: String = ""
    ) -> WProfileSettingsModel {
        ProfileSettingsAdapter.model(ProfileSettingsAdapter.Input(
            enabled: true, suggestEnabled: false, name: "N", role: "R", organization: "O", about: "A",
            routingProfile: routing, facts: facts, suggestions: suggestions))
    }

    func testProfileFieldsAndSwitchesPassThrough() {
        let m = profileModel()
        XCTAssertTrue(m.enabled)
        XCTAssertFalse(m.suggestEnabled)
        XCTAssertEqual([m.name, m.role, m.organization, m.about], ["N", "R", "O", "A"])
    }

    func testProfileWithNothingToSendSaysSo() {
        let m = profileModel()
        XCTAssertEqual(m.preview, String(localized: "Nothing is sent."))
        XCTAssertTrue(m.facts.isEmpty)
        XCTAssertTrue(m.suggestions.isEmpty)
        XCTAssertEqual(m.aboutFooter, String(localized: "\(0) of \(ProfileComposer.maxChars) characters sent"))
    }

    func testProfileShowsTheExactTextThatIsSentAndItsLength() {
        let m = profileModel(routing: "Name: N")
        XCTAssertEqual(m.preview, "Name: N")
        XCTAssertEqual(m.aboutFooter, String(localized: "\(7) of \(ProfileComposer.maxChars) characters sent"))
    }

    func testProfileFactsAndSuggestionsKeepTheirOrderIdsAndMeetingLinks() {
        let m = profileModel(
            facts: [ProfileFact(id: "f1", text: "One", sourceMeetingId: 7), ProfileFact(id: "f2", text: "Two", sourceMeetingId: nil)],
            suggestions: [try! JSONDecoder().decode(CoreProfileSuggestion.self, from: Data(#"{"id":5,"text":"Three","source_meeting_id":9}"#.utf8))])
        XCTAssertEqual(m.facts.map(\.id), ["f1", "f2"])
        XCTAssertEqual(m.facts.map(\.hasMeeting), [true, false])
        XCTAssertEqual(m.suggestions.map(\.id), ["5"])
        XCTAssertEqual(m.suggestions.map(\.text), ["Three"])
        XCTAssertEqual(m.suggestions.map(\.hasMeeting), [true])
    }

    func testProfileFactErrorKeepsTheClassicWords() {
        XCTAssertEqual(profileModel().labels.factError, String(localized: "That fact is blank, already listed, too long (200 characters), or the list is full (30)."))
    }

    // MARK: Plan & billing

    private let ready = SyncStatus.ready(tursoToken: "t", tursoDatabaseUrl: "u")

    private func planModel(
        status: SyncStatus, plan: AccountPlan? = nil, failed: Bool = false, balance: AIBalance? = nil, error: String? = nil, working: Bool = false
    ) -> WPlanSettingsModel {
        PlanSettingsAdapter.model(status: status, plan: plan, planReadFailed: failed, balance: balance, billingError: error, isWorking: working)
    }

    func testPlanStatesFollowTheClassicPaneStates() {
        XCTAssertEqual(planModel(status: .notSignedIn).state, .signedOut)
        XCTAssertEqual(planModel(status: ready).state, .loading)
        XCTAssertEqual(planModel(status: ready, failed: true).state, .readFailed)
        XCTAssertEqual(planModel(status: .signingIn).state, .finishSigningIn)
        XCTAssertEqual(planModel(status: ready, plan: AccountPlan(plan: "free", until: nil, interval: nil, billing: true)).state, .plan)
    }

    func testPlanWithNoPlanShowsNoPurchaseOrManageControls() {
        for status in [SyncStatus.notSignedIn, .signingIn, ready] {
            let m = planModel(status: status)
            XCTAssertFalse(m.showsManage)
            XCTAssertEqual(m.purchase, .none)
            XCTAssertNil(m.planLine)
        }
    }

    func testFreeAccountWhereTheServerSellsPlusGetsPlusButtons() {
        let plan = AccountPlan(plan: "free", until: nil, interval: nil, plusAvailable: true, billing: true)
        let m = planModel(status: ready, plan: plan)
        XCTAssertEqual(m.purchase, .plus)
        XCTAssertFalse(m.showsManage)
        XCTAssertEqual(m.planLine, PlusCopy.planLine(plan))
    }

    func testFreeAccountWithoutPlusForSaleGetsCloudButtons() {
        for available in [nil, false] as [Bool?] {
            let plan = AccountPlan(plan: "free", until: nil, interval: nil, plusAvailable: available, billing: true)
            XCTAssertEqual(planModel(status: ready, plan: plan).purchase, .cloud)
        }
    }

    func testPaidAccountGetsManageAndNoPurchase() {
        let plan = AccountPlan(plan: "cloud", until: 1, interval: "month", planTier: "plus", billing: true)
        let m = planModel(status: ready, plan: plan)
        XCTAssertTrue(m.showsManage)
        XCTAssertEqual(m.purchase, .none)
    }

    func testAnAccountWithoutBillingGetsNeitherManageNorPurchase() {
        let plan = AccountPlan(plan: "free", until: nil, interval: nil, plusAvailable: true, billing: false)
        let m = planModel(status: ready, plan: plan, error: "boom")
        XCTAssertFalse(m.showsManage)
        XCTAssertEqual(m.purchase, .none)
        XCTAssertNil(m.billingError)
    }

    func testBillingErrorAndWorkingFlagReachTheModel() {
        let plan = AccountPlan(plan: "free", until: nil, interval: nil, billing: true)
        let m = planModel(status: ready, plan: plan, error: "boom", working: true)
        XCTAssertEqual(m.billingError, "boom")
        XCTAssertTrue(m.isWorking)
    }

    func testPlanPricesAreTheClassicPanesWords() {
        let l = PlanSettingsAdapter.labels()
        XCTAssertEqual(l.plusMonthly, String(localized: "Get Plus: $15 a month"))
        XCTAssertEqual(l.plusYearly, String(localized: "Get Plus: $144 a year"))
        XCTAssertEqual(l.cloudMonthly, String(localized: "Upgrade to Cloud: $5 a month"))
        XCTAssertEqual(l.cloudYearly, String(localized: "Upgrade to Cloud: $50 a year"))
    }
}
