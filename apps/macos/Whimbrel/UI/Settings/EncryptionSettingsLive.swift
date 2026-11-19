import AppKit
import Combine
import SwiftUI

/// Everything the Settings "Database encryption" card needs from the launch (X4 Task 11b-2):
/// the state badge, the encrypt actions, the launch's notes with their buttons, Faster search
/// and the local speed check. Built once the launch opened the store (`AppHost`).
struct EncryptionSettingsLive {
    var outcome: CoreOpenOutcome?
    /// Set when the launch went on without the store: the pane says so and offers nothing.
    var attention: String? = nil
    /// "Encrypt the database" finished in an earlier session and waits for a restart.
    var pendingNextLaunch: Bool
    var actions: EncryptionSettingsModel.Actions
    var notes: EncryptionNotesModel
    /// Nil when this Mac has too little memory to offer it.
    var fastSearch: FastSearchSetting?
    var speedCheck: SpeedCheckModel

    /// Faster search and the speed check need an open store: hidden while running without it.
    var showsStoreTools: Bool { attention == nil }

    /// "Encryption finishes at the next launch" (Restart only): the flag is set, the store
    /// is still plain, and core will try at the next launch, which it does not after too
    /// many failures (Retry first) or with an unreadable state file (Reset first).
    static func pendingNextLaunch(flag: Bool, openedWithKey: Bool, notes: [LaunchNote]) -> Bool {
        flag && !openedWithKey && !notes.contains {
            switch $0 {
            case .failedBefore, .migrationSkipped(.stateUnreadable, _): return true
            default: return false
            }
        }
    }

    /// The badge: encrypted when the launch opened the store with a key, else plain.
    static func outcome(openedWithKey: Bool) -> CoreOpenOutcome? { openedWithKey ? .opened : nil }

    /// The card's encrypt actions. `encrypt` NEVER migrates the open store: it keeps the
    /// setup's key as `db-key-pending` (never replacing a key) and sets the "migrate at next
    /// launch" flag; the next launch migrates before the store opens.
    static func actions(keys: LaunchKeyStore, settings: LaunchSettingsStore, dbPath: URL, recording: @escaping () -> Bool,
                        precheck: @escaping (URL, Bool) async -> CoreEncryptionMigration.Outcome?,
                        setup: @escaping (_ existingKey: Data?) -> EncryptionSetupModel.Actions,
                        exportEverything: @escaping () -> Void, restart: @escaping () -> Void) -> EncryptionSettingsModel.Actions {
        func found(_ l: DbKeyLookup) -> Data? { if case let .found(k) = l { return k } else { return nil } }
        func unavailable(_ l: DbKeyLookup) -> Bool { if case .unavailable = l { return true } else { return false } }
        return .init(
            exportEverything: exportEverything,
            precheck: { @MainActor in
                // A key that cannot be read is never replaced by a fresh one: stop here,
                // before the setup makes words for a key that would not be used.
                if unavailable(keys.lookup()) || unavailable(keys.lookupPending()) {
                    return EncryptionCopy.precheckKeychainUnavailable
                }
                // db-key and a different pending key: the next launch would migrate with
                // db-key while the pending key is left beside it and refused every launch.
                if let k = found(keys.lookup()), let p = found(keys.lookupPending()), k != p {
                    return EncryptionCopy.twoKeysFound
                }
                // `open_in_this_process` (this app's own open store) maps to "finishes at the
                // next launch", so it does not stop the setup. Core answers it before it looks
                // for other holders, so while this app's store is open Settings cannot see
                // another program holding the database: the launch's own pre-check, which runs
                // before anything opens, is what catches that (and says so in a note).
                // `open_elsewhere` (another program; seen here only when this app's store is
                // not open) is a real refusal with its own message.
                return EncryptionSettingsModel.skipMessage(from: await precheck(dbPath, recording()))
            },
            plainBackupExists: { CoreEncryptionMigration.plainBackupExists(dbPath: dbPath) },
            pendingKey: { found(keys.lookupPending()) },
            // db-key first: with it the next launch migrates with db-key (core
            // `plain_file`), so the words must be its words. Else the pending key, else fresh.
            setup: { _ in setup(found(keys.lookup()) ?? found(keys.lookupPending())) },
            restart: restart,
            encrypt: { key, _ in
                switch (keys.lookup(), keys.lookupPending()) {
                case let (.found(k), .found(p)) where k != p:
                    return .failed(EncryptionCopy.twoKeysFound)
                case let (.found(k), _):
                    guard k == key else { return .failed(EncryptionCopy.encryptFailed) }
                case let (.absent, .found(p)):
                    guard p == key else { return .failed(EncryptionCopy.encryptFailed) }
                case (.absent, .absent):
                    do { try keys.storePending(key) } catch { return .failed(EncryptionCopy.encryptFailed) }
                default:
                    return .failed(EncryptionCopy.encryptFailed)
                }
                settings.migrateAtNextLaunch = true
                return .finishesAtNextLaunch
            })
    }

    /// The speed check on the open main store, on `accessQueue`, and the save panel.
    /// Cancel (the model's) returns at once and drops the result; core's check has no stop
    /// hook in its C ABI, so the queries already started finish on `accessQueue`.
    static func speedActions(store: @escaping () -> CoreMemoryStore?, openMs: @escaping () -> UInt64?, physicalMemory: UInt64,
                             save: @escaping @MainActor (String) -> URL?,
                             recording: @escaping () -> Bool = { false }) -> SpeedCheckModel.Actions {
        .init(
            run: { progress in
                guard let s = store() else { return nil }
                let ms = openMs()
                nonisolated(unsafe) let store = s
                return await withCheckedContinuation { c in
                    CoreMemoryStore.accessQueue.async {
                        c.resume(returning: store.speedCheck(openMs: ms, physicalMemory: physicalMemory, progress: progress))
                    }
                }
            },
            text: { CoreSpeedCheck.text(reportJSON: $0) },
            save: save,
            isRecording: recording)
    }
}

// MARK: - Notes

enum EncryptionNoteAction: Hashable {
    case retryAfterFailedBefore, resetEncryptionStatus, deletePlainBackup
}

/// One launch note as Settings shows it: copy from the typed note only (never
/// `diagnosticOnly`), and the one button it offers, if any.
struct EncryptionNoteRow: Identifiable {
    let id: Int
    let text: [String]
    let action: EncryptionNoteAction?

    init(_ note: LaunchNote, id: Int = 0) {
        self.id = id
        switch note {
        case .keyStoreUnavailable:
            (text, action) = ([EncryptionCopy.noteKeyStoreUnavailable], nil)
        case .failedBefore:
            (text, action) = ([EncryptionCopy.encryptFailed], .retryAfterFailedBefore)
        case let .migrationSkipped(.precheck, skip):
            let message: String
            switch skip?.reason {
            case "recording": message = EncryptionCopy.noteSkippedRecording
            case "low_disk", "free_space_unknown": message = EncryptionCopy.noteSkippedLowDisk
            case "open_elsewhere": message = EncryptionCopy.noteSkippedOpenElsewhere
            // Not expected at launch (nothing is open yet): the generic note.
            case "open_in_this_process": message = EncryptionCopy.noteSkippedOther
            default: message = EncryptionCopy.noteSkippedOther
            }
            (text, action) = ([message], nil)
        case .migrationSkipped(.stateUnreadable, _):
            (text, action) = ([EncryptionCopy.noteStateUnreadable], .resetEncryptionStatus)
        case .migrationFailed:
            (text, action) = ([EncryptionCopy.noteMigrationFailed], nil)
        case .encryptionAvailable:
            (text, action) = ([EncryptionCopy.noteEncryptionAvailable], nil)
        case .plainBackupKept:
            (text, action) = ([EncryptionCopy.notePlainBackupKept, EncryptionCopy.deleteNotGuaranteed], .deletePlainBackup)
        case let .restoredCopy(moved, warning):
            var t = [EncryptionCopy.noteRestoredCopy]
            if moved > 0 { t.append(EncryptionCopy.noteRestoredMovedAside) }
            if warning == .stateNotMovedAside { t.append(EncryptionCopy.noteRestoredResetHint) }
            (text, action) = (t, nil)
        case .pendingKeyNotPromoted:
            (text, action) = ([EncryptionCopy.notePendingKeyNotPromoted], nil)
        case .pendingKeyNotDeleted:
            (text, action) = ([EncryptionCopy.notePendingKeyKept], nil)
        case let .recoveredKeyNotStored(failure, _):
            (text, action) = ([failure == .noPreviousSlot ? EncryptionCopy.noteRecoveredKeyNoSlot : EncryptionCopy.noteRecoveredKeyNotStored], nil)
        case .recoveredKeyStoreUnverified:
            (text, action) = ([EncryptionCopy.noteRecoveredKeyUnverified], nil)
        case .memoryDatabaseUnavailable:
            // The pane's state line already says "could not be opened": only what to do.
            (text, action) = ([EncryptionCopy.noteStoreUnavailableHint], nil)
        case .libraryMissing:
            // Shown once in the main window (`LibraryMissingNoticeHost`); `EncryptionNotesModel`
            // leaves it out of Settings so it is not said twice.
            (text, action) = ([EncryptionCopy.libraryMissingNotice], nil)
        }
    }
}

/// The launch's notes (`LaunchCoordinator.notes`, the only channel) and their buttons, which
/// do nothing while the deferred backup cleanup runs (`cleanupRunning`).
@MainActor
final class EncryptionNotesModel: ObservableObject {
    struct Actions {
        var deletePlainBackup: () -> CoreEncryptionFileAction
        var resetEncryptionStatus: () -> CoreEncryptionFileAction
        var retryAfterFailedBefore: () -> Bool
    }

    @Published private(set) var rows: [EncryptionNoteRow] = []
    @Published private(set) var cleanupRunning = false
    /// What the last press of each button did, when there is something to say.
    @Published private(set) var feedback: [EncryptionNoteAction: String] = [:]
    private let actions: Actions
    private var cancellables = Set<AnyCancellable>()

    init(notes: AnyPublisher<[LaunchNote], Never>, cleanupRunning: AnyPublisher<Bool, Never>, actions: Actions) {
        self.actions = actions
        notes.sink { [weak self] n in
            self?.rows = n.filter { $0 != .libraryMissing }.enumerated().map { EncryptionNoteRow($1, id: $0) }
        }.store(in: &cancellables)
        cleanupRunning.sink { [weak self] in self?.cleanupRunning = $0 }.store(in: &cancellables)
    }

    convenience init(coordinator: LaunchCoordinator<CoreMemoryStore>) {
        self.init(notes: coordinator.$notes.eraseToAnyPublisher(), cleanupRunning: coordinator.$cleanupRunning.eraseToAnyPublisher(),
                  actions: .init(deletePlainBackup: { [weak coordinator] in coordinator?.deletePlainBackup() ?? .refused },
                                 resetEncryptionStatus: { [weak coordinator] in coordinator?.resetEncryptionStatus() ?? .refused },
                                 retryAfterFailedBefore: { [weak coordinator] in coordinator?.retryAfterFailedBefore() ?? false }))
    }

    func perform(_ action: EncryptionNoteAction) {
        guard !cleanupRunning else { return }
        switch action {
        case .deletePlainBackup:
            feedback[action] = actions.deletePlainBackup() == .done ? nil : EncryptionCopy.deletePlainBackupFailed
        case .resetEncryptionStatus:
            feedback[action] = actions.resetEncryptionStatus() == .done ? nil : EncryptionCopy.resetStatusFailed
        case .retryAfterFailedBefore:
            feedback[action] = actions.retryAfterFailedBefore() ? EncryptionCopy.noteRetryScheduled : EncryptionCopy.noteRetryFailed
        }
    }

    static func title(_ action: EncryptionNoteAction) -> String {
        switch action {
        case .retryAfterFailedBefore: return EncryptionCopy.tryAgain
        case .resetEncryptionStatus: return EncryptionCopy.resetStatusAction
        case .deletePlainBackup: return EncryptionCopy.deletePlainBackupAction
        }
    }
}

// MARK: - Faster search

/// "Faster search (uses more memory)": per device (`storage.fastSearch` in this Mac's
/// defaults, never synced), read by the next launch. Nil when this Mac has too little memory
/// (core `fast_search_available`).
@MainActor
final class FastSearchSetting: ObservableObject {
    private let defaults: UserDefaults
    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: UserDefaultsLaunchSettings.fastSearchKey) }
    }

    init?(defaults: UserDefaults, physicalMemory: UInt64) {
        guard CoreLaunch.fastSearchAvailable(physicalMemory: physicalMemory) else { return nil }
        self.defaults = defaults
        enabled = defaults.bool(forKey: UserDefaultsLaunchSettings.fastSearchKey)
    }
}

// MARK: - Speed check

/// "Check encryption speed" (plan Task 11 Step 0b): core times searches on this Mac's own
/// store; the numbers show here, and the text file is saved only where the user picks. No
/// network call anywhere: the user decides whether to send the file.
@MainActor
final class SpeedCheckModel: ObservableObject {
    struct Actions {
        /// Core's report JSON (`{"ok":...}`); progress 0...1 from any thread.
        var run: (_ progress: @escaping @Sendable (Float) -> Void) async -> String?
        /// The text file's contents for the report object (`CoreSpeedCheck.text`).
        var text: (String) -> String?
        /// Saves the text where the user chooses; nil when cancelled or failed.
        var save: @MainActor (String) -> URL?
        /// A meeting is being recorded: the check does not start (it would compete with it).
        var isRecording: () -> Bool = { false }
    }

    enum Phase: Equatable {
        case idle
        case running(Double)
        case done([String])
        case failed
        case blockedByRecording
    }

    @Published private(set) var phase: Phase = .idle
    @Published private(set) var savedPath: String?
    /// A cancelled check whose queries still run on the serial `accessQueue`: no new check
    /// starts until it returned.
    @Published private(set) var draining = false
    var canCheck: Bool {
        if case .running = phase { return false }
        return !draining
    }
    private var reportObject: String?
    private let actions: Actions
    /// Bumped by `cancel()`: a check whose number moved on drops its result.
    private var generation = 0

    init(actions: Actions) { self.actions = actions }

    func check() async {
        guard canCheck else { return }
        savedPath = nil
        reportObject = nil
        guard !actions.isRecording() else {
            phase = .blockedByRecording
            return
        }
        phase = .running(0)
        generation += 1
        let mine = generation
        let json = await actions.run { [weak self] fraction in
            Task { @MainActor in
                guard let self, self.generation == mine, case let .running(old) = self.phase, Double(fraction) > old else { return }
                self.phase = .running(Double(fraction))
            }
        }
        guard generation == mine, case .running = phase else {
            // A cancelled run came back: the queue is free again.
            draining = false
            return
        }
        guard let json, let data = json.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any], o["ok"] as? Bool == true,
              let report = o["report"] as? [String: Any],
              let reportData = try? JSONSerialization.data(withJSONObject: report) else {
            phase = .failed
            return
        }
        reportObject = String(data: reportData, encoding: .utf8)
        phase = .done(Self.lines(report))
    }

    /// Back to idle at once; the running check's numbers are dropped when it returns.
    func cancel() {
        guard case .running = phase else { return }
        generation += 1
        draining = true
        phase = .idle
    }

    func saveReport() {
        guard let reportObject, let text = actions.text(reportObject) else { return }
        savedPath = actions.save(text)?.path
    }

    /// The numbers as short lines (timings and the kind of Mac only).
    static func lines(_ r: [String: Any]) -> [String] {
        func num(_ k: String) -> Double? { (r[k] as? NSNumber)?.doubleValue }
        func ms(_ k: String) -> String { num(k).map { String(format: "%.1f", $0) } ?? EncryptionCopy.speedNotMeasured }
        var out = [r["encrypted"] as? Bool == true ? EncryptionCopy.speedEncrypted : EncryptionCopy.speedNotEncrypted]
        out.append(EncryptionCopy.speedOpen(num("open_ms").map { String(Int($0)) } ?? EncryptionCopy.speedNotMeasured))
        if r["too_little_data"] as? Bool == true {
            out.append(EncryptionCopy.speedNotEnoughData)
        } else {
            out.append(EncryptionCopy.speedKeyword(ms("keyword_p50_ms"), ms("keyword_p95_ms")))
            out.append(r["semantic_unavailable"] as? Bool == true
                       ? EncryptionCopy.speedSemanticUnavailable
                       : EncryptionCopy.speedSemantic(ms("semantic_p50_ms"), ms("semantic_p95_ms")))
        }
        out.append(EncryptionCopy.speedComputer(r["cpu_arch"] as? String ?? "?",
                                                num("ram_gib").map { String(Int($0)) } ?? EncryptionCopy.speedNotMeasured,
                                                r["os"] as? String ?? "?"))
        return out
    }
}

/// Saves the speed report where the user picks, then shows it in Finder.
enum SpeedReportPanel {
    @MainActor
    static func save(_ text: String) -> URL? {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = EncryptionCopy.speedReportFileName
        panel.allowedContentTypes = [.plainText]
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        guard (try? Data(text.utf8).write(to: url, options: .atomic)) != nil else { return nil }
        NSWorkspace.shared.activateFileViewerSelecting([url])
        return url
    }
}

// MARK: - Views

struct EncryptionNotesView: View {
    @ObservedObject var model: EncryptionNotesModel

    var body: some View {
        if !model.rows.isEmpty {
            VStack(alignment: .leading, spacing: Spacing.sm) {
                ForEach(model.rows) { row in
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        ForEach(row.text, id: \.self) { t in
                            Text(t).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextPrimary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        if let action = row.action {
                            Button(EncryptionNotesModel.title(action)) { model.perform(action) }
                                .buttonStyle(.whimbrelSecondary)
                                .disabled(model.cleanupRunning)
                            if let f = model.feedback[action] {
                                Text(f).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                            }
                        }
                    }
                    .padding(Spacing.md)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 8).stroke(Color.whimbrelWarning.opacity(0.6)))
                }
            }
        }
    }
}

struct FastSearchToggleView: View {
    @ObservedObject var setting: FastSearchSetting

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Toggle(EncryptionCopy.fastSearchToggle, isOn: $setting.enabled)
            Text(EncryptionCopy.fastSearchHelp).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct SpeedCheckView: View {
    @ObservedObject var model: SpeedCheckModel

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack {
                Button(EncryptionCopy.speedCheckAction) { Task { await model.check() } }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(!model.canCheck)
                if case .done = model.phase {
                    Button(EncryptionCopy.saveReportAction) { model.saveReport() }.buttonStyle(.whimbrelSecondary)
                }
                if case .running = model.phase {
                    Button(EncryptionCopy.cancel) { model.cancel() }.buttonStyle(.whimbrelSecondary)
                }
            }
            switch model.phase {
            case .idle:
                EmptyView()
            case let .running(f):
                Text(EncryptionCopy.speedChecking).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                ProgressView(value: f).frame(maxWidth: 320)
            case let .done(lines):
                ForEach(lines, id: \.self) { l in
                    Text(l).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextPrimary).monospacedDigit()
                }
            case .failed:
                Text(EncryptionCopy.speedCheckFailed).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            case .blockedByRecording:
                Text(EncryptionCopy.speedCheckRecording).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if let path = model.savedPath {
                Text(EncryptionCopy.savedTo(path)).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            Text(EncryptionCopy.speedCheckPrivacy).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
