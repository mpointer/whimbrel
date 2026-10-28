import Combine
import Foundation

/// Orchestrates settings sync: gathers the current value + timestamp for
/// every syncable field across `RoutingSettings`/`RetentionSettings`/
/// `CaptureExclusions`/the Keychain-backed API keys, hands them to the core for
/// one push/pull round trip (`CoreMemoryStore.syncSettingsIteration`, which owns the
/// push rules — notably the API-key tombstone timestamp rule), asks the core which
/// pulled fields to apply (`syncSettingsApplyPlan`: newer-than-local gate, API-key
/// toggle gate, hidden-episode partition) and applies them. This is the one place
/// that knows the mapping between a `field_key` string and each store's real typed
/// property — platform-bound (UserDefaults/Keychain), so it stays in Swift.
///
/// Runs only while the account is enrolled (`SyncState.status ==
/// .ready`) — before that there's no Turso token or group key to sync
/// with. Periodic only (docs/superpowers/plans/2026-07-30-sync-settings-data-layer.md):
/// no immediate push on every individual settings change, just a timer.
/// Settings changes propagate within one polling interval, which is fine
/// for values that aren't time-critical the way device-approval polling
/// is.
@MainActor
final class SettingsSyncCoordinator: ObservableObject {
    @Published private(set) var lastSyncError: String?
    @Published private(set) var lastSyncedAt: Date?

    private let routing: RoutingSettings
    private let retention: RetentionSettings
    private let promptLibrary: PromptLibrarySettings
    private let profile: ProfileSettings
    private let routineHost: RoutineHostSettings
    private let syncState: SyncState
    /// Source of the `episode.hidden.*` fields (cross-device hidden-episode marks).
    /// The core owns their timestamps and last-writer-wins, so they bypass
    /// `SettingsFieldTimestamps` entirely.
    private let coreStore: CoreMemoryStore?

    private var periodicTask: Task<Void, Never>?
    private var statusSubscription: AnyCancellable?

    static let syncIntervalSeconds: UInt64 = 60

    init(routing: RoutingSettings, retention: RetentionSettings, promptLibrary: PromptLibrarySettings, profile: ProfileSettings, routineHost: RoutineHostSettings, syncState: SyncState, coreStore: CoreMemoryStore? = nil) {
        self.routing = routing
        self.retention = retention
        self.promptLibrary = promptLibrary
        self.profile = profile
        self.routineHost = routineHost
        self.syncState = syncState
        self.coreStore = coreStore
    }

    /// Starts (or stops) the periodic timer automatically as `syncState`
    /// enters/leaves `.ready`. Call once, e.g. from `AppState.init()`.
    func observeSyncState() {
        statusSubscription = syncState.$status
            .sink { [weak self] status in
                guard let self else { return }
                if case .ready = status {
                    self.startPeriodicSync()
                } else {
                    self.stopPeriodicSync()
                }
            }
    }

    func startPeriodicSync() {
        guard periodicTask == nil else { return }
        periodicTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.syncNow()
                try? await Task.sleep(nanoseconds: Self.syncIntervalSeconds * 1_000_000_000)
            }
        }
    }

    func stopPeriodicSync() {
        periodicTask?.cancel()
        periodicTask = nil
    }

    /// One push/pull round trip. Safe to call even when not enrolled —
    /// just returns early. Errors are recorded in `lastSyncError`, never
    /// thrown — a failed settings sync shouldn't interrupt anything else
    /// in the app.
    func syncNow() async {
        guard case let .ready(tursoToken, tursoDatabaseUrl) = syncState.status else { return }
        // `SyncKeychain.loadGroupKey()` is `throws -> String?`; `try?` on a
        // function that already returns an Optional flattens to that same
        // Optional (SE-0230) rather than double-wrapping — one `guard let`
        // here correctly handles both "threw" and "returned nil".
        guard let groupKeyBase64 = try? SyncKeychain.loadGroupKey() else {
            lastSyncError = "No group key available yet."
            return
        }

        var local = gatherLocalSettings()
        // Store reads happen on accessQueue and are awaited, never `sync`'d from the
        // main actor: a long store job already in flight (reindex, segmentation) would
        // otherwise freeze the UI for its whole duration.
        if let coreStore {
            local.hiddenEpisodeMarks = await Self.hiddenEpisodeFields(from: coreStore)
            for (key, value) in RoutineSyncFields.values(from: await Self.routines(from: coreStore)) {
                add(&local.fields, key, value)
            }
        }
        // The core call blocks on a real network round trip to Turso. This class is
        // `@MainActor`, so `Task.detached` runs it off the main actor; awaiting `.value`
        // suspends this function (freeing the main actor) until it finishes, then
        // resumes here so the apply below is main-actor safe.
        let iteration = await Task.detached(priority: .utility) {
            CoreMemoryStore.syncSettingsIteration(
                tursoURL: tursoDatabaseUrl,
                tursoToken: tursoToken,
                groupKeyBase64: groupKeyBase64,
                local: local
            )
        }.value
        guard let iteration else {
            lastSyncError = "Settings sync failed. Will retry automatically."
            return
        }

        applyDelta(iteration.pulled)
        lastSyncError = nil
        lastSyncedAt = Date()
    }

    // MARK: - Gathering local state for push

    private func gatherLocalSettings() -> CoreMemoryStore.SyncLocalSettings {
        var fields: [String: CoreMemoryStore.SyncLocalField] = [:]

        add(&fields, "routing.preferredProvider", routing.preferredProvider.rawValue)
        add(&fields, "routing.ollamaDefaultModel", routing.ollamaDefaultModel)
        add(&fields, "routing.anthropicDefaultModel", routing.anthropicDefaultModel)
        add(&fields, "routing.openaiDefaultModel", routing.openaiDefaultModel)
        add(&fields, "routing.googleDefaultModel", routing.googleDefaultModel)
        add(&fields, "routing.userDailyCapCents", routing.userDailyCapCents.map(String.init) ?? "")
        add(&fields, "routing.globalDailyCapCents", routing.globalDailyCapCents.map(String.init) ?? "")
        add(&fields, "routing.localOnlyMode", routing.localOnlyMode ? "true" : "false")
        add(&fields, "summary.customPromptInstructions", routing.customPromptInstructions)

        for surface in PromptSurface.allCases {
            add(&fields, PromptLibrarySettings.fieldKey(for: surface), promptLibrary.instructions[surface] ?? "")
        }

        for (key, value) in profile.syncValues() {
            add(&fields, key, value)
        }
        add(&fields, RoutineHostSettings.fieldKey, routineHost.primaryDeviceId)

        add(&fields, "retention.captureItemDays", String(retention.captureItemDays))
        add(&fields, "retention.captureItemKeepForever", retention.captureItemKeepForever ? "true" : "false")
        add(&fields, "retention.meetingDays", String(retention.meetingDays))
        add(&fields, "retention.meetingKeepForever", retention.meetingKeepForever ? "true" : "false")

        let excludedJSON = (try? JSONEncoder().encode(CaptureExclusions.shared.excludedBundleIds.sorted()))
            .flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
        add(&fields, "captureExclusions.excludedBundleIds", excludedJSON)

        add(&fields, "apiKey.anthropic.syncEnabled", routing.anthropicKeySyncEnabled ? "true" : "false")
        add(&fields, "apiKey.openai.syncEnabled", routing.openaiKeySyncEnabled ? "true" : "false")
        add(&fields, "apiKey.google.syncEnabled", routing.googleKeySyncEnabled ? "true" : "false")

        var local = CoreMemoryStore.SyncLocalSettings()
        local.fields = fields
        local.apiKeys = [
            apiKeyState(fieldKey: "apiKey.anthropic", provider: .anthropic, syncEnabled: routing.anthropicKeySyncEnabled),
            apiKeyState(fieldKey: "apiKey.openai", provider: .openai, syncEnabled: routing.openaiKeySyncEnabled),
            apiKeyState(fieldKey: "apiKey.google", provider: .google, syncEnabled: routing.googleKeySyncEnabled),
        ]
        return local
    }

    private static func routines(from store: CoreMemoryStore) async -> [CoreRoutine] {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async {
                continuation.resume(returning: store.listRoutines() ?? [])
            }
        }
    }

    private static func hiddenEpisodeFields(from store: CoreMemoryStore) async -> [String: CoreMemoryStore.SyncSettingsField] {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async {
                continuation.resume(returning: store.hiddenEpisodeSyncFields())
            }
        }
    }

    private func add(_ fields: inout [String: CoreMemoryStore.SyncLocalField], _ fieldKey: String, _ value: String) {
        fields[fieldKey] = CoreMemoryStore.SyncLocalField(
            value: value,
            updatedAtUnixMs: SettingsFieldTimestamps.timestamp(for: fieldKey)
        )
    }

    /// The Keychain value is only read while the toggle is on; what gets pushed (real
    /// key, tombstone, or nothing) and with which timestamp is the core's decision.
    private func apiKeyState(fieldKey: String, provider: KeychainService.Provider, syncEnabled: Bool) -> CoreMemoryStore.SyncApiKeyState {
        CoreMemoryStore.SyncApiKeyState(
            fieldKey: fieldKey,
            syncEnabled: syncEnabled,
            key: syncEnabled ? KeychainService.load(for: provider) : nil,
            keyUpdatedAtUnixMs: SettingsFieldTimestamps.timestamp(for: fieldKey),
            toggleUpdatedAtUnixMs: SettingsFieldTimestamps.timestamp(for: fieldKey + ".syncEnabled")
        )
    }

    // MARK: - Applying a pulled delta

    private func applyDelta(_ pulled: [String: CoreMemoryStore.SyncSettingsField]) {
        // Timestamps are re-read AFTER the round trip: `gatherLocalSettings()` snapshotted
        // local state before the network wait, and a local edit that landed meanwhile
        // must not be overwritten by a delta computed from the older snapshot.
        var localTimestamps: [String: Int64] = [:]
        for fieldKey in pulled.keys {
            localTimestamps[fieldKey] = SettingsFieldTimestamps.timestamp(for: fieldKey)
        }
        let toggles: [String: Bool] = [
            "apiKey.anthropic": routing.anthropicKeySyncEnabled,
            "apiKey.openai": routing.openaiKeySyncEnabled,
            "apiKey.google": routing.googleKeySyncEnabled,
        ]
        guard let plan = CoreMemoryStore.syncSettingsApplyPlan(pulled: pulled, localTimestamps: localTimestamps, apiKeySyncEnabled: toggles) else {
            return
        }
        // Hidden-episode marks go to the core in one batch: it keeps their timestamps
        // and resolves last-writer-wins itself, so they never touch
        // `SettingsFieldTimestamps`. A remote hide shows on the next episodes-tab load.
        if !plan.hiddenEpisodeMarks.isEmpty, let coreStore {
            let marks = plan.hiddenEpisodeMarks
            CoreMemoryStore.accessQueue.async { coreStore.applyHiddenEpisodeSyncFields(marks) }
        }
        var routineFields: [CoreMemoryStore.SyncPulledField] = []
        for pulledField in plan.fields {
            let field = CoreMemoryStore.SyncSettingsField(value: pulledField.value, updatedAtUnixMs: pulledField.updatedAtUnixMs, isDeleted: pulledField.isDeleted)
            // `apply` reports whether it actually changed local state. Several branches
            // silently no-op (unparseable Int, unknown Provider raw value, malformed
            // capture-exclusions JSON, unrecognized field key) — then the local timestamp
            // must NOT be bumped, or this device would wrongly believe it's in sync on
            // that field and never re-pull the correct value.
            if RoutineSyncFields.isRoutineKey(pulledField.fieldKey) {
                // Written to the core store off the main actor; its timestamp is set only if it applied.
                if !pulledField.isDeleted { routineFields.append(pulledField) }
                continue
            }
            guard apply(fieldKey: pulledField.fieldKey, field: field) else { continue }
            // The pulled value's own timestamp becomes the new local timestamp — NOT
            // "now" — so the very next sync doesn't push it right back. Must run AFTER
            // the property assignment, whose own `didSet` touched the timestamp to "now".
            SettingsFieldTimestamps.setTimestamp(pulledField.updatedAtUnixMs, for: pulledField.fieldKey)
        }
        applyRoutineFields(routineFields)
    }

    /// Writes pulled routine on/off and schedule values into the local routine rows, off the main actor.
    /// A field is stamped as synced only if it was actually applied; a value that is malformed or has
    /// no routine of that template yet stays unstamped, so it is pulled again next sync.
    private func applyRoutineFields(_ fields: [CoreMemoryStore.SyncPulledField]) {
        guard !fields.isEmpty, let coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let routines = coreStore.listRoutines() ?? []
            var applied: [CoreMemoryStore.SyncPulledField] = []
            for field in fields {
                guard let change = RoutineSyncFields.change(key: field.fieldKey, value: field.value, routines: routines) else { continue }
                let ok: Bool
                switch change {
                case let .enabled(id, enabled): ok = coreStore.setRoutineEnabled(id: id, enabled: enabled)
                case let .schedule(id, hhmm): ok = coreStore.updateRoutineSchedule(id: id, scheduleHhmm: hhmm)
                }
                if ok { applied.append(field) }
            }
            DispatchQueue.main.async {
                for field in applied { SettingsFieldTimestamps.setTimestamp(field.updatedAtUnixMs, for: field.fieldKey) }
            }
        }
    }

    /// Applies one pulled field to local state. Returns `true` only when a
    /// value was actually assigned — `false` for every no-op path (see
    /// `applyDelta` for why that distinction matters).
    @discardableResult
    private func apply(fieldKey: String, field: CoreMemoryStore.SyncSettingsField) -> Bool {
        switch fieldKey {
        case "routing.preferredProvider":
            guard let provider = RoutingSettings.Provider(rawValue: field.value) else { return false }
            routing.preferredProvider = provider
        case "routing.ollamaDefaultModel":
            routing.ollamaDefaultModel = field.value
        case "routing.anthropicDefaultModel":
            routing.anthropicDefaultModel = field.value
        case "routing.openaiDefaultModel":
            routing.openaiDefaultModel = field.value
        case "routing.googleDefaultModel":
            routing.googleDefaultModel = field.value
        case "routing.userDailyCapCents":
            if field.value.isEmpty {
                routing.userDailyCapCents = nil
            } else {
                guard let cents = Int(field.value) else { return false }
                routing.userDailyCapCents = cents
            }
        case "routing.globalDailyCapCents":
            if field.value.isEmpty {
                routing.globalDailyCapCents = nil
            } else {
                guard let cents = Int(field.value) else { return false }
                routing.globalDailyCapCents = cents
            }
        case "routing.localOnlyMode":
            guard field.value == "true" || field.value == "false" else { return false }
            routing.localOnlyMode = field.value == "true"
        case "summary.customPromptInstructions":
            routing.customPromptInstructions = field.value
        case "retention.captureItemDays":
            guard let days = Int(field.value) else { return false }
            retention.captureItemDays = days
        case "retention.captureItemKeepForever":
            guard field.value == "true" || field.value == "false" else { return false }
            retention.captureItemKeepForever = field.value == "true"
        case "retention.meetingDays":
            guard let days = Int(field.value) else { return false }
            retention.meetingDays = days
        case "retention.meetingKeepForever":
            guard field.value == "true" || field.value == "false" else { return false }
            retention.meetingKeepForever = field.value == "true"
        case "captureExclusions.excludedBundleIds":
            return applyCaptureExclusions(field.value)
        case "apiKey.anthropic.syncEnabled":
            routing.anthropicKeySyncEnabled = field.value == "true"
        case "apiKey.openai.syncEnabled":
            routing.openaiKeySyncEnabled = field.value == "true"
        case "apiKey.google.syncEnabled":
            routing.googleKeySyncEnabled = field.value == "true"
        case "apiKey.anthropic":
            return applyApiKey(field, provider: .anthropic)
        case "apiKey.openai":
            return applyApiKey(field, provider: .openai)
        case "apiKey.google":
            return applyApiKey(field, provider: .google)
        default:
            if fieldKey == RoutineHostSettings.fieldKey {
                return routineHost.apply(fieldKey: fieldKey, value: field.value)
            }
            if fieldKey.hasPrefix("profile.") {
                return profile.apply(fieldKey: fieldKey, value: field.value)
            }
            guard let surface = PromptSurface.allCases.first(where: { PromptLibrarySettings.fieldKey(for: $0) == fieldKey }) else {
                return false
            }
            promptLibrary.instructions[surface] = field.value
        }
        return true
    }

    @discardableResult
    private func applyCaptureExclusions(_ json: String) -> Bool {
        guard let data = json.data(using: .utf8),
              let bundleIds = try? JSONDecoder().decode([String].self, from: data) else { return false }
        let current = CaptureExclusions.shared.excludedBundleIds
        for id in current where !bundleIds.contains(id) {
            CaptureExclusions.shared.setExcluded(false, for: id)
        }
        for id in bundleIds where !current.contains(id) {
            CaptureExclusions.shared.setExcluded(true, for: id)
        }
        return true
    }

    /// Routes through `RoutingSettings`'s own key-management methods (rather than
    /// `KeychainService` directly) so a pulled key change gets the same
    /// `objectWillChange.send()` + model-list refresh a locally-typed key gets. The
    /// core's apply plan has already applied the toggle gate (a key is neither saved
    /// nor deleted while this device's own sync toggle is off) and dropped malformed
    /// empty non-deleted keys; `false` here means only that the Keychain save failed.
    private func applyApiKey(_ field: CoreMemoryStore.SyncSettingsField, provider: KeychainService.Provider) -> Bool {
        if field.isDeleted {
            switch provider {
            case .anthropic: routing.clearAnthropicKey()
            case .openai: routing.clearOpenAIKey()
            case .google: routing.clearGoogleKey()
            case .typesafe, .imageSearch: break // local to this Mac, never synced
            }
            return true
        }
        guard !field.value.isEmpty else { return false }
        switch provider {
        case .anthropic: return routing.saveAnthropicKey(field.value)
        case .openai: return routing.saveOpenAIKey(field.value)
        case .google: return routing.saveGoogleKey(field.value)
        case .typesafe, .imageSearch: return false // local to this Mac, never synced
        }
    }
}
