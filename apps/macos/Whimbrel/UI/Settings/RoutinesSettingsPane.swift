import SwiftUI

/// FR-RTN-1..4: lets the user enable/disable a routine (e.g. the built-in Daily
/// Brief) and pick its HH:MM schedule. Distinct from `RoutinesInboxView` (Task 7),
/// which browses past routine *output* — this pane only touches the routine's
/// config (`whimbrel_routines_list`/`_set_enabled`/`_set_schedule`, Task 8).
struct RoutinesSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var routines: [CoreRoutine] = []
    /// Each routine's recent runs (its own and synced ones), for the cloud routines' "Updated" and "Late" lines.
    @State private var runs: [Int64: [CloudRunStamp]] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Routines")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Scheduled automations that run on their own and deliver a notification when done.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            Divider().overlay(Color.whimbrelBorder)

            runsOnPicker

            Divider().overlay(Color.whimbrelBorder)

            // Away Mode's row only owns the organizer drafts it files; it has no schedule or switch.
            ForEach(routines.filter { $0.template != "away_follow_up" }) { routine in
                VStack(alignment: .leading, spacing: Spacing.sm) {
                    Toggle(routine.name, isOn: Binding(
                        get: { routine.enabled },
                        set: { newValue in setEnabled(routine, newValue) }
                    ))
                    .toggleStyle(.switch)

                    // Only the write-capable templates have a write step for standing
                    // authorization to apply to. Daily Brief is read-only, so the toggle
                    // there would do nothing at all -- offering a security-relevant
                    // permission control that has no effect is worse than omitting it.
                    if Self.performsWriteActions(routine) {
                        Toggle("Auto-approve write actions", isOn: Binding(
                            get: { routine.autoApproveWriteActions },
                            set: { newValue in setAutoApprove(routine, newValue) }
                        ))
                        .toggleStyle(.switch)
                    }

                    if routine.template == "meeting_prep" {
                        // Event-driven: there is no time of day, only how long before a meeting.
                        Stepper(
                            value: Binding(
                                get: { appState.meetingPrepSettings.leadMinutes },
                                set: { appState.meetingPrepSettings.leadMinutes = $0 }
                            ),
                            in: MeetingPrepSettings.range, step: 5
                        ) {
                            Text("Prepare a brief \(Int(appState.meetingPrepSettings.leadMinutes)) minutes before each meeting")
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                        }
                    } else {
                        HStack {
                            Text("Runs daily at")
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                            TextField("HH:MM", text: Binding(
                                get: { routine.scheduleHhmm },
                                set: { newValue in setSchedule(routine, newValue) }
                            ))
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 80)
                        }
                    }

                    if Self.callsAModel(routine) {
                        modelPicker(for: routine)
                    }

                    if let info = CloudRoutineInfo.load(routine.template), CloudRoutineCopy.showsToggle(info),
                       CloudRoutineCopy.showsCloudSection(flag: CloudRoutineCopy.uiFlag(), devices: appState.syncState.devices,
                                                          on: appState.settingsSyncCoordinator.cloudRoutines.isCloud(routine.template)) {
                        let devices = appState.syncState.devices
                        CloudRoutineSection(
                            routine: routine, info: info, cloud: appState.settingsSyncCoordinator.cloudRoutines,
                            routing: appState.routing, runs: runs[routine.id] ?? [],
                            // An empty list has not loaded yet: say nothing about the runner.
                            runnerMissing: CloudRoutineCopy.runnerMissing(devices: devices),
                            syncReady: { if case .ready = appState.syncState.status { return true } else { return false } }()
                        )
                    }
                }
                Divider().overlay(Color.whimbrelBorder)
            }
        }
        .onAppear { refresh() }
        // A result the runner (or another device) made shows once it syncs.
        .onReceive(appState.contentSyncCoordinator.$lastSyncedAt.dropFirst()) { _ in refresh() }
    }

    /// Which model chain this routine's runs use (core `routine.model_pref`): the chain
    /// the user's default provider gives, the cheapest model they have, or one provider's
    /// default model pinned first. The Model usage section in AI settings shows which model
    /// each run used, so a routine set to Cheapest can be judged on a week of output.
    private func modelPicker(for routine: CoreRoutine) -> some View {
        let options = Self.modelPrefOptions(routing: appState.routing)
        let current = options.contains { $0.value == routine.modelPref } ? routine.modelPref : "default"
        return HStack {
            Text("Model")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            Picker("", selection: Binding(
                get: { current },
                set: { newValue in setModelPref(routine, newValue) }
            )) {
                ForEach(options, id: \.value) { Text($0.label).tag($0.value) }
            }
            .labelsHidden()
            .frame(width: 260)
            .accessibilityIdentifier("routine-model-\(routine.template)")
        }
    }

    struct ModelPrefOption: Equatable {
        let value: String
        let label: String
    }

    /// Default, Cheapest, then one entry per provider the user can actually run: Ollama
    /// with its default model, and each hosted provider that has a key, at that
    /// provider's default model.
    @MainActor
    static func modelPrefOptions(routing: RoutingSettings) -> [ModelPrefOption] {
        var options = [
            ModelPrefOption(value: "default", label: "Default (\(routing.preferredProvider.displayName))"),
            ModelPrefOption(value: "cheapest", label: "Cheapest model you have"),
        ]
        // The stored model, never this Mac's in-memory stand-in: a routine's pref syncs.
        if !routing.storedOllamaModel.isEmpty {
            options.append(ModelPrefOption(value: "ollama/\(routing.storedOllamaModel)", label: "Ollama (\(routing.storedOllamaModel))"))
        }
        if routing.hasAnthropicKey {
            options.append(ModelPrefOption(value: "anthropic/\(routing.anthropicDefaultModel)", label: "Anthropic (\(routing.anthropicDefaultModel))"))
        }
        if routing.hasOpenAIKey {
            options.append(ModelPrefOption(value: "openai/\(routing.openaiDefaultModel)", label: "OpenAI (\(routing.openaiDefaultModel))"))
        }
        if routing.hasGoogleKey {
            options.append(ModelPrefOption(value: "google/\(routing.googleDefaultModel)", label: "Google (\(routing.googleDefaultModel))"))
        }
        if routing.hasOpenRouterKey, !routing.openrouterDefaultModel.isEmpty {
            options.append(ModelPrefOption(value: "openrouter/\(routing.openrouterDefaultModel)", label: "OpenRouter (\(routing.openrouterDefaultModel))"))
        }
        return options
    }

    /// Templates whose runs call a model through the routine's own preference. Reply
    /// Priorities makes no model call, Meeting Prep runs through `api::meeting_prep` with
    /// the standard chain, and Away follow-up only files drafts.
    static func callsAModel(_ routine: CoreRoutine) -> Bool {
        routine.template == "daily_brief" || routine.template == "eod_recap" || routine.template == "weekly_digest"
    }

    /// Which device runs routines. Empty = every device runs its own (the default). Finished runs from
    /// the chosen device sync to the others and show in the Routines history.
    private var runsOnPicker: some View {
        // Whimbrel Cloud runs only the routines set to run in the cloud; it is never the device routines run on.
        let approved = appState.syncState.devices.filter { $0.status == "approved" && !CloudRoutineCopy.isRunner($0) }
        let choice = appState.routineHost.primaryDeviceId
        return VStack(alignment: .leading, spacing: Spacing.xs) {
            Picker("Routines run on", selection: Binding(
                get: { appState.routineHost.primaryDeviceId },
                set: { appState.routineHost.choose($0, by: appState.deviceId) }
            )) {
                Text("Any device (default)").tag("")
                ForEach(approved, id: \.publicKey) { device in
                    Text(appState.deviceLabel(for: device.publicKey)).tag(device.publicKey)
                }
            }
            .accessibilityIdentifier("routines-run-on")
            if !appState.routineHost.setBy.isEmpty {
                let at = Date(timeIntervalSince1970: Double(SettingsFieldTimestamps.timestamp(for: RoutineHostSettings.setByFieldKey)) / 1000)
                Text("Chosen on \(appState.deviceLabel(for: appState.routineHost.setBy)), \(at.formatted(date: .abbreviated, time: .shortened)).")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
            if choice.isEmpty, approved.count >= 2 {
                Text("Each device runs its own routines until you choose one.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else if !choice.isEmpty, choice != appState.deviceId, approved.contains(where: { $0.publicKey == choice }) {
                Text("Routines run on \(appState.deviceLabel(for: choice)). If it has not run Daily Brief or Reply Priorities 3 hours after their time, this device runs them (devices in different time zones may then both run). Their on/off and time sync between devices. Results sync to every device.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private func refresh() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let fetched = store.listRoutines() ?? []
            let recent = store.recentRoutineRuns(limit: 200) ?? []
            let byRoutine = Dictionary(grouping: recent, by: \.routineId)
                .mapValues { $0.map { CloudRunStamp(ranAtUnixMs: $0.ranAtUnixMs, status: $0.status) } }
            DispatchQueue.main.async {
                routines = fetched
                runs = byRoutine
            }
        }
    }

    private func setEnabled(_ routine: CoreRoutine, _ enabled: Bool) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.setRoutineEnabled(id: routine.id, enabled: enabled)
            RoutineSyncFields.noteLocalChange(template: routine.template, field: .enabled)
            DispatchQueue.main.async { refresh() }
        }
    }

    private func setSchedule(_ routine: CoreRoutine, _ scheduleHhmm: String) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.updateRoutineSchedule(id: routine.id, scheduleHhmm: scheduleHhmm)
            RoutineSyncFields.noteLocalChange(template: routine.template, field: .schedule)
            DispatchQueue.main.async { refresh() }
        }
    }

    private func setModelPref(_ routine: CoreRoutine, _ pref: String) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.setRoutineModelPref(id: routine.id, pref: pref)
            RoutineSyncFields.noteLocalChange(template: routine.template, field: .modelPref)
            DispatchQueue.main.async { refresh() }
        }
    }

    private func setAutoApprove(_ routine: CoreRoutine, _ autoApprove: Bool) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.setRoutineAutoApprove(id: routine.id, autoApprove: autoApprove)
            DispatchQueue.main.async { refresh() }
        }
    }

    /// Templates that have a real write step the approval gate applies to. Matches
    /// `core/memory`'s `RoutineTemplate` snake_case serialization (`daily_brief`,
    /// `focus_time`, `eod_recap`, `weekly_digest`), and `whimbrel_routines_tick`'s own
    /// arms: `FocusTime` (calendar event), `EodRecap`, and `WeeklyDigest` (both email
    /// delivery) are the only ones that ever draft a `PendingApproval` write action.
    /// Unknown templates are treated as read-only, so a future template must opt in here
    /// deliberately rather than inherit a write toggle.
    private static func performsWriteActions(_ routine: CoreRoutine) -> Bool {
        routine.template == "focus_time" || routine.template == "eod_recap"
            || routine.template == "weekly_digest"
    }
}

/// "Run in the cloud" for one routine core lets run there: the toggle with its grant and limits, the ready-by time,
/// and how fresh the last result is.
private struct CloudRoutineSection: View {
    let routine: CoreRoutine
    let info: CloudRoutineInfo
    @ObservedObject var cloud: CloudRoutineSettings
    @ObservedObject var routing: RoutingSettings
    let runs: [CloudRunStamp]
    let runnerMissing: Bool
    let syncReady: Bool

    var body: some View {
        let on = cloud.isCloud(routine.template)
        let blocked = blockedReason
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Toggle(CloudRoutineCopy.toggleLabel, isOn: Binding(
                get: { on },
                set: { cloud.setCloud($0, for: routine.template, routineEnabled: routine.enabled) }
            ))
            .toggleStyle(.switch)
            // Turning it off is always allowed; turning it on needs a key the runner can use.
            .disabled(!on && blocked != nil)
            .accessibilityIdentifier("routine-cloud-\(routine.template)")
            caption(CloudRoutineCopy.toggleCopy(info))
            caption(CloudRoutineCopy.disclosure)
            if let note = CloudRoutineCopy.statusNote(on: on, blockedReason: blocked, runnerMissing: runnerMissing, template: routine.template) {
                caption(note)
            }
            if on {
                // Only the daily routines are ever late, so only they have a ready-by time.
                if CloudRoutineCopy.canBeLate(template: routine.template, enabled: true) { readyByRow }
                SwiftUI.TimelineView(.periodic(from: .now, by: 60)) { context in
                    let calendar = Calendar.current
                    let late = CloudRoutineCopy.canBeLate(template: routine.template, enabled: routine.enabled)
                    let freshness = CloudRoutineCopy.freshness(runs: runs, readyBy: late ? readyBy : "", now: context.date, calendar: calendar)
                    if let label = CloudRoutineCopy.label(freshness, now: context.date, calendar: calendar) {
                        caption(label)
                    }
                }
            }
        }
    }

    /// The chosen ready-by time, else an hour after the routine's own time.
    private var readyBy: String {
        cloud.readyBy[routine.template] ?? CloudRoutineCopy.defaultReadyBy(schedule: routine.scheduleHhmm) ?? routine.scheduleHhmm
    }

    private var readyByRow: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack {
                Text(CloudRoutineCopy.readyByLabel)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                DatePicker("", selection: Binding(
                    get: { CloudRoutineCopy.readyByTime(readyBy, on: Date(), calendar: .current) ?? Date() },
                    set: { date in
                        let parts = Calendar.current.dateComponents([.hour, .minute], from: date)
                        cloud.setReadyBy(String(format: "%02d:%02d", parts.hour ?? 0, parts.minute ?? 0), for: routine.template)
                    }
                ), displayedComponents: .hourAndMinute)
                .labelsHidden()
                .frame(width: 100)
                .accessibilityIdentifier("routine-ready-by-\(routine.template)")
            }
            caption(CloudRoutineCopy.readyByNote)
        }
    }

    /// Why the toggle cannot be turned on: the runner has no model of its own, only a synced provider key.
    private var blockedReason: String? {
        let synced = (routing.anthropicKeySyncEnabled && routing.hasAnthropicKey) || (routing.openaiKeySyncEnabled && routing.hasOpenAIKey)
            || (routing.googleKeySyncEnabled && routing.hasGoogleKey) || (routing.openrouterKeySyncEnabled && routing.hasOpenRouterKey)
        return CloudRoutineCopy.blockedReason(syncReady: syncReady, routineEnabled: routine.enabled, localOnly: routing.localOnlyMode, hasSyncedKey: synced)
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .font(.whimbrelBody(.caption))
            .foregroundStyle(Color.whimbrelTextSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}
