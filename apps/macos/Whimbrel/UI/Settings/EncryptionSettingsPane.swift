import SwiftUI

/// Settings, Capture & Privacy, "Database encryption" (X4 design 3.6, release N opt-in): the
/// state of the memory database from the launch's open outcome, and the "Encrypt the
/// database" action. That action first offers Export everything, then runs the migration
/// pre-checks (recording, disk space, open elsewhere) BEFORE any recovery key is made, then
/// requires the recovery-key setup, then hands the setup's key to the encrypt step. The
/// model takes every effect as a closure; the launch wiring (Task 11) supplies the real ones.
@MainActor
final class EncryptionSettingsModel: ObservableObject {
    enum State: Equatable {
        case notEncrypted
        case encrypted
        case needsAttention(String)
    }

    enum Phase: Equatable {
        case idle
        /// The first step after "Encrypt the database": offer Export everything.
        case offeringExport
        /// Running the pre-checks before the setup sheet opens.
        case checking
        /// The recovery-key setup (`setupModel`).
        case settingUp
        case encrypting(Double)
        case finished(String)
        case failed(String)
    }

    enum EncryptResult: Equatable {
        case encrypted
        /// Stored the key and recovery; the launch flow encrypts before the store opens.
        case finishesAtNextLaunch
        /// Did not start (precheck); the message says why. Nothing changed.
        case skipped(String)
        case failed(String)
    }

    struct Actions {
        /// Opens the existing Export everything flow (Settings, Your data).
        var exportEverything: () -> Void
        /// The migration pre-checks (`CoreEncryptionMigration.precheck`): nil when encryption
        /// may start, else the message to show (`skipMessage(from:)`). Runs before setup.
        var precheck: () async -> String?
        /// Whether `<db>.plain.bak` exists (`CoreEncryptionMigration.plainBackupExists`).
        var plainBackupExists: () -> Bool
        /// The key a previous attempt stored as `db-key-pending`, if any (Task 11: the
        /// Keychain's `lookupPending()`). Never replaced by a fresh one (plan Task 11 Step 0c).
        var pendingKey: () -> Data?
        /// The setup's collaborators, made when the setup opens, for the pending key if there
        /// is one (`EncryptionSetupModel.Actions.live(existingKey:)`), so a retry keeps the words.
        var setup: (_ pendingKey: Data?) -> EncryptionSetupModel.Actions
        /// Restarts Whimbrel (offered when encryption finishes at the next launch).
        var restart: () -> Void
        /// Encrypts with the key setup produced; `progress` (0...1) may be called off the
        /// main thread. Task 11: store this key as `db-key-pending` first and reuse it
        /// (`EncryptionSetupModel.Actions.live(existingKey:)`) on any retry, so a retry never
        /// makes new words.
        var encrypt: (Data, @escaping @Sendable (Double) -> Void) async -> EncryptResult
    }

    @Published private(set) var state: State
    private let actions: Actions
    @Published private(set) var phase: Phase = .idle
    @Published private(set) var exportOffered = false
    @Published private(set) var setupModel: EncryptionSetupModel?
    /// Why the last attempt did not start (a pre-check), shown while idle.
    @Published private(set) var notice: String?
    /// The unencrypted rollback copy is still on disk.
    @Published private(set) var plainBackupPresent: Bool
    /// The launch's "unencrypted copy is kept" note is showing (Task 11b-2): the three-launch
    /// cleanup will not remove that copy, so the automatic-deletion notice would be wrong.
    @Published var plainBackupKeptByNote = false
    var showsPlainBackupNotice: Bool { plainBackupPresent && !plainBackupKeptByNote }
    /// Encryption was handed to the next launch: nothing more may start until then (a second
    /// run would make a fresh key and new words).
    @Published private(set) var pendingNextLaunch = false

    /// `pendingNextLaunch`: "Encrypt the database" finished in an earlier session (the
    /// launch's "migrate at next launch" flag is set) and only a restart is left to do.
    /// `attention`: a problem the open outcome cannot express (the store did not open at all).
    init(outcome: CoreOpenOutcome?, actions: Actions, pendingNextLaunch: Bool = false, attention: String? = nil) {
        self.state = attention.map(State.needsAttention) ?? Self.state(for: outcome)
        self.actions = actions
        self.plainBackupPresent = actions.plainBackupExists()
        if pendingNextLaunch, state == .notEncrypted {
            self.pendingNextLaunch = true
            self.phase = .finished(EncryptionCopy.encryptAtNextLaunch)
        }
    }

    /// The pane's state from the launch's open outcome. `nil` means the store was opened the
    /// old, plain way (no keyed launch yet), so the database is plain. Core's messages stay
    /// in logs; the pane shows fixed English.
    static func state(for outcome: CoreOpenOutcome?) -> State {
        guard let outcome else { return .notEncrypted }
        switch outcome {
        case .opened: return .encrypted
        case .needsMigration, .newInstall: return .notEncrypted
        case .needsKey: return .needsAttention(EncryptionCopy.attentionNeedsKey)
        case .keyRejected: return .needsAttention(EncryptionCopy.attentionKeyRejected)
        case .keyUnavailable: return .needsAttention(EncryptionCopy.attentionKeyUnavailable)
        case .migrationInterrupted: return .needsAttention(EncryptionCopy.attentionMigrationInterrupted)
        case .damaged: return .needsAttention(EncryptionCopy.attentionDamaged)
        case .startupFailed: return .needsAttention(EncryptionCopy.attentionStartupFailed)
        case .unsupported: return .needsAttention(EncryptionCopy.attentionUnsupported)
        case .invalidArgument, .unknown: return .needsAttention(EncryptionCopy.attentionUnexpected(outcome.code))
        }
    }

    /// Core's migration answer as the pane's result. `failed_before` is a failure (Task 11
    /// adds its Retry, which calls `resetFailures`).
    static func result(from outcome: CoreEncryptionMigration.Outcome?) -> EncryptResult {
        guard let outcome else { return .failed(EncryptionCopy.encryptFailed) }
        if outcome.ok { return .encrypted }
        guard let error = outcome.error, error.kind == "skipped", let reason = error.skip?.reason else {
            return .failed(EncryptionCopy.encryptFailed)
        }
        switch reason {
        case "recording": return .skipped(EncryptionCopy.skipRecording)
        case "low_disk", "free_space_unknown": return .skipped(EncryptionCopy.skipLowDisk)
        // Another program has the file open.
        case "open_elsewhere": return .skipped(EncryptionCopy.skipOpenElsewhere)
        // This app's own open store: the run happens at the next launch, before it opens.
        case "open_in_this_process": return .finishesAtNextLaunch
        case "failed_before": return .failed(EncryptionCopy.encryptFailed)
        default: return .skipped(EncryptionCopy.skipOther)
        }
    }

    /// A precheck answer as the `precheck` closure's message: nil when it may start.
    static func skipMessage(from outcome: CoreEncryptionMigration.Outcome?) -> String? {
        switch result(from: outcome) {
        case .encrypted, .finishesAtNextLaunch: return nil
        case let .skipped(m), let .failed(m): return m
        }
    }

    var canStartEncrypt: Bool { state == .notEncrypted && phase == .idle && !pendingNextLaunch }

    func startEncrypt() {
        guard canStartEncrypt else { return }
        notice = nil
        phase = .offeringExport
    }

    func exportEverything() {
        guard phase == .offeringExport else { return }
        actions.exportEverything()
        exportOffered = true
    }

    /// Runs the pre-checks, then opens the setup. A failed check goes back to idle with its
    /// message, before any key or recovery file exists.
    func continueToSetup() async {
        guard phase == .offeringExport else { return }
        phase = .checking
        let message = await actions.precheck()
        guard phase == .checking else { return }
        if let message {
            notice = message
            phase = .idle
            return
        }
        setupModel = EncryptionSetupModel(actions: actions.setup(actions.pendingKey()), continueTitle: EncryptionCopy.encryptContinue) { [weak self] key in
            Task { await self?.encrypt(with: key) }
        }
        phase = .settingUp
    }

    func cancel() {
        guard phase == .offeringExport || phase == .settingUp || phase == .checking else { return }
        setupModel = nil
        phase = .idle
    }

    /// Back to idle from a failed run only (a finished one stays: encrypted, or pending the
    /// next launch with Restart as its only action).
    func dismissResult() {
        if case .failed = phase { phase = .idle }
    }

    /// Looks at `<db>.plain.bak` again (after a note's Delete button).
    func refreshPlainBackup() {
        plainBackupPresent = actions.plainBackupExists()
    }

    func restart() {
        guard pendingNextLaunch else { return }
        actions.restart()
    }

    func encrypt(with key: Data) async {
        guard phase == .settingUp else { return }
        setupModel = nil
        phase = .encrypting(0)
        let result = await actions.encrypt(key) { [weak self] fraction in
            Task { @MainActor in
                guard let self, case .encrypting = self.phase else { return }
                self.phase = .encrypting(fraction)
            }
        }
        plainBackupPresent = actions.plainBackupExists()
        switch result {
        case .encrypted:
            state = .encrypted
            phase = .finished(EncryptionCopy.encryptDone)
        case .finishesAtNextLaunch:
            pendingNextLaunch = true
            phase = .finished(EncryptionCopy.encryptAtNextLaunch)
        case let .skipped(message), let .failed(message): phase = .failed(message)
        }
    }
}

struct EncryptionSettingsPane: View {
    @ObservedObject var model: EncryptionSettingsModel

    /// The launch wiring's hook (Task 11b-2, `AppHost`): the open outcome, the real actions,
    /// the launch notes, Faster search and the speed check. Until it is set (the launch has
    /// not opened the store), Settings does not show this card.
    static var live: EncryptionSettingsLive?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            HStack {
                Text(EncryptionCopy.settingsTitle).font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary).settingsAnchor("capture.encryption")
                Spacer()
                stateBadge
            }
            Text(stateBody).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(EncryptionCopy.notEncryptedList).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(EncryptionCopy.notProtectedAgainst).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if model.showsPlainBackupNotice {
                Text(EncryptionCopy.plainBackupNotice).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelWarning)
                    .fixedSize(horizontal: false, vertical: true)
            }
            phaseContent
        }
        .sheet(isPresented: Binding(get: { model.setupModel != nil }, set: { if !$0 { model.cancel() } })) {
            if let setup = model.setupModel {
                RedesignSwitch(enabled: RedesignFlag.isOn()) {
                    KeySetupRedesignView(model: setup, leave: .init(title: EncryptionCopy.cancel, isEscape: true, isEnabled: true, run: { model.cancel() }))
                        .frame(width: 680, height: 720)
                } old: {
                    VStack(spacing: 0) {
                        EncryptionSetupView(model: setup)
                        HStack {
                            Button(EncryptionCopy.cancel) { model.cancel() }.buttonStyle(.whimbrelSecondary)
                            Spacer()
                        }
                        .padding([.horizontal, .bottom], Spacing.lg)
                    }
                    .frame(width: 640, height: 720)
                }
            }
        }
    }

    private var stateBody: String {
        switch model.state {
        case .notEncrypted: return EncryptionCopy.settingsNotEncryptedBody
        case .encrypted: return EncryptionCopy.settingsEncryptedBody
        case let .needsAttention(message): return message
        }
    }

    private var stateBadge: some View {
        let (text, color): (String, Color) = {
            switch model.state {
            case .notEncrypted: return (EncryptionCopy.stateNotEncrypted, Color.whimbrelTextSecondary)
            case .encrypted: return (EncryptionCopy.stateEncrypted, Color.whimbrelSuccess)
            case .needsAttention: return (EncryptionCopy.stateNeedsAttention, Color.whimbrelWarning)
            }
        }()
        return Text(text).font(.whimbrelBody(.caption).weight(.semibold)).foregroundStyle(color)
            .padding(.horizontal, Spacing.sm).padding(.vertical, 2)
            .overlay(Capsule().stroke(color, lineWidth: 1))
    }

    @ViewBuilder
    private var phaseContent: some View {
        switch model.phase {
        case .idle:
            if let notice = model.notice {
                Text(notice).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if model.state == .notEncrypted {
                Button(EncryptionCopy.encryptAction) { model.startEncrypt() }
                    .buttonStyle(.whimbrelPrimary)
                    .disabled(!model.canStartEncrypt)
            }
        case .offeringExport:
            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text(EncryptionCopy.exportFirstTitle).font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
                Text(EncryptionCopy.exportFirstBody).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button(EncryptionCopy.exportAction) { model.exportEverything() }.buttonStyle(.whimbrelSecondary)
                    Button(model.exportOffered ? EncryptionCopy.continueAfterExport : EncryptionCopy.continueWithoutExport) {
                        Task { await model.continueToSetup() }
                    }
                    .buttonStyle(.whimbrelPrimary)
                    Button(EncryptionCopy.cancel) { model.cancel() }.buttonStyle(.whimbrelSecondary)
                }
            }
            .padding(Spacing.md)
            .background(RoundedRectangle(cornerRadius: 8).stroke(Color.whimbrelBorder))
        case .checking:
            HStack(spacing: Spacing.sm) {
                ProgressView().controlSize(.small)
                Text(EncryptionCopy.checkingFirst).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        case .settingUp:
            EmptyView()
        case let .encrypting(fraction):
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text(EncryptionCopy.encrypting).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                ProgressView(value: fraction).frame(maxWidth: 320)
            }
        case let .finished(message):
            HStack {
                Label(message, systemImage: "checkmark.circle.fill").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelSuccess)
                if model.pendingNextLaunch {
                    Button(EncryptionCopy.restartAction) { model.restart() }.buttonStyle(.whimbrelPrimary)
                }
            }
        case let .failed(message):
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text(message).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
                    .fixedSize(horizontal: false, vertical: true)
                Button(EncryptionCopy.dismiss) { model.dismissResult() }.buttonStyle(.whimbrelSecondary)
            }
        }
    }
}

/// Mounts the pane in Settings only once the launch wiring has supplied its actions: the
/// pane, the launch's notes, Faster search (when this Mac has the memory) and the speed check.
struct EncryptionSettingsCard: View {
    @StateObject private var model: EncryptionSettingsModel
    private let live: EncryptionSettingsLive

    init(live: EncryptionSettingsLive) {
        self.live = live
        _model = StateObject(wrappedValue: EncryptionSettingsModel(outcome: live.outcome, actions: live.actions,
                                                                   pendingNextLaunch: live.pendingNextLaunch,
                                                                   attention: live.attention))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            EncryptionSettingsPane(model: model)
            EncryptionNotesView(model: live.notes)
                .onReceive(live.notes.$rows) { rows in
                    model.plainBackupKeptByNote = rows.contains { $0.action == .deletePlainBackup }
                    model.refreshPlainBackup()
                }
            if live.showsStoreTools {
                Divider().overlay(Color.whimbrelBorder)
                if let fast = live.fastSearch { FastSearchToggleView(setting: fast) }
                SpeedCheckView(model: live.speedCheck)
            }
        }
    }
}
