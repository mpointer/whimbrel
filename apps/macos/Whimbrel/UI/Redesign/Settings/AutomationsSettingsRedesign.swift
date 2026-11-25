import SwiftUI
import WhimbrelUI

/// Automations in the redesigned Settings window. This file feeds the package's `WAutomationsSettingsView` from the app's
/// state and calls the same functions `RoutinesSettingsPane` calls (`setRoutineEnabled`, `updateRoutineSchedule`,
/// `setRoutineModelPref`, `setRoutineAutoApprove`, `RoutineSyncFields.noteLocalChange`, `routineHost.choose`, the cloud
/// settings). The classic pane is not changed.

// MARK: Adapter (pure, tested)

enum AutomationsAdapter {
    /// Templates with a real write step, which the approval gate applies to. The classic pane's own rule.
    static func performsWriteActions(template: String) -> Bool {
        template == "focus_time" || template == "eod_recap" || template == "weekly_digest"
    }

    /// Reads " → " and " — " out of sentences the classic copy writes that way, so every redesigned sentence is plain.
    static func plain(_ text: String) -> String {
        text.replacingOccurrences(of: " \u{2192} ", with: ", ").replacingOccurrences(of: " \u{2014} ", with: ", ")
    }

    /// The runs-on choices: Any device, then each approved device that is not the cloud runner.
    static func runsOnOptions(devices: [SyncDeviceRecord], label: (String) -> String) -> [WChoice] {
        [WChoice(id: "", title: String(localized: "Any device (default)"))]
            + devices.filter { $0.status == "approved" && !CloudRoutineCopy.isRunner($0) }.map { WChoice(id: $0.publicKey, title: label($0.publicKey)) }
    }

    /// Why Run in the cloud cannot be on, as the classic section decides it.
    static func blockedReason(syncReady: Bool, routineEnabled: Bool, localOnly: Bool, hasSyncedKey: Bool) -> String? {
        CloudRoutineCopy.blockedReason(syncReady: syncReady, routineEnabled: routineEnabled, localOnly: localOnly, hasSyncedKey: hasSyncedKey).map(plain)
    }

    /// The note under the cloud switch. With the switch off and a reason shown on the switch itself, the reason is not said twice.
    static func statusNote(on: Bool, blockedReason: String?, runnerMissing: Bool, template: String) -> String? {
        let note = CloudRoutineCopy.statusNote(on: on, blockedReason: blockedReason, runnerMissing: runnerMissing, template: template).map(plain)
        return !on && note == blockedReason ? nil : note
    }

    /// The ready-by time as a date on `day`; the picker shows only its hour and minute.
    static func readyByDate(_ hhmm: String, day: Date, calendar: Calendar) -> Date {
        CloudRoutineCopy.readyByTime(hhmm, on: day, calendar: calendar) ?? day
    }

    static func hhmm(from date: Date, calendar: Calendar) -> String {
        let parts = calendar.dateComponents([.hour, .minute], from: date)
        return String(format: "%02d:%02d", parts.hour ?? 0, parts.minute ?? 0)
    }

    static var ownStrings: [String] {
        let l = labels()
        return [l.intro, l.timeInvalid, l.prepareLeadNote(15)]
            + [CloudRoutineCopy.disclosure, CloudRoutineCopy.readyByNote, CloudRoutineCopy.noRunner, CloudRoutineCopy.noRunnerPrep].map(plain)
            + [blockedReason(syncReady: false, routineEnabled: true, localOnly: false, hasSyncedKey: false),
               blockedReason(syncReady: true, routineEnabled: true, localOnly: false, hasSyncedKey: false)].compactMap { $0 }
    }

    static func labels() -> WAutomationsLabels {
        var l = WAutomationsLabels()
        l.heading = String(localized: "Routines")
        l.intro = String(localized: "Scheduled automations that run on their own and deliver a notification when done.")
        l.runsOn = String(localized: "Routines run on")
        l.noRoutines = String(localized: "No routines yet.", comment: "Settings: shown when there are no routines.")
        l.autoApprove = String(localized: "Auto-approve write actions")
        l.runsDailyAt = String(localized: "Runs daily at")
        l.timePlaceholder = String(localized: "HH:MM")
        l.timeInvalid = String(localized: "Use a time like 07:30.", comment: "Settings: shown when the time a routine runs is not a valid HH:MM time.")
        l.prepareLeadNote = { String(localized: "Prepare a brief \($0) minutes before each meeting") }
        l.minutes = { String(localized: "\($0) min") }
        l.model = String(localized: "Model")
        l.cloudToggle = CloudRoutineCopy.toggleLabel
        l.readyBy = CloudRoutineCopy.readyByLabel
        return l
    }
}

// MARK: Runtime state

/// What `RoutinesSettingsPane` keeps in `@State`, with the same reads.
@MainActor
final class AutomationsRuntime: ObservableObject {
    unowned let appState: AppState
    init(appState: AppState) { self.appState = appState }

    @Published var routines: [CoreRoutine] = []
    @Published var runs: [Int64: [CloudRunStamp]] = [:]

    func refresh() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let fetched = store.listRoutines() ?? []
            let recent = store.recentRoutineRuns(limit: 200) ?? []
            let byRoutine = Dictionary(grouping: recent, by: \.routineId)
                .mapValues { $0.map { CloudRunStamp(ranAtUnixMs: $0.ranAtUnixMs, status: $0.status) } }
            DispatchQueue.main.async {
                self.routines = fetched
                self.runs = byRoutine
            }
        }
    }

    func write(_ work: @escaping (CoreMemoryStore) -> Void) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            work(store)
            DispatchQueue.main.async { self.refresh() }
        }
    }
}

// MARK: The view

struct AutomationsSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let highlighted: String?

    var body: some View {
        AutomationsSettingsRedesignBody(appState: appState, highlighted: highlighted)
    }
}

private struct AutomationsSettingsRedesignBody: View {
    let appState: AppState
    let highlighted: String?
    @StateObject private var runtime: AutomationsRuntime
    @ObservedObject private var syncState: SyncState
    @ObservedObject private var cloud: CloudRoutineSettings
    @ObservedObject private var routing: RoutingSettings
    @ObservedObject private var prep: MeetingPrepSettings
    @ObservedObject private var host: RoutineHostSettings

    init(appState: AppState, highlighted: String?) {
        self.appState = appState
        self.highlighted = highlighted
        _runtime = StateObject(wrappedValue: AutomationsRuntime(appState: appState))
        syncState = appState.syncState
        cloud = appState.settingsSyncCoordinator.cloudRoutines
        routing = appState.routing
        prep = appState.meetingPrepSettings
        host = appState.routineHost
    }

    var body: some View {
        // The freshness lines change with the clock, so the view is rebuilt every minute.
        SwiftUI.TimelineView(.periodic(from: .now, by: 60)) { context in
            WAutomationsSettingsView(model: model(now: context.date), actions: actions(), highlighted: highlighted)
        }
        .onAppear { runtime.refresh() }
        // A result the runner (or another device) made shows once it syncs.
        .onReceive(appState.contentSyncCoordinator.$lastSyncedAt.dropFirst()) { _ in runtime.refresh() }
    }

    private var syncReady: Bool { if case .ready = syncState.status { return true } else { return false } }

    private func model(now: Date) -> WAutomationsModel {
        let devices = syncState.devices
        let choice = host.primaryDeviceId
        let approved = devices.filter { $0.status == "approved" && !CloudRoutineCopy.isRunner($0) }
        var chosenLine: String?
        if !host.setBy.isEmpty {
            let at = Date(timeIntervalSince1970: Double(SettingsFieldTimestamps.timestamp(for: RoutineHostSettings.setByFieldKey)) / 1000)
            chosenLine = String(localized: "Chosen on \(appState.deviceLabel(for: host.setBy)), \(at.formatted(date: .abbreviated, time: .shortened)).")
        }
        var note: String?
        if choice.isEmpty, approved.count >= 2 {
            note = String(localized: "Each device runs its own routines until you choose one.")
        } else if !choice.isEmpty, choice != appState.deviceId, approved.contains(where: { $0.publicKey == choice }) {
            note = String(localized: "Routines run on \(appState.deviceLabel(for: choice)). If it has not run Daily Brief or Reply Priorities 3 hours after their time, this device runs them (devices in different time zones may then both run). Their on/off and time sync between devices. Results sync to every device.")
        }
        let calendar = Calendar.current
        let rows = runtime.routines.filter { $0.template != "away_follow_up" }.map { routine in
            routineRow(routine, devices: devices, now: now, calendar: calendar)
        }
        return WAutomationsModel(
            labels: AutomationsAdapter.labels(),
            runsOn: WRunsOnModel(
                options: AutomationsAdapter.runsOnOptions(devices: devices, label: { appState.deviceLabel(for: $0) }), selection: choice,
                chosenLine: chosenLine, note: note),
            routines: rows)
    }

    private func routineRow(_ routine: CoreRoutine, devices: [SyncDeviceRecord], now: Date, calendar: Calendar) -> WRoutineRow {
        let timing: WRoutineRow.Timing = routine.template == "meeting_prep"
            ? .leadMinutes(Int(prep.leadMinutes), range: Int(MeetingPrepSettings.range.lowerBound)...Int(MeetingPrepSettings.range.upperBound))
            : .dailyAt(routine.scheduleHhmm)
        var options: [WChoice]?
        var selection = ""
        if RoutinesSettingsPane.callsAModel(routine) {
            let list = RoutinesSettingsPane.modelPrefOptions(routing: routing)
            options = list.map { WChoice(id: $0.value, title: $0.label) }
            selection = list.contains { $0.value == routine.modelPref } ? routine.modelPref : "default"
        }
        var cloudModel: WRoutineCloud?
        if let info = CloudRoutineInfo.load(routine.template), CloudRoutineCopy.showsToggle(info),
           CloudRoutineCopy.showsCloudSection(flag: CloudRoutineCopy.uiFlag(), devices: devices, on: cloud.isCloud(routine.template)) {
            let on = cloud.isCloud(routine.template)
            let synced = (routing.anthropicKeySyncEnabled && routing.hasAnthropicKey) || (routing.openaiKeySyncEnabled && routing.hasOpenAIKey)
                || (routing.googleKeySyncEnabled && routing.hasGoogleKey) || (routing.openrouterKeySyncEnabled && routing.hasOpenRouterKey)
            let blocked = AutomationsAdapter.blockedReason(syncReady: syncReady, routineEnabled: routine.enabled, localOnly: routing.localOnlyMode, hasSyncedKey: synced)
            let canBeLate = CloudRoutineCopy.canBeLate(template: routine.template, enabled: true)
            let readyBy = cloud.readyBy[routine.template] ?? CloudRoutineCopy.defaultReadyBy(schedule: routine.scheduleHhmm) ?? routine.scheduleHhmm
            let late = CloudRoutineCopy.canBeLate(template: routine.template, enabled: routine.enabled)
            let freshness = CloudRoutineCopy.freshness(runs: runtime.runs[routine.id] ?? [], readyBy: late ? readyBy : "", now: now, calendar: calendar)
            cloudModel = WRoutineCloud(
                isOn: on, blockedReason: blocked, toggleCopy: AutomationsAdapter.plain(CloudRoutineCopy.toggleCopy(info)),
                disclosure: CloudRoutineCopy.disclosure,
                statusNote: AutomationsAdapter.statusNote(on: on, blockedReason: blocked, runnerMissing: CloudRoutineCopy.runnerMissing(devices: devices), template: routine.template),
                readyBy: on && canBeLate ? AutomationsAdapter.readyByDate(readyBy, day: now, calendar: calendar) : nil,
                readyByNote: CloudRoutineCopy.readyByNote, freshness: CloudRoutineCopy.label(freshness, now: now, calendar: calendar))
        }
        return WRoutineRow(
            id: String(routine.id), name: routine.name, enabled: routine.enabled,
            autoApprove: AutomationsAdapter.performsWriteActions(template: routine.template) ? routine.autoApproveWriteActions : nil,
            timing: timing, modelOptions: options, modelSelection: selection, cloud: cloudModel)
    }

    private func routine(_ id: String) -> CoreRoutine? { runtime.routines.first { String($0.id) == id } }

    private func actions() -> WAutomationsActions {
        WAutomationsActions(
            setRunsOn: { host.choose($0, by: appState.deviceId) },
            setEnabled: { id, enabled in
                guard let r = routine(id) else { return }
                runtime.write { store in
                    _ = store.setRoutineEnabled(id: r.id, enabled: enabled)
                    RoutineSyncFields.noteLocalChange(template: r.template, field: .enabled)
                }
            },
            setAutoApprove: { id, on in
                guard let r = routine(id) else { return }
                runtime.write { _ = $0.setRoutineAutoApprove(id: r.id, autoApprove: on) }
            },
            setLeadMinutes: { prep.leadMinutes = Double($0) },
            setSchedule: { id, time in
                guard let r = routine(id) else { return }
                runtime.write { store in
                    _ = store.updateRoutineSchedule(id: r.id, scheduleHhmm: time)
                    RoutineSyncFields.noteLocalChange(template: r.template, field: .schedule)
                }
            },
            setModel: { id, pref in
                guard let r = routine(id) else { return }
                runtime.write { store in
                    _ = store.setRoutineModelPref(id: r.id, pref: pref)
                    RoutineSyncFields.noteLocalChange(template: r.template, field: .modelPref)
                }
            },
            setCloud: { id, on in
                guard let r = routine(id) else { return }
                cloud.setCloud(on, for: r.template, routineEnabled: r.enabled)
            },
            setReadyBy: { id, date in
                guard let r = routine(id) else { return }
                cloud.setReadyBy(AutomationsAdapter.hhmm(from: date, calendar: .current), for: r.template)
            })
    }
}
