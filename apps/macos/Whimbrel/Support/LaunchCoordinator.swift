import Foundation
import os
import Security

// The Mac launch coordinator (X4 encryption at rest, plan Task 11b-1, design 3.2/3.4/3.5/3.6).
//
// Core decides (`api::launch_decision`, over `whimbrel_launch_decision`); this type gathers
// the inputs, runs the returned plan step by step through injected collaborators, records
// what happened in `LaunchThisLaunch` and asks again after every step that changes the
// world. It never opens anything the plan did not say, never starts a migration while a
// recording is in progress, starts the MCP server only after an open and as the last
// launch-time step, and writes the good-launch counter only after `firstWindowRendered()`.
// No user-facing strings live here: screens and notes are typed values for the presenter.

// MARK: - The plan

/// Which key account an open or a migration uses (core `KeySource`).
enum LaunchKeySource: String, Equatable {
    case dbKey = "db_key"
    case pendingKey = "pending_key"
}

/// Why the recovery screen shows (core `RecoveryReason`).
enum LaunchRecoveryReason: String, Equatable {
    case needsKey = "needs_key"
    case keyRejected = "key_rejected"
}

/// Why the damage screen shows (core `DamageReason`).
enum LaunchDamageReason: String, Equatable {
    case damaged
    case noAccess = "no_access"
    case openElsewhere = "open_elsewhere"
}

/// Why the "encryption did not start" note shows (core `SkipNote`).
enum LaunchSkipReason: String, Equatable {
    /// The precheck or the run answered Skipped this launch; retried at the next launch.
    case precheck
    /// The migration state file cannot be read; "Reset encryption status" is the way out.
    case stateUnreadable = "state_unreadable"
}

/// One step of core's launch plan (`api::launch_decision::LaunchAction`), decoded from the
/// pinned `WHIMBREL_LAUNCH_*` code. A code or shape this build does not know is `unknown`,
/// and the coordinator stops on it rather than guess.
enum LaunchStep: Equatable {
    case openPlain
    case openPlainRecordKeyUnavailable
    case hardStopNoKeyStorage
    case showKeyUnavailableScreen
    case showRecoveryScreen(LaunchRecoveryReason)
    case showMigrationInterrupted
    case showDamaged(LaunchDamageReason)
    case showFailedBefore(attempts: UInt32)
    case showMigrationSkipped(LaunchSkipReason)
    case showMigrationFailed
    case newInstallOnboarding(reusePendingKey: Bool)
    case finishUndoViaResume
    case precheckMigration(recording: Bool)
    case runMigration(LaunchKeySource, recording: Bool)
    case openKeyed(LaunchKeySource, cacheKib: Int64?)
    case promotePendingKey
    case deletePendingKey
    case clearMigrateFlag
    case startCleanupCounter
    case recordGoodLaunch(count: UInt32)
    case cleanupPlainBackup(goodLaunches: UInt32)
    case resetCleanupCounter
    case startMcpServer
    case showEncryptionAvailable
    case showEncryptedFileMissing(backupPresent: Bool, encryptedCopyPresent: Bool)
    case showPlainBackupKept
    /// After the open: the library vanished and a new empty store opened (Task 13c item 4).
    case noticeLibraryMissing
    case unknown(code: Int)

    init(json o: [String: Any]) {
        let code = (o["code"] as? NSNumber)?.intValue ?? -1
        func bool(_ k: String) -> Bool? { (o[k] as? NSNumber).map(\.boolValue) }
        func u32(_ k: String) -> UInt32? { (o[k] as? NSNumber).map(\.uint32Value) }
        func source() -> LaunchKeySource? { (o["key_source"] as? String).flatMap(LaunchKeySource.init(rawValue:)) }
        let step: LaunchStep?
        switch Int32(truncatingIfNeeded: code) {
        case WHIMBREL_LAUNCH_OPEN_PLAIN: step = .openPlain
        case WHIMBREL_LAUNCH_OPEN_PLAIN_RECORD_KEY_UNAVAILABLE: step = .openPlainRecordKeyUnavailable
        case WHIMBREL_LAUNCH_HARD_STOP_NO_KEY_STORAGE: step = .hardStopNoKeyStorage
        case WHIMBREL_LAUNCH_SHOW_KEY_UNAVAILABLE_SCREEN: step = .showKeyUnavailableScreen
        case WHIMBREL_LAUNCH_SHOW_RECOVERY_SCREEN:
            step = (o["reason"] as? String).flatMap(LaunchRecoveryReason.init(rawValue:)).map { .showRecoveryScreen($0) }
        case WHIMBREL_LAUNCH_SHOW_MIGRATION_INTERRUPTED: step = .showMigrationInterrupted
        case WHIMBREL_LAUNCH_SHOW_DAMAGED:
            step = (o["reason"] as? String).flatMap(LaunchDamageReason.init(rawValue:)).map { .showDamaged($0) }
        case WHIMBREL_LAUNCH_SHOW_FAILED_BEFORE: step = u32("attempts").map { .showFailedBefore(attempts: $0) }
        case WHIMBREL_LAUNCH_SHOW_MIGRATION_SKIPPED:
            step = (o["reason"] as? String).flatMap(LaunchSkipReason.init(rawValue:)).map { .showMigrationSkipped($0) }
        case WHIMBREL_LAUNCH_SHOW_MIGRATION_FAILED: step = .showMigrationFailed
        case WHIMBREL_LAUNCH_NEW_INSTALL_ONBOARDING:
            step = bool("reuse_pending_key").map { .newInstallOnboarding(reusePendingKey: $0) }
        case WHIMBREL_LAUNCH_FINISH_UNDO_VIA_RESUME: step = .finishUndoViaResume
        case WHIMBREL_LAUNCH_PRECHECK_MIGRATION: step = bool("recording").map { .precheckMigration(recording: $0) }
        case WHIMBREL_LAUNCH_RUN_MIGRATION:
            if let s = source(), let r = bool("recording") { step = .runMigration(s, recording: r) } else { step = nil }
        case WHIMBREL_LAUNCH_OPEN_KEYED:
            if let s = source(), o.keys.contains("cache_kib") {
                step = .openKeyed(s, cacheKib: (o["cache_kib"] as? NSNumber)?.int64Value)
            } else {
                step = nil
            }
        case WHIMBREL_LAUNCH_PROMOTE_PENDING_KEY: step = .promotePendingKey
        case WHIMBREL_LAUNCH_DELETE_PENDING_KEY: step = .deletePendingKey
        case WHIMBREL_LAUNCH_CLEAR_MIGRATE_FLAG: step = .clearMigrateFlag
        case WHIMBREL_LAUNCH_START_CLEANUP_COUNTER: step = .startCleanupCounter
        case WHIMBREL_LAUNCH_RECORD_GOOD_LAUNCH: step = u32("count").map { .recordGoodLaunch(count: $0) }
        case WHIMBREL_LAUNCH_CLEANUP_PLAIN_BACKUP: step = u32("good_launches").map { .cleanupPlainBackup(goodLaunches: $0) }
        case WHIMBREL_LAUNCH_RESET_CLEANUP_COUNTER: step = .resetCleanupCounter
        case WHIMBREL_LAUNCH_START_MCP_SERVER: step = .startMcpServer
        case WHIMBREL_LAUNCH_SHOW_ENCRYPTION_AVAILABLE: step = .showEncryptionAvailable
        case WHIMBREL_LAUNCH_SHOW_ENCRYPTED_FILE_MISSING:
            if let b = bool("backup_present"), let c = bool("encrypted_copy_present") {
                step = .showEncryptedFileMissing(backupPresent: b, encryptedCopyPresent: c)
            } else {
                step = nil
            }
        case WHIMBREL_LAUNCH_SHOW_PLAIN_BACKUP_KEPT: step = .showPlainBackupKept
        case WHIMBREL_LAUNCH_NOTICE_LIBRARY_MISSING: step = .noticeLibraryMissing
        default: step = nil
        }
        self = step ?? .unknown(code: code)
    }

    /// The plan JSON `{"steps":[...]}`; nil when it is not that shape.
    static func plan(json: String) -> [LaunchStep]? {
        guard let o = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
              let steps = o["steps"] as? [[String: Any]] else { return nil }
        return steps.map(LaunchStep.init(json:))
    }

    /// The pinned code (for error screens and logs).
    var code: Int {
        switch self {
        case .openPlain: return Int(WHIMBREL_LAUNCH_OPEN_PLAIN)
        case .openPlainRecordKeyUnavailable: return Int(WHIMBREL_LAUNCH_OPEN_PLAIN_RECORD_KEY_UNAVAILABLE)
        case .hardStopNoKeyStorage: return Int(WHIMBREL_LAUNCH_HARD_STOP_NO_KEY_STORAGE)
        case .showKeyUnavailableScreen: return Int(WHIMBREL_LAUNCH_SHOW_KEY_UNAVAILABLE_SCREEN)
        case .showRecoveryScreen: return Int(WHIMBREL_LAUNCH_SHOW_RECOVERY_SCREEN)
        case .showMigrationInterrupted: return Int(WHIMBREL_LAUNCH_SHOW_MIGRATION_INTERRUPTED)
        case .showDamaged: return Int(WHIMBREL_LAUNCH_SHOW_DAMAGED)
        case .showFailedBefore: return Int(WHIMBREL_LAUNCH_SHOW_FAILED_BEFORE)
        case .showMigrationSkipped: return Int(WHIMBREL_LAUNCH_SHOW_MIGRATION_SKIPPED)
        case .showMigrationFailed: return Int(WHIMBREL_LAUNCH_SHOW_MIGRATION_FAILED)
        case .newInstallOnboarding: return Int(WHIMBREL_LAUNCH_NEW_INSTALL_ONBOARDING)
        case .finishUndoViaResume: return Int(WHIMBREL_LAUNCH_FINISH_UNDO_VIA_RESUME)
        case .precheckMigration: return Int(WHIMBREL_LAUNCH_PRECHECK_MIGRATION)
        case .runMigration: return Int(WHIMBREL_LAUNCH_RUN_MIGRATION)
        case .openKeyed: return Int(WHIMBREL_LAUNCH_OPEN_KEYED)
        case .promotePendingKey: return Int(WHIMBREL_LAUNCH_PROMOTE_PENDING_KEY)
        case .deletePendingKey: return Int(WHIMBREL_LAUNCH_DELETE_PENDING_KEY)
        case .clearMigrateFlag: return Int(WHIMBREL_LAUNCH_CLEAR_MIGRATE_FLAG)
        case .startCleanupCounter: return Int(WHIMBREL_LAUNCH_START_CLEANUP_COUNTER)
        case .recordGoodLaunch: return Int(WHIMBREL_LAUNCH_RECORD_GOOD_LAUNCH)
        case .cleanupPlainBackup: return Int(WHIMBREL_LAUNCH_CLEANUP_PLAIN_BACKUP)
        case .resetCleanupCounter: return Int(WHIMBREL_LAUNCH_RESET_CLEANUP_COUNTER)
        case .startMcpServer: return Int(WHIMBREL_LAUNCH_START_MCP_SERVER)
        case .showEncryptionAvailable: return Int(WHIMBREL_LAUNCH_SHOW_ENCRYPTION_AVAILABLE)
        case .showEncryptedFileMissing: return Int(WHIMBREL_LAUNCH_SHOW_ENCRYPTED_FILE_MISSING)
        case .showPlainBackupKept: return Int(WHIMBREL_LAUNCH_SHOW_PLAIN_BACKUP_KEPT)
        case .noticeLibraryMissing: return Int(WHIMBREL_LAUNCH_NOTICE_LIBRARY_MISSING)
        case let .unknown(code): return code
        }
    }
}

/// What already happened during this launch (core `ThisLaunch`). Every field is sent: core
/// rejects a missing or misspelt one rather than default it.
struct LaunchThisLaunch: Equatable {
    enum Resume: String { case notRun = "not_run", ran, failed }
    enum Migration: String { case notTried = "not_tried", skipped, failed, done }

    var resume: Resume = .notRun
    var migration: Migration = .notTried
    var dbKeyRejected = false
    var pendingKeyRejected = false
    var keyStoreFailed = false

    var json: [String: Any] {
        ["resume": resume.rawValue, "migration": migration.rawValue, "db_key_rejected": dbKeyRejected,
         "pending_key_rejected": pendingKeyRejected, "key_store_failed": keyStoreFailed]
    }
}

// MARK: - What the presenter sees

/// The result of a screen's file action, shown on the same screen when it did not finish.
enum LaunchFileActionResult: Equatable {
    case restore(CoreRestoreResult)
    case reset(CoreEncryptionFileAction)
}

/// Why the launch stopped with an error screen (never a guess at what to do next).
enum LaunchError: Equatable {
    /// `whimbrel_launch_observe_files` gave nothing.
    case filesUnreadable
    /// The decision answered NULL (unparseable input) or not a plan.
    case decisionUnavailable
    /// More than `maxDecisions` automatic decisions in one go.
    case tooManyDecisions
    /// A step code or shape this build does not know.
    case unknownStep(code: Int)
    /// A step in a place the plan contract forbids (the MCP server or key and counter
    /// bookkeeping before an open, a migration step after one).
    case stepOutOfOrder(code: Int)
    case plainOpenFailed
    /// The keyed open answered something other than opened or key rejected.
    case keyedOpenFailed(CoreOpenOutcome)
    /// A migration call was malformed (nothing was done).
    case migrationCallFailed
    /// The onboarding returned a key other than the pending key it was given.
    case onboardingKeyMismatch
    /// The plan asked for onboarding while this launch's key lookups say a key exists (or
    /// cannot be read): a fresh key is never made then.
    case freshKeyRefused
    /// The plan asked to open or migrate with a key this launch's lookup did not find (it
    /// went away between the lookup and the step). Never retried silently.
    case keyMissing(LaunchKeySource)
    /// A restored key is waiting to be tried, but the database file is missing: opening would
    /// CREATE a database, which proves nothing. Nothing is decided, opened or stored; the key
    /// is kept, and Retry looks at the files again (the key is tried once the file is back).
    case databaseMissingForRecoveredKey
}

/// Raw Keychain or Swift error text, for logs and support only. NEVER rendered: the UI
/// shows copy chosen from the typed value next to it (Task 11b-2).
struct LaunchDiagnostic: Equatable {
    let diagnosticOnly: String
}

/// Why a key write after the open did not happen (typed, for the UI's copy).
enum LaunchKeyFailure: Equatable {
    /// The Keychain refused or could not be read.
    case keychainUnavailable
    /// A different key is already stored and is never replaced without proof.
    case differentKeyStored
    /// Every `db-key-previous*` slot is taken: the recovery screen shows at every launch until
    /// a slot is freed (Delete everything frees them; "remove old keys" is a follow-up).
    case noPreviousSlot
    /// The pending key differs from the key the database opened with: kept, not deleted.
    case pendingDiffers
    /// The Keychain said the write was done but the read-back did not confirm it.
    case unverified
    /// The stored db-key changed after the database rejected it (another writer): nothing
    /// was replaced.
    case keyChangedMeanwhile
    case other

    init(_ error: Error) {
        switch error as? DbKeyStoreError {
        case .writeFailed?, .readFailed?, .randomFailed?: self = .keychainUnavailable
        case .mismatch?, .alreadyExists?, .notRejectedThisLaunch?: self = .differentKeyStored
        case .rejectedKeyChanged?: self = .keyChangedMeanwhile
        case .pendingNotOpened?: self = .pendingDiffers
        case .storeUnverified?: self = .unverified
        case .noPreviousSlot?: self = .noPreviousSlot
        default: self = .other
        }
    }
}

/// A screen the launch stops on until the user answers. Which answers each one offers is
/// `offers(_:)`; any other answer shows the screen again.
enum LaunchScreen: Equatable {
    /// New install and the Keychain cannot be used: "Whimbrel can't save its encryption key".
    /// Retry. Nothing was created.
    case keyStorageUnavailable
    /// An encrypted file and the Keychain did not answer (not the recovery screen). Retry.
    case keyUnavailable(LaunchDiagnostic)
    /// The recovery screen: answer `.restoredKey` with the key from the file or the words.
    /// The key is only kept in memory and tried by the plan's own keyed open; it is stored
    /// in the Keychain only after it opened the database. A wrong key shows this screen
    /// again with `.keyRejected`, and nothing was written.
    case recovery(LaunchRecoveryReason)
    /// An encryption upgrade was interrupted; Retry runs resume again.
    case migrationInterrupted
    /// The file cannot be used. `.noAccess` offers Retry (the files are looked at again,
    /// after the user fixed the permissions), as does `.openElsewhere` (another program
    /// holds the file); `.damaged` offers only Quit.
    case damaged(LaunchDamageReason)
    /// The database is missing: `backupPresent` offers `.restoreRollbackCopy` (confirmed by
    /// the user), neither copy offers `.resetEncryptionStatus`, only the encrypted copy offers
    /// nothing (support). `lastAction` is the previous press when it did not finish.
    case encryptedFileMissing(backupPresent: Bool, encryptedCopyPresent: Bool, lastAction: LaunchFileActionResult?)
    /// Something the coordinator will not guess about. Retry (a fresh start) or Quit.
    case error(LaunchError)

    func offers(_ r: LaunchScreenResponse) -> Bool {
        switch (self, r) {
        case (_, .quit): return true
        case (.keyStorageUnavailable, .retry), (.keyUnavailable, .retry), (.migrationInterrupted, .retry), (.error, .retry),
             (.damaged(.noAccess), .retry), (.damaged(.openElsewhere), .retry):
            return true
        case (.recovery, .restoredKey): return true
        // Only a failed PLAIN open may go on without the store (Task 11b-2 fix round 1,
        // pending Mike's decision): the app ran without a store before X4 in that case.
        case (.error(.plainOpenFailed), .continueWithoutStore): return true
        case let (.encryptedFileMissing(backup, _, _), .restoreRollbackCopy): return backup
        case let (.encryptedFileMissing(backup, copy, _), .resetEncryptionStatus): return !backup && !copy
        default: return false
        }
    }
}

/// The user's answer to a screen.
enum LaunchScreenResponse: Equatable {
    case retry
    case restoreRollbackCopy
    case resetEncryptionStatus
    /// The key the recovery screen restored (file plus passphrase, or the 24 words).
    case restoredKey(Data)
    /// "Continue without the memory database" (offered only after a failed plain open).
    case continueWithoutStore
    case quit
}

/// A Settings note after the open (no launch stops on these).
enum LaunchNote: Equatable {
    /// Opened plain; the Keychain did not answer, so encryption cannot start yet.
    case keyStoreUnavailable(LaunchDiagnostic)
    /// "Encryption could not be completed; your data is unchanged" with Retry
    /// (`retryAfterFailedBefore()`).
    case failedBefore(attempts: UInt32)
    /// The encryption did not start; `skip` is the precheck's reason when there was one.
    /// `.stateUnreadable` offers "Reset encryption status" (`resetEncryptionStatus()`).
    case migrationSkipped(LaunchSkipReason, skip: CoreEncryptionMigration.Skip?)
    /// This launch's attempt failed; retried at the next launch.
    case migrationFailed
    /// `db-key` exists from an earlier encryption; "Encrypt the database" reuses it.
    case encryptionAvailable
    /// The unencrypted copy is kept with no finished migration recorded: "Delete the
    /// unencrypted copy" (`deletePlainBackup()`).
    case plainBackupKept
    /// "Put back the unencrypted copy" finished: leftover files set aside, and its warning.
    case restoredCopy(movedAside: UInt32, warning: CoreRestoreWarning)
    /// The pending key could not be promoted or deleted after the open (retried next launch).
    case pendingKeyNotPromoted(LaunchKeyFailure, LaunchDiagnostic)
    case pendingKeyNotDeleted(LaunchKeyFailure, LaunchDiagnostic)
    /// The restored recovery key opened the database, but storing it as `db-key` failed.
    /// The store is open; the next launch asks for the recovery key again.
    case recoveredKeyNotStored(LaunchKeyFailure, LaunchDiagnostic)
    /// The restored key opened the database and the Keychain reported it stored, but the
    /// read-back did not confirm it: it may or may not be stored (the next launch shows).
    case recoveredKeyStoreUnverified(LaunchDiagnostic)
    /// The plain store could not be opened and the user went on without it.
    case memoryDatabaseUnavailable
    /// No database and no copy where a store had been: a new empty one opened. Shown ONCE as
    /// a notice in the main window (`LibraryMissingNoticeHost`) until dismissed.
    case libraryMissing

    /// Notes about what this launch did (the decision cannot repeat them).
    var isLaunchOnly: Bool {
        switch self {
        case .memoryDatabaseUnavailable, .libraryMissing, .keyStoreUnavailable, .restoredCopy, .pendingKeyNotPromoted, .pendingKeyNotDeleted, .recoveredKeyNotStored,
             .recoveredKeyStoreUnverified:
            return true
        default:
            return false
        }
    }
}

/// What the launch produced: the opened main store (its key, nil for plain, and how long
/// the open took, for the speed check), or the user quit on a screen.
struct LaunchOpened<Store> {
    let store: Store
    let key: Data?
    let openMs: UInt64
}

enum LaunchOutcome<Store> {
    case opened(LaunchOpened<Store>)
    /// The plain open failed and the user chose to go on without the store: no MCP server,
    /// no good-launch bookkeeping, the note `.memoryDatabaseUnavailable`.
    case openedWithoutStore
    case quit
}

// MARK: - Collaborators

/// The Keychain as the launch needs it. `DbKeyStore` is the live one.
protocol LaunchKeyStore {
    func lookup() -> DbKeyLookup
    func lookupPending() -> DbKeyLookup
    /// Onboarding with a fresh key: kept as `db-key-pending` first, so a failed `db-key`
    /// write keeps the key the user's recovery words belong to.
    func storePending(_ key: Data) throws
    func storeKey(_ key: Data) throws
    /// A key that has just OPENED the existing database, stored as `db-key`. With no `db-key`
    /// it is a plain store. A different `db-key` is replaced only when `rejected` holds
    /// exactly its bytes (the Keychain db-key the database refused this launch; nil when none
    /// was refused); the old key is kept under `db-key-previous*`, never destroyed. Live:
    /// `DbKeyStore.replaceAfterRecovery`.
    func replaceAfterRecovery(proven key: Data, replacing rejected: Data?) throws
    /// `PromotePendingKey`: `opened` is the key the store just opened with (must equal the
    /// stored pending key); same `rejected` rule.
    func promotePendingKey(openedWith opened: Data, replacing rejected: Data?) throws
    func deletePending() throws
    /// The migration's key-store callback (0 or an OSStatus).
    func migrationWrite(_ key: Data, account: String) -> OSStatus
}

extension DbKeyStore: LaunchKeyStore {
    func storePending(_ key: Data) throws { try store(key, account: Self.pendingAccount) }
    func storeKey(_ key: Data) throws { try store(key, account: Self.keyAccount) }
}

/// The shell's own launch settings. Core has no settings store before the database opens,
/// so the "migrate at next launch" flag and the good-launch counter live here.
protocol LaunchSettingsStore: AnyObject {
    var fastSearch: Bool { get }
    var migrateAtNextLaunch: Bool { get set }
    var goodLaunches: UInt32 { get set }
}

/// `LaunchSettingsStore` in `UserDefaults` (device-local, never synced).
final class UserDefaultsLaunchSettings: LaunchSettingsStore {
    /// Core's pinned key (`api::settings::FAST_SEARCH_KEY`).
    static let fastSearchKey = "storage.fastSearch"
    static let migrateAtNextLaunchKey = "encryption.migrateAtNextLaunch"
    static let goodLaunchesKey = "encryption.goodLaunches"

    private let defaults: UserDefaults
    init(defaults: UserDefaults = .standard) { self.defaults = defaults }

    var fastSearch: Bool { defaults.bool(forKey: Self.fastSearchKey) }
    var migrateAtNextLaunch: Bool {
        get { defaults.bool(forKey: Self.migrateAtNextLaunchKey) }
        set { defaults.set(newValue, forKey: Self.migrateAtNextLaunchKey) }
    }
    var goodLaunches: UInt32 {
        get { UInt32(clamping: max(defaults.integer(forKey: Self.goodLaunchesKey), 0)) }
        set { defaults.set(Int(newValue), forKey: Self.goodLaunchesKey) }
    }
}

/// The migration and the launch screens' file actions. `CoreLaunchMigrationRunner` is the
/// live one (blocking C calls, run off the main thread by the async ones).
protocol LaunchMigrationRunner {
    func resume() async -> CoreEncryptionMigration.Outcome?
    func precheck(recording: Bool) async -> CoreEncryptionMigration.Outcome?
    func run(key: Data, recording: Bool, storeKey: @escaping (String, Data) -> OSStatus,
             progress: @escaping @Sendable (CoreEncryptionMigration.Progress) -> Void) async -> CoreEncryptionMigration.Outcome?
    /// Deletes a file: async so it never blocks the main actor (`firstWindowRendered`).
    func cleanupPlainBackup(goodLaunches: UInt32) async -> Bool
    func resetFailures() -> CoreEncryptionMigration.Outcome?
    func restoreRollbackCopy() -> CoreRestoreResult
    func resetEncryptionStatus() -> CoreEncryptionFileAction
    func deletePlainBackup(openedWithKeyThisLaunch: Bool) -> (CoreEncryptionFileAction, removed: UInt32)
}

struct CoreLaunchMigrationRunner: LaunchMigrationRunner {
    let dbPath: URL

    func resume() async -> CoreEncryptionMigration.Outcome? {
        let p = dbPath
        return await Task.detached { CoreEncryptionMigration.resume(dbPath: p) }.value
    }
    func precheck(recording: Bool) async -> CoreEncryptionMigration.Outcome? {
        let p = dbPath
        return await Task.detached { CoreEncryptionMigration.precheck(dbPath: p, recording: recording) }.value
    }
    func run(key: Data, recording: Bool, storeKey: @escaping (String, Data) -> OSStatus,
             progress: @escaping @Sendable (CoreEncryptionMigration.Progress) -> Void) async -> CoreEncryptionMigration.Outcome? {
        let p = dbPath
        return await Task.detached {
            CoreEncryptionMigration.run(dbPath: p, recording: recording, key: key, storeKey: storeKey, progress: progress)
        }.value
    }
    func cleanupPlainBackup(goodLaunches: UInt32) async -> Bool {
        let p = dbPath
        return await Task.detached { CoreEncryptionMigration.cleanupPlainBackup(dbPath: p, goodLaunches: goodLaunches) }.value
    }
    func resetFailures() -> CoreEncryptionMigration.Outcome? { CoreEncryptionMigration.resetFailures(dbPath: dbPath) }
    func restoreRollbackCopy() -> CoreRestoreResult { CoreEncryptionMigration.restoreRollbackCopy(dbPath: dbPath) }
    func resetEncryptionStatus() -> CoreEncryptionFileAction { CoreEncryptionMigration.resetEncryptionStatus(dbPath: dbPath) }
    func deletePlainBackup(openedWithKeyThisLaunch: Bool) -> (CoreEncryptionFileAction, removed: UInt32) {
        CoreEncryptionMigration.deletePlainBackup(dbPath: dbPath, openedWithKeyThisLaunch: openedWithKeyThisLaunch)
    }
}

/// The UI side of the launch (Task 11b-2 implements it with the Encryption* screens).
protocol LaunchPresenter: AnyObject {
    /// Show `screen` and return the user's answer.
    func present(_ screen: LaunchScreen) async -> LaunchScreenResponse
    /// New install: run the recovery setup (`EncryptionSetupModel` with
    /// `Actions.live(existingKey:)`) and return the setup's key once the user continues, or
    /// nil if they quit. With `existingKey` (the pending key) the setup must wrap that key.
    func runOnboarding(existingKey: Data?) async -> Data?
    /// Migration progress. Any thread (a Rust helper thread during the copy): hop to the
    /// main queue for UI.
    func migrationProgress(_ progress: CoreEncryptionMigration.Progress)
}

// MARK: - The coordinator

/// Notes reach the UI through ONE channel: the observable `notes` property (Settings
/// observes the coordinator). Each note's button calls the coordinator
/// (`deletePlainBackup()`, `resetEncryptionStatus()`, `retryAfterFailedBefore()`).
///
/// `startMcpServer` runs inside `run()`, before `run()` returns: the closure must use the key
/// it is handed, not state set after `run()`.
@MainActor
final class LaunchCoordinator<Store>: ObservableObject {
    struct Collaborators {
        /// `files` JSON (`CoreLaunch.observeFiles`), passed to the decision untouched.
        var observeFiles: () -> String?
        /// Input JSON -> plan JSON (`CoreLaunch.decide`).
        var decide: (String) -> String?
        var keys: LaunchKeyStore
        var settings: LaunchSettingsStore
        var physicalMemory: () -> UInt64
        /// A meeting is being recorded now. Read for the input, again for the precheck, and
        /// again right before a migration would start.
        var recording: () -> Bool
        var migration: LaunchMigrationRunner
        var openPlain: () async -> Store?
        /// The main store's keyed open with the plan's cache (nil: SQLite's default).
        /// `mustExist`: never create (`whimbrel_memory_open_keyed_existing`); set only when the
        /// open is the proof of a restored recovery key. A missing file is then `.damaged`.
        var openKeyed: (_ key: Data, _ cacheKib: Int64?, _ mustExist: Bool) async -> (CoreOpenOutcome, Store?)
        /// The MCP server, given the key the main store opened with (nil for plain). Never a
        /// cache: only the main store gets one.
        var startMcpServer: (_ key: Data?) -> Void
        var presenter: LaunchPresenter
        /// Milliseconds on a monotonic clock (the open time).
        var now: () -> UInt64
        /// Records that a store was opened here (`store_existed`, Task 13c item 4): after every
        /// launch that opened the store, never one that went on without it or quit.
        var markStoreExisted: () -> Void = {}
        /// Removes that marker at the "library not found" notice step (written again after the
        /// open, so the notice shows once).
        var clearStoreExisted: () -> Void = {}
    }

    /// Automatic decisions in one go before the error screen (core settles within a few;
    /// `core/ffi/tests/launch_e2e.rs` caps at 8 too). A user's answer starts a fresh count.
    static var maxDecisions: Int { 8 }

    private let c: Collaborators
    private(set) var thisLaunch = LaunchThisLaunch()
    /// The Settings notes of this launch (after the open), refreshed after a note's action.
    @Published private(set) var notes: [LaunchNote] = []
    /// A key the recovery screen restored: in memory only, tried by the plan's own keyed open
    /// as `db_key`, and stored only once it opened the database (prove before persist).
    private var candidateKey: Data?
    /// The bytes of the Keychain's own `db-key` that the database refused during this launch:
    /// the proof `replaceAfterRecovery` and `promotePendingKey` need, checked against the
    /// stored item. Cleared once a replacement succeeded.
    private var rejectedDbKey: Data?
    /// The deferred backup cleanup is running: note actions wait (they do nothing meanwhile).
    @Published private(set) var cleanupRunning = false
    /// Notes the decision cannot repeat (they describe what this launch did).
    private var launchNotes: [LaunchNote] = []
    private var deferred: [LaunchStep] = []
    private var deferredDone = false
    private var openedWithKey = false
    private var opened = false
    private var lastSkip: CoreEncryptionMigration.Skip?
    /// A fresh onboarding key whose Keychain write failed this launch (kept in memory only,
    /// so a Retry stores the same key instead of making new words).
    private var unsavedOnboardingKey: Data?

    init(_ collaborators: Collaborators) {
        c = collaborators
    }

    /// The input core decides on (presence only, never key bytes).
    private func input(files: Any, db: DbKeyLookup, pending: DbKeyLookup) -> String? {
        let o: [String: Any] = [
            "files": files,
            "db_key": Self.presence(db),
            "pending_key": Self.presence(pending),
            "migrate_at_next_launch": c.settings.migrateAtNextLaunch,
            "recording": c.recording(),
            "settings": ["fast_search": c.settings.fastSearch],
            "physical_ram_bytes": c.physicalMemory(),
            "good_launches": c.settings.goodLaunches,
            "this_launch": thisLaunch.json,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: o) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private static func presence(_ l: DbKeyLookup) -> String {
        switch l {
        case .found: return "found"
        case .absent: return "absent"
        case .unavailable: return "unavailable"
        }
    }

    private static func bytes(_ l: DbKeyLookup) -> Data? {
        if case let .found(k) = l { return k }
        return nil
    }

    private static func detail(_ db: DbKeyLookup, _ pending: DbKeyLookup) -> LaunchDiagnostic {
        if case let .unavailable(m) = db { return LaunchDiagnostic(diagnosticOnly: m) }
        if case let .unavailable(m) = pending { return LaunchDiagnostic(diagnosticOnly: m) }
        return LaunchDiagnostic(diagnosticOnly: "")
    }

    /// One ask: observe, look up both keys, decide. Nil with the error to show.
    private func ask() -> (steps: [LaunchStep], db: DbKeyLookup, pending: DbKeyLookup)? {
        guard let filesJSON = c.observeFiles(),
              let files = try? JSONSerialization.jsonObject(with: Data(filesJSON.utf8)) else {
            failure = .filesUnreadable
            return nil
        }
        if candidateKey != nil {
            switch Self.candidateUse(files as? [String: Any]) {
            case .tryIt:
                break
            case .wait:
                // A restored key waits while the database file is missing and nothing else is
                // there: core would plan a CREATING open with it. Nothing is decided, opened
                // or written; the key is KEPT and tried once the file is back (the screen's
                // Retry only looks at the files again).
                failure = .databaseMissingForRecoveredKey
                return nil
            case .drop:
                // Any other state (a plain file, leftovers of an encryption upgrade, an
                // unfinished run, an unreadable state, a damaged file): the unproven key is
                // dropped, never written, and core decides from the Keychain as before (the
                // user re-enters the recovery key if it is still needed).
                candidateKey = nil
                thisLaunch.dbKeyRejected = rejectedDbKey != nil
            }
        }
        // A restored, not yet proven key stands in for db-key, only on an encrypted file
        // (never written before it opens there).
        let keychainDb = c.keys.lookup()
        let db = candidateKey.map(DbKeyLookup.found) ?? keychainDb
        let pending = c.keys.lookupPending()
        guard let json = input(files: files, db: db, pending: pending),
              let planJSON = c.decide(json), let steps = LaunchStep.plan(json: planJSON) else {
            failure = .decisionUnavailable
            return nil
        }
        return (steps, db, pending)
    }

    private enum CandidateUse { case tryIt, wait, drop }

    /// Stores a restored key that has just opened the existing database (the old db-key, if
    /// any, kept under db-key-previous*). Returns the note when storing it failed.
    private func persistProvenCandidate(_ key: Data) -> LaunchNote? {
        candidateKey = nil
        do {
            try c.keys.replaceAfterRecovery(proven: key, replacing: rejectedDbKey)
            rejectedDbKey = nil
            return nil
        } catch DbKeyStoreError.storeUnverified(let m) {
            return .recoveredKeyStoreUnverified(LaunchDiagnostic(diagnosticOnly: m))
        } catch {
            return .recoveredKeyNotStored(LaunchKeyFailure(error), LaunchDiagnostic(diagnosticOnly: "\(error)"))
        }
    }

    /// What a waiting restored key may do with these files: stand in for `db_key` only on an
    /// encrypted file (`not_plain`); wait only for a missing file with nothing beside it (no
    /// `.plain.bak`, no `.enc.tmp`, no unfinished run, a readable state); else be dropped.
    private static func candidateUse(_ f: [String: Any]?) -> CandidateUse {
        let dbFile = f?["db_file"] as? String
        if dbFile == "not_plain" { return .tryIt }
        let kind = (f?["migration"] as? [String: Any])?["kind"] as? String
        let leftovers = (f?["plain_backup_exists"] as? Bool) != false || (f?["encrypted_tmp_exists"] as? Bool) != false
            || kind == "in_progress" || kind == "unreadable" || kind == nil
        return dbFile == "missing" && !leftovers ? .wait : .drop
    }

    private var failure: LaunchError?

    /// Runs the launch to an open or a quit.
    func run() async -> LaunchOutcome<Store> {
        var budget = 0
        var lastFileAction: LaunchFileActionResult?
        launch: while true {
            budget += 1
            if budget > Self.maxDecisions {
                if await stop(.tooManyDecisions) { budget = 0; continue launch }
                return .quit
            }
            guard let (steps, db, pending) = ask() else {
                if await stop(failure ?? .decisionUnavailable) { budget = 0; continue launch }
                return .quit
            }
            var store: Store?
            var storeKey: Data?
            var openMs: UInt64 = 0
            var planNotes: [LaunchNote] = []
            var planDeferred: [LaunchStep] = []

            // The whole plan is checked before any of it runs, so a bad step can never come
            // after keys were promoted or a store was opened.
            if let bad = Self.validate(steps) {
                if await stop(bad) { budget = 0; continue launch }
                return .quit
            }

            for step in steps {
                switch step {
                case .finishUndoViaResume:
                    let out = await c.migration.resume()
                    thisLaunch.resume = out?.ok == true ? .ran : .failed
                    continue launch

                case let .precheckMigration(recording):
                    let out = await c.migration.precheck(recording: recording || c.recording())
                    if out?.ok != true {
                        lastSkip = out?.error?.skip
                        thisLaunch.migration = .skipped
                        continue launch
                    }

                case let .runMigration(source, recording):
                    // Never start a migration while a meeting is being recorded, whatever the
                    // plan was decided on.
                    if recording || c.recording() {
                        lastSkip = CoreEncryptionMigration.Skip(reason: "recording", needBytes: nil, freeBytes: nil,
                                                                phase: nil, attempts: nil, last: nil)
                        thisLaunch.migration = .skipped
                        continue launch
                    }
                    guard let key = Self.bytes(source == .dbKey ? db : pending) else {
                        if await stop(.keyMissing(source)) { budget = 0; continue launch }
                        return .quit
                    }
                    let keys = c.keys
                    let presenter = c.presenter
                    let out = await c.migration.run(
                        key: key, recording: false,
                        storeKey: { account, bytes in keys.migrationWrite(bytes, account: account) },
                        progress: { presenter.migrationProgress($0) })
                    guard let out else {
                        if await stop(.migrationCallFailed) { budget = 0; continue launch }
                        return .quit
                    }
                    if out.ok, out.phase == "done" {
                        thisLaunch.migration = .done
                        // A new encrypted file, made with this key and stored as db-key by the
                        // run: rejections recorded earlier this launch were about another file.
                        thisLaunch.dbKeyRejected = false
                        thisLaunch.pendingKeyRejected = false
                        rejectedDbKey = nil
                    } else if out.error?.kind == "skipped" {
                        lastSkip = out.error?.skip
                        thisLaunch.migration = .skipped
                    } else {
                        thisLaunch.migration = .failed
                    }
                    continue launch

                case let .newInstallOnboarding(reuse):
                    // Never a fresh key while either key exists or cannot be read (checked
                    // here too, not only by the decision): the user would save recovery
                    // material for a key that is never used.
                    let freshAllowed = db == .absent && pending == .absent
                    let reuseAllowed = db == .absent && Self.bytes(pending) != nil
                    guard reuse ? reuseAllowed : freshAllowed else {
                        if await stop(.freshKeyRefused) { budget = 0; continue launch }
                        return .quit
                    }
                    let key: Data
                    if !reuse, let unsaved = unsavedOnboardingKey {
                        // The setup already ran this launch and only the Keychain write
                        // failed: retry the writes with that key; the words the user saved
                        // stay valid.
                        key = unsaved
                    } else {
                        let existing = reuse ? Self.bytes(pending) : nil
                        guard let k = await c.presenter.runOnboarding(existingKey: existing) else { return .quit }
                        if reuse && k != existing {
                            if await stop(.onboardingKeyMismatch) { budget = 0; continue launch }
                            return .quit
                        }
                        key = k
                    }
                    do {
                        if !reuse { try c.keys.storePending(key) }
                        try c.keys.storeKey(key)
                        unsavedOnboardingKey = nil
                    } catch {
                        if !reuse { unsavedOnboardingKey = key }
                        thisLaunch.keyStoreFailed = true
                    }
                    continue launch

                case .openPlain, .openPlainRecordKeyUnavailable:
                    let started = c.now()
                    guard let s = await c.openPlain() else {
                        var response = await c.presenter.present(.error(.plainOpenFailed))
                        while !LaunchScreen.error(.plainOpenFailed).offers(response) {
                            response = await c.presenter.present(.error(.plainOpenFailed))
                        }
                        switch response {
                        case .retry: budget = 0; continue launch
                        case .continueWithoutStore:
                            // Nothing after the open runs: no keys, no counter, no MCP server.
                            notes = launchNotes + [.memoryDatabaseUnavailable]
                            return .openedWithoutStore
                        default: return .quit
                        }
                    }
                    openMs = c.now() &- started
                    store = s
                    if step == .openPlainRecordKeyUnavailable {
                        planNotes.append(.keyStoreUnavailable(Self.detail(db, pending)))
                    }

                case let .openKeyed(source, cache):
                    guard let key = Self.bytes(source == .dbKey ? db : pending) else {
                        if await stop(.keyMissing(source)) { budget = 0; continue launch }
                        return .quit
                    }
                    let trialOfCandidate = source == .dbKey && candidateKey != nil && key == candidateKey
                    // `ask()` never plans with a candidate on a missing file; the proof open
                    // itself also never creates (`mustExist`), closing the race in between.
                    let started = c.now()
                    let (outcome, s) = await c.openKeyed(key, cache, trialOfCandidate)
                    if outcome == .opened, let s {
                        openMs = c.now() &- started
                        store = s
                        storeKey = key
                        if trialOfCandidate {
                            // Proven: it opened the existing database. Only now is it stored
                            // (the old db-key, if any, is kept under db-key-previous*).
                            if let note = persistProvenCandidate(key) { planNotes.append(note) }
                        }
                    } else if case .keyRejected = outcome {
                        if source == .dbKey {
                            thisLaunch.dbKeyRejected = true
                            // A wrong restored key is dropped with nothing written; the
                            // Keychain's own db-key is now proven wrong (by its bytes).
                            if trialOfCandidate { candidateKey = nil } else { rejectedDbKey = key }
                        } else {
                            thisLaunch.pendingKeyRejected = true
                        }
                        continue launch
                    } else {
                        if trialOfCandidate {
                            if case .startupFailed = outcome {
                                // The key opened the file (only the startup sweep failed): it
                                // is proven, so it is stored rather than lost.
                                if let note = persistProvenCandidate(key) { launchNotes.append(note) }
                            } else {
                                // The file vanished between the look and the open (the
                                // never-create open answered "no database file"): keep the
                                // key and ask again, where `ask()` waits or drops it by what
                                // is on disk now. Anything else drops it.
                                let goneNow = c.observeFiles().flatMap {
                                    try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any]
                                }?["db_file"] as? String == "missing"
                                if goneNow { continue launch }
                                candidateKey = nil
                            }
                        }
                        if await stop(.keyedOpenFailed(outcome)) { budget = 0; continue launch }
                        return .quit
                    }

                case .promotePendingKey:
                    do {
                        // `validate` guarantees the open was OpenKeyed(pending_key): storeKey is
                        // the pending key's bytes.
                        try c.keys.promotePendingKey(openedWith: storeKey ?? Data(), replacing: rejectedDbKey)
                        rejectedDbKey = nil
                    } catch {
                        planNotes.append(.pendingKeyNotPromoted(LaunchKeyFailure(error), LaunchDiagnostic(diagnosticOnly: "\(error)")))
                    }
                case .deletePendingKey:
                    // A key is never destroyed: the pending copy goes only when it is the very
                    // key the database just opened with (then it lives on under db-key).
                    if Self.bytes(pending) != storeKey {
                        planNotes.append(.pendingKeyNotDeleted(.pendingDiffers, LaunchDiagnostic(diagnosticOnly: "")))
                    } else {
                        do { try c.keys.deletePending() } catch {
                            planNotes.append(.pendingKeyNotDeleted(LaunchKeyFailure(error), LaunchDiagnostic(diagnosticOnly: "\(error)")))
                        }
                    }
                case .clearMigrateFlag:
                    c.settings.migrateAtNextLaunch = false
                case .startCleanupCounter, .recordGoodLaunch, .cleanupPlainBackup, .resetCleanupCounter:
                    planDeferred.append(step)
                case .startMcpServer:
                    c.startMcpServer(storeKey)

                case let .showFailedBefore(attempts): planNotes.append(.failedBefore(attempts: attempts))
                case let .showMigrationSkipped(reason):
                    planNotes.append(.migrationSkipped(reason, skip: reason == .precheck ? lastSkip : nil))
                case .showMigrationFailed: planNotes.append(.migrationFailed)
                case .showEncryptionAvailable: planNotes.append(.encryptionAvailable)
                case .showPlainBackupKept: planNotes.append(.plainBackupKept)
                case .noticeLibraryMissing:
                    // Shown once: the marker goes now and is written again after the open.
                    c.clearStoreExisted()
                    planNotes.append(.libraryMissing)

                case .hardStopNoKeyStorage, .showKeyUnavailableScreen, .showRecoveryScreen, .showMigrationInterrupted,
                     .showDamaged, .showEncryptedFileMissing:
                    let screen = Self.screen(step, db: db, pending: pending, lastAction: lastFileAction)
                    lastFileAction = nil
                    var response = await c.presenter.present(screen)
                    while !screen.offers(response) { response = await c.presenter.present(screen) }
                    budget = 0
                    switch response {
                    case .quit, .continueWithoutStore: // never offered by a plan's screen
                        return .quit
                    case .retry:
                        if step == .showMigrationInterrupted { thisLaunch.resume = .notRun }
                        if step == .hardStopNoKeyStorage { thisLaunch.keyStoreFailed = false }
                    case let .restoredKey(key):
                        // Prove before persist: nothing is written here. The next ask sees
                        // the key as db-key, and the plan's own keyed open tries it.
                        candidateKey = key
                        thisLaunch.dbKeyRejected = false
                    case .restoreRollbackCopy:
                        let r = c.migration.restoreRollbackCopy()
                        if r.action == .done {
                            launchNotes.append(.restoredCopy(movedAside: r.movedAside, warning: r.warning))
                            // The world changed: a resume that failed earlier is no reason to
                            // show the same screen again.
                            thisLaunch.resume = .notRun
                        } else {
                            lastFileAction = .restore(r)
                        }
                    case .resetEncryptionStatus:
                        let r = c.migration.resetEncryptionStatus()
                        if r == .done {
                            // The unfinished run's state is aside: core would otherwise keep
                            // deciding from "resume failed" and show this screen again.
                            thisLaunch.resume = .notRun
                            // The user chose an empty start: no "library not found" notice
                            // right after it (Task 13c fix C).
                            c.clearStoreExisted()
                        } else {
                            lastFileAction = .reset(r)
                        }
                    }
                    continue launch

                case let .unknown(code):
                    if await stop(.unknownStep(code: code)) { budget = 0; continue launch }
                    return .quit
                }
            }

            guard let store else {
                if await stop(.decisionUnavailable) { budget = 0; continue launch }
                return .quit
            }
            opened = true
            openedWithKey = storeKey != nil
            deferred = planDeferred
            notes = launchNotes + planNotes
            // A store was opened here: a later launch that finds neither it nor a copy says so.
            c.markStoreExisted()
            return .opened(LaunchOpened(store: store, key: storeKey, openMs: openMs))
        }
    }

    /// The plan contract, checked before anything runs: every code known; bookkeeping, notes
    /// and the MCP server only after an open; nothing that resumes, migrates, onboards, opens
    /// or shows a screen after one; nothing after the MCP server.
    static func validate(_ steps: [LaunchStep]) -> LaunchError? {
        var open = false
        var openedWithPending = false
        var mcp = false
        for s in steps {
            if case let .unknown(code) = s { return .unknownStep(code: code) }
            if mcp || (!open && needsOpen(s)) || (open && beforeOpenOnly(s)) { return .stepOutOfOrder(code: s.code) }
            // Only the pending key the store just opened with is promoted.
            if s == .promotePendingKey && !openedWithPending { return .stepOutOfOrder(code: s.code) }
            switch s {
            case let .openKeyed(source, _): open = true; openedWithPending = source == .pendingKey
            case .openPlain, .openPlainRecordKeyUnavailable: open = true
            case .startMcpServer: mcp = true
            default: break
            }
        }
        return nil
    }

    /// Steps that only make sense after the store opened.
    private static func needsOpen(_ s: LaunchStep) -> Bool {
        switch s {
        case .promotePendingKey, .deletePendingKey, .clearMigrateFlag, .startCleanupCounter, .recordGoodLaunch,
             .cleanupPlainBackup, .resetCleanupCounter, .startMcpServer, .showFailedBefore, .showMigrationSkipped,
             .showMigrationFailed, .showEncryptionAvailable, .showPlainBackupKept, .noticeLibraryMissing:
            return true
        default:
            return false
        }
    }

    /// Steps that must never run once a store is open (resume, migration, onboarding, a
    /// second open, a screen).
    private static func beforeOpenOnly(_ s: LaunchStep) -> Bool {
        switch s {
        case .finishUndoViaResume, .precheckMigration, .runMigration, .newInstallOnboarding, .openPlain,
             .openPlainRecordKeyUnavailable, .openKeyed, .hardStopNoKeyStorage, .showKeyUnavailableScreen,
             .showRecoveryScreen, .showMigrationInterrupted, .showDamaged, .showEncryptedFileMissing:
            return true
        default:
            return false
        }
    }

    private static func screen(_ step: LaunchStep, db: DbKeyLookup, pending: DbKeyLookup,
                               lastAction: LaunchFileActionResult?) -> LaunchScreen {
        switch step {
        case .hardStopNoKeyStorage: return .keyStorageUnavailable
        case .showKeyUnavailableScreen: return .keyUnavailable(detail(db, pending))
        case let .showRecoveryScreen(reason): return .recovery(reason)
        case .showMigrationInterrupted: return .migrationInterrupted
        case let .showDamaged(reason): return .damaged(reason)
        case let .showEncryptedFileMissing(backup, copy):
            return .encryptedFileMissing(backupPresent: backup, encryptedCopyPresent: copy, lastAction: lastAction)
        default: return .error(.unknownStep(code: step.code))
        }
    }

    /// Shows the error screen; true when the user asked to try again.
    private func stop(_ error: LaunchError) async -> Bool {
        var response = await c.presenter.present(.error(error))
        while !LaunchScreen.error(error).offers(response) { response = await c.presenter.present(.error(error)) }
        return response == .retry
    }

    // MARK: After the open

    /// The first window has rendered: write the good-launch counter (and run the backup
    /// cleanup it allows), once. Never called means never counted.
    /// The file delete of the cleanup runs off the main actor.
    func firstWindowRendered() async {
        guard opened, !deferredDone else { return }
        deferredDone = true
        let steps = deferred
        deferred = []
        cleanupRunning = true
        defer { cleanupRunning = false }
        for step in steps {
            switch step {
            case .startCleanupCounter: c.settings.goodLaunches = 1
            case let .recordGoodLaunch(count): c.settings.goodLaunches = count
            case let .cleanupPlainBackup(n):
                if await c.migration.cleanupPlainBackup(goodLaunches: n) { c.settings.goodLaunches = 0 }
            case .resetCleanupCounter: c.settings.goodLaunches = 0
            default: break
            }
        }
    }

    /// The "library not found" notice was dismissed: it is gone for this launch (the marker
    /// was already written again, so no later launch shows it).
    func dismissLibraryMissingNotice() {
        notes.removeAll { $0 == .libraryMissing }
    }

    /// "Delete the unencrypted copy" (note `.plainBackupKept`), then the notes are asked again.
    @discardableResult
    func deletePlainBackup() -> CoreEncryptionFileAction {
        // The note actions do nothing while the deferred cleanup (which may delete the same
        // backup) is running; the UI disables their buttons while `cleanupRunning`.
        guard !cleanupRunning else { return .refused }
        let (r, _) = c.migration.deletePlainBackup(openedWithKeyThisLaunch: openedWithKey)
        refreshNotes()
        return r
    }

    /// "Reset encryption status" (note `.migrationSkipped(.stateUnreadable)`), then the notes
    /// are asked again.
    @discardableResult
    func resetEncryptionStatus() -> CoreEncryptionFileAction {
        guard !cleanupRunning else { return .refused }
        let r = c.migration.resetEncryptionStatus()
        refreshNotes()
        return r
    }

    /// Retry after `.failedBefore`: forget the failures and keep the opt-in, so the migration
    /// runs at the next launch (never in this one: the store is open).
    @discardableResult
    func retryAfterFailedBefore() -> Bool {
        guard !cleanupRunning else { return false }
        let ok = c.migration.resetFailures()?.ok == true
        if ok { c.settings.migrateAtNextLaunch = true }
        refreshNotes()
        return ok
    }

    /// Asks the decision again and keeps only its notes. Executes nothing: the store is open,
    /// so a plan that begins with resume, a migration or an open is the next launch's work.
    private func refreshNotes() {
        guard opened, !cleanupRunning, let (steps, _, _) = ask() else { return }
        var fresh: [LaunchNote] = []
        for step in steps {
            switch step {
            case let .showFailedBefore(attempts): fresh.append(.failedBefore(attempts: attempts))
            case let .showMigrationSkipped(reason):
                fresh.append(.migrationSkipped(reason, skip: reason == .precheck ? lastSkip : nil))
            case .showMigrationFailed: fresh.append(.migrationFailed)
            case .showEncryptionAvailable: fresh.append(.encryptionAvailable)
            case .showPlainBackupKept: fresh.append(.plainBackupKept)
            default: break
            }
        }
        // Notes about what this launch did stay; the decision's notes are replaced.
        notes = notes.filter(\.isLaunchOnly) + fresh
    }
}

// MARK: - Live collaborators

extension LaunchCoordinator.Collaborators where Store == CoreMemoryStore {
    /// The real C ABI for everything but the Keychain, the settings, the presenter, the
    /// recording flag and the MCP server, which the app passes in.
    static func core(dbPath: URL, modelsDir: URL?, keys: LaunchKeyStore, settings: LaunchSettingsStore,
                     presenter: LaunchPresenter, recording: @escaping () -> Bool,
                     startMcpServer: @escaping (Data?) -> Void) -> Self {
        Self(
            observeFiles: { CoreLaunch.observeFiles(dbPath: dbPath) },
            decide: { CoreLaunch.decide(inputJSON: $0) },
            keys: keys,
            settings: settings,
            physicalMemory: { ProcessInfo.processInfo.physicalMemory },
            recording: recording,
            migration: CoreLaunchMigrationRunner(dbPath: dbPath),
            openPlain: {
                await Task.detached { CoreMemoryStore(path: dbPath, modelsDir: modelsDir) }.value
            },
            openKeyed: { key, cache, mustExist in
                await Task.detached {
                    CoreMemoryStore.openKeyed(path: dbPath, modelsDir: modelsDir, lookup: .found(key), cacheKib: cache,
                                              mustExist: mustExist)
                }.value
            },
            startMcpServer: startMcpServer,
            presenter: presenter,
            now: { DispatchTime.now().uptimeNanoseconds / 1_000_000 },
            // A failure is logged (no path, no data): a mark that failed only means a later
            // vanished library goes unnoticed; a clear that failed (the notice step, or after
            // "Reset encryption status and start empty") can show the notice once more.
            markStoreExisted: {
                if !CoreLaunch.markStoreExisted(dbPath: dbPath) {
                    Logger(subsystem: "com.mpointer.kestrel", category: "launch").error("Could not write the store_existed marker.")
                }
            },
            clearStoreExisted: {
                if !CoreLaunch.clearStoreExisted(dbPath: dbPath) {
                    Logger(subsystem: "com.mpointer.kestrel", category: "launch").error("Could not remove the store_existed marker.")
                }
            }
        )
    }
}
