import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the redesigned Meetings, AI and Topics sections: the anchors, the mapping from plain inputs to the
/// section models, the budget and cap rules the app applies, and the naming rules. Nothing here opens a window, reads
/// the app's defaults or depends on the machine's language, time zone or date.
final class SettingsRedesignMeetingsAITests: XCTestCase {
    // MARK: Anchors

    func testEachSectionDeclaresExactlyItsRegistryIds() {
        for (section, declared) in [(SettingsSection.meetings, SettingsAnchorID.meetings.all), (.ai, SettingsAnchorID.ai.all), (.topics, SettingsAnchorID.topics.all)] {
            let registry = SettingsSearch.entries.filter { $0.section == section }.map(\.id)
            XCTAssertEqual(Set(declared), Set(registry), "\(section)")
            XCTAssertEqual(declared.count, Set(declared).count, "\(section)")
            XCTAssertEqual(SettingsRedesign.declaredAnchorIDs(for: section), declared)
            XCTAssertFalse(SettingsRedesign.hostsClassicPanes(section))
        }
        XCTAssertEqual(SettingsAnchorID.meetings.all.count, 8)
        XCTAssertEqual(SettingsAnchorID.ai.all.count, 14)
        XCTAssertEqual(SettingsAnchorID.topics.all.count, 3)
    }

    func testEverySearchResultOfTheThreeSectionsOpensItsSection() {
        for entry in SettingsSearch.entries where [.meetings, .ai, .topics].contains(entry.section) {
            XCTAssertEqual(SettingsRedesign.section(forResult: entry.id), entry.section, entry.id)
        }
    }

    // MARK: Meetings

    func testTheCheckInChoicesAreTheClassicHours() {
        let options = MeetingsAdapter.checkInOptions(hours: [0, 1, 2, 4])
        XCTAssertEqual(options.map(\.id), ["0", "1", "2", "4"])
        XCTAssertEqual(options.first?.title, String(localized: "Off"))
        XCTAssertEqual(MeetingsAdapter.checkInOptions().map(\.id), RecordingCheckInSettings.allowedHours.map(String.init))
    }

    func testQualityChoicesAndTheDownloadRule() {
        XCTAssertEqual(MeetingsAdapter.qualityOptions().map(\.id), ["automatic", "better", "fast"])
        XCTAssertTrue(MeetingsAdapter.startsDownload(from: .automatic, to: .better))
        XCTAssertFalse(MeetingsAdapter.startsDownload(from: .better, to: .better))
        XCTAssertFalse(MeetingsAdapter.startsDownload(from: .better, to: .fast))
        XCTAssertNotEqual(MeetingsAdapter.qualityDetail(.better), MeetingsAdapter.qualityDetail(.fast))
    }

    func testTheDownloadRowFollowsThePhase() {
        XCTAssertEqual(MeetingsAdapter.download(.idle).state, .none)
        XCTAssertEqual(MeetingsAdapter.download(.ready).state, .none)
        XCTAssertEqual(MeetingsAdapter.download(.failed).state, .failed)
        XCTAssertEqual(MeetingsAdapter.download(.downloading(fraction: 0.5, status: "x")).state, .downloading(fraction: 0.5))
    }

    func testTheLiveQuestionSwitchIsOffUntilThereIsAName() {
        XCTAssertFalse(MeetingsAdapter.liveQuestionOn(hasNames: false, enabled: true))
        XCTAssertTrue(MeetingsAdapter.liveQuestionOn(hasNames: true, enabled: true))
        XCTAssertFalse(MeetingsAdapter.liveQuestionOn(hasNames: true, enabled: false))
    }

    func testTurningOtherLanguagesOnAsksForTheModelsOnlyWhenTheyAreMissing() {
        XCTAssertTrue(MeetingsAdapter.asksForModels(turnedOn: true, allModelsPresent: false))
        XCTAssertFalse(MeetingsAdapter.asksForModels(turnedOn: true, allModelsPresent: true))
        XCTAssertFalse(MeetingsAdapter.asksForModels(turnedOn: false, allModelsPresent: false))
    }

    // MARK: AI

    func testModelStatusFollowsTheClassicLines() {
        XCTAssertNil(AIAdapter.modelStatus(hasKey: false, isLoading: false, models: [], error: nil))
        XCTAssertEqual(AIAdapter.modelStatus(hasKey: true, isLoading: true, models: [], error: nil)?.isLoading, true)
        XCTAssertEqual(AIAdapter.modelStatus(hasKey: true, isLoading: false, models: [], error: "offline")?.isError, true)
        XCTAssertEqual(AIAdapter.modelStatus(hasKey: true, isLoading: false, models: ["a", "b"], error: nil)?.isError, false)
    }

    func testAKeyThatWasRefusedShowsNoModelCountAndOneLinkPatternReadsRightForEveryProvider() {
        XCTAssertNil(AIAdapter.modelStatus(hasKey: true, isLoading: false, models: ["a", "b"], error: nil, keyRefused: true))
        XCTAssertNotNil(AIAdapter.modelStatus(hasKey: true, isLoading: false, models: ["a", "b"], error: nil, keyRefused: false))
        for name in ["Anthropic", "OpenAI", "Google", "OpenRouter"] {
            let title = AIAdapter.keyLinkTitle(providerName: name)
            XCTAssertTrue(title.contains(name), title)
        }
        let row = WKeyRowModel(title: "k", isSaved: true, refused: true)
        XCTAssertTrue(row.refused)
    }

    @MainActor
    func testTheStandardRecordingReminderIsReadFromTheRecordingNoticeSettings() {
        XCTAssertEqual(MeetingsAdapter.defaultNoticeText, String(localized: "I'm recording this meeting with Whimbrel, on my own computer, for my notes."))
    }

    func testTheDefaultModelControlFollowsTheClassicRules() {
        XCTAssertEqual(AIAdapter.defaultModelControl(provider: .ollama, options: [], selection: "", ollamaModel: "m"), .text(value: "m"))
        XCTAssertEqual(AIAdapter.defaultModelControl(provider: .anthropic, options: [], selection: "", ollamaModel: "m"), WDefaultModelControl.none)
        XCTAssertEqual(AIAdapter.defaultModelControl(provider: .anthropic, options: ["a"], selection: "a", ollamaModel: "m"), .picker(options: ["a"], selection: "a"))
        XCTAssertEqual(AIAdapter.openRouterControl(options: [], selection: "x"), .text(value: "x"))
        XCTAssertEqual(AIAdapter.providerChoices().map(\.id), RoutingSettings.Provider.allCases.map(\.rawValue))
    }

    /// A key row is built from Bools and text the app chose; a saved key is never an input, so no key can reach a model.
    func testAKeyRowIsBuiltFromWhetherAKeyIsSavedAndNeverItsValue() {
        XCTAssertEqual(AIAdapter.placeholder(hasKey: false, prefixHint: "sk-"), "")
        XCTAssertEqual(AIAdapter.placeholder(hasKey: true, prefixHint: "sk-"), "sk-")
        XCTAssertEqual(AIAdapter.status(text: "Saved.", isError: false), .info("Saved."))
        XCTAssertEqual(AIAdapter.status(text: "Failed to save key.", isError: true), .problem("Failed to save key."))
        let row = WKeyRowModel(title: "Anthropic API Key", isSaved: true, placeholder: AIAdapter.placeholder(hasKey: true, prefixHint: "sk-"))
        XCTAssertEqual(Set(Mirror(reflecting: row).children.compactMap(\.label)), ["title", "isSaved", "placeholder", "status", "anchor", "refused"])
    }

    func testTheBudgetAndCapRulesMatchWhatTheClassicFieldsStore() {
        // Budget: blank and zero both mean none.
        XCTAssertNil(RoutingSettings.budgetCents(dollars: WBudgetRule.dollars(for: .clear)))
        XCTAssertNil(RoutingSettings.budgetCents(dollars: WBudgetRule.dollars(for: .set(0))))
        XCTAssertEqual(RoutingSettings.budgetCents(dollars: WBudgetRule.dollars(for: .set(12.5))), 1250)
        XCTAssertEqual(WBudgetRule.rule(maxCents: RoutingSettings.maxBudgetCents).maximum, Double(RoutingSettings.maxBudgetCents) / 100)
        // Caps: blank is the built-in default (nothing stored), zero turns the cap off.
        XCTAssertNil(WCapRule.cents(for: .clear))
        XCTAssertEqual(WCapRule.cents(for: .set(0)), 0)
        XCTAssertEqual(WCapRule.cents(for: .set(300)), 300)
    }

    func testUsageRowsAreGroupedByDayInTheOrderTheyArrive() {
        let day: Int64 = 86_400_000
        func row(_ ts: Int64, _ task: String, local: Bool = false) -> CoreUsageRow {
            CoreUsageRow(tsUnixMs: ts, task: task, provider: "p", model: "m", inputTokens: 10, outputTokens: 2, estimatedCostCents: 5, local: local)
        }
        let model = AIAdapter.usageDays([row(10 * day, "a"), row(10 * day - 1000, "b", local: true), row(8 * day, "c")])
        XCTAssertEqual(model.flatMap { $0.rows.map(\.task) }, ["a", "b", "c"])
        XCTAssertEqual(model.flatMap(\.rows).first { $0.task == "a" }?.cost, AIProvidersSettingsPane.dollars(5))
        XCTAssertEqual(model.flatMap(\.rows).first { $0.task == "b" }?.cost, String(localized: "local"))
        XCTAssertEqual(model.flatMap(\.rows).first?.tokens, "10 in, 2 out")
    }

    func testEstimateNotesCarryTheRetryAndStartButtons() {
        let notes = EstimateAdapter.notes(WEstimateInput(awaitingStart: true, estimateFailed: true, startFailed: true, keyRejected: true, overBudget: true), feature: .sort)
        XCTAssertEqual(notes.map(\.id), ["retry", "startFailed", "keyRejected", "overBudget"])
        XCTAssertNotNil(notes.first?.buttonTitle)
        let ready = EstimateAdapter.notes(WEstimateInput(awaitingStart: true, estimate: .init(indexing: false, sources: 9, usd: 0.1)), feature: .steps)
        XCTAssertEqual(ready.map(\.id), ["start"])
        XCTAssertNotNil(ready.first?.buttonTitle)
        XCTAssertNotEqual(
            EstimateAdapter.notes(WEstimateInput(startFailed: true), feature: .sort).first?.text,
            EstimateAdapter.notes(WEstimateInput(startFailed: true), feature: .steps).first?.text)
    }

    // MARK: Topics

    func testKindChoicesStartWithNoneAndNameTheFourKinds() {
        let choices = TopicsAdapter.kindChoices()
        XCTAssertEqual(choices.map(\.id), ["", "customer", "product", "project", "subject"])
        XCTAssertEqual(choices[1].title, TopicsSettingsPane.kindLabel("customer"))
    }

    func testNameChipsComeFromCoresSuggestionsAndTheProfileName() {
        XCTAssertEqual(TopicsAdapter.chips(names: ["Sam"], suggested: ["sam", "Sammy"], profileName: "Sam Rivera"), ["Sammy", "Sam Rivera"])
    }

    func testFindingStepsIsOffWithAReasonUntilAKeyIsSaved() {
        XCTAssertNotNil(TopicsAdapter.stepsDisabledReason(hasKey: false))
        XCTAssertNil(TopicsAdapter.stepsDisabledReason(hasKey: true))
        XCTAssertEqual(TopicsAdapter.stepsNotes(WEstimateInput(), noModel: true).map(\.id), ["noModel"])
        XCTAssertTrue(TopicsAdapter.stepsNotes(WEstimateInput(), noModel: false).isEmpty)
    }

    // MARK: Words

    func testNoRedesignedStringSaysAIAndRoutingOrAnIssueNumber() {
        let all = AIAdapter.ownStrings + MeetingsAdapter.ownStrings + TopicsAdapter.ownStrings
        for text in all {
            XCTAssertFalse(text.contains("AI & Routing"), text)
            XCTAssertFalse(text.contains("AI Providers"), text)
            XCTAssertFalse(text.contains("#64"), text)
            XCTAssertFalse(text.contains("\u{2014}") || text.contains("\u{2013}") || text.contains("\u{2192}"), text)
        }
        XCTAssertEqual(WAILabels().heading, "AI")
        XCTAssertEqual(WMeetingsLabels().calendarsHeading, "Always treat as meetings")
    }

    func testTheRecordingNoticeAndConsentControlsAreTheFirstCard() {
        let l = MeetingsAdapter.labels()
        XCTAssertFalse(l.noticeToggle.isEmpty)
        XCTAssertFalse(l.consentNote.isEmpty)
        XCTAssertEqual(SettingsAnchorID.meetings.recordingNotice, "meetings.recordingNotice")
    }
}
