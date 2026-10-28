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

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("AI & Routing").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Configure hosted LLM providers for Ask/Digest. Keys are stored in the macOS Keychain, never in a plaintext file.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            keyRow(
                title: "Anthropic API Key",
                hasKey: appState.routing.hasAnthropicKey,
                input: $anthropicKeyInput,
                status: anthropicStatus,
                save: {
                    if appState.routing.saveAnthropicKey(anthropicKeyInput) {
                        anthropicKeyInput = ""
                        anthropicStatus = "Saved."
                    } else {
                        anthropicStatus = "Failed to save key."
                    }
                },
                clear: {
                    appState.routing.clearAnthropicKey()
                    anthropicStatus = "Cleared."
                }
            )
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

            keyRow(
                title: "OpenAI API Key",
                hasKey: appState.routing.hasOpenAIKey,
                input: $openaiKeyInput,
                status: openaiStatus,
                save: {
                    if appState.routing.saveOpenAIKey(openaiKeyInput) {
                        openaiKeyInput = ""
                        openaiStatus = "Saved."
                    } else {
                        openaiStatus = "Failed to save key."
                    }
                },
                clear: {
                    appState.routing.clearOpenAIKey()
                    openaiStatus = "Cleared."
                }
            )
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

            keyRow(
                title: "Google API Key",
                hasKey: appState.routing.hasGoogleKey,
                input: $googleKeyInput,
                status: googleStatus,
                save: {
                    if appState.routing.saveGoogleKey(googleKeyInput) {
                        googleKeyInput = ""
                        googleStatus = "Saved."
                    } else {
                        googleStatus = "Failed to save key."
                    }
                },
                clear: {
                    appState.routing.clearGoogleKey()
                    googleStatus = "Cleared."
                }
            )
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

            Text("Ollama").font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Local server — no API key needed.")
                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            modelStatusRow(
                hasKey: true,
                isLoading: appState.routing.isLoadingOllamaModels,
                models: appState.routing.ollamaModels,
                error: appState.routing.ollamaModelsError,
                refresh: { Task { await appState.routing.refreshOllamaModels() } }
            )

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
        }
        .onChange(of: appState.routing.preferredProvider) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.ollamaModels) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.anthropicModels) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.openaiModels) { _, _ in clampDefaultModelIfNeeded() }
        .onChange(of: appState.routing.googleModels) { _, _ in clampDefaultModelIfNeeded() }
        .onAppear {
            refreshSpend()
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
        }
    }

    @ViewBuilder
    private func keyRow(
        title: String,
        hasKey: Bool,
        input: Binding<String>,
        placeholder: String = "sk-…",
        status: String?,
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
            if let status {
                Text(status).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
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
