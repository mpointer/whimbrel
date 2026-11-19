import Foundation
import Security

/// The result of opening the main store when it may be encrypted (X4, core
/// `api::store_key::OpenOutcome`), one case per `WHIMBREL_OPEN_*` code. Which screen each
/// one leads to is the launch flow's job; every case but `opened` left the file untouched.
enum CoreOpenOutcome: Equatable {
    case opened
    /// No file, no key: create a key (`DbKeyStore.create()`), then open again with it.
    case newInstall
    /// An encrypted file and no key: the key-gone recovery screen.
    case needsKey
    /// The Keychain could not be read: retry screen (not the key-gone screen).
    case keyUnavailable(String)
    /// The key does not open this file.
    case keyRejected(String)
    /// A plain database: migrate it first (`CoreEncryptionMigration`).
    case needsMigration
    case damaged(String)
    case startupFailed(String)
    /// This build has no SQLCipher (a packaging problem).
    case unsupported(String)
    /// An encryption upgrade was interrupted: `CoreEncryptionMigration.resume`, then open again.
    case migrationInterrupted(String)
    /// The call was malformed (for example a key that is not 32 bytes); not an outcome.
    case invalidArgument
    /// A code this build does not know (a newer core). Never treated as a known outcome:
    /// the launch flow must stop and show the code, not guess.
    case unknown(Int32, String)

    init(code: Int32, message: String?) {
        let m = message ?? ""
        switch code {
        case WHIMBREL_OPEN_OPENED: self = .opened
        case WHIMBREL_OPEN_NEW_INSTALL: self = .newInstall
        case WHIMBREL_OPEN_NEEDS_KEY: self = .needsKey
        case WHIMBREL_OPEN_KEY_UNAVAILABLE: self = .keyUnavailable(m)
        case WHIMBREL_OPEN_KEY_REJECTED: self = .keyRejected(m)
        case WHIMBREL_OPEN_NEEDS_MIGRATION: self = .needsMigration
        case WHIMBREL_OPEN_DAMAGED: self = .damaged(m)
        case WHIMBREL_OPEN_STARTUP_FAILED: self = .startupFailed(m)
        case WHIMBREL_OPEN_UNSUPPORTED: self = .unsupported(m)
        case WHIMBREL_OPEN_MIGRATION_INTERRUPTED: self = .migrationInterrupted(m)
        case WHIMBREL_OPEN_INVALID_ARGUMENT: self = .invalidArgument
        default: self = .unknown(code, m)
        }
    }

    /// The `WHIMBREL_OPEN_*` code.
    var code: Int32 {
        switch self {
        case .opened: return WHIMBREL_OPEN_OPENED
        case .newInstall: return WHIMBREL_OPEN_NEW_INSTALL
        case .needsKey: return WHIMBREL_OPEN_NEEDS_KEY
        case .keyUnavailable: return WHIMBREL_OPEN_KEY_UNAVAILABLE
        case .keyRejected: return WHIMBREL_OPEN_KEY_REJECTED
        case .needsMigration: return WHIMBREL_OPEN_NEEDS_MIGRATION
        case .damaged: return WHIMBREL_OPEN_DAMAGED
        case .startupFailed: return WHIMBREL_OPEN_STARTUP_FAILED
        case .unsupported: return WHIMBREL_OPEN_UNSUPPORTED
        case .migrationInterrupted: return WHIMBREL_OPEN_MIGRATION_INTERRUPTED
        case .invalidArgument: return WHIMBREL_OPEN_INVALID_ARGUMENT
        case let .unknown(code, _): return code
        }
    }
}

extension CoreMemoryStore {
    /// Opens the main store through core's `open_store` with the Keychain lookup result
    /// (`DbKeyStore.lookup()`). The store comes back only with `.opened`; it remembers the
    /// key for its slow lane. Runs the startup sweep like `init(path:modelsDir:)`.
    /// `cacheKib`: the launch plan's cache for the MAIN store (Faster search); nil keeps
    /// SQLite's default. The MCP server and per-call opens never get one.
    /// `mustExist`: never create (`whimbrel_memory_open_keyed_existing`), for the open that
    /// proves a recovered key on the existing database; a missing file is `.damaged`.
    static func openKeyed(path: URL, modelsDir: URL?, lookup: DbKeyLookup,
                          cacheKib: Int64? = nil, mustExist: Bool = false) -> (CoreOpenOutcome, CoreMemoryStore?) {
        let state: Int32
        var key: Data?
        var unavailable: String?
        switch lookup {
        case let .found(k):
            state = WHIMBREL_KEY_FOUND
            key = k
        case .absent:
            state = WHIMBREL_KEY_ABSENT
        case let .unavailable(m):
            state = WHIMBREL_KEY_UNAVAILABLE
            unavailable = m
        }
        var handle: OpaquePointer?
        var message: UnsafeMutablePointer<CChar>?
        let code: Int32 = path.path.withCString { p in
            withOptionalCString(modelsDir?.path) { m in
                withOptionalCString(unavailable) { u in
                    withKeyBytes(key) { k, n in
                        mustExist && state == WHIMBREL_KEY_FOUND
                            ? whimbrel_memory_open_keyed_existing(p, m, k, n, max(cacheKib ?? 0, 0), &handle, &message)
                            : whimbrel_memory_open_keyed_with_cache(p, m, state, k, n, u, max(cacheKib ?? 0, 0),
                                                                    &handle, &message)
                    }
                }
            }
        }
        var text: String?
        if let message {
            text = String(cString: message)
            whimbrel_core_free_string(message)
        }
        let outcome = CoreOpenOutcome(code: code, message: text)
        guard code == WHIMBREL_OPEN_OPENED, let handle else { return (outcome, nil) }
        return (outcome, CoreMemoryStore(adopting: handle, dbKey: key))
    }
}

private func withOptionalCString<R>(_ s: String?, _ body: (UnsafePointer<CChar>?) -> R) -> R {
    guard let s else { return body(nil) }
    return s.withCString { body($0) }
}

/// The encryption migration of a plain database (core `api::encrypt_migration`, design
/// 3.5). Core never touches the Keychain: `run` hands it a key-store callback that writes
/// `db-key-pending` then `db-key` through `DbKeyStore`. All calls block; run them off the
/// main thread.
enum CoreEncryptionMigration {
    /// `step`: 0 checkpoint, 1 copy, 2 verify, 3 swap. `overall` (0...1) never goes back.
    struct Progress: Sendable {
        let step: Int32
        let stepFraction: Double
        let overall: Double
        let note: String?
    }

    /// Core's JSON answer: `ok` with the phase reached (`"done"` after `run`), or an error.
    struct Outcome: Decodable, Equatable {
        let ok: Bool
        let phase: String?
        let error: Failure?
    }

    struct Failure: Decodable, Equatable {
        /// `skipped`, `key_store`, `copy`, `verify`, `swap`, `state`, `recovery`, `refused` or
        /// `partly_done`.
        let kind: String
        let message: String
        let skip: Skip?
    }

    /// Why a run was not started (`kind == "skipped"`).
    struct Skip: Decodable, Equatable {
        /// `recording`, `low_disk`, `free_space_unknown`, `open_elsewhere` (another process has
        /// the file open), `open_in_this_process` (this app's own store has it open),
        /// `not_plain`, `interrupted_run`, `plain_backup_exists`, `failed_before` or `unreadable`.
        let reason: String
        let needBytes: UInt64?
        let freeBytes: UInt64?
        let phase: String?
        let attempts: UInt32?
        let last: String?

        enum CodingKeys: String, CodingKey {
            case reason, phase, attempts, last
            case needBytes = "need_bytes"
            case freeBytes = "free_bytes"
        }
    }

    /// Holds the Swift closures for the C callbacks for the length of one `run`.
    private final class Callbacks {
        let storeKey: (String, Data) -> OSStatus
        let progress: (@Sendable (Progress) -> Void)?
        init(storeKey: @escaping (String, Data) -> OSStatus, progress: (@Sendable (Progress) -> Void)?) {
            self.storeKey = storeKey
            self.progress = progress
        }
    }

    /// Encrypts the plain database at `dbPath` with `key`, storing the key through
    /// `keyStore` (pending first, then promoted). The caller deletes `db-key-pending` only
    /// after `phase == "done"`. Nil on malformed arguments (nothing was done).
    ///
    /// Threads: `run` blocks its calling thread (never the main thread). `storeKey` runs on
    /// that thread. `progress` runs on that thread **or on a Rust helper thread** during the
    /// copy; calls never overlap, but a caller that updates UI must hop to the main queue.
    static func run(dbPath: URL, recording: Bool, key: Data, keyStore: DbKeyStore,
                    progress: (@Sendable (Progress) -> Void)?) -> Outcome? {
        run(dbPath: dbPath, recording: recording, key: key,
            storeKey: { account, bytes in keyStore.migrationWrite(bytes, account: account) },
            progress: progress)
    }

    /// `run` with the key-store callback given directly (0 = stored, else an OSStatus).
    /// Same threading as above.
    static func run(dbPath: URL, recording: Bool, key: Data, storeKey: @escaping (String, Data) -> OSStatus,
                    progress: (@Sendable (Progress) -> Void)?) -> Outcome? {
        let callbacks = Callbacks(storeKey: storeKey, progress: progress)
        let ctx = Unmanaged.passUnretained(callbacks).toOpaque()
        let storeFn: whimbrel_key_store_fn = { ctx, account, bytes, count in
            guard let ctx, let account, let bytes else { return Int32(errSecParam) }
            let callbacks = Unmanaged<Callbacks>.fromOpaque(ctx).takeUnretainedValue()
            return callbacks.storeKey(String(cString: account), Data(bytes: bytes, count: count))
        }
        let progressFn: whimbrel_migration_progress_fn = { ctx, step, stepFraction, overall, note in
            guard let ctx else { return }
            let callbacks = Unmanaged<Callbacks>.fromOpaque(ctx).takeUnretainedValue()
            callbacks.progress?(Progress(step: step, stepFraction: stepFraction, overall: overall,
                                         note: note.map { String(cString: $0) }))
        }
        let json = withExtendedLifetime(callbacks) {
            dbPath.path.withCString { p in
                CoreMemoryStore.withKeyBytes(key) { k, n in
                    whimbrel_encrypt_migration_run(p, recording, k, n, storeFn, ctx,
                                                   progress == nil ? nil : progressFn, ctx)
                }
            }
        }
        return decode(json)
    }

    /// Finish or undo an interrupted run; call at launch before opening the store.
    /// `phase` nil: nothing to do; `"done"`: recorded finished; any other: undone (plain again).
    static func resume(dbPath: URL) -> Outcome? {
        decode(dbPath.path.withCString { whimbrel_encrypt_migration_resume($0) })
    }

    /// `ok` when a run may start now, else `error.skip` says why. Changes nothing.
    static func precheck(dbPath: URL, recording: Bool) -> Outcome? {
        decode(dbPath.path.withCString { whimbrel_encrypt_migration_precheck($0, recording) })
    }

    /// Retry after `failed_before`: forgets the failure record.
    static func resetFailures(dbPath: URL) -> Outcome? {
        decode(dbPath.path.withCString { whimbrel_encrypt_migration_reset_failures($0) })
    }

    static func plainBackupExists(dbPath: URL) -> Bool {
        dbPath.path.withCString { whimbrel_encrypt_migration_plain_backup_exists($0) }
    }

    /// Deletes `<db>.plain.bak` after three good launches on the encrypted file.
    static func cleanupPlainBackup(dbPath: URL, goodLaunches: UInt32) -> Bool {
        dbPath.path.withCString { whimbrel_encrypt_migration_cleanup_plain_backup($0, goodLaunches) }
    }

    private static func decode(_ json: UnsafeMutablePointer<CChar>?) -> Outcome? {
        guard let json else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(Outcome.self, from: Data(String(cString: json).utf8))
    }
}
