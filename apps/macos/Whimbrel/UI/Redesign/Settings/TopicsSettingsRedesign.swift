import AppKit
import SwiftUI
import WhimbrelUI

/// Topics in the redesigned Settings window. This file feeds the package's `WTopicsSettingsView` from the app's state and
/// calls the same functions `TopicsSettingsPane` calls. The classic pane is not changed.

// MARK: Adapter (pure, tested)

enum TopicsAdapter {
    /// The kind picker: None, then the four kinds, in the classic order and words.
    static func kindChoices() -> [WChoice] {
        [WChoice(id: "", title: String(localized: "None"))]
            + WTopicEditorRule.kinds.map { WChoice(id: $0, title: TopicsSettingsPane.kindLabel($0)) }
    }

    static func seedModels(_ seeds: [TopicSeed]) -> [WTopicSeedModel] {
        seeds.map {
            WTopicSeedModel(
                id: $0.key, name: $0.name, aliases: $0.aliases, kind: $0.kind, kindLabel: $0.kind.map(TopicsSettingsPane.kindLabel),
                itemsText: String(localized: "\($0.itemCount) items"))
        }
    }

    static func suggestionModels(_ suggestions: [TopicSuggestion]) -> [WTopicSuggestionModel] {
        suggestions.map { WTopicSuggestionModel(id: String($0.topicId), name: $0.name, itemsText: String(localized: "\($0.itemCount) items")) }
    }

    /// The names the person goes by and the chips offered to add: core's suggestions and the Profile name.
    static func chips(names: [String], suggested: [String], profileName: String) -> [String] {
        WNamesRule.chips(existing: names, suggested: suggested, profileName: profileName)
    }

    /// The "Find steps" switch is off, with the reason, until a TypeSafe key is saved.
    static func stepsDisabledReason(hasKey: Bool) -> String? {
        hasKey ? nil : String(localized: "Save a TypeSafe key first.", comment: "Shown on a switch that needs a TypeSafe key, until one is saved.")
    }

    static func stepsNotes(_ input: WEstimateInput, noModel: Bool) -> [WSettingNote] {
        var notes = EstimateAdapter.notes(input, feature: .steps)
        if noModel {
            notes.append(WSettingNote(id: "noModel", text: String(localized: "Finding steps needs a local model (Settings, AI).", comment: "Shown under the Find steps switch when no local model is available. Settings, AI is the path to the AI section.")))
        }
        return notes
    }

    static var ownStrings: [String] {
        let l = labels()
        return [
            l.intro, l.noTopics, l.noTopicsAndNoSuggestions, l.deletePointer, l.suggestedEmpty, l.placeholderNote, l.namesNote, l.findStepsNote, l.nameRequired,
            l.mergeSentence("x"), l.mergeNamed("x"), stepsDisabledReason(hasKey: false) ?? "",
        ] + stepsNotes(WEstimateInput(), noModel: true).map(\.text)
    }

    static func labels() -> WTopicsLabels {
        var l = WTopicsLabels()
        l.heading = String(localized: "Topics")
        l.intro = String(localized: "Name the customers, products and projects you care about. Whimbrel groups your meetings, emails and notes under them.")
        l.yourTopics = String(localized: "Your topics")
        l.noTopics = String(localized: "No topics yet. Add one, or accept a suggestion below.")
        l.noTopicsAndNoSuggestions = String(localized: "No topics yet. Add one.", comment: "Settings, Topics: shown when there are no topics and no suggested topics either.")
        l.addTopicLabel = String(localized: "Your own topic", comment: "Label of the Settings row with the Add topic button.")
        l.addTopic = String(localized: "Add topic")
        l.editTopic = String(localized: "Edit topic")
        l.edit = String(localized: "Edit")
        l.editNamed = { name in String(localized: "Edit \(name)", comment: "Spoken name of the Edit button on a list row. The placeholder is the item's text.") }
        l.mergeInto = String(localized: "Merge into…")
        l.mergeNamed = { name in String(localized: "Merge \(name) into another topic", comment: "Spoken name of the Merge into button on a topic. The placeholder is the topic's name.") }
        l.mergeSentence = { name in String(localized: "Merge \(name) into") }
        l.merge = String(localized: "Merge")
        l.delete = String(localized: "Delete")
        l.deleteNamed = { name in String(localized: "Delete \(name)", comment: "Spoken name of the Delete button on a list row. The placeholder is the item's text.") }
        l.deletePointer = String(localized: "To delete a client and what Whimbrel holds on them, open the client's page and choose Delete this client.")
        l.close = String(localized: "Close")
        l.cancel = String(localized: "Cancel")
        l.save = String(localized: "Save")
        l.name = String(localized: "Name")
        l.aliases = String(localized: "Aliases, separated by commas")
        l.kind = String(localized: "Kind")
        l.nameRequired = String(localized: "Enter a name to save.", comment: "Shown in the topic editor while the name is empty.")
        l.suggested = String(localized: "Suggested")
        l.suggestedEmpty = String(localized: "When a theme shows up across several of your meetings, emails and notes, Whimbrel suggests it here. Nothing is added until you press Add.")
        l.placeholderNote = String(localized: "Some suggested topics have placeholder names until an AI model names them.")
        l.add = String(localized: "Add")
        l.dismiss = String(localized: "Dismiss")
        l.namesHeading = String(localized: "Names I go by")
        l.namesNote = String(localized: "Whimbrel uses these to tell steps you owe from steps owed to you.")
        l.namesList = String(localized: "Your names", comment: "Label of the list of names you go by in Settings.")
        l.noNames = String(localized: "No names yet.", comment: "Shown in Settings when the list of names you go by is empty.")
        l.addNameLabel = String(localized: "Add a name")
        l.addName = String(localized: "Add a name")
        l.suggestedNames = String(localized: "Suggested names", comment: "Label of the buttons that add a suggested name to the names you go by.")
        l.addChip = { name in String(localized: "+ \(name)") }
        l.remove = String(localized: "Remove")
        l.removeNamed = { name in String(localized: "Remove \(name)", comment: "Spoken name of the Remove button on a list row. The placeholder is the item's text.") }
        l.findSteps = String(localized: "Find steps with TypeSafe")
        l.findStepsNote = String(localized: "Sends short excerpts of topic emails, meeting action items and Teams screens to TypeSafe to find steps you owe and are owed. Off: your local model does this.")
        return l
    }
}

// MARK: Runtime state

/// What `TopicsSettingsPane` keeps in `@State`, with the same reads and writes.
@MainActor
final class TopicsRuntime: ObservableObject {
    unowned let appState: AppState
    init(appState: AppState) { self.appState = appState }

    @Published var seeds: [TopicSeed] = []
    @Published var suggestions: [TopicSuggestion] = []
    @Published var errorText: String?
    @Published var names: [String] = []
    @Published var suggestedNames: [String] = []
    @Published var provisionalCount = 0

    /// Reads the lists off the main thread and publishes them on main (as the classic pane does).
    func reload() {
        guard let store = appState.coreStore else { return }
        appState.refreshTopicStepsEstimate()
        CoreMemoryStore.accessQueue.async {
            let s = store.topicSeeds()
            let g = store.topicSuggestions()
            let n = store.topicNames()
            let p = store.topicNamingStatus()?.provisionalCount
            DispatchQueue.main.async {
                if let p { self.provisionalCount = p }
                if let s { self.seeds = s }
                if let g { self.suggestions = g }
                if let n { self.names = n.names; self.suggestedNames = n.suggested }
            }
        }
    }

    /// Runs a store write off the main thread, then reloads; shows `failure` if it returns false.
    func run(_ work: @escaping (CoreMemoryStore) -> Bool, failure: String) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let ok = work(store)
            DispatchQueue.main.async {
                self.errorText = ok ? nil : failure
                self.reload()
            }
        }
    }

    func saveNames(_ list: [String]) {
        let clean = WNamesRule.cleaned(list)
        names = clean
        run({ $0.topicNamesSet(clean) }, failure: String(localized: "Could not save your names."))
    }
}

// MARK: The view

struct TopicsSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let highlighted: String?

    var body: some View {
        TopicsSettingsRedesignBody(appState: appState, highlighted: highlighted)
    }
}

private struct TopicsSettingsRedesignBody: View {
    let appState: AppState
    let highlighted: String?
    @StateObject private var runtime: TopicsRuntime

    init(appState: AppState, highlighted: String?) {
        self.appState = appState
        self.highlighted = highlighted
        _runtime = StateObject(wrappedValue: TopicsRuntime(appState: appState))
    }

    var body: some View {
        WTopicsSettingsView(model: model(), actions: actions(), highlighted: highlighted)
            .task {
                // Reload on appear and every 30 s; the task is cancelled when the section goes away.
                while !Task.isCancelled {
                    runtime.reload()
                    try? await Task.sleep(for: .seconds(30))
                }
            }
    }

    private func model() -> WTopicsModel {
        let steps = appState.topicSteps
        let estimate = steps.estimate.map { WEstimateInput.Estimate(indexing: $0.indexing, sources: $0.sources, usd: $0.usd) }
        let input = WEstimateInput(
            awaitingStart: steps.awaitingStart, estimateFailed: steps.estimateFailed, estimate: estimate, startFailed: steps.startFailed,
            keyRejected: steps.keyRejected, overBudget: steps.overBudget)
        return WTopicsModel(
            labels: TopicsAdapter.labels(), error: runtime.errorText, seeds: TopicsAdapter.seedModels(runtime.seeds), kindChoices: TopicsAdapter.kindChoices(),
            suggestions: TopicsAdapter.suggestionModels(runtime.suggestions), showsPlaceholderNote: TopicPlaceholderNote.isVisible(provisionalCount: runtime.provisionalCount),
            names: runtime.names, nameChips: TopicsAdapter.chips(names: runtime.names, suggested: runtime.suggestedNames, profileName: appState.profile.name),
            stepsOn: steps.enabled, stepsDisabledReason: TopicsAdapter.stepsDisabledReason(hasKey: appState.replyPriority.hasKey),
            stepsNotes: TopicsAdapter.stepsNotes(input, noModel: appState.topicStepsNoModel), anchors: SettingsAnchorID.topics)
    }

    private func actions() -> WTopicsActions {
        WTopicsActions(
            saveTopic: { key, name, aliases, kind in
                runtime.run({ $0.topicSeedSave(key: key, name: name, aliases: aliases, kind: kind) }, failure: String(localized: "Could not save that topic."))
            },
            merge: { keep, absorb in
                runtime.run({ $0.topicSeedMerge(keep: keep, absorb: absorb) }, failure: String(localized: "Could not merge those topics."))
            },
            acceptSuggestion: { id, name in
                guard let topicId = Int64(id) else { return }
                runtime.run({ $0.topicSuggestionAccept(topicId: topicId, name: name) }, failure: String(localized: "Could not add that topic."))
            },
            dismissSuggestion: { id in
                guard let topicId = Int64(id) else { return }
                runtime.run({ $0.topicSuggestionDismiss(topicId: topicId) }, failure: String(localized: "Could not dismiss that suggestion."))
            },
            addName: { runtime.saveNames(runtime.names + [$0]) },
            removeName: { name in runtime.saveNames(runtime.names.filter { $0 != name }) },
            setSteps: { appState.setTopicStepsEnabled($0) },
            noteAction: { id in
                if id == "retry" { appState.refreshTopicStepsEstimate() } else if id == "start" { appState.startTopicSteps() }
            })
    }
}
