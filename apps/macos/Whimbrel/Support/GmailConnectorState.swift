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
/// (a hard gate -- the connector process must report `send_email`) plus, for an account
/// whose grant includes reading, a best-effort `search_threads` call that never sends a
/// real email just to prove connectivity (see `performMcpRoundTrip`).
@MainActor
final class GmailConnectorState: ObservableObject {
    @Published private(set) var accounts: [ConnectedGmailAccount] = []
    @Published private(set) var defaultEmail: String?
    @Published private(set) var addAccountState: AddAccountState = .idle
    /// Emails whose stored grant is gone (no Keychain tokens, or Google revoked the
    /// refresh token), flagged by the polling service. Cleared by reconnecting or removing.
    @Published private(set) var needsReconnect: Set<String> = []
    /// Emails whose granted scopes lack `gmail.readonly`: they can send but not read, so
    /// polling skips them and their read tools are never offered.
    @Published private(set) var sendOnly: Set<String> = []

    private var oauthHelpers: [String: GoogleOAuthHelper] = [:]
    private let accountStore: ConnectedAccountStore
    /// The OAuth client the connector signs in with (`GoogleOAuthClient.active()`): the
    /// built-in one, or the user's own from Settings → Connectors → Advanced.
    private var client: GoogleOAuthClient?
    private var configError: String? {
        client == nil ? String(localized: "Gmail connector isn't configured in this build. See apps/macos/README.md to set up Secrets.xcconfig.") : nil
    }
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "gmail-connector")
    private weak var coreStore: CoreMemoryStore?

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
    //
    // 2026-11-12 (Google verification): the public app requests gmail.send only.
    // gmail.readonly is requested only with the user's own Google Cloud client
    // (GoogleConnectorScopes.gmail); gmail.compose is dropped (no shipped feature makes
    // drafts). Existing tokens keep their old grant until the account reconnects, and
    // what an account may do is decided from its GRANTED scopes (GmailReadAccess).
    private var scope: String { GoogleConnectorScopes.gmail(customClient: client?.isCustom ?? false) }
    private static let oldFixedKeychainAccount = "gmail-connector"

    init(coreStore: CoreMemoryStore?) {
        self.coreStore = coreStore
        accountStore = ConnectedAccountStore(connectorId: "gmail", store: coreStore)
        client = GoogleOAuthClient.active()

        migrateOldSingleAccountIfNeeded()
        for email in accountStore.emails {
            oauthHelpers[email] = makeHelper(email: email)
        }
        defaultEmail = accountStore.defaultEmail
        accounts = accountStore.emails.map { ConnectedGmailAccount(email: $0) }
        refreshAccountFlags()
    }

    /// Re-reads the active OAuth client after the custom client is saved or cleared.
    /// Accounts whose tokens another client issued are flagged for reconnect; their
    /// tokens are never refreshed with the new client.
    func reloadClient() {
        client = GoogleOAuthClient.active()
        for email in accountStore.emails {
            oauthHelpers[email] = makeHelper(email: email)
        }
        // Back to exactly the accounts another client signed in: clearing the custom
        // client makes built-in accounts usable again, and a send-only account is never
        // polled, so nothing else would clear its flag. A revoked grant is flagged again
        // by its next use.
        needsReconnect = Set(accountStore.emails.filter { oauthHelpers[$0]?.tokensIssuedByThisClient == false })
        refreshAccountFlags()
    }

    /// Recomputes the reconnect and send-only flags from the Keychain (no network).
    func refreshAccountFlags() {
        for email in accountStore.emails {
            guard let helper = oauthHelpers[email] else { continue }
            if !helper.tokensIssuedByThisClient { markNeedsReconnect(email) }
            let canRead = GmailReadAccess.canRead(grantedScope: helper.storedGrant().grantedScope)
            if canRead, sendOnly.contains(email) { sendOnly.remove(email) }
            if !canRead, !sendOnly.contains(email) { sendOnly.insert(email) }
        }
    }

    /// Whether `email`'s grant includes reading mail (an unrecorded, legacy grant does).
    func canRead(email: String) -> Bool {
        guard let helper = oauthHelpers[email] else { return false }
        return GmailReadAccess.canRead(grantedScope: helper.storedGrant().grantedScope)
    }

    /// Whether `email`'s tokens were issued by the current client.
    func isIssuedByCurrentClient(email: String) -> Bool {
        oauthHelpers[email]?.tokensIssuedByThisClient ?? false
    }

    /// Adopts the pre-multi-account single Keychain item, if one exists and the new
    /// index is empty. Unlike Calendar's migration, the OLD token predates `openid
    /// email` (Gmail never requested it before this task), so its ID token — if present
    /// at all — likely has no `email` claim to decode. Falls back to `"unknown"` as the
    /// migrated account's placeholder identity (same fallback `connect()` already used
    /// for this exact "no email available yet" case) rather than skipping migration
    /// entirely and silently dropping the user's existing connection.
    private func migrateOldSingleAccountIfNeeded() {
        guard accountStore.emails.isEmpty, let client else { return }
        let oldHelper = GoogleOAuthHelper(
            clientId: client.clientId, clientSecret: client.clientSecret, scope: scope,
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
            clientId: client?.clientId ?? "", clientSecret: client?.clientSecret ?? "", scope: scope,
            keychainAccount: "gmail-connector:\(email)"
        )
    }

    func oauthHelper(forEmail email: String) -> GoogleOAuthHelper? {
        oauthHelpers[email]
    }

    func addAccount(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async {
        guard addAccountState != .connecting else { return }
        guard let client else {
            addAccountState = .error(configError ?? String(localized: "Gmail connector is not configured"))
            return
        }
        addAccountState = .connecting
        let helper = GoogleOAuthHelper(clientId: client.clientId, clientSecret: client.clientSecret, scope: scope, keychainAccount: "gmail-connector:pending-\(UUID().uuidString)")
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
                try await performMcpRoundTrip(accessToken: accessToken, grantedScope: realHelper.storedGrant().grantedScope)
                oauthHelpers[email] = realHelper
                refreshAccountFlags()
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

    /// Set by `AppState`: whether another Whimbrel connection on this Mac (Calendar, or the sync
    /// sign-in) uses `email`'s Google grant, so revoking it would cut that connection off.
    var grantIsSharedElsewhere: @MainActor (String) -> Bool = { _ in false }

    /// Whether removing `email` also revokes Whimbrel's access with Google (what the
    /// confirmation promises).
    func removalRevokesAccess(email: String) -> Bool { !grantIsSharedElsewhere(email) }

    /// Removes the account: tokens deleted here first, its stored Gmail data deleted from this
    /// Mac, then Whimbrel's access revoked with Google in the background (best effort: offline
    /// or a refusal changes nothing local; only a reason code is logged).
    func removeAccount(email: String) {
        let helper = oauthHelpers[email]
        let revokes = removalRevokesAccess(email: email)
        let log = self.log
        let reason = GoogleDisconnect.disconnect(
            refreshToken: helper?.storedRefreshToken(), revoke: revokes,
            deleteLocal: { helper?.deleteStoredTokens() },
            startRevoke: { token in
                Task.detached(priority: .utility) {
                    let code = GoogleDisconnect.revokeNow(token)
                    log.log("Gmail disconnect: Google revoke \(code, privacy: .public)")
                }
            }
        )
        if let reason { log.log("Gmail disconnect: no revoke sent (\(reason, privacy: .public))") }
        oauthHelpers.removeValue(forKey: email)
        accounts.removeAll { $0.email == email }
        clearNeedsReconnect(email)
        if sendOnly.contains(email) { sendOnly.remove(email) }
        // Queued after the account leaves `accounts`, so a poll that already passed its
        // still-connected check has queued its write first and this delete runs after it.
        if let store = coreStore {
            CoreMemoryStore.accessQueue.async { store.gmailDeleteAccountData(account: email) }
        } else {
            log.error("removeAccount: no store, so \(email, privacy: .private)'s stored mail was not deleted")
        }
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

    /// Verifies a Gmail connection. `listTools` is a hard gate on `send_email` -- if the
    /// connector process didn't start or doesn't report it, something about the local
    /// setup is broken and the account shouldn't be kept. Read tools are offered only
    /// when the grant includes reading (`GmailReadAccess.offeredTools`); for such an
    /// account a `search_threads` call is best-effort only: a declined read checkbox or a
    /// transient API error must not destroy an account that can still send.
    private func performMcpRoundTrip(accessToken: String, grantedScope: String?) async throws {
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

            guard let allToolNames = host.listTools(connectorId: connectorId)?.map(\.name) else {
                throw GmailConnectorError.listToolsFailed
            }
            let toolNames = GmailReadAccess.offeredTools(allToolNames, grantedScope: grantedScope)
            guard toolNames.contains("send_email") else {
                throw GmailConnectorError.listToolsFailed
            }

            if toolNames.contains("search_threads"), host.callTool(connectorId: connectorId, toolName: "search_threads", argumentsJSON: "{\"q\":\"\",\"max_results\":1}") == nil {
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
        case .connectorScriptNotFound: return String(localized: "The Gmail connector's files weren't found in this build.")
        case .hostUnavailable: return "Couldn't start the connector host."
        case .connectFailed: return "Couldn't start the Gmail connector process."
        case .listToolsFailed: return "Connected, but the connector didn't report its expected capability."
        case .callToolFailed: return "Connected, but searching your mailbox failed."
        }
    }
}
