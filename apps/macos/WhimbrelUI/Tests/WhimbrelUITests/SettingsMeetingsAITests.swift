import Foundation
import Testing
@testable import WhimbrelUI

private let enUS = Locale(identifier: "en_US")

@Suite struct SettingsMeetingsAITests {
    @Test func theEmptyTopicsLineOnlyPointsToSuggestionsWhenThereAreSome() {
        let l = WTopicsLabels()
        #expect(l.noTopicsText(hasSuggestions: true).contains("suggestion"))
        #expect(!l.noTopicsText(hasSuggestions: false).contains("suggestion"))
        #expect(l.noTopicsText(hasSuggestions: false) == "No topics yet. Add one.")
    }

    // MARK: Key rows

    @Test func aKeyStatusIsAnInfoLineOrAnErrorLine() {
        #expect(WKeyStatusMapping.status(text: nil, isError: false) == nil)
        #expect(WKeyStatusMapping.status(text: "", isError: true) == nil)
        #expect(WKeyStatusMapping.status(text: "Saved.", isError: false) == .info("Saved."))
        #expect(WKeyStatusMapping.status(text: "Failed to save key.", isError: true) == .problem("Failed to save key."))
    }

    @Test func aKeyRowModelCarriesOnlyWhetherAKeyIsSaved() {
        let row = WKeyRowModel(title: "Anthropic API Key", isSaved: true, placeholder: "sk-example-not-a-key")
        // The model has no field a secret could be stored in other than the fixed placeholder the app chooses.
        let mirror = Mirror(reflecting: row)
        #expect(Set(mirror.children.compactMap(\.label)) == ["title", "isSaved", "placeholder", "status", "anchor", "refused"])
        #expect(row.isSaved)
    }

    // MARK: Budgets

    @Test func aBudgetIsBlankZeroOrAFigureWithinTheRange() {
        let rule = WBudgetRule.rule(maxCents: 100_000_000)
        #expect(rule.allowsEmpty)
        #expect(rule.fractionDigits == 2)
        #expect(rule.minimum == 0)
        #expect(rule.maximum == 1_000_000)
        #expect(WNumberInput.parse("", rule: rule, locale: enUS) == .empty)
        #expect(WNumberInput.parse("25", rule: rule, locale: enUS) == .value(25))
        #expect(WNumberInput.parse("0", rule: rule, locale: enUS) == .value(0))
        #expect(WNumberInput.parse("12.345", rule: rule, locale: enUS) == .invalid(.tooManyDecimals(2)))
        #expect(WNumberInput.parse("-1", rule: rule, locale: enUS) == .invalid(.belowMinimum(0)))
        #expect(WNumberInput.parse("2000000", rule: rule, locale: enUS) == .invalid(.aboveMaximum(1_000_000)))
        #expect(WNumberInput.parse("abc", rule: rule, locale: enUS) == .invalid(.notANumber))
    }

    @Test func aBudgetFieldShowsNoneForNoBudgetAndDollarsOtherwise() {
        #expect(WBudgetRule.dollars(cents: nil) == nil)
        #expect(WBudgetRule.dollars(cents: 0) == nil)
        #expect(WBudgetRule.dollars(cents: 2500) == 25)
        #expect(WBudgetRule.dollars(for: .clear) == nil)
        #expect(WBudgetRule.dollars(for: .set(12.5)) == 12.5)
    }

    // MARK: Daily caps

    @Test func aBlankCapIsTheBuiltInDefaultAndZeroTurnsTheCapOff() {
        #expect(WCapRule.cents(for: .clear) == nil)
        #expect(WCapRule.cents(for: .set(0)) == 0)
        #expect(WCapRule.cents(for: .set(250)) == 250)
        #expect(WCapRule.meaning(of: nil) == .builtInDefault)
        #expect(WCapRule.meaning(of: 0) == .off)
        #expect(WCapRule.meaning(of: 40) == .limit(40))
        #expect(WCapRule.value(cents: nil) == nil)
        #expect(WCapRule.value(cents: 0) == 0)
    }

    @Test func aCapEntryThatCannotBeStoredIsRefusedNotTrapped() {
        let rule = WCapRule.rule
        #expect(WNumberInput.parse("1.5", rule: rule, locale: enUS) == .invalid(.tooManyDecimals(0)))
        #expect(WNumberInput.parse("-3", rule: rule, locale: enUS) == .invalid(.belowMinimum(0)))
        #expect(WNumberInput.parse("99999999999999999999", rule: rule, locale: enUS) == .invalid(.aboveMaximum(WCapRule.maximum)))
        #expect(WCapRule.cents(for: .set(.infinity)) == nil)
        #expect(WCapRule.cents(for: .set(.nan)) == nil)
        #expect(WCapRule.cents(for: .set(-1)) == nil)
        #expect(WCapRule.cents(for: .set(WCapRule.maximum)) == 1_000_000_000)
    }

    // MARK: Model downloads and the local model

    @Test func aDownloadShowsProgressWhileRunningAndRetryAfterAFailure() {
        #expect(!WModelDownloadState.none.showsProgress)
        #expect(!WModelDownloadState.none.canRetry)
        #expect(WModelDownloadState.downloading(fraction: 0.4).showsProgress)
        #expect(WModelDownloadState.downloading(fraction: 0.4).fraction == 0.4)
        #expect(WModelDownloadState.downloading(fraction: 1.7).fraction == 1)
        #expect(WModelDownloadState.downloading(fraction: -1).fraction == 0)
        #expect(WModelDownloadState.downloading(fraction: nil).fraction == nil)
        #expect(WModelDownloadState.downloading(fraction: .nan).fraction == nil)
        #expect(WModelDownloadState.failed.canRetry)
        #expect(!WModelDownloadState.failed.showsProgress)
    }

    @Test func theSmallLocalModelOffersRemoveOnlyWhenInstalled() {
        #expect(!WLocalModelCard.notInstalled.offersRemove)
        #expect(WLocalModelCard.installed.offersRemove)
    }

    // MARK: Estimates

    @Test func anEstimateLineFollowsTheClassicOrder() {
        #expect(WEstimateLines.lines(WEstimateInput()).isEmpty)
        #expect(WEstimateLines.lines(WEstimateInput(awaitingStart: true, estimateFailed: true)) == [.estimateFailed])
        #expect(WEstimateLines.lines(WEstimateInput(awaitingStart: true, estimate: .init(indexing: true, sources: 40, usd: 0.5))) == [.stillSorting])
        #expect(WEstimateLines.lines(WEstimateInput(awaitingStart: true, estimate: .init(indexing: false, sources: 0, usd: 0))) == [.stillSorting])
        #expect(WEstimateLines.lines(WEstimateInput(awaitingStart: true, estimate: .init(indexing: false, sources: 40, usd: 0.5))) == [.ready(items: 40, usd: 0.5)])
        // Not awaiting: an estimate on its own shows nothing.
        #expect(WEstimateLines.lines(WEstimateInput(estimate: .init(indexing: false, sources: 40, usd: 0.5))).isEmpty)
        #expect(WEstimateLines.lines(WEstimateInput(startFailed: true, keyRejected: true, overBudget: true)) == [.startFailed, .keyRejected, .overBudget])
    }

    // MARK: Topics

    @Test func aTopicNeedsANameAndItsAliasesAreSplitOnCommas() {
        #expect(WTopicEditorRule.problem(name: "") == .emptyName)
        #expect(WTopicEditorRule.problem(name: "   ") == .emptyName)
        #expect(WTopicEditorRule.problem(name: " Harbor ") == nil)
        #expect(WTopicEditorRule.trimmedName(" Harbor ") == "Harbor")
        #expect(WTopicEditorRule.aliases(from: "Harbor, HR ,, ") == ["Harbor", "HR"])
        #expect(WTopicEditorRule.aliases(from: "") == [])
        #expect(WTopicEditorRule.aliasesText(["a", "b"]) == "a, b")
        #expect(WTopicEditorRule.kinds == ["customer", "product", "project", "subject"])
    }

    @Test func aTopicDetailLineIsTheKindThenTheAliases() {
        #expect(WTopicEditorRule.detail(kindLabel: "Project", aliases: ["HR", "Harbor"]) == "Project \u{00B7} HR, Harbor")
        #expect(WTopicEditorRule.detail(kindLabel: nil, aliases: ["HR"]) == "HR")
        #expect(WTopicEditorRule.detail(kindLabel: "Customer", aliases: []) == "Customer")
        #expect(WTopicEditorRule.detail(kindLabel: nil, aliases: []) == nil)
    }

    @Test func theNamesYouGoByAreTrimmedWithoutBlanksOrRepeats() {
        #expect(WNamesRule.cleaned([" Sam ", "", "sam", "Sam Rivera", "SAM RIVERA"]) == ["Sam", "Sam Rivera"])
        #expect(WNamesRule.cleaned([]) == [])
    }

    @Test func nameChipsLeaveOutNamesAlreadyListedAndStopAtFour() {
        #expect(WNamesRule.chips(existing: ["Sam"], suggested: ["sam", "Sammy", " "], profileName: "Sam Rivera") == ["Sammy", "Sam Rivera"])
        #expect(WNamesRule.chips(existing: [], suggested: ["a", "b", "c", "d", "e"], profileName: "f").count == 4)
        #expect(WNamesRule.chips(existing: ["a"], suggested: [], profileName: "A").isEmpty)
    }

    // MARK: Usage

    @Test func aUsageRowIsSpokenInColumnOrderWithoutEmptyColumns() {
        let row = WUsageRowModel(id: "1", time: "9:41 AM", task: "ask", model: "anthropic example", tokens: "12 in, 3 out", cost: "$0.04")
        #expect(row.spoken == "9:41 AM, ask, anthropic example, 12 in, 3 out, $0.04")
        let local = WUsageRowModel(id: "2", time: "9:12 AM", task: "title", model: "ollama example", tokens: "", cost: "local")
        #expect(local.spoken == "9:12 AM, title, ollama example, local")
    }

    // MARK: Anchors and the gallery

    @Test func theSectionsDeclareEveryAnchorOnce() async {
        await MainActor.run {
            #expect(WSettingsWindowGallery.anchorsMeetings.all.count == 8)
            #expect(WSettingsWindowGallery.anchorsAI.all.count == 14)
            #expect(WSettingsWindowGallery.anchorsTopics.all.count == 3)
            let all = WSettingsWindowGallery.anchorsMeetings.all + WSettingsWindowGallery.anchorsAI.all + WSettingsWindowGallery.anchorsTopics.all
            #expect(Set(all).count == 25)
            #expect(WSettingsWindowGallery.meetingsAITopicsCases().count > 40)
        }
    }

    @Test func noSampleSentenceHasAnEmOrEnDash() {
        let m = WMeetingsLabels()
        let a = WAILabels()
        let t = WTopicsLabels()
        let all = [
            m.consentNote, m.qualifyingNote, m.heuristicNote, m.checkInNote, m.awayNote, m.liveTranscriptNote, m.snapshotsNote, m.questionsNote,
            m.addYourNameFirst, m.rememberVoicesNote, m.calendarsNote, m.noCalendars, m.modelFileNote, m.multilingualNote, m.diarizationNote,
            a.intro, a.localOnlyNote, a.onRampIntro, a.onRampNote, a.noModelsLoaded, a.cheapFirstNote, a.imageSearchNote, a.rankEmailNote,
            a.sortTopicsNote, a.titleCheckNote, a.capNote, a.promptsNote, a.removeKeyMessage, a.removeKeyTitle("Anthropic"),
            t.intro, t.noTopics, t.deletePointer, t.suggestedEmpty, t.placeholderNote, t.namesNote, t.findStepsNote, t.mergeSentence("x"),
        ]
        for text in all { #expect(!text.contains("\u{2014}") && !text.contains("\u{2013}"), Comment(rawValue: text)) }
    }

    @Test func theAISectionNeverNamesItselfAiAndRouting() {
        let a = WAILabels()
        #expect(a.heading == "AI")
        for text in [a.heading, a.intro, a.localOnlyNote, a.defaultsHeading] { #expect(!text.contains("AI & Routing")) }
    }
}
