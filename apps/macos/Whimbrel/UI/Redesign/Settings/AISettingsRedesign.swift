import AppKit
import SwiftUI
import WhimbrelUI

/// AI in the redesigned Settings window. This file feeds the package's `WAISettingsView` from the app's state and calls the
/// same functions `AIProvidersSettingsPane` and `PromptsSettingsPane` call. The classic panes are not changed.
/// A saved key never reaches a model: the adapter is given only whether a key is saved.
/// The card that names the section is "AI", not "AI & Routing".

// MARK: Adapter (pure, tested)

enum EstimateAdapter {
    enum Feature { case sort, steps }

    /// The lines under an estimate-backed switch, in the classic order and words. The ids "retry" and "start" are the buttons.
    static func notes(_ input: WEstimateInput, feature: Feature) -> [WSettingNote] {
        WEstimateLines.lines(input).map { line in
            switch line {
            case .estimateFailed:
                return WSettingNote(id: "retry", text: String(localized: "Couldn't get an estimate. Try again."), tone: .problem, buttonTitle: String(localized: "Try again"))
            case .stillSorting:
                return WSettingNote(id: "still", text: String(localized: "Still sorting your items. The estimate will appear when that's done."))
            case .ready(let items, let usd):
                return WSettingNote(id: "start", text: String(localized: "About \(items) items, about \(usd, format: .currency(code: "USD")). Start?"), buttonTitle: String(localized: "Start"))
            case .startFailed:
                return WSettingNote(
                    id: "startFailed", text: feature == .sort ? String(localized: "Couldn't start topic sorting. Try again.") : String(localized: "Couldn't start finding steps. Try again."), tone: .problem)
            case .keyRejected:
                return WSettingNote(id: "keyRejected", text: String(localized: "TypeSafe rejected this key. Check it and save it again."), tone: .problem)
            case .overBudget:
                return WSettingNote(
                    id: "overBudget",
                    text: feature == .sort ? String(localized: "The topic sorting budget is used up. Turn the switch off and on again to see a new estimate.") : String(localized: "The budget for finding steps is used up. Turn the switch off and on again to see a new estimate."),
                    tone: .problem)
            }
        }
    }
}

enum AIAdapter {
    /// The status under a model list: nothing without a key, then loading, an error or the count.
    static func modelStatus(hasKey: Bool, isLoading: Bool, models: [String], error: String?, keyRefused: Bool = false) -> WModelListStatus? {
        // A key the provider refused has no model list to report: the count that was loaded earlier is not true any more.
        guard hasKey, !keyRefused else { return nil }
        if isLoading { return WModelListStatus(text: String(localized: "Loading models…"), isLoading: true) }
        if let error { return WModelListStatus(text: String(localized: "Couldn't load models: \(error)"), isError: true) }
        return WModelListStatus(text: String(localized: "\(models.count) models available"))
    }

    /// The link to a provider's key page. One pattern that reads right for every name ("Get an API key from OpenAI", not "Get a
    /// Anthropic key").
    static func keyLinkTitle(providerName: String) -> String {
        String(localized: "Get an API key from \(providerName)", comment: "Link in Settings, AI, to the page where you create an API key. The placeholder is the provider's name, for example Anthropic.")
    }

    /// What a key row shows after Save, Remove or a check. The same words the classic pane writes.
    static func status(text: String?, isError: Bool) -> WSettingsStatus? { WKeyStatusMapping.status(text: text, isError: isError) }

    /// The shown placeholder: the classic pane shows "Not configured" until a key is saved, then its prefix hint.
    static func placeholder(hasKey: Bool, prefixHint: String) -> String { hasKey ? prefixHint : "" }

    /// Which control picks the default model, by provider and by what is loaded (the classic rules).
    static func defaultModelControl(provider: RoutingSettings.Provider, options: [String], selection: String, ollamaModel: String) -> WDefaultModelControl {
        if provider == .ollama && options.isEmpty { return .text(value: ollamaModel) }
        if options.isEmpty { return .none }
        return .picker(options: options, selection: selection)
    }

    /// The OpenRouter model: typed while no list is loaded, else chosen with "Not picked" first.
    static func openRouterControl(options: [String], selection: String) -> WDefaultModelControl {
        options.isEmpty ? .text(value: selection) : .picker(options: options, selection: selection)
    }

    static func providerChoices() -> [WChoice] {
        RoutingSettings.Provider.allCases.map { WChoice(id: $0.rawValue, title: $0.displayName) }
    }

    static func usageDays(_ rows: [CoreUsageRow]) -> [WUsageDayModel] {
        AIProvidersSettingsPane.groupedByDay(rows).map { group in
            WUsageDayModel(day: group.day, rows: group.rows.map { row in
                WUsageRowModel(
                    id: row.id, time: AIProvidersSettingsPane.timeLabel(row.tsUnixMs), task: row.task, model: "\(row.provider) \(row.model)",
                    tokens: AIProvidersSettingsPane.tokensLabel(row), cost: row.local ? String(localized: "local") : AIProvidersSettingsPane.dollars(row.estimatedCostCents))
            })
        }
    }

    static func promptChoices() -> [WChoice] { PromptSurface.allCases.map { WChoice(id: $0.rawValue, title: $0.displayName) } }

    /// Every sentence the redesigned section writes of its own, for the copy tests.
    static var ownStrings: [String] {
        let l = labels()
        return [
            l.heading, l.intro, l.localOnlyNote, l.smallLocalModel, l.onRampIntro, l.onRampNote, l.signInToUse, l.overBudget, l.saveKeyFirst,
            l.googleBillingNote, l.noModelsLoaded, l.removeKeyTitle("x"), l.removeKeyMessage, l.ollamaNote, l.remoteGoogleDataNote, l.cheapFirstNote,
            l.imageSearchNote, l.saveTypesafeFirst, l.rankEmailNote, l.sortTopicsNote, l.titleCheckNote, l.capNote, l.noCalls, l.usageRange,
            l.promptsNote, l.resetPromptLabel, l.promptIsDefault, l.defaultsHeading, l.addCredit, l.spendToday,
        ]
    }

    static func labels() -> WAILabels {
        var l = WAILabels()
        l.heading = String(localized: "AI", comment: "Heading of the first Settings card in the AI section, and the name of the section.")
        l.intro = String(localized: "Configure hosted LLM providers for Ask/Digest. Keys are stored in the macOS Keychain, never in a plaintext file.")
        l.localOnly = String(localized: "Local-only mode")
        l.localOnlyNote = String(localized: "When on, every conversation uses Ollama only, regardless of the provider picked in Ask or Digest. Hosted providers are never called.")
        l.smallLocalModel = String(localized: "Small local model", comment: "Label of the Settings row that removes the small local AI model from this Mac.")
        l.removeLocalModel = String(localized: "Remove the Small Local Model")
        l.onRampHeading = String(localized: "Pick where AI runs")
        l.onRampIntro = String(localized: "Whimbrel works without any AI model: capture, transcripts and search still work. Chat, summaries and routines need one.")
        l.installOllama = String(localized: "Install Ollama")
        l.freeGeminiKey = String(localized: "Get a free Gemini key")
        l.onRampNote = String(localized: "Ollama is free and runs on this Mac, so nothing leaves it unless you point Whimbrel at another computer. Gemini's free tier is free, and Google may use prompts to improve its products. Or paste any Anthropic, OpenAI or Google key below.")
        l.whimbrelAI = String(localized: "Whimbrel AI")
        l.addCredit = String(localized: "Add credit", comment: "Label of the Settings row with the buttons that buy Whimbrel AI credit.")
        l.signInToUse = String(localized: "Sign in to sync to use Whimbrel AI")
        l.refreshModels = String(localized: "Refresh Models")
        l.monthlyBudget = String(localized: "Monthly budget")
        l.dollar = "$"
        l.none = String(localized: "None")
        l.overBudget = String(localized: "At or over the monthly budget.", comment: "Shown under the monthly spend bar when the spend has reached the budget.")
        l.syncToDevices = String(localized: "Sync to devices")
        l.saveKeyFirst = String(localized: "Save a key first.", comment: "Shown on a switch that needs an API key, until one is saved.")
        l.googleBilling = String(localized: "My Google AI Studio project has billing on")
        l.googleBillingNote = String(localized: "Gmail and Calendar text goes to Gemini only when its project has billing on, because Google may train on the free tier.")
        l.openRouterModel = String(localized: "OpenRouter model", comment: "Label of the Settings row that picks the OpenRouter model.")
        l.openRouterPlaceholder = String(localized: "vendor/model or vendor/model:free")
        l.notPicked = String(localized: "Not picked")
        l.noModelsLoaded = String(localized: "No models loaded. Save a key above, then choose Refresh Models.", comment: "Shown on the Default model row when no model list has loaded.")
        l.includesOtherComputers = String(localized: "Includes your other computers.")
        l.remove = String(localized: "Remove")
        l.removeKeyTitle = { name in String(localized: "Remove your \(name) key?", comment: "Question before an API key is removed. The placeholder is the provider's name.") }
        l.removeKeyMessage = String(localized: "Anything that uses it stops working until you save a key again.", comment: "Said under the question before an API key is removed.")
        l.ollama = String(localized: "Ollama")
        l.ollamaNote = String(localized: "Local server, no API key needed. It runs on this Mac. If you point Whimbrel at an Ollama server on another computer, your prompts go to that server.")
        l.useLocalModel = String(localized: "Use local model")
        l.remoteGoogleData = String(localized: "Allow Gmail and Calendar text with a cloud or remote Ollama model")
        l.remoteGoogleDataNote = String(localized: "Off keeps Gmail and Calendar text on this computer when Ollama runs elsewhere.")
        l.cheapFirst = String(localized: "Use the cheapest model for small tasks")
        l.cheapFirstNote = String(localized: "Conversation titles, topic names and profile suggestions go to the local model first, then to the cheapest model you have a key for. Chat, summaries and digests keep using your default provider. Each routine picks its own model in Routines.")
        l.imageSearchNote = String(localized: "Lets the skin chat in General settings find images on the web, with strict safe-search. Searches count toward the daily spending limit. The key stays on this Mac. Local-only mode blocks it.")
        l.typesafeHeading = String(localized: "Reply priorities (TypeSafe)")
        l.saveTypesafeFirst = String(localized: "Save a TypeSafe key first.", comment: "Shown on a switch that needs a TypeSafe key, until one is saved.")
        l.rankEmail = String(localized: "Rank email by reply priority")
        l.rankEmailNote = String(localized: "Off by default. Gmail threads wait until TypeSafe confirms it doesn't train on them. Then, for each thread waiting on you, Whimbrel sends TypeSafe only the redacted subject, the sender's name and domain, and the latest message cut to 1,000 characters. The key and this setting stay on this Mac. Local-only mode blocks it.")
        l.typesafeRejected = String(localized: "TypeSafe rejected this key. Check it and save it again.")
        l.sortTopics = String(localized: "Sort items into topics with TypeSafe")
        l.sortTopicsNote = String(localized: "Sends short excerpts of emails, meetings, calendar events and screen activity to TypeSafe to choose a topic. Gmail and Google Calendar items wait until TypeSafe confirms it doesn't train on them.")
        l.titleCheck = String(localized: "Check calendar meeting titles")
        l.titleCheckNote = String(localized: "Off by default. When on, after a calendar meeting is summarized Whimbrel sends the event title, attendee names and the summary text to TypeSafe to check that the title matches what was discussed. Uses the key above. Never runs in Local-only mode.")
        l.defaultsHeading = String(localized: "Defaults and limits", comment: "Heading of the Settings card with the default provider and model and the daily caps.")
        l.defaultProvider = String(localized: "Default provider")
        l.defaultModel = String(localized: "Default model")
        l.ollamaModelPlaceholder = String(localized: "e.g. gemma4:12b")
        l.userCap = String(localized: "Daily cap per identity (cents)")
        l.globalCap = String(localized: "Daily cap, global (cents)")
        l.defaultWord = String(localized: "default")
        l.capNote = String(localized: "Leave blank to use the conservative built-in default; set to 0 to explicitly disable that cap.")
        l.spendToday = String(localized: "Spend today", comment: "Label of the Settings row that shows today's AI spend.")
        l.refresh = String(localized: "Refresh")
        l.usageHeading = String(localized: "Model usage")
        l.noCalls = String(localized: "No model calls yet.")
        l.usageRange = String(localized: "Time range", comment: "Label of the Settings row with the buttons that switch the usage list between 7 and 30 days.")
        l.showLast7 = String(localized: "Show last 7 days")
        l.showLast30 = String(localized: "Show last 30 days")
        l.promptsHeading = String(localized: "Prompts")
        l.promptsNote = String(localized: "Tune the instructions folded into each surface's prompt. Each surface is independent and syncs across your devices.")
        l.promptSurface = String(localized: "Surface", comment: "Label of the Settings menu that picks which part of Whimbrel a prompt applies to.")
        l.resetPromptLabel = String(localized: "Standard instructions", comment: "Label of the Settings row with the Reset to Default button for a prompt.")
        l.resetPrompt = String(localized: "Reset to Default")
        l.promptIsDefault = String(localized: "This surface already uses the standard instructions.", comment: "Shown on Reset to Default when the prompt has not been changed.")
        return l
    }
}

// MARK: Runtime state

/// What `AIProvidersSettingsPane` keeps in `@State`, with the same calls.
@MainActor
final class AIRuntime: ObservableObject {
    unowned let appState: AppState
    init(appState: AppState) { self.appState = appState }

    struct KeyStatus { var text: String; var isError: Bool; var refused = false }

    @Published var statuses: [String: KeyStatus] = [:]
    @Published var spendToday: CoreSpendToday?
    @Published var usageTotals: CoreUsageTotals?
    @Published var usageRows: [CoreUsageRow] = []
    @Published var usageShowsAll = false
    @Published var providers: [CoreProviderInfo] = CoreMemoryStore.providers()
    @Published var hasImageSearchKey = KeychainService.exists(for: .imageSearch)
    @Published var localModelInstalled = LocalModel.isInstalled

    var routing: RoutingSettings { appState.routing }

    // MARK: Keys (the classic save, check and clear steps)

    func hasKey(_ id: String) -> Bool {
        switch id {
        case "anthropic": return routing.hasAnthropicKey
        case "openai": return routing.hasOpenAIKey
        case "google": return routing.hasGoogleKey
        case "openrouter": return routing.hasOpenRouterKey
        case "typesafe": return appState.replyPriority.hasKey
        case "imageSearch": return hasImageSearchKey
        default: return false
        }
    }

    /// True when the key was saved, so the row can clear what was typed; false keeps it for another try.
    @discardableResult
    func saveKey(_ id: String, value: String) -> Bool {
        switch id {
        case "typesafe":
            let saved = appState.replyPriority.saveKey(value)
            statuses[id] = saved ? KeyStatus(text: String(localized: "Saved."), isError: false) : KeyStatus(text: String(localized: "Failed to save key."), isError: true)
            return saved
        case "imageSearch":
            let saved = KeychainService.save(value, for: .imageSearch)
            statuses[id] = saved ? KeyStatus(text: String(localized: "Saved."), isError: false) : KeyStatus(text: String(localized: "Failed to save key."), isError: true)
            hasImageSearchKey = KeychainService.exists(for: .imageSearch)
            return saved
        default:
            guard let provider = RoutingSettings.Provider(rawValue: id) else { return false }
            let saved: Bool
            switch provider {
            case .anthropic: saved = routing.saveAnthropicKey(value)
            case .openai: saved = routing.saveOpenAIKey(value)
            case .google: saved = routing.saveGoogleKey(value)
            case .openrouter: saved = routing.saveOpenRouterKey(value)
            default: return false
            }
            if saved {
                statuses[id] = KeyStatus(text: String(localized: "Saved. Checking the key…"), isError: false)
                Task {
                    if let check = await routing.checkSavedKey(provider) {
                        var refused = false
                        if case .refused = check { refused = true }
                        statuses[id] = KeyStatus(text: check.message, isError: check.isError, refused: refused)
                    }
                }
            } else {
                statuses[id] = KeyStatus(text: String(localized: "Failed to save key."), isError: true)
            }
            return saved
        }
    }

    func removeKey(_ id: String) {
        switch id {
        case "typesafe": appState.replyPriority.clearKey()
        case "imageSearch":
            KeychainService.delete(for: .imageSearch)
            hasImageSearchKey = KeychainService.exists(for: .imageSearch)
        case "anthropic": routing.clearAnthropicKey()
        case "openai": routing.clearOpenAIKey()
        case "google": routing.clearGoogleKey()
        case "openrouter": routing.clearOpenRouterKey()
        default: return
        }
        statuses[id] = KeyStatus(text: String(localized: "Cleared."), isError: false)
    }

    func keyHint(_ id: String, typed: String) -> String? {
        guard !typed.isEmpty, RoutingSettings.Provider(rawValue: id) != nil else { return nil }
        return CoreMemoryStore.keyFormatHint(provider: id, key: typed)
    }

    func status(_ id: String) -> WSettingsStatus? { AIAdapter.status(text: statuses[id]?.text, isError: statuses[id]?.isError ?? false) }

    // MARK: Models

    func options(for provider: RoutingSettings.Provider) -> [String] {
        switch provider {
        case .ollama: return routing.ollamaModels
        case .anthropic: return routing.anthropicModels
        case .openai: return routing.openaiModels
        case .google: return routing.googleModels
        case .openrouter: return routing.openrouterModels
        case .whimbrel: return routing.whimbrelModelOptions
        }
    }

    func refreshModels(_ id: String) {
        Task {
            switch id {
            case "anthropic": await routing.refreshAnthropicModels()
            case "openai": await routing.refreshOpenAIModels()
            case "google": await routing.refreshGoogleModels()
            case "openrouter": await routing.refreshOpenRouterModels()
            case "ollama": await routing.refreshOllamaModels()
            case "whimbrel": await routing.refreshWhimbrelModels()
            default: break
            }
        }
    }

    func setModel(_ value: String) {
        switch routing.preferredProvider {
        case .ollama: routing.ollamaDefaultModel = value
        case .anthropic: routing.anthropicDefaultModel = value
        case .openai: routing.openaiDefaultModel = value
        case .google: routing.googleDefaultModel = value
        case .openrouter: routing.openrouterDefaultModel = value
        case .whimbrel: routing.whimbrelDefaultModel = value
        }
    }

    /// Keeps the default provider's model valid whenever it's shown as a picker (the classic clamp, unchanged).
    func clampDefaultModelIfNeeded() {
        let options = options(for: routing.preferredProvider)
        guard !options.isEmpty, !options.contains(routing.preferredModel) else { return }
        switch routing.preferredProvider {
        case .anthropic: routing.anthropicDefaultModel = options[0]
        case .openai: routing.openaiDefaultModel = options[0]
        case .google: routing.googleDefaultModel = options[0]
        case .whimbrel: routing.whimbrelDefaultModel = RoutingSettings.whimbrelFallback(in: options)
        // OpenRouter is never snapped to a model the person did not pick; Ollama keeps their pick too.
        case .ollama, .openrouter: break
        }
    }

    // MARK: Spend and usage

    func refreshSpend() {
        guard let store = appState.coreStore else { spendToday = nil; return }
        CoreMemoryStore.accessQueue.async {
            let result = store.spendToday()
            DispatchQueue.main.async { self.spendToday = result }
        }
    }

    func refreshUsage() {
        guard let store = appState.coreStore else {
            usageTotals = nil
            usageRows = []
            return
        }
        let days: Int64 = usageShowsAll ? 30 : 7
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        CoreMemoryStore.accessQueue.async {
            let totals = store.usageTotals()
            let rows = store.usageList(sinceUnixMs: now - days * 24 * 60 * 60 * 1000, untilUnixMs: now + 1, limit: 200) ?? []
            DispatchQueue.main.async {
                self.usageTotals = totals
                self.usageRows = rows
            }
        }
    }

    /// Everything the classic pane does when it appears.
    func appear() {
        refreshSpend()
        refreshUsage()
        localModelInstalled = LocalModel.isInstalled
        hasImageSearchKey = KeychainService.exists(for: .imageSearch)
        if routing.ollamaModels.isEmpty { Task { await routing.refreshOllamaModels() } }
        if routing.hasAnthropicKey && routing.anthropicModels.isEmpty { Task { await routing.refreshAnthropicModels() } }
        if routing.hasOpenAIKey && routing.openaiModels.isEmpty { Task { await routing.refreshOpenAIModels() } }
        if routing.hasGoogleKey && routing.googleModels.isEmpty { Task { await routing.refreshGoogleModels() } }
        if routing.hasOpenRouterKey && routing.openrouterModels.isEmpty { Task { await routing.refreshOpenRouterModels() } }
        if routing.hasKey(for: .whimbrel) && routing.fetchedWhimbrelModels.isEmpty { Task { await routing.refreshWhimbrelModels() } }
        clampDefaultModelIfNeeded()
        appState.refreshModelNotes()
        Task { await appState.syncState.refreshAIBalance() }
    }

    func syncStatusChanged(_ status: SyncStatus) {
        if case .ready = status {
            Task { await appState.syncState.refreshAIBalance() }
            if routing.fetchedWhimbrelModels.isEmpty { Task { await routing.refreshWhimbrelModels() } }
        }
    }
}

// MARK: The view

struct AISettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let highlighted: String?

    var body: some View {
        AISettingsRedesignBody(appState: appState, highlighted: highlighted)
    }
}

private struct AISettingsRedesignBody: View {
    let appState: AppState
    let highlighted: String?
    @StateObject private var runtime: AIRuntime
    @State private var surface: PromptSurface = .ask

    init(appState: AppState, highlighted: String?) {
        self.appState = appState
        self.highlighted = highlighted
        _runtime = StateObject(wrappedValue: AIRuntime(appState: appState))
    }

    private var routing: RoutingSettings { appState.routing }

    var body: some View {
        modelTriggers(WAISettingsView(model: model(), actions: actions(), highlighted: highlighted))
            .onAppear { runtime.appear() }
            .onChange(of: appState.syncState.status) { _, status in runtime.syncStatusChanged(status) }
    }

    /// The model list changes that can leave the default model pointing at one that is gone (the classic triggers).
    private func modelTriggers<V: View>(_ view: V) -> some View {
        let clamp: () -> Void = { runtime.clampDefaultModelIfNeeded() }
        return view
            .onChange(of: routing.preferredProvider) { _, _ in clamp() }
            .onChange(of: routing.ollamaModels) { _, _ in clamp() }
            .onChange(of: routing.anthropicModels) { _, _ in clamp() }
            .onChange(of: routing.openaiModels) { _, _ in clamp() }
            .onChange(of: routing.googleModels) { _, _ in clamp() }
            .onChange(of: routing.fetchedWhimbrelModels) { _, _ in clamp() }
    }

    // MARK: Model

    private func keyModel(id: String, title: String, prefix: String = "sk-…", anchor: SettingsAnchorID? = nil) -> WKeyRowModel {
        let has = runtime.hasKey(id)
        return WKeyRowModel(title: title, isSaved: has, placeholder: AIAdapter.placeholder(hasKey: has, prefixHint: prefix), status: runtime.status(id), anchor: anchor?.rawValue, refused: (runtime.statuses[id]?.refused ?? false) && has)
    }

    private func providerModel(_ provider: RoutingSettings.Provider, keyTitle: String, prefix: String = "sk-…", anchor: SettingsAnchorID) -> WProviderModel {
        let id = provider.rawValue
        let info = runtime.providers.first { $0.id == id }
        let totals = runtime.usageTotals
        let budget = routing.monthlyBudgetCents[id]
        var links: [WProviderLink] = []
        if let info {
            if let page = info.keyPageUrl, URL(string: page) != nil { links.append(WProviderLink(title: AIAdapter.keyLinkTitle(providerName: info.name), url: page)) }
            if let policy = info.policyUrl, URL(string: policy) != nil { links.append(WProviderLink(title: String(localized: "Data policy"), url: policy)) }
        }
        var month: (text: String, fraction: Double?, over: Bool)?
        if let totals { month = AIProvidersSettingsPane.thisMonthLine(spentCents: totals.monthCents(for: id), budgetCents: budget.map(Int64.init)) }
        let state = modelState(provider)
        return WProviderModel(
            id: id, name: info?.name ?? provider.displayName, links: links, key: keyModel(id: id, title: keyTitle, prefix: prefix, anchor: anchor),
            keyNote: provider == .openrouter
                ? String(localized: "One key for any model on OpenRouter's catalogue, free ones included. Pick the model below; until you do, OpenRouter is left out of every chain, because their catalogue changes weekly and there is no fallback model to substitute.") : nil,
            costLine: info.map { AIProvidersSettingsPane.costLine(info: $0, totals: totals) },
            termsLine: info?.dataTerms.map { String(localized: "\($0.localizedText) (Checked \($0.checked).)") },
            monthText: month?.text, monthFraction: month?.fraction, monthOver: month?.over ?? false, budgetDollars: WBudgetRule.dollars(cents: budget),
            syncOn: syncEnabled(provider), models: AIAdapter.modelStatus(hasKey: state.has, isLoading: state.loading, models: state.models, error: state.error, keyRefused: runtime.statuses[id]?.refused ?? false),
            billingConfirmed: provider == .google ? routing.googleAiBillingConfirmed : nil)
    }

    private func modelState(_ p: RoutingSettings.Provider) -> (has: Bool, loading: Bool, models: [String], error: String?) {
        switch p {
        case .anthropic: return (routing.hasAnthropicKey, routing.isLoadingAnthropicModels, routing.anthropicModels, routing.anthropicModelsError)
        case .openai: return (routing.hasOpenAIKey, routing.isLoadingOpenAIModels, routing.openaiModels, routing.openaiModelsError)
        case .google: return (routing.hasGoogleKey, routing.isLoadingGoogleModels, routing.googleModels, routing.googleModelsError)
        case .openrouter: return (routing.hasOpenRouterKey, routing.isLoadingOpenRouterModels, routing.openrouterModels, routing.openrouterModelsError)
        case .ollama: return (true, routing.isLoadingOllamaModels, routing.ollamaModels, routing.ollamaModelsError)
        case .whimbrel: return (routing.hasKey(for: .whimbrel), routing.isLoadingWhimbrelModels, routing.whimbrelModelOptions, routing.whimbrelModelsError)
        }
    }

    private func syncEnabled(_ p: RoutingSettings.Provider) -> Bool {
        switch p {
        case .anthropic: return routing.anthropicKeySyncEnabled
        case .openai: return routing.openaiKeySyncEnabled
        case .google: return routing.googleKeySyncEnabled
        case .openrouter: return routing.openrouterKeySyncEnabled
        default: return false
        }
    }

    private func whimbrelModel() -> WWhimbrelAIModel {
        let sync = appState.syncState
        var ready = false
        if case .ready = sync.status { ready = true }
        guard ready else {
            return WWhimbrelAIModel(signedIn: false, terms: runtime.providers.first { $0.id == "whimbrel" }?.dataTerms?.localizedText)
        }
        var allowance: String?
        var balanceLine: String?
        var low: String?
        if let balance = sync.aiBalance {
            allowance = PlusCopy.allowanceLine(balance, renews: sync.plan?.renews)
            if let prepaid = PlusCopy.prepaidLine(balance) {
                balanceLine = prepaid
            } else if PlusCopy.showsPlainBalanceLine(balance) {
                balanceLine = String(localized: "\(AIProvidersSettingsPane.dollars(micros: balance.balanceMicros)) left")
            }
            if PlusCopy.showsLowWarning(balance) { low = String(localized: "Credit is running low.") }
        }
        let state = modelState(.whimbrel)
        return WWhimbrelAIModel(
            signedIn: true, allowanceLine: allowance, balanceLine: balanceLine, lowWarning: low,
            packs: [WCreditPack(id: "10", title: String(localized: "Add $10")), WCreditPack(id: "25", title: String(localized: "Add $25"))],
            billingError: sync.aiBillingError,
            models: AIAdapter.modelStatus(hasKey: state.has, isLoading: state.loading, models: state.models, error: state.error),
            terms: runtime.providers.first { $0.id == "whimbrel" }?.dataTerms?.localizedText)
    }

    private func typeSafeModel() -> WTypeSafeModel {
        let sort = appState.topicSort
        let input = WEstimateInput(
            awaitingStart: sort.awaitingStart, estimateFailed: sort.estimateFailed,
            estimate: sort.estimate.map { WEstimateInput.Estimate(indexing: $0.indexing, sources: $0.sources, usd: $0.usd) },
            startFailed: sort.startFailed, keyRejected: sort.keyRejected, overBudget: sort.overBudget)
        return WTypeSafeModel(
            key: keyModel(id: "typesafe", title: String(localized: "TypeSafe API Key")), hasKey: appState.replyPriority.hasKey,
            rankEmail: appState.replyPriority.enabled, rankRejected: appState.replyPriority.keyRejected, sortTopics: sort.enabled,
            sortNotes: EstimateAdapter.notes(input, feature: .sort), titleCheck: appState.meetingTitleCheck.enabled)
    }

    private func routingModel() -> WRoutingModel {
        let provider = routing.preferredProvider
        var notes: [String] = []
        for p in [RoutingSettings.Provider.anthropic, .openai, .google] {
            if let note = routing.unavailableNote(for: p) {
                notes.append(String(localized: "\(note.model) isn't available to your \(p.displayName) key; using \(note.using) for now."))
            }
        }
        let spend = runtime.spendToday.map { String(localized: "\($0.userCents)¢ (this identity) / \($0.globalCents)¢ (global)") } ?? String(localized: "No figure yet", comment: "Shown as today's AI spend when the figure is not available.")
        return WRoutingModel(
            providers: AIAdapter.providerChoices(), provider: provider.rawValue,
            model: AIAdapter.defaultModelControl(provider: provider, options: runtime.options(for: provider), selection: routing.preferredModel, ollamaModel: routing.ollamaDefaultModel),
            unavailableNotes: notes, userCapCents: routing.userDailyCapCents, globalCapCents: routing.globalDailyCapCents, spendText: spend)
    }

    private func usageModel() -> WUsageModel {
        let totals = runtime.usageTotals.map {
            String(localized: "Today \(AIProvidersSettingsPane.dollars($0.todayCents)), last 30 days \(AIProvidersSettingsPane.dollars($0.last30DaysCents)), this month \(AIProvidersSettingsPane.dollars($0.monthToDateCents)). Local calls are not metered.")
        }
        return WUsageModel(totalsLine: totals, days: AIAdapter.usageDays(runtime.usageRows), showsAll: runtime.usageShowsAll)
    }

    private func model() -> WAIModel {
        let ollamaState = modelState(.ollama)
        var ramp: WOnRampModel?
        if AIProvidersSettingsPane.showsOnRamp(routing: routing) {
            ramp = WOnRampModel(installOllamaURL: "https://ollama.com/download", geminiKeyURL: runtime.providers.first { $0.id == "google" }?.keyPageUrl)
        }
        let missing = routing.ollamaMissingChoice.map {
            String(localized: "\($0.chosen) isn't installed. Whimbrel is using \($0.using) until you install it or choose another model.")
        }
        return WAIModel(
            labels: AIAdapter.labels(), localOnly: routing.localOnlyMode, onRamp: ramp, localModel: runtime.localModelInstalled ? .installed : .notInstalled,
            whimbrel: whimbrelModel(),
            providers: [
                providerModel(.anthropic, keyTitle: String(localized: "Anthropic API Key"), anchor: .aiAnthropicKey),
                providerModel(.openai, keyTitle: String(localized: "OpenAI API Key"), anchor: .aiOpenaiKey),
                providerModel(.google, keyTitle: String(localized: "Google API Key"), anchor: .aiGoogleKey),
                providerModel(.openrouter, keyTitle: String(localized: "OpenRouter API Key"), prefix: "sk-or-…", anchor: .aiOpenrouterKey),
            ],
            includesOtherComputers: runtime.usageTotals != nil,
            openRouterModel: AIAdapter.openRouterControl(options: routing.openrouterModels, selection: routing.openrouterDefaultModel),
            ollama: WOllamaModel(
                useLocalModel: routing.useLocalModel,
                useLocalCaption: routing.localOnlyMode
                    ? String(localized: "Local-only mode is on, so the local model is always used.")
                    : String(localized: "Off skips Ollama entirely. Turn it off on a Mac that does not run Ollama, so calls stop trying to reach it first."),
                models: AIAdapter.modelStatus(hasKey: true, isLoading: ollamaState.loading, models: ollamaState.models, error: ollamaState.error) ?? WModelListStatus(text: ""),
                missingChoice: missing, remoteGoogleData: routing.remoteOllamaGoogleData),
            cheapFirst: routing.cheapFirst, imageSearchKey: keyModel(id: "imageSearch", title: String(localized: "Image Search Key (Brave Search)"), prefix: String(localized: "Brave Search API key")),
            typeSafe: typeSafeModel(), routing: routingModel(), usage: usageModel(),
            prompts: WPromptsModel(
                surfaces: AIAdapter.promptChoices(), selected: surface.rawValue, text: appState.promptLibrary.instructions[surface] ?? "", help: surface.helpText),
            maxBudgetCents: RoutingSettings.maxBudgetCents, anchors: SettingsAnchorID.ai)
    }

    // MARK: Actions

    private func actions() -> WAIActions {
        WAIActions(
            setLocalOnly: { routing.localOnlyMode = $0 },
            removeLocalModel: { LocalModel.remove(); runtime.localModelInstalled = LocalModel.isInstalled },
            openURL: { if let url = URL(string: $0) { NSWorkspace.shared.open(url) } },
            saveKey: { runtime.saveKey($0, value: $1) }, removeKey: { runtime.removeKey($0) }, keyHint: { runtime.keyHint($0, typed: $1) },
            setSync: { id, on in
                switch id {
                case "anthropic": routing.anthropicKeySyncEnabled = on
                case "openai": routing.openaiKeySyncEnabled = on
                case "google": routing.googleKeySyncEnabled = on
                case "openrouter": routing.openrouterKeySyncEnabled = on
                default: break
                }
            },
            refreshModels: { runtime.refreshModels($0) },
            setBudget: { id, commit in routing.setBudget(id, cents: RoutingSettings.budgetCents(dollars: WBudgetRule.dollars(for: commit))) },
            setGoogleBilling: { routing.googleAiBillingConfirmed = $0 },
            setOpenRouterModel: { routing.openrouterDefaultModel = $0.trimmingCharacters(in: .whitespacesAndNewlines) },
            buyCredit: { pack in Task { await appState.syncState.buyAICredit(pack: pack) } },
            setUseLocalModel: { routing.useLocalModel = $0 }, setRemoteGoogleData: { routing.remoteOllamaGoogleData = $0 },
            setCheapFirst: { routing.cheapFirst = $0 },
            setRankEmail: { appState.setReplyPriorityEnabled($0) }, setSortTopics: { appState.setTopicSortEnabled($0) },
            setTitleCheck: { appState.meetingTitleCheck.enabled = $0 },
            noteAction: { id in
                if id == "retry" { appState.refreshTopicSortEstimate() } else if id == "start" { appState.startTopicSort() }
            },
            setProvider: { if let p = RoutingSettings.Provider(rawValue: $0) { routing.preferredProvider = p } },
            setModel: { runtime.setModel($0) },
            setUserCap: { routing.userDailyCapCents = WCapRule.cents(for: $0) },
            setGlobalCap: { routing.globalDailyCapCents = WCapRule.cents(for: $0) },
            refreshSpend: { runtime.refreshSpend() },
            toggleUsageRange: { runtime.usageShowsAll.toggle(); runtime.refreshUsage() },
            refreshUsage: { runtime.refreshUsage() },
            selectPrompt: { surface = PromptSurface(rawValue: $0) ?? .ask },
            setPromptText: { appState.promptLibrary.instructions[surface] = $0 },
            resetPrompt: { appState.promptLibrary.instructions[surface] = "" })
    }
}
