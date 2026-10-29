import AppKit
import SwiftUI

struct AIProvidersSettingsPane: View {
    @EnvironmentObject var appState: AppState

    @State private var anthropicKeyInput: String = ""
    @State private var openaiKeyInput: String = ""
    @State private var googleKeyInput: String = ""
    @State private var anthropicStatus: String?
    @State private var openaiStatus: String?
    @State private var googleStatus: String?
    @State private var typesafeKeyInput: String = ""
    @State private var typesafeStatus: String?
    @State private var spendToday: CoreSpendToday?
    @State private var imageSearchKeyInput: String = ""
    @State private var imageSearchStatus: String?
    @State private var hasImageSearchKey = KeychainService.exists(for: .imageSearch)
    @State private var anthropicStatusIsError = false
    @State private var openaiStatusIsError = false
    @State private var googleStatusIsError = false
    @State private var openrouterKeyInput: String = ""
    @State private var openrouterStatus: String?
    @State private var openrouterStatusIsError = false
    /// The key on-ramp table from core (key pages, policy pages, data-terms notes).
    @State private var providers: [CoreProviderInfo] = CoreMemoryStore.providers()
    @State private var usageTotals: CoreUsageTotals?
    @State private var usageRows: [CoreUsageRow] = []
    @State private var usageShowsAll = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("AI & Routing").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Configure hosted LLM providers for Ask/Digest. Keys are stored in the macOS Keychain, never in a plaintext file.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            if Self.showsOnRamp(routing: appState.routing) {
                onRamp
            }

            providerLinks(for: "anthropic")
            keyRow(
                title: "Anthropic API Key",
                hasKey: appState.routing.hasAnthropicKey,
                input: $anthropicKeyInput,
                status: anthropicStatus,
                statusIsError: anthropicStatusIsError,
                formatHint: formatHint(provider: "anthropic", key: anthropicKeyInput),
                save: {
                    if appState.routing.saveAnthropicKey(anthropicKeyInput) {
                        anthropicKeyInput = ""
                        anthropicStatus = "Saved. Checking the key…"
                        anthropicStatusIsError = false
                        Task {
                            if let check = await appState.routing.checkSavedKey(.anthropic) {
                                anthropicStatus = check.message
                                anthropicStatusIsError = check.isError
                            }
                        }
                    } else {
                        anthropicStatus = "Failed to save key."
                        anthropicStatusIsError = true
                    }
                },
                clear: {
                    appState.routing.clearAnthropicKey()
                    anthropicStatus = "Cleared."
                    anthropicStatusIsError = false
                }
            )
            providerNotes(for: "anthropic")
            Toggle("Sync to devices", isOn: $appState.routing.anthropicKeySyncEnabled)
                .disabled(!appState.routing.hasAnthropicKey)
                .font(.whimbrelBody(.caption))
            modelStatusRow(
                hasKey: appState.routing.hasAnthropicKey,
                isLoading: appState.routing.isLoadingAnthropicModels,
                models: appState.routing.anthropicModels,
                error: appState.routing.anthropicModelsError,
                refresh: { Task { await appState.routing.refreshAnthropicModels() } }
            )

            providerLinks(for: "openai")
            keyRow(
                title: "OpenAI API Key",
                hasKey: appState.routing.hasOpenAIKey,
                input: $openaiKeyInput,
                status: openaiStatus,
                statusIsError: openaiStatusIsError,
                formatHint: formatHint(provider: "openai", key: openaiKeyInput),
                save: {
                    if appState.routing.saveOpenAIKey(openaiKeyInput) {
                        openaiKeyInput = ""
                        openaiStatus = "Saved. Checking the key…"
                        openaiStatusIsError = false
                        Task {
                            if let check = await appState.routing.checkSavedKey(.openai) {
                                openaiStatus = check.message
                                openaiStatusIsError = check.isError
                            }
                        }
                    } else {
                        openaiStatus = "Failed to save key."
                        openaiStatusIsError = true
                    }
                },
                clear: {
                    appState.routing.clearOpenAIKey()
                    openaiStatus = "Cleared."
                    openaiStatusIsError = false
                }
            )
            providerNotes(for: "openai")
            Toggle("Sync to devices", isOn: $appState.routing.openaiKeySyncEnabled)
                .disabled(!appState.routing.hasOpenAIKey)
                .font(.whimbrelBody(.caption))
            modelStatusRow(
                hasKey: appState.routing.hasOpenAIKey,
                isLoading: appState.routing.isLoadingOpenAIModels,
                models: appState.routing.openaiModels,
                error: appState.routing.openaiModelsError,
                refresh: { Task { await appState.routing.refreshOpenAIModels() } }
            )

            providerLinks(for: "google")
            keyRow(
                title: "Google API Key",
                hasKey: appState.routing.hasGoogleKey,
                input: $googleKeyInput,
                status: googleStatus,
                statusIsError: googleStatusIsError,
                formatHint: formatHint(provider: "google", key: googleKeyInput),
                save: {
                    if appState.routing.saveGoogleKey(googleKeyInput) {
                        googleKeyInput = ""
                        googleStatus = "Saved. Checking the key…"
                        googleStatusIsError = false
                        Task {
                            if let check = await appState.routing.checkSavedKey(.google) {
                                googleStatus = check.message
                                googleStatusIsError = check.isError
                            }
                        }
                    } else {
                        googleStatus = "Failed to save key."
                        googleStatusIsError = true
                    }
                },
                clear: {
                    appState.routing.clearGoogleKey()
                    googleStatus = "Cleared."
                    googleStatusIsError = false
                }
            )
            providerNotes(for: "google")
            Toggle("Sync to devices", isOn: $appState.routing.googleKeySyncEnabled)
                .disabled(!appState.routing.hasGoogleKey)
                .font(.whimbrelBody(.caption))
            modelStatusRow(
                hasKey: appState.routing.hasGoogleKey,
                isLoading: appState.routing.isLoadingGoogleModels,
                models: appState.routing.googleModels,
                error: appState.routing.googleModelsError,
                refresh: { Task { await appState.routing.refreshGoogleModels() } }
            )

            providerLinks(for: "openrouter")
            keyRow(
                title: "OpenRouter API Key",
                hasKey: appState.routing.hasOpenRouterKey,
                input: $openrouterKeyInput,
                placeholder: "sk-or-…",
                status: openrouterStatus,
                statusIsError: openrouterStatusIsError,
                formatHint: formatHint(provider: "openrouter", key: openrouterKeyInput),
                save: {
                    if appState.routing.saveOpenRouterKey(openrouterKeyInput) {
                        openrouterKeyInput = ""
                        openrouterStatus = "Saved. Checking the key…"
                        openrouterStatusIsError = false
                        Task {
                            if let check = await appState.routing.checkSavedKey(.openrouter) {
                                openrouterStatus = check.message
                                openrouterStatusIsError = check.isError
                            }
                        }
                    } else {
                        openrouterStatus = "Failed to save key."
                        openrouterStatusIsError = true
                    }
                },
                clear: {
                    appState.routing.clearOpenRouterKey()
                    openrouterStatus = "Cleared."
                    openrouterStatusIsError = false
                }
            )
            Text("One key for any model on OpenRouter's catalogue, free ones included. Pick the model below; until you do, OpenRouter is left out of every chain, because their catalogue changes weekly and there is no fallback model to substitute.")
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            providerNotes(for: "openrouter")
            Toggle("Sync to devices", isOn: $appState.routing.openrouterKeySyncEnabled)
                .disabled(!appState.routing.hasOpenRouterKey)
                .font(.whimbrelBody(.caption))
            modelStatusRow(
                hasKey: appState.routing.hasOpenRouterKey,
                isLoading: appState.routing.isLoadingOpenRouterModels,
                models: appState.routing.openrouterModels,
                error: appState.routing.openrouterModelsError,
                refresh: { Task { await appState.routing.refreshOpenRouterModels() } }
            )
            if appState.routing.hasOpenRouterKey {
                HStack {
                    Text("OpenRouter model:").frame(width: 160, alignment: .leading)
                    if appState.routing.openrouterModels.isEmpty {
                        TextField("vendor/model or vendor/model:free", text: Binding(
                            get: { appState.routing.openrouterDefaultModel },
                            set: { appState.routing.openrouterDefaultModel = $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                        ))
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 260)
                    } else {
                        Picker("", selection: Binding(
                            get: { appState.routing.openrouterDefaultModel },
                            set: { appState.routing.openrouterDefaultModel = $0 }
                        )) {
                            Text("Not picked").tag("")
                            ForEach(appState.routing.openrouterModels, id: \.self) { Text($0).tag($0) }
                        }
                        .labelsHidden()
                        .frame(width: 260)
                    }
                }
            }

            Text("Ollama").font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Local server, no API key needed. Runs on this Mac and nothing leaves it.")
                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            Toggle("Use local model", isOn: Binding(
                get: { appState.routing.useLocalModel },
                set: { appState.routing.useLocalModel = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)
            Text(appState.routing.localOnlyMode
                 ? "Local-only mode is on, so the local model is always used."
                 : "Off skips Ollama entirely. Turn it off on a Mac that does not run Ollama, so calls stop trying to reach it first.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            modelStatusRow(
                hasKey: true,
                isLoading: appState.routing.isLoadingOllamaModels,
                models: appState.routing.ollamaModels,
                error: appState.routing.ollamaModelsError,
                refresh: { Task { await appState.routing.refreshOllamaModels() } }
            )

            Divider().overlay(Color.whimbrelBorder)

            Toggle("Use the cheapest model for small tasks", isOn: Binding(
                get: { appState.routing.cheapFirst },
                set: { appState.routing.cheapFirst = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)
            Text("Conversation titles, topic names and profile suggestions go to the local model first, then to the cheapest model you have a key for. Chat, summaries and digests keep using your default provider. Each routine picks its own model in Routines.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            keyRow(
                title: "Image Search Key (Brave Search)",
                hasKey: hasImageSearchKey,
                input: $imageSearchKeyInput,
                placeholder: "Brave Search API key",
                status: imageSearchStatus,
                save: {
                    if KeychainService.save(imageSearchKeyInput, for: .imageSearch) {
                        imageSearchKeyInput = ""
                        imageSearchStatus = "Saved."
                    } else {
                        imageSearchStatus = "Failed to save key."
                    }
                    hasImageSearchKey = KeychainService.exists(for: .imageSearch)
                },
                clear: {
                    KeychainService.delete(for: .imageSearch)
                    hasImageSearchKey = KeychainService.exists(for: .imageSearch)
                    imageSearchStatus = "Cleared."
                }
            )
            Text("Lets the skin chat in General settings find images on the web, with strict safe-search. Searches count toward the daily spending limit. The key stays on this Mac. Local-only mode blocks it.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            Text("Reply priorities (TypeSafe)")
                .font(.whimbrelBody(.headline))
                .foregroundStyle(Color.whimbrelTextPrimary)
            keyRow(
                title: "TypeSafe API Key",
                hasKey: appState.replyPriority.hasKey,
                input: $typesafeKeyInput,
                status: typesafeStatus,
                save: {
                    if appState.replyPriority.saveKey(typesafeKeyInput) {
                        typesafeKeyInput = ""
                        typesafeStatus = "Saved."
                    } else {
                        typesafeStatus = "Failed to save key."
                    }
                },
                clear: {
                    appState.replyPriority.clearKey()
                    typesafeStatus = "Cleared."
                }
            )
            Toggle("Rank email by reply priority", isOn: Binding(
                get: { appState.replyPriority.enabled },
                set: { appState.setReplyPriorityEnabled($0) }
            ))
            .tint(Color.whimbrelAccentPrimary)
            .disabled(!appState.replyPriority.hasKey)
            Text("Off by default. When on, Whimbrel sends only these to TypeSafe for each Gmail thread waiting on you: the redacted subject, the sender's name and domain, and the latest message cut to 1,000 characters. The key and this setting stay on this Mac. Local-only mode blocks it.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if appState.replyPriority.keyRejected {
                Text("TypeSafe rejected this key. Check it and save it again.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }
            Toggle("Check calendar meeting titles", isOn: Binding(
                get: { appState.meetingTitleCheck.enabled },
                set: { appState.meetingTitleCheck.enabled = $0 }
            ))
                .tint(Color.whimbrelAccentPrimary)
                .disabled(!appState.replyPriority.hasKey)
            Text("Off by default. When on, after a calendar meeting is summarized Whimbrel sends the event title, attendee names and the summary text to TypeSafe to check that the title matches what was discussed. Uses the key above. Never runs in Local-only mode.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            HStack {
                Text("Default provider:").frame(width: 160, alignment: .leading)
                Picker("", selection: Binding(
                    get: { appState.routing.preferredProvider },
                    set: { appState.routing.preferredProvider = $0 }
                )) {
                    ForEach(RoutingSettings.Provider.allCases) { Text($0.displayName).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: 280)
            }

            HStack {
                Text("Default model:").frame(width: 160, alignment: .leading)
                defaultModelControl
            }

            Divider().overlay(Color.whimbrelBorder)

            capRow(
                label: "Daily cap per identity (cents)",
                value: Binding(
                    get: { appState.routing.userDailyCapCents },
                    set: { appState.routing.userDailyCapCents = $0 }
                )
            )
            capRow(
                label: "Daily cap, global (cents)",
                value: Binding(
                    get: { appState.routing.globalDailyCapCents },
                    set: { appState.routing.globalDailyCapCents = $0 }
                )
            )
            Text("Leave blank to use the conservative built-in default; set to 0 to explicitly disable that cap.")
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)

            Divider().overlay(Color.whimbrelBorder)

            Toggle("Local-only mode", isOn: Binding(
                get: { appState.routing.localOnlyMode },
                set: { appState.routing.localOnlyMode = $0 }
            ))
            .tint(Color.whimbrelAccentPrimary)
            Text("When on, every conversation uses Ollama only, regardless of the provider picked in Ask or Digest. Hosted providers are never called.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Divider().overlay(Color.whimbrelBorder)

            HStack {
                Text("Spend today:").font(.whimbrelBody(.callout))
                if let spendToday {
                    Text("\(spendToday.userCents)¢ (this identity) / \(spendToday.globalCents)¢ (global)")
                        .font(.whimbrelBody(.callout).bold())
                } else {
                    Text("—").font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
                }
                Button("Refresh") { refreshSpend() }
                    .buttonStyle(.whimbrelSecondary)
            }

            Divider().overlay(Color.whimbrelBorder)

            usageSection
        }
        .onChange(of: appState.routing.preferredProvider) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.ollamaModels) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.anthropicModels) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.openaiModels) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.googleModels) { _, _ in clampDefaultModelIfNeeded() }
        .onAppear {
            refreshSpend()
            refreshUsage()
            if appState.routing.ollamaModels.isEmpty {
                Task { await appState.routing.refreshOllamaModels() }
            }
            if appState.routing.hasAnthropicKey && appState.routing.anthropicModels.isEmpty {
                Task { await appState.routing.refreshAnthropicModels() }
            }
            if appState.routing.hasOpenAIKey && appState.routing.openaiModels.isEmpty {
                Task { await appState.routing.refreshOpenAIModels() }
            }
            if appState.routing.hasGoogleKey && appState.routing.googleModels.isEmpty {
                Task { await appState.routing.refreshGoogleModels() }
            }
            if appState.routing.hasOpenRouterKey && appState.routing.openrouterModels.isEmpty {
                Task { await appState.routing.refreshOpenRouterModels() }
            }
            clampDefaultModelIfNeeded()
        }
    }

    /// Options for the default-model control below, for whichever provider is currently
    /// the default — live-fetched via `RoutingSettings` (`ollamaModels`/`anthropicModels`/
    /// `openaiModels`), matching Timeline's picker.
    private var defaultModelOptions: [String] {
        switch appState.routing.preferredProvider {
        case .ollama: return appState.routing.ollamaModels
        case .anthropic: return appState.routing.anthropicModels
        case .openai: return appState.routing.openaiModels
        case .google: return appState.routing.googleModels
        case .openrouter: return appState.routing.openrouterModels
        }
    }

    @ViewBuilder
    private var defaultModelControl: some View {
        if appState.routing.preferredProvider == .ollama && defaultModelOptions.isEmpty {
            // Ollama has no fixed catalog to fall back to — if its local server can't be
            // reached, free-text entry is the only way to specify a model at all.
            TextField("e.g. gemma4:12b", text: Binding(
                get: { appState.routing.ollamaDefaultModel },
                set: { appState.routing.ollamaDefaultModel = $0 }
            ))
            .textFieldStyle(.roundedBorder)
            .frame(width: 220)
        } else if defaultModelOptions.isEmpty {
            Text("No models loaded — save a key above, then Refresh Models.")
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            Picker("", selection: Binding(
                get: { appState.routing.preferredModel },
                set: { newValue in
                    switch appState.routing.preferredProvider {
                    case .ollama: appState.routing.ollamaDefaultModel = newValue
                    case .anthropic: appState.routing.anthropicDefaultModel = newValue
                    case .openai: appState.routing.openaiDefaultModel = newValue
                    case .google: appState.routing.googleDefaultModel = newValue
                    case .openrouter: appState.routing.openrouterDefaultModel = newValue
                    }
                }
            )) {
                ForEach(defaultModelOptions, id: \.self) { Text($0).tag($0) }
            }
            .labelsHidden()
            .frame(width: 260)
        }
    }

    /// Keeps the default provider's model valid whenever it's shown as a picker: if the
    /// stored value isn't one of the currently-fetched models (e.g. it's still the
    /// original hardcoded default, or the model list just finished loading), snap it to
    /// the first available option — mirrors Timeline's `clampSelectedModelIfNeeded`,
    /// which the final review required for the exact same reason (an invalid model id
    /// silently sent to a hosted provider fails with no user-visible error).
    private func clampDefaultModelIfNeeded() {
        guard !defaultModelOptions.isEmpty, !defaultModelOptions.contains(appState.routing.preferredModel) else { return }
        switch appState.routing.preferredProvider {
        case .ollama: appState.routing.ollamaDefaultModel = defaultModelOptions[0]
        case .anthropic: appState.routing.anthropicDefaultModel = defaultModelOptions[0]
        case .openai: appState.routing.openaiDefaultModel = defaultModelOptions[0]
        case .google: appState.routing.googleDefaultModel = defaultModelOptions[0]
        // OpenRouter is never snapped to a model the user did not pick: an unpicked
        // model keeps it out of the chain on purpose.
        case .openrouter: break
        }
    }

    @ViewBuilder
    private func keyRow(
        title: String,
        hasKey: Bool,
        input: Binding<String>,
        placeholder: String = "sk-…",
        status: String?,
        statusIsError: Bool = false,
        formatHint: String? = nil,
        save: @escaping () -> Void,
        clear: @escaping () -> Void
    ) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text(title).font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
            HStack {
                SecureField(hasKey ? placeholder : "Not configured", text: input)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 280)
                Button("Save") { save() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(input.wrappedValue.isEmpty)
                Button("Clear") { clear() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(!hasKey)
            }
            if let formatHint {
                // Advisory: prefixes change, so the key saves either way.
                Text(formatHint).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if let status {
                Text(status)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(statusIsError ? Color.whimbrelError : Color.whimbrelTextSecondary)
            }
        }
    }

    /// The format hint for what is in the field right now, nil when it looks right or
    /// the field is empty.
    private func formatHint(provider: String, key: String) -> String? {
        guard !key.isEmpty else { return nil }
        return CoreMemoryStore.keyFormatHint(provider: provider, key: key)
    }

    /// "Get a key" and the policy link for one provider, above its key row.
    @ViewBuilder
    private func providerLinks(for id: String) -> some View {
        if let info = providers.first(where: { $0.id == id }) {
            HStack(spacing: Spacing.md) {
                if let page = info.keyPageUrl, let url = URL(string: page) {
                    Link("Get a \(info.name) key", destination: url)
                        .font(.whimbrelBody(.caption))
                }
                if let policy = info.policyUrl, let url = URL(string: policy) {
                    Link("Data policy", destination: url)
                        .font(.whimbrelBody(.caption))
                }
            }
        }
    }

    /// Under a key row: the light-task model and this provider's last-30-day spend, and
    /// the dated data-terms note where the provider's free tier may use prompts.
    @ViewBuilder
    private func providerNotes(for id: String) -> some View {
        if let info = providers.first(where: { $0.id == id }) {
            Text(Self.costLine(info: info, totals: usageTotals))
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let terms = info.dataTerms {
                Text("\(terms.text) (Checked \(terms.checked).)")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// The cost line for one provider: the light-task model, then the user's own figure
    /// once there is one, else the one general claim doc 16 makes.
    static func costLine(info: CoreProviderInfo, totals: CoreUsageTotals?) -> String {
        var parts: [String] = []
        if !info.cheapModel.isEmpty {
            parts.append("Small tasks use \(info.cheapModel).")
        }
        if let totals, totals.last30DaysCents > 0 {
            parts.append("In the last 30 days you spent \(Self.dollars(totals.cents(for: info.id))) on \(info.name).")
        } else {
            parts.append("Most people spend under a dollar a month at this setting.")
        }
        return parts.joined(separator: " ")
    }

    static func dollars(_ cents: Int64) -> String {
        String(format: "$%.2f", Double(cents) / 100)
    }

    /// Whether the on-ramp card shows: no local model is reachable (or local is off) and
    /// no hosted key is saved, so chat, summaries and routines have nothing to call.
    @MainActor
    static func showsOnRamp(routing: RoutingSettings) -> Bool {
        let localAvailable = routing.useLocalModel && !routing.ollamaModels.isEmpty
        return !localAvailable && !routing.hasAnyHostedKey
    }

    private var onRamp: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Pick where AI runs").font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Whimbrel works without any AI model: capture, transcripts and search still work. Chat, summaries and routines need one.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: Spacing.md) {
                if let url = URL(string: "https://ollama.com/download") {
                    Button("Install Ollama") { NSWorkspace.shared.open(url) }.buttonStyle(.whimbrelSecondary)
                }
                if let page = providers.first(where: { $0.id == "google" })?.keyPageUrl, let url = URL(string: page) {
                    Button("Get a free Gemini key") { NSWorkspace.shared.open(url) }.buttonStyle(.whimbrelSecondary)
                }
            }
            Text("Ollama is free, runs on this Mac and nothing leaves it. Gemini's free tier is free, and Google may use prompts to improve its products. Or paste any Anthropic, OpenAI or Google key below.")
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(Spacing.md)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.whimbrelBorder))
        .accessibilityIdentifier("ai-on-ramp")
    }

    /// Model usage: what every call cost, from the ledger the spend caps already keep.
    private var usageSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Model usage").font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
            if let usageTotals {
                Text("Today \(Self.dollars(usageTotals.todayCents)), last 30 days \(Self.dollars(usageTotals.last30DaysCents)), this month \(Self.dollars(usageTotals.monthToDateCents)). Local calls are not metered.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if usageRows.isEmpty {
                Text("No model calls yet.").font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                ForEach(Self.groupedByDay(usageRows), id: \.day) { group in
                    Text(group.day).font(.whimbrelBody(.caption).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                    ForEach(group.rows) { row in
                        HStack(spacing: Spacing.md) {
                            Text(Self.timeLabel(row.tsUnixMs)).frame(width: 56, alignment: .leading)
                            Text(row.task).frame(width: 120, alignment: .leading)
                            Text("\(row.provider) \(row.model)").frame(minWidth: 180, alignment: .leading).lineLimit(1)
                            Text(Self.tokensLabel(row)).frame(width: 110, alignment: .trailing)
                            Text(row.local ? "local" : Self.dollars(row.estimatedCostCents)).frame(width: 60, alignment: .trailing)
                        }
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                }
            }
            HStack {
                Button(usageShowsAll ? "Show last 7 days" : "Show last 30 days") {
                    usageShowsAll.toggle()
                    refreshUsage()
                }
                .buttonStyle(.whimbrelSecondary)
                Button("Refresh") { refreshUsage() }.buttonStyle(.whimbrelSecondary)
            }
        }
        .accessibilityIdentifier("model-usage")
    }

    struct UsageDay {
        let day: String
        let rows: [CoreUsageRow]
    }

    /// Rows grouped by local calendar day, in the order they arrive (newest first).
    static func groupedByDay(_ rows: [CoreUsageRow]) -> [UsageDay] {
        var groups: [UsageDay] = []
        for row in rows {
            let day = dayLabel(row.tsUnixMs)
            if let last = groups.last, last.day == day {
                groups[groups.count - 1] = UsageDay(day: day, rows: last.rows + [row])
            } else {
                groups.append(UsageDay(day: day, rows: [row]))
            }
        }
        return groups
    }

    static func dayLabel(_ unixMs: Int64) -> String {
        let date = Date(timeIntervalSince1970: Double(unixMs) / 1000)
        return date.formatted(date: .abbreviated, time: .omitted)
    }

    static func timeLabel(_ unixMs: Int64) -> String {
        let date = Date(timeIntervalSince1970: Double(unixMs) / 1000)
        return date.formatted(date: .omitted, time: .shortened)
    }

    static func tokensLabel(_ row: CoreUsageRow) -> String {
        guard let input = row.inputTokens, let output = row.outputTokens else { return "" }
        return "\(input) in, \(output) out"
    }

    private func refreshUsage() {
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
                usageTotals = totals
                usageRows = rows
            }
        }
    }

    /// Live model-list status for one hosted provider (fetched via `HostedModelsService`,
    /// see `RoutingSettings`) — shown right under that provider's key row.
    @ViewBuilder
    private func modelStatusRow(
        hasKey: Bool, isLoading: Bool, models: [String], error: String?, refresh: @escaping () -> Void
    ) -> some View {
        if hasKey {
            HStack {
                if isLoading {
                    ProgressView().controlSize(.small)
                    Text("Loading models…").font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
                } else if let error {
                    Text("Couldn't load models: \(error)").font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelError)
                } else {
                    Text("\(models.count) model\(models.count == 1 ? "" : "s") available")
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                Button("Refresh Models") { refresh() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(isLoading)
            }
        }
    }

    @ViewBuilder
    private func capRow(label: String, value: Binding<Int?>) -> some View {
        HStack {
            Text(label).frame(width: 260, alignment: .leading)
            TextField("default", text: Binding(
                get: { value.wrappedValue.map(String.init) ?? "" },
                set: { newValue in
                    value.wrappedValue = newValue.isEmpty ? nil : Int(newValue).map { max(0, $0) }
                }
            ))
            .textFieldStyle(.roundedBorder)
            .frame(width: 100)
        }
    }

    private func refreshSpend() {
        guard let store = appState.coreStore else {
            spendToday = nil
            return
        }
        CoreMemoryStore.accessQueue.async {
            let result = store.spendToday()
            DispatchQueue.main.async {
                spendToday = result
            }
        }
    }
}
