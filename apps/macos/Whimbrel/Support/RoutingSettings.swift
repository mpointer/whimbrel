import Foundation

/// Sub-project 2: default LLM provider/model, spend caps, and local-only mode —
/// persisted in `UserDefaults` (matching `RetentionSettings`'s pattern). API keys
/// themselves live in the Keychain via `KeychainService`, never here — this class only
/// tracks whether a key is present, not its value.
@MainActor
final class RoutingSettings: ObservableObject {
    enum Provider: String, CaseIterable, Identifiable {
        case ollama
        case anthropic
        case openai
        case google
        /// One key, any model on OpenRouter's catalogue (free ones included).
        case openrouter
        var id: String { rawValue }

        var displayName: String {
            switch self {
            case .ollama: return "Ollama"
            case .anthropic: return "Anthropic"
            case .openai: return "OpenAI"
            case .google: return "Google"
            case .openrouter: return "OpenRouter"
            }
        }

        /// The Keychain item for a hosted provider's key; nil for Ollama.
        var keychain: KeychainService.Provider? {
            switch self {
            case .ollama: return nil
            case .anthropic: return .anthropic
            case .openai: return .openai
            case .google: return .google
            case .openrouter: return .openrouter
            }
        }
    }

    /// Mirrors `core/llm::DEFAULT_USER_DAILY_CAP_CENTS` (core/llm/src/ledger.rs) — kept
    /// here since Swift has no visibility into that Rust constant across the FFI
    /// boundary. Used by the Ask-chat usage-today line (LB0-5) to show the effective
    /// cap when `userDailyCapCents` is unset (`nil`); must be updated if the Rust
    /// default ever changes.
    static let defaultUserDailyCapCents: Int64 = 200

    private static let preferredProviderKey = "com.mpointer.kestrel.routingPreferredProvider"
    private static let ollamaDefaultModelKey = "com.mpointer.kestrel.routingOllamaDefaultModel"
    private static let anthropicDefaultModelKey = "com.mpointer.kestrel.routingAnthropicDefaultModel"
    private static let openaiDefaultModelKey = "com.mpointer.kestrel.routingOpenAIDefaultModel"
    private static let googleDefaultModelKey = "com.mpointer.kestrel.routingGoogleDefaultModel"
    private static let openrouterDefaultModelKey = "com.mpointer.kestrel.routingOpenRouterDefaultModel"
    private static let openrouterPricesKey = "com.mpointer.kestrel.routingOpenRouterPrices"
    private static let userDailyCapCentsKey = "com.mpointer.kestrel.routingUserDailyCapCents"
    private static let globalDailyCapCentsKey = "com.mpointer.kestrel.routingGlobalDailyCapCents"
    private static let localOnlyModeKey = "com.mpointer.kestrel.routingLocalOnlyMode"
    private static let cheapFirstKey = "com.mpointer.kestrel.routingCheapFirst"
    private static let useLocalModelKey = "com.mpointer.kestrel.routingUseLocalModel"
    private static let customPromptInstructionsKey = "com.mpointer.kestrel.summaryCustomPromptInstructions"

    @Published var preferredProvider: Provider {
        didSet {
            UserDefaults.standard.set(preferredProvider.rawValue, forKey: Self.preferredProviderKey)
            SettingsFieldTimestamps.touch("routing.preferredProvider")
        }
    }
    /// Each provider gets its own persisted default model — a single shared field would
    /// mean picking a default Anthropic model overwrites the remembered Ollama default
    /// (and vice versa) the moment the default provider switches.
    @Published var ollamaDefaultModel: String {
        didSet {
            UserDefaults.standard.set(ollamaDefaultModel, forKey: Self.ollamaDefaultModelKey)
            SettingsFieldTimestamps.touch("routing.ollamaDefaultModel")
        }
    }
    @Published var anthropicDefaultModel: String {
        didSet {
            UserDefaults.standard.set(anthropicDefaultModel, forKey: Self.anthropicDefaultModelKey)
            SettingsFieldTimestamps.touch("routing.anthropicDefaultModel")
        }
    }
    @Published var openaiDefaultModel: String {
        didSet {
            UserDefaults.standard.set(openaiDefaultModel, forKey: Self.openaiDefaultModelKey)
            SettingsFieldTimestamps.touch("routing.openaiDefaultModel")
        }
    }
    @Published var googleDefaultModel: String {
        didSet {
            UserDefaults.standard.set(googleDefaultModel, forKey: Self.googleDefaultModelKey)
            SettingsFieldTimestamps.touch("routing.googleDefaultModel")
        }
    }
    /// The user's OpenRouter pick, also the model light tasks use there. Empty until
    /// picked: core then leaves OpenRouter out of every chain, because their catalogue
    /// changes weekly and there is no fallback model to substitute.
    @Published var openrouterDefaultModel: String {
        didSet {
            UserDefaults.standard.set(openrouterDefaultModel, forKey: Self.openrouterDefaultModelKey)
            SettingsFieldTimestamps.touch("routing.openrouterDefaultModel")
        }
    }
    /// Cents per 1K input and output tokens per OpenRouter model id, cached from the
    /// catalogue when the key was last checked; core prices calls from it.
    @Published var openrouterPrices: [String: CoreRoutingConfig.Price] {
        didSet {
            let data = try? JSONEncoder().encode(openrouterPrices)
            UserDefaults.standard.set(data, forKey: Self.openrouterPricesKey)
        }
    }
    /// The current default provider's own model string — convenience read-only
    /// accessor; set the provider-specific `xDefaultModel` property directly to change it.
    var preferredModel: String {
        switch preferredProvider {
        case .ollama: return ollamaDefaultModel
        case .anthropic: return anthropicDefaultModel
        case .openai: return openaiDefaultModel
        case .google: return googleDefaultModel
        case .openrouter: return openrouterDefaultModel
        }
    }
    /// `nil` means "never set — use the conservative built-in default"
    /// (`core/llm::SpendCapConfig`'s existing `Option<i64>` semantics); `Some(0)` is the
    /// distinct, meaningful "explicit opt-out" value. `UserDefaults.object(forKey:)`
    /// returns nil when the key was never set, unlike `integer(forKey:)`'s
    /// type-coercing 0 default — matching `RetentionSettings`'s own nil-vs-zero note,
    /// but as an actual `Int?` here (rather than a separate boolean flag), since 0
    /// itself is a meaningful cap value, not just a placeholder for "forever."
    @Published var userDailyCapCents: Int? {
        didSet {
            if let userDailyCapCents {
                UserDefaults.standard.set(userDailyCapCents, forKey: Self.userDailyCapCentsKey)
            } else {
                UserDefaults.standard.removeObject(forKey: Self.userDailyCapCentsKey)
            }
            SettingsFieldTimestamps.touch("routing.userDailyCapCents")
        }
    }
    @Published var globalDailyCapCents: Int? {
        didSet {
            if let globalDailyCapCents {
                UserDefaults.standard.set(globalDailyCapCents, forKey: Self.globalDailyCapCentsKey)
            } else {
                UserDefaults.standard.removeObject(forKey: Self.globalDailyCapCentsKey)
            }
            SettingsFieldTimestamps.touch("routing.globalDailyCapCents")
        }
    }
    @Published var localOnlyMode: Bool {
        didSet {
            UserDefaults.standard.set(localOnlyMode, forKey: Self.localOnlyModeKey)
            SettingsFieldTimestamps.touch("routing.localOnlyMode")
        }
    }
    /// Light tasks (titles, topic names, profile suggestions) use the cheapest model the
    /// user has. On by default, matching core
    /// (docs/superpowers/specs/2026-10-29-cheap-first-routing-and-key-on-ramp-design.md).
    @Published var cheapFirst: Bool {
        didSet {
            UserDefaults.standard.set(cheapFirst, forKey: Self.cheapFirstKey)
            SettingsFieldTimestamps.touch("routing.cheapFirst")
        }
    }
    /// Whether Ollama is a link in the chain at all. Off saves a refused connection on
    /// every call for a Mac without Ollama; ignored in local-only mode. On by default.
    @Published var useLocalModel: Bool {
        didSet {
            UserDefaults.standard.set(useLocalModel, forKey: Self.useLocalModelKey)
            SettingsFieldTimestamps.touch("routing.useLocalModel")
        }
    }
    /// FR-NOTES-1: a global standing-rules text folded into every chat, daily-digest,
    /// and meeting-summary prompt (core/llm's answer_with_citations,
    /// generate_daily_digest/generate_daily_digest_from_context, and
    /// summarize_whole_transcript/summarize_chunk/reduce_chunk_summaries) — see
    /// docs/superpowers/specs/2026-08-17-custom-instructions-everywhere-design.md.
    /// Originally scoped to meeting summaries only (see
    /// docs/superpowers/specs/2026-08-06-custom-meeting-summary-prompt-design.md);
    /// widened to apply everywhere as of that later spec. Empty string (the default)
    /// is a no-op on the Rust side — the built prompt is byte-identical to before this
    /// setting existed. Existing users who had set meeting-specific instructions will
    /// now see the same text also shape chat answers and daily digests.
    @Published var customPromptInstructions: String {
        didSet {
            UserDefaults.standard.set(customPromptInstructions, forKey: Self.customPromptInstructionsKey)
            SettingsFieldTimestamps.touch("summary.customPromptInstructions")
        }
    }

    private static let anthropicKeySyncEnabledKey = "com.mpointer.kestrel.apiKeyAnthropicSyncEnabled"
    private static let openaiKeySyncEnabledKey = "com.mpointer.kestrel.apiKeyOpenAISyncEnabled"
    private static let googleKeySyncEnabledKey = "com.mpointer.kestrel.apiKeyGoogleSyncEnabled"
    private static let openrouterKeySyncEnabledKey = "com.mpointer.kestrel.apiKeyOpenRouterSyncEnabled"

    /// Off by default (docs/superpowers/specs/2026-07-30-sync-data-layer-design.md):
    /// the Anthropic key value only ever crosses sync while this is true.
    /// Turning it off pushes a tombstone that deletes any previously-synced
    /// value from every other device.
    @Published var anthropicKeySyncEnabled: Bool {
        didSet {
            UserDefaults.standard.set(anthropicKeySyncEnabled, forKey: Self.anthropicKeySyncEnabledKey)
            SettingsFieldTimestamps.touch("apiKey.anthropic.syncEnabled")
        }
    }
    @Published var openaiKeySyncEnabled: Bool {
        didSet {
            UserDefaults.standard.set(openaiKeySyncEnabled, forKey: Self.openaiKeySyncEnabledKey)
            SettingsFieldTimestamps.touch("apiKey.openai.syncEnabled")
        }
    }
    @Published var googleKeySyncEnabled: Bool {
        didSet {
            UserDefaults.standard.set(googleKeySyncEnabled, forKey: Self.googleKeySyncEnabledKey)
            SettingsFieldTimestamps.touch("apiKey.google.syncEnabled")
        }
    }
    @Published var openrouterKeySyncEnabled: Bool {
        didSet {
            UserDefaults.standard.set(openrouterKeySyncEnabled, forKey: Self.openrouterKeySyncEnabledKey)
            SettingsFieldTimestamps.touch("apiKey.openrouter.syncEnabled")
        }
    }

    /// Live-fetched from Anthropic's/OpenAI's own model-listing APIs, and from Ollama's
    /// local `/api/tags` (`HostedModelsService`) rather than hardcoded — model lineups
    /// change too often to pin in source. Empty until a successful fetch; not persisted,
    /// since a stale cached list would defeat the point of fetching live in the first
    /// place. Ollama needs no API key (it's a local, unauthenticated server), so its
    /// list is fetched unconditionally rather than gated on a saved key.
    @Published var ollamaModels: [String] = []
    @Published var anthropicModels: [String] = []
    @Published var openaiModels: [String] = []
    @Published var googleModels: [String] = []
    @Published var openrouterModels: [String] = []
    @Published var isLoadingOllamaModels = false
    @Published var isLoadingAnthropicModels = false
    @Published var isLoadingOpenAIModels = false
    @Published var isLoadingGoogleModels = false
    @Published var isLoadingOpenRouterModels = false
    @Published var ollamaModelsError: String?
    @Published var anthropicModelsError: String?
    @Published var openaiModelsError: String?
    @Published var googleModelsError: String?
    @Published var openrouterModelsError: String?

    init() {
        let defaults = UserDefaults.standard
        let storedProvider = defaults.string(forKey: Self.preferredProviderKey).flatMap(Provider.init(rawValue:))
        preferredProvider = storedProvider ?? .ollama
        ollamaDefaultModel = defaults.string(forKey: Self.ollamaDefaultModelKey) ?? "gemma4:12b"
        // Bug fixed 2026-08-06: these two used to default to "" until the user picked a
        // model from Settings' fetched-model dropdown. An empty model name reaches
        // core/ffi's build_chain() unchanged and gets sent as `"model": ""` in the real
        // API request — both Anthropic and OpenAI correctly reject that with a 400
        // ("model: String should have at least 1 character"), which the provider chain's
        // generic failure handling then shows as the misleading "Couldn't reach the AI
        // provider" even though the API key itself is perfectly valid. Fallback values
        // match build_chain()'s own FALLBACK_ANTHROPIC_MODEL/FALLBACK_OPENAI_MODEL
        // constants (core/ffi/src/lib.rs) exactly, so a never-configured default behaves
        // the same as the Rust-side fallback an explicit-but-wrong-provider pick already uses.
        anthropicDefaultModel = defaults.string(forKey: Self.anthropicDefaultModelKey) ?? "claude-haiku-4-5"
        openaiDefaultModel = defaults.string(forKey: Self.openaiDefaultModelKey) ?? "gpt-4o-mini"
        googleDefaultModel = defaults.string(forKey: Self.googleDefaultModelKey) ?? "gemini-2.5-flash"
        openrouterDefaultModel = defaults.string(forKey: Self.openrouterDefaultModelKey) ?? ""
        openrouterPrices = defaults.data(forKey: Self.openrouterPricesKey)
            .flatMap { try? JSONDecoder().decode([String: CoreRoutingConfig.Price].self, from: $0) } ?? [:]
        userDailyCapCents = defaults.object(forKey: Self.userDailyCapCentsKey) as? Int
        globalDailyCapCents = defaults.object(forKey: Self.globalDailyCapCentsKey) as? Int
        localOnlyMode = defaults.bool(forKey: Self.localOnlyModeKey)
        // `bool(forKey:)` reads an unset key as false; these two default to true.
        cheapFirst = defaults.object(forKey: Self.cheapFirstKey) as? Bool ?? true
        useLocalModel = defaults.object(forKey: Self.useLocalModelKey) as? Bool ?? true
        customPromptInstructions = defaults.string(forKey: Self.customPromptInstructionsKey) ?? ""
        anthropicKeySyncEnabled = defaults.bool(forKey: Self.anthropicKeySyncEnabledKey)
        openaiKeySyncEnabled = defaults.bool(forKey: Self.openaiKeySyncEnabledKey)
        googleKeySyncEnabled = defaults.bool(forKey: Self.googleKeySyncEnabledKey)
        openrouterKeySyncEnabled = defaults.bool(forKey: Self.openrouterKeySyncEnabledKey)
    }

    var hasAnthropicKey: Bool { KeychainService.exists(for: .anthropic) }
    var hasOpenAIKey: Bool { KeychainService.exists(for: .openai) }
    var hasGoogleKey: Bool { KeychainService.exists(for: .google) }
    var hasOpenRouterKey: Bool { KeychainService.exists(for: .openrouter) }
    /// Whether any hosted provider has a key: with none and no local model, the AI
    /// screen shows the on-ramp instead of silently falling back.
    var hasAnyHostedKey: Bool { hasAnthropicKey || hasOpenAIKey || hasGoogleKey || hasOpenRouterKey }

    /// Whether `provider` can be called right now: Ollama always, a hosted provider
    /// with its key saved, OpenRouter only once a model is picked too.
    func hasKey(for provider: Provider) -> Bool {
        switch provider {
        case .ollama: return true
        case .anthropic: return hasAnthropicKey
        case .openai: return hasOpenAIKey
        case .google: return hasGoogleKey
        case .openrouter: return hasOpenRouterKey
        }
    }

    /// Keys are trimmed before they are saved: a pasted key often carries a trailing
    /// newline, which every provider then rejects.
    nonisolated static func trimmed(_ key: String) -> String {
        key.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    @discardableResult
    func saveAnthropicKey(_ key: String) -> Bool {
        let saved = KeychainService.save(Self.trimmed(key), for: .anthropic)
        if saved {
            SettingsFieldTimestamps.touch("apiKey.anthropic")
            objectWillChange.send()
            Task { await refreshAnthropicModels() }
        }
        return saved
    }

    @discardableResult
    func saveGoogleKey(_ key: String) -> Bool {
        let saved = KeychainService.save(Self.trimmed(key), for: .google)
        if saved {
            SettingsFieldTimestamps.touch("apiKey.google")
            objectWillChange.send()
            Task { await refreshGoogleModels() }
        }
        return saved
    }

    @discardableResult
    func saveOpenAIKey(_ key: String) -> Bool {
        let saved = KeychainService.save(Self.trimmed(key), for: .openai)
        if saved {
            SettingsFieldTimestamps.touch("apiKey.openai")
            objectWillChange.send()
            Task { await refreshOpenAIModels() }
        }
        return saved
    }

    @discardableResult
    func saveOpenRouterKey(_ key: String) -> Bool {
        let saved = KeychainService.save(Self.trimmed(key), for: .openrouter)
        if saved {
            SettingsFieldTimestamps.touch("apiKey.openrouter")
            objectWillChange.send()
            Task { await refreshOpenRouterModels() }
        }
        return saved
    }

    func clearOpenRouterKey() {
        KeychainService.delete(for: .openrouter)
        SettingsFieldTimestamps.touch("apiKey.openrouter")
        openrouterModels = []
        openrouterModelsError = nil
        objectWillChange.send()
    }

    /// Fetches OpenRouter's catalogue via `HostedModelsService`, using the Keychain-stored
    /// key. No-op (clears the list) if no key is saved. Prices are cached by
    /// `checkSavedKey`, which is what a save runs.
    func refreshOpenRouterModels() async {
        guard let key = KeychainService.load(for: .openrouter) else {
            openrouterModels = []
            openrouterModelsError = nil
            return
        }
        isLoadingOpenRouterModels = true
        openrouterModelsError = nil
        defer { isLoadingOpenRouterModels = false }
        do {
            openrouterModels = try await HostedModelsService.fetchOpenRouterModels(apiKey: key)
        } catch {
            openrouterModelsError = error.localizedDescription
        }
    }

    /// Checks the saved key for `provider` against its model list, off the main actor,
    /// and fills that provider's model list on success (and, for OpenRouter, the price
    /// cache). Nil when no key is saved.
    func checkSavedKey(_ provider: Provider) async -> CoreKeyCheck? {
        guard let keychain = provider.keychain, let key = KeychainService.load(for: keychain) else { return nil }
        let check = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.keyCheck(provider: provider.rawValue, apiKey: key)
        }.value
        if case let .valid(models, prices)? = check {
            switch provider {
            case .anthropic: anthropicModels = models; anthropicModelsError = nil
            case .openai: openaiModels = models; openaiModelsError = nil
            case .google: googleModels = models; googleModelsError = nil
            case .openrouter:
                openrouterModels = models
                openrouterModelsError = nil
                if !prices.isEmpty {
                    openrouterPrices = Dictionary(uniqueKeysWithValues: prices.map { ($0.id, CoreRoutingConfig.Price(in: $0.inCentsPer1k, out: $0.outCentsPer1k)) })
                }
            case .ollama: break
            }
        }
        return check
    }

    func clearAnthropicKey() {
        KeychainService.delete(for: .anthropic)
        SettingsFieldTimestamps.touch("apiKey.anthropic")
        anthropicModels = []
        anthropicModelsError = nil
        objectWillChange.send()
    }

    func clearOpenAIKey() {
        KeychainService.delete(for: .openai)
        SettingsFieldTimestamps.touch("apiKey.openai")
        openaiModels = []
        openaiModelsError = nil
        objectWillChange.send()
    }

    func clearGoogleKey() {
        KeychainService.delete(for: .google)
        SettingsFieldTimestamps.touch("apiKey.google")
        googleModels = []
        googleModelsError = nil
        objectWillChange.send()
    }

    /// Fetches Ollama's locally-installed model list via `HostedModelsService`. No key
    /// needed; fails gracefully (leaving `ollamaModels` empty, setting
    /// `ollamaModelsError`) if Ollama isn't running — callers should fall back to
    /// free-text model entry in that case.
    func refreshOllamaModels() async {
        isLoadingOllamaModels = true
        ollamaModelsError = nil
        defer { isLoadingOllamaModels = false }
        do {
            ollamaModels = try await HostedModelsService.fetchOllamaModels()
        } catch {
            ollamaModelsError = error.localizedDescription
        }
    }

    /// Fetches Anthropic's current model list via `HostedModelsService`, using the
    /// Keychain-stored key. No-op (clears the list) if no key is saved.
    func refreshAnthropicModels() async {
        guard let key = KeychainService.load(for: .anthropic) else {
            anthropicModels = []
            anthropicModelsError = nil
            return
        }
        isLoadingAnthropicModels = true
        anthropicModelsError = nil
        defer { isLoadingAnthropicModels = false }
        do {
            anthropicModels = try await HostedModelsService.fetchAnthropicModels(apiKey: key)
        } catch {
            anthropicModelsError = error.localizedDescription
        }
    }

    /// Fetches OpenAI's current model list via `HostedModelsService`, using the
    /// Keychain-stored key. No-op (clears the list) if no key is saved.
    func refreshOpenAIModels() async {
        guard let key = KeychainService.load(for: .openai) else {
            openaiModels = []
            openaiModelsError = nil
            return
        }
        isLoadingOpenAIModels = true
        openaiModelsError = nil
        defer { isLoadingOpenAIModels = false }
        do {
            openaiModels = try await HostedModelsService.fetchOpenAIModels(apiKey: key)
        } catch {
            openaiModelsError = error.localizedDescription
        }
    }

    /// Fetches Google's current model list via `HostedModelsService`, using the
    /// Keychain-stored key. No-op (clears the list) if no key is saved.
    func refreshGoogleModels() async {
        guard let key = KeychainService.load(for: .google) else {
            googleModels = []
            googleModelsError = nil
            return
        }
        isLoadingGoogleModels = true
        googleModelsError = nil
        defer { isLoadingGoogleModels = false }
        do {
            googleModels = try await HostedModelsService.fetchGoogleModels(apiKey: key)
        } catch {
            googleModelsError = error.localizedDescription
        }
    }

    /// Builds the `CoreRoutingConfig` sent with one `askQuestion`/`dailyDigest` call.
    /// Key/cap/sensitive fields come from these persisted settings; `preferredProvider`/
    /// `preferredModel` are passed in since Timeline's per-conversation picker (an
    /// explicit per-conversation pick becomes the chain's first link) can differ from
    /// these settings' own "default provider/model" fields. Reads keys fresh from
    /// Keychain on every call rather than caching them in a `@Published` property, so a
    /// saved secret doesn't linger in memory longer than a single call needs it.
    /// Resolves the provider/model defaults a picker should seed its `@State` from on
    /// appear — pulled out of `TimelineView`'s and `MeetingDetailView`'s near-identical
    /// `.onAppear` blocks so the two screens don't each hardcode the same
    /// provider-to-default-model switch. `hostedModel` is only meaningful when
    /// `provider` isn't `.ollama`; when it is, callers ignore `hostedModel` and use
    /// `ollamaModel` instead (there's no single "hosted default" when Ollama is the
    /// preferred provider, so this just falls back to `ollamaDefaultModel` — a value
    /// callers won't actually display in that case).
    func defaultSelection() -> (provider: Provider, ollamaModel: String, hostedModel: String) {
        let hostedModel: String
        switch preferredProvider {
        case .anthropic: hostedModel = anthropicDefaultModel
        case .openai: hostedModel = openaiDefaultModel
        case .google: hostedModel = googleDefaultModel
        case .openrouter: hostedModel = openrouterDefaultModel
        case .ollama: hostedModel = ollamaDefaultModel // arbitrary fallback, never actually shown for .ollama
        }
        return (preferredProvider, ollamaDefaultModel, hostedModel)
    }

    /// The user's Profile text, filled by `AppState` from `ProfileSettings` (empty when off).
    var profileProvider: () -> String = { "" }

    func routingConfig(
        preferredProvider: Provider,
        preferredModel: String,
        customInstructions: String = "",
        dailyDigestInstructions: String = "",
        weeklyDigestInstructions: String = "",
        typesafeKey: String? = nil
    ) -> CoreRoutingConfig {
        CoreRoutingConfig(
            anthropicKey: KeychainService.load(for: .anthropic),
            openaiKey: KeychainService.load(for: .openai),
            googleKey: KeychainService.load(for: .google),
            typesafeKey: typesafeKey,
            userDailyCapCents: userDailyCapCents,
            globalDailyCapCents: globalDailyCapCents,
            sensitive: localOnlyMode,
            preferredProvider: preferredProvider.rawValue,
            preferredModel: preferredModel,
            customInstructions: customInstructions,
            dailyDigestInstructions: dailyDigestInstructions,
            weeklyDigestInstructions: weeklyDigestInstructions,
            profile: profileProvider(),
            cheapFirst: cheapFirst,
            useLocalModel: useLocalModel,
            openrouterKey: KeychainService.load(for: .openrouter),
            openrouterModel: openrouterDefaultModel,
            openrouterPrices: openrouterPrices
        )
    }
}
