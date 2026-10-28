import Foundation
import OSLog

/// Which accounts are connected for one connector (Calendar, Gmail, Outlook 365) and
/// which one is the default. The registry itself is the `connector_account` table in
/// core/memory (core/api::connectors::accounts, M3.0 Task 9); the macOS Keychain holds
/// the actual OAuth tokens (one item per account). This class is the main-actor cache
/// the connector states read synchronously; every mutation goes through the core under
/// `CoreMemoryStore.accessQueue` and refreshes the cache from what the core returns, so
/// the first-account-is-default / removing-the-default-promotes-the-next rules live in
/// one place.
///
/// Deliberately outside `SettingsFieldTimestamps`/settings sync: connected accounts are
/// inherently per-Mac (a Keychain item never crosses devices).
@MainActor
final class ConnectedAccountStore {
    private let connectorId: String
    private let store: CoreMemoryStore?
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "connected-accounts")

    private(set) var emails: [String] = []
    private(set) var defaultEmail: String?

    /// Loads the registry synchronously. Only ever constructed during `AppState`'s own
    /// initialisation, before anything else has been queued on `accessQueue`, so the
    /// `.sync` hop is instant; it must not be constructed later from UI code.
    init(connectorId: String, store: CoreMemoryStore?, defaults: UserDefaults = .standard) {
        self.connectorId = connectorId
        self.store = store
        guard let store else { return }
        let migration = Self.legacyRegistry(connectorId: connectorId, defaults: defaults)
        let loaded = CoreMemoryStore.accessQueue.sync { () -> CoreConnectorAccounts? in
            var current = store.connectorAccounts(connector: connectorId)
            if let migration, current?.emails.isEmpty ?? true {
                // One-time move from the pre-M3.0 UserDefaults list (shipped 2026-08-05).
                // Kept for one release; remove once every install has launched this build.
                let now = Int64(Date().timeIntervalSince1970 * 1000)
                for email in migration.emails {
                    _ = store.addConnectorAccount(connector: connectorId, email: email, nowUnixMs: now)
                }
                if let legacyDefault = migration.defaultEmail {
                    _ = store.setDefaultConnectorAccount(connector: connectorId, email: legacyDefault)
                }
                current = store.connectorAccounts(connector: connectorId)
                Self.log.info("Migrated \(migration.emails.count, privacy: .public) \(connectorId, privacy: .public) account(s) from UserDefaults to connector_account")
            }
            return current
        }
        if migration != nil {
            defaults.removeObject(forKey: Self.legacyEmailsKey(connectorId))
            defaults.removeObject(forKey: Self.legacyDefaultEmailKey(connectorId))
        }
        apply(loaded)
    }

    /// `add(_:)` for the connector states' own one-time migrations, which run inside
    /// their `init` (before anything else is queued on `accessQueue`, so the `.sync`
    /// hop is instant) and read `emails` back synchronously right after.
    @discardableResult
    func addDuringInit(_ email: String) -> Bool {
        guard let store else { return false }
        let connectorId = self.connectorId
        let (added, snapshot) = CoreMemoryStore.accessQueue.sync { () -> (Bool, CoreConnectorAccounts?) in
            let rc = store.addConnectorAccount(connector: connectorId, email: email, nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000))
            return (rc == 1, store.connectorAccounts(connector: connectorId))
        }
        apply(snapshot)
        return added
    }

    /// Adds `email` if it isn't already connected (default automatically if it's the
    /// first account). Returns `false` (no-op) if `email` is already present.
    @discardableResult
    func add(_ email: String) async -> Bool {
        guard let store else { return false }
        let connectorId = self.connectorId
        let (added, snapshot) = await Self.withStoreAccess {
            let rc = store.addConnectorAccount(connector: connectorId, email: email, nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000))
            return (rc == 1, store.connectorAccounts(connector: connectorId))
        }
        apply(snapshot)
        return added
    }

    /// Removes `email`, promoting the next remaining account if it was the default.
    func remove(_ email: String) async {
        guard let store else { return }
        let connectorId = self.connectorId
        let snapshot = await Self.withStoreAccess {
            _ = store.removeConnectorAccount(connector: connectorId, email: email)
            return store.connectorAccounts(connector: connectorId)
        }
        apply(snapshot)
    }

    /// No-op if `email` isn't currently connected.
    func makeDefault(_ email: String) async {
        guard let store else { return }
        let connectorId = self.connectorId
        let snapshot = await Self.withStoreAccess {
            _ = store.setDefaultConnectorAccount(connector: connectorId, email: email)
            return store.connectorAccounts(connector: connectorId)
        }
        apply(snapshot)
    }

    private func apply(_ snapshot: CoreConnectorAccounts?) {
        guard let snapshot else { return }
        emails = snapshot.emails
        defaultEmail = snapshot.defaultEmail
    }

    private static func withStoreAccess<T>(_ work: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async { continuation.resume(returning: work()) }
        }
    }

    // MARK: - Legacy UserDefaults registry (pre-M3.0), read once for migration

    static func legacyEmailsKey(_ connectorId: String) -> String { "littlebird.\(connectorId).connectedAccountEmails" }
    static func legacyDefaultEmailKey(_ connectorId: String) -> String { "littlebird.\(connectorId).defaultAccountEmail" }

    private static func legacyRegistry(connectorId: String, defaults: UserDefaults) -> (emails: [String], defaultEmail: String?)? {
        guard let emails = defaults.stringArray(forKey: legacyEmailsKey(connectorId)) else { return nil }
        return (emails, defaults.string(forKey: legacyDefaultEmailKey(connectorId)))
    }
}
