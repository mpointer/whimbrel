import Foundation

/// The shared launch decision over the C ABI (X4 encryption at rest, core
/// `api::launch_decision`): what is on disk, and the plan for this launch. Both calls are
/// pure reads; nothing is opened or changed. `LaunchCoordinator` runs the plan.
enum CoreLaunch {
    /// `whimbrel_launch_observe_files`: the decision input's `files` object as JSON (passed
    /// through to `decide` untouched). Nil only for a path core cannot take.
    static func observeFiles(dbPath: URL) -> String? {
        take(dbPath.path.withCString { whimbrel_launch_observe_files($0) })
    }

    /// `whimbrel_launch_decision`: the plan JSON `{"steps":[...]}` for a `LaunchInput` JSON.
    /// Nil when core could not parse the input: do nothing and show an error.
    static func decide(inputJSON: String) -> String? {
        take(inputJSON.withCString { whimbrel_launch_decision($0) })
    }

    /// Whether to offer "Faster search (uses more memory)" on this computer.
    static func fastSearchAvailable(physicalMemory: UInt64) -> Bool {
        whimbrel_fast_search_available(physicalMemory)
    }

    /// `whimbrel_launch_mark_store_existed`: records beside `dbPath` that a store was opened
    /// here (the `store_existed` marker, no user data). After every successful open.
    @discardableResult
    static func markStoreExisted(dbPath: URL) -> Bool {
        dbPath.path.withCString { whimbrel_launch_mark_store_existed($0) }
    }

    /// `whimbrel_launch_clear_store_existed`: removes the marker (at the "library not found"
    /// notice step, and in Delete everything's flag cleanup). True when it is gone.
    @discardableResult
    static func clearStoreExisted(dbPath: URL) -> Bool {
        dbPath.path.withCString { whimbrel_launch_clear_store_existed($0) }
    }

    fileprivate static func take(_ s: UnsafeMutablePointer<CChar>?) -> String? {
        guard let s else { return nil }
        defer { whimbrel_core_free_string(s) }
        return String(cString: s)
    }
}

/// The files the encryption work set aside beside the database (core `api::foreign_files`,
/// Task 13c): `<db>.foreign-<digits>` and its sidecars. Settings > Your data shows them only
/// while there are some, with a user-confirmed delete. Never automatic.
enum CoreForeignFiles {
    struct File: Decodable, Equatable {
        let name: String
        let size: Int64
    }

    struct DeleteResult: Decodable, Equatable {
        let removed: [File]
        let failed: [String]
    }

    private struct Listing: Decodable { let files: [File] }
    private struct Deletion: Decodable {
        let ok: Bool
        let removed: [File]?
        let failed: [String]?
    }

    /// `whimbrel_foreign_files_list_json`: empty when there are none or the folder cannot be read.
    static func list(dbPath: URL) -> [File] {
        guard let json = CoreLaunch.take(dbPath.path.withCString { whimbrel_foreign_files_list_json($0) }),
              let listing = try? JSONDecoder().decode(Listing.self, from: Data(json.utf8)) else { return [] }
        return listing.files
    }

    /// `whimbrel_foreign_files_delete_all_json`: lists afresh and removes only the matches.
    /// Blocking. Nil when the folder could not be read (nothing was removed).
    static func deleteAll(dbPath: URL) -> DeleteResult? {
        guard let json = CoreLaunch.take(dbPath.path.withCString { whimbrel_foreign_files_delete_all_json($0) }),
              let d = try? JSONDecoder().decode(Deletion.self, from: Data(json.utf8)), d.ok else { return nil }
        return DeleteResult(removed: d.removed ?? [], failed: d.failed ?? [])
    }
}

/// Delete everything's database step over the C ABI (core `api::delete_everything`, X4 Task 12).
enum CoreDeleteEverything {
    private struct Removal: Decodable {
        let remaining: [String]
    }

    /// `whimbrel_delete_everything_remove_database_files_json`: removes the memory database's
    /// files beside `dbPath` (the database and its sidecars, `.plain.bak`, `.enc.tmp`,
    /// `.foreign-*`, the encryption state file) and returns the ones still there. An answer
    /// core could not give counts as the database itself remaining, so the keys stay.
    static func removeDatabaseFiles(_ dbPath: URL) -> [URL] {
        guard let json = CoreLaunch.take(dbPath.path.withCString { whimbrel_delete_everything_remove_database_files_json($0) }),
              let removal = try? JSONDecoder().decode(Removal.self, from: Data(json.utf8)) else { return [dbPath] }
        return removal.remaining.map { URL(fileURLWithPath: $0) }
    }

    /// `whimbrel_delete_everything_confine`: `candidate` resolved under `root` when it is an
    /// absolute direct child of it (symlinks on the way resolved, no `.`/`..`); nil when it
    /// must not be deleted.
    static func confine(root: URL, candidate: String) -> URL? {
        let resolved = root.path.withCString { r in candidate.withCString { c in whimbrel_delete_everything_confine(r, c) } }
        return CoreLaunch.take(resolved).map { URL(fileURLWithPath: $0) }
    }
}

/// The result of a user-confirmed file action behind a launch screen
/// (`WHIMBREL_ENC_ACTION_*`).
enum CoreEncryptionFileAction: Equatable {
    case done
    /// Its conditions do not hold; nothing was changed.
    case refused
    /// Restore only: stray sidecars were moved aside (kept), then a later step failed. The
    /// copy did not move; the button may be pressed again.
    case partlyDone
    /// A move or delete failed; files may have changed. Ask the decision again.
    case failed
    case invalidArgument
    /// A code this build does not know.
    case unknown(Int32)

    init(code: Int32) {
        switch code {
        case WHIMBREL_ENC_ACTION_DONE: self = .done
        case WHIMBREL_ENC_ACTION_REFUSED: self = .refused
        case WHIMBREL_ENC_ACTION_PARTLY_DONE: self = .partlyDone
        case WHIMBREL_ENC_ACTION_FAILED: self = .failed
        case WHIMBREL_ENC_ACTION_INVALID_ARGUMENT: self = .invalidArgument
        default: self = .unknown(code)
        }
    }
}

/// What a finished restore could not put in order (`WHIMBREL_RESTORE_WARNING_*`). The copy
/// is back either way; the restore is never repeated.
enum CoreRestoreWarning: Equatable {
    case none
    /// The unreadable state file could not be moved aside: offer "Reset encryption status".
    case stateNotMovedAside
    /// The state file could not be rewritten; the next launch decides from it.
    case stateNotCleared
    case unknown(Int32)

    init(code: Int32) {
        switch code {
        case WHIMBREL_RESTORE_WARNING_NONE: self = .none
        case WHIMBREL_RESTORE_WARNING_STATE_NOT_MOVED_ASIDE: self = .stateNotMovedAside
        case WHIMBREL_RESTORE_WARNING_STATE_NOT_CLEARED: self = .stateNotCleared
        default: self = .unknown(code)
        }
    }
}

/// "Put back the unencrypted copy": the action, how many stray sidecars at the database
/// path were moved aside (kept as `<db>.foreign-<ms>-*`), and the warning.
struct CoreRestoreResult: Equatable {
    let action: CoreEncryptionFileAction
    let movedAside: UInt32
    let warning: CoreRestoreWarning
}

extension CoreEncryptionMigration {
    /// `whimbrel_encrypt_restore_rollback_copy` (screen: encrypted file missing, backup present).
    static func restoreRollbackCopy(dbPath: URL) -> CoreRestoreResult {
        var moved: UInt32 = 0
        var warning: Int32 = 0
        let code = dbPath.path.withCString { whimbrel_encrypt_restore_rollback_copy($0, &moved, &warning) }
        return CoreRestoreResult(action: CoreEncryptionFileAction(code: code), movedAside: moved,
                                 warning: CoreRestoreWarning(code: warning))
    }

    /// `whimbrel_encrypt_reset_encryption_status` (screen: encrypted file missing with no copy;
    /// note: migration skipped because the state file is unreadable).
    static func resetEncryptionStatus(dbPath: URL) -> CoreEncryptionFileAction {
        CoreEncryptionFileAction(code: dbPath.path.withCString { whimbrel_encrypt_reset_encryption_status($0) })
    }

    /// `whimbrel_encrypt_delete_plain_backup` (note: plain backup kept). `openedWithKeyThisLaunch`
    /// is what the launch did, not what it planned.
    static func deletePlainBackup(dbPath: URL, openedWithKeyThisLaunch: Bool) -> (CoreEncryptionFileAction, removed: UInt32) {
        var removed: UInt32 = 0
        let code = dbPath.path.withCString {
            whimbrel_encrypt_delete_plain_backup($0, openedWithKeyThisLaunch, &removed)
        }
        return (CoreEncryptionFileAction(code: code), removed)
    }
}

/// "Check encryption speed" (core `api::speed_check`), local only.
enum CoreSpeedCheck {
    /// The local text file's contents for a report JSON object from `CoreMemoryStore.speedCheck`.
    static func text(reportJSON: String) -> String? {
        CoreLaunch.take(reportJSON.withCString { whimbrel_speed_check_text($0) })
    }
}
