import AuthenticationServices
import Foundation
import OSLog
import Security

struct ConnectedGmailAccount: Identifiable, Equatable {
    var id: String { email }
    let email: String
}

/// Drives the Gmail MCP connector for every connected account. Mirrors
/// `CalendarConnectorState`'s exact multi-account shape -- see that file's doc comment
/// for the design this follows. `addAccount` verifies a connection via `listTools`
/// (a hard gate -- the connector process must report both `send_email` and
/// `search_threads`) plus a best-effort `search_threads` call that never sends a
/// real email just to prove connectivity (see `performMcpRoundTrip`).
@MainActor
final class GmailConnectorState: ObservableObject {
    @Published private(set) var accounts: [ConnectedGmailAccount] = []
    @Published private(set) var defaultEmail: String?
    @Published private(set) var addAccountState: AddAccountState = .idle
    /// Emails whose stored grant is gone (no Keychain tokens, or Google revoked the
    /// refresh token), flagged by the polling service. Cleared by reconnecting or removing.
    @Published private(set) var needsReconnect: Set<String> = []

    private var oauthHelpers: [String: GoogleOAuthHelper] = [:]
    private let accountStore: ConnectedAccountStore
    private let configError: String?
    private let clientId: String?
    private let clientSecret: String?
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "gmail-connector")

    // openid email added so a connected account can be identified/labeled in a
    // multi-account list -- Gmail previously requested gmail.send only. Existing
    // connected users see one re-consent prompt to pick this up (same precedent as
    // Calendar's own scope broadening).
    //
    // gmail.readonly + gmail.compose added 2026-08-12 for search_threads/get_thread/
    // create_draft (docs/superpowers/specs/2026-08-12-gmail-read-tool-design.md).
    // gmail.send was dropped in the same change -- gmail.compose is Google's
    // documented superset of it ("Manage drafts and send emails"), so keeping both
    // only added a redundant line to the consent screen. Existing connected users
    // pick this up by reconnecting (see addAccount's isReAdd handling, which now
    // refreshes an existing account's tokens in place instead of discarding them).
    //
    // gmail.readonly and gmail.compose are both Google "restricted" scopes (unlike
    // gmail.send's lighter "sensitive" tier) -- shipping this connector to anyone
    // beyond a personal Google Cloud project would require Google's OAuth app
    // verification plus an annual third-party security assessment. Not relevant for
    // personal/internal use; flag it if this project ever moves toward public
    // distribution.
    private static let scope = "openid email https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose"
    private static let oldFixedKeychainAccount = "gmail-connector"

    init(coreStore: CoreMemoryStore?) {
        accountStore = ConnectedAccountStore(connectorId: "gmail", store: coreStore)
        do {
            // Resolved into locals first, then assigned once both succeed -- see
            // CalendarConnectorState's identical init() for why (a real Swift
            // definite-initialization error, not a style choice).
            let resolvedClientId = try SyncConfig.googleOAuthClientId()
            let resolvedClientSecret = try SyncConfig.googleOAuthClientSecret()
            clientId = resolvedClientId
            clientSecret = resolvedClientSecret
            configError = nil
        } catch {
            clientId = nil
            clientSecret = nil
            configError = "Gmail connector isn't configured in this build. See apps/macos/README.md to set up Secrets.xcconfig."
        }

        migrateOldSingleAccountIfNeeded()
        for email in accountStore.emails {
            oauthHelpers[email] = makeHelper(email: email)
        }
        defaultEmail = accountStore.defaultEmail
        accounts = accountStore.emails.map { ConnectedGmailAccount(email: $0) }
    }

    /// Adopts the pre-multi-account single Keychain item, if one exists and the new
    /// index is empty. Unlike Calendar's migration, the OLD token predates `openid
    /// email` (Gmail never requested it before this task), so its ID token — if present
    /// at all — likely has no `email` claim to decode. Falls back to `"unknown"` as the
    /// migrated account's placeholder identity (same fallback `connect()` already used
    /// for this exact "no email available yet" case) rather than skipping migration
    /// entirely and silently dropping the user's existing connection.
    private func migrateOldSingleAccountIfNeeded() {
        guard accountStore.emails.isEmpty, let clientId, let clientSecret else { return }
        let oldHelper = GoogleOAuthHelper(
            clientId: clientId, clientSecret: clientSecret, scope: Self.scope,
            keychainAccount: Self.oldFixedKeychainAccount
        )
        guard (try? oldHelper.storedIdTokenForMigration()) != nil || Self.oldKeychainItemHasStoredTokens() else {
            return // no old connection to migrate
        }
        let email = (try? oldHelper.storedIdTokenForMigration()).flatMap { $0.flatMap(GoogleOAuthHelper.decodeJwtEmailClaim) } ?? "unknown"
        guard oldHelper.copyStoredTokens(toKeychainAccount: "gmail-connector:\(email)") else {
            log.error("migrateOldSingleAccountIfNeeded: found an old Gmail token but couldn't copy it to the new per-account Keychain slot")
            return
        }
        accountStore.addDuringInit(email)
        // Delete the OLD fixed-key item now that it's been copied and adopted -- if it
        // survived, removing this (the user's only) account later would clear the index
        // but leave the old item in place, and the next launch would silently re-adopt
        // it as a connected, default account the user explicitly removed.
        oldHelper.deleteStoredTokens()
        log.info("Migrated existing Gmail connection to per-account scheme as \(email, privacy: .private)")
    }

    /// Sync existence check for the old fixed-key Keychain item. Deliberately not
    /// `GoogleOAuthHelper.validAccessToken()` -- that method is `async` (it may refresh
    /// over the network), but this runs from `init()`, which is synchronous, so it can't
    /// be awaited here. Mirrors the exact service/account query
    /// `GoogleOAuthHelper.deleteStoredTokens()` already uses. Existence alone (not
    /// validity) is enough to know there's an old connection worth migrating -- an
    /// expired-but-present refresh token still means the user was connected.
    private static func oldKeychainItemHasStoredTokens() -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: GoogleOAuthHelper.keychainService,
            kSecAttrAccount as String: oldFixedKeychainAccount,
        ]
        return KeychainAccess.copyMatching(query as CFDictionary, nil) == errSecSuccess
    }

    private func makeHelper(email: String) -> GoogleOAuthHelper {
        GoogleOAuthHelper(
            clientId: clientId ?? "", clientSecret: clientSecret ?? "", scope: Self.scope,
            keychainAccount: "gmail-connector:\(email)"
        )
    }

    func oauthHelper(forEmail email: String) -> GoogleOAuthHelper? {
        oauthHelpers[email]
    }

    func addAccount(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async {
        guard addAccountState != .connecting else { return }
        guard let clientId, let clientSecret else {
            addAccountState = .error(configError ?? "Gmail connector is not configured")
            return
        }
        addAccountState = .connecting
        let helper = GoogleOAuthHelper(clientId: clientId, clientSecret: clientSecret, scope: Self.scope, keychainAccount: "gmail-connector:pending-\(UUID().uuidString)")
        do {
            try await helper.authorize(presentationContextProvider: presentationContextProvider)
            let email = (try? await helper.validIdToken()).flatMap(GoogleOAuthHelper.decodeJwtEmailClaim) ?? "unknown"
            // A migrated pre-upgrade user starts out labeled "unknown" (old Gmail tokens
            // predate `openid email`, so there's no real email to decode yet). If this
            // connect resolved a real address AND "unknown" is still sitting in the
            // list, this is that placeholder being resolved, not a brand-new account --
            // fold it into the real entry instead of leaving both as separate rows with
            // their own Keychain items.
            var resolvedUnknownWasDefault = false
            if email != "unknown", accountStore.emails.contains("unknown") {
                resolvedUnknownWasDefault = accountStore.defaultEmail == "unknown"
                makeHelper(email: "unknown").deleteStoredTokens()
                await accountStore.remove("unknown")
                accounts.removeAll { $0.email == "unknown" }
                oauthHelpers.removeValue(forKey: "unknown")
            }
            // Re-authorizing an already-connected email (e.g. to pick up a widened
            // OAuth scope) must refresh that account's stored tokens, not discard the
            // fresh ones -- accountStore.add(email) returning false only means
            // "already in the list," it isn't a signal to reject the newly granted
            // tokens.
            let isReAdd = accountStore.emails.contains(email)
            if !isReAdd {
                guard await accountStore.add(email) else {
                    helper.deleteStoredTokens()
                    addAccountState = .idle
                    return
                }
            }
            guard helper.copyStoredTokens(toKeychainAccount: "gmail-connector:\(email)") else {
                if !isReAdd { await accountStore.remove(email) }
                helper.deleteStoredTokens()
                addAccountState = .error("Couldn't save this account's credentials.")
                return
            }
            helper.deleteStoredTokens()
            clearNeedsReconnect(email)
            // From here on, `email` is durably persisted (index + a real per-email
            // Keychain item). A failure past this point must roll both back for a
            // brand-new account -- otherwise the account would be invisible this
            // session yet reappear as "connected" after the next relaunch. For a
            // re-add, the tokens just copied in above are left in place even on a
            // later failure -- the failure is about the connector environment, not
            // about whether these tokens are valid, and deleting them would leave a
            // previously-working account worse off than before this attempt.
            let realHelper = makeHelper(email: email)
            do {
                let accessToken = try await realHelper.validAccessToken()
                try await performMcpRoundTrip(accessToken: accessToken)
                oauthHelpers[email] = realHelper
                if !isReAdd {
                    accounts.append(ConnectedGmailAccount(email: email))
                }
                if resolvedUnknownWasDefault {
                    await accountStore.makeDefault(email)
                }
                defaultEmail = accountStore.defaultEmail
                addAccountState = .idle
            } catch {
                if !isReAdd {
                    await accountStore.remove(email)
                    accounts.removeAll { $0.email == email }
                    realHelper.deleteStoredTokens()
                }
                log.log("addAccount: failed reading back the newly-connected account with \(String(describing: error), privacy: .public)")
                addAccountState = .error(userMessage(for: error))
            }
        } catch {
            log.log("addAccount: failed with \(String(describing: error), privacy: .public)")
            addAccountState = .error(userMessage(for: error))
        }
    }

    func removeAccount(email: String) {
        oauthHelpers[email]?.deleteStoredTokens()
        oauthHelpers.removeValue(forKey: email)
        accounts.removeAll { $0.email == email }
        clearNeedsReconnect(email)
        Task {
            await accountStore.remove(email)
            defaultEmail = accountStore.defaultEmail
        }
    }

    func markNeedsReconnect(_ email: String) {
        if !needsReconnect.contains(email) { needsReconnect.insert(email) }
    }

    func clearNeedsReconnect(_ email: String) {
        if needsReconnect.contains(email) { needsReconnect.remove(email) }
    }

    func makeDefault(email: String) {
        Task {
            await accountStore.makeDefault(email)
            defaultEmail = accountStore.defaultEmail
        }
    }

    /// Verifies a Gmail connection. `listTools` is a hard gate -- if the connector
    /// process didn't start or doesn't report its expected tools, something about
    /// the local setup is broken and the account shouldn't be kept. The deeper
    /// `search_threads` call is best-effort only: if the user declined the
    /// read/compose consent checkboxes (Google shows them separately from send),
    /// or hits a transient API error, that must not destroy an otherwise-valid
    /// account -- `gmail.compose`'s grant may still make send work fine. Log and
    /// continue rather than throw.
    private func performMcpRoundTrip(accessToken: String) async throws {
        let log = self.log
        try await Task.detached(priority: .userInitiated) {
            guard let nodePath = McpNodeLocator.nodeExecutablePath() else {
                throw GmailConnectorError.nodeNotFound
            }
            guard let scriptPath = McpNodeLocator.gmailConnectorScriptPath() else {
                throw GmailConnectorError.connectorScriptNotFound
            }
            guard let host = McpHost() else {
                throw GmailConnectorError.hostUnavailable
            }
            let args = McpNodeLocator.nodeNeedsEnvWrapper(resolvedPath: nodePath) ? ["node", scriptPath] : [scriptPath]
            guard let connectorId = host.connect(
                command: nodePath, args: args,
                env: [("GMAIL_ACCESS_TOKEN", accessToken)]
            ) else {
                throw GmailConnectorError.connectFailed
            }
            defer { host.disconnect(connectorId: connectorId) }

            guard let toolNames = host.listTools(connectorId: connectorId)?.map(\.name),
                  toolNames.contains("send_email"), toolNames.contains("search_threads") else {
                throw GmailConnectorError.listToolsFailed
            }

            if host.callTool(connectorId: connectorId, toolName: "search_threads", argumentsJSON: "{\"q\":\"\",\"max_results\":1}") == nil {
                log.log("performMcpRoundTrip: search_threads verification failed -- account will still be connected")
            }
        }.value
    }

    private func userMessage(for error: Error) -> String {
        if case GoogleOAuthError.signInCancelled = error {
            return "Sign-in cancelled."
        }
        if let connectorError = error as? GmailConnectorError {
            return connectorError.description
        }
        return "Couldn't connect: \(error.localizedDescription)"
    }
}

enum GmailConnectorError: Error, CustomStringConvertible {
    case nodeNotFound
    case connectorScriptNotFound
    case hostUnavailable
    case connectFailed
    case listToolsFailed
    case callToolFailed

    var description: String {
        switch self {
        case .nodeNotFound: return "Node.js isn't installed on this Mac. The Gmail connector needs it to run."
        case .connectorScriptNotFound: return "The Gmail connector's files weren't found in this build."
        case .hostUnavailable: return "Couldn't start the connector host."
        case .connectFailed: return "Couldn't start the Gmail connector process."
        case .listToolsFailed: return "Connected, but the connector didn't report its expected capability."
        case .callToolFailed: return "Connected, but searching your mailbox failed."
        }
    }
}
