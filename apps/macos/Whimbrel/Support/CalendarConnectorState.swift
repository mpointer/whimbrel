import AuthenticationServices
import Foundation
import OSLog

struct ConnectedCalendarAccount: Identifiable, Equatable {
    var id: String { email }
    let email: String
    let eventCount: Int?
}

/// Drives the Google Calendar MCP connector for every connected account. Each account
/// gets its own `GoogleOAuthHelper` (own Keychain item, keyed
/// `"google-calendar-connector:<email>"`) and its own real proof-of-connection
/// `list_events` round trip via `McpHost`. `ConnectedAccountStore` (Task 1) tracks which
/// emails are connected and which is default; this class owns the live
/// `GoogleOAuthHelper` instances and the `list_events`-verified event counts.
@MainActor
final class CalendarConnectorState: ObservableObject {
    @Published private(set) var accounts: [ConnectedCalendarAccount] = []
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
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "calendar-connector")

    private static let scope = "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events openid email"
    private static let oldFixedKeychainAccount = "google-calendar-connector"

    init(coreStore: CoreMemoryStore?) {
        accountStore = ConnectedAccountStore(connectorId: "calendar", store: coreStore)
        do {
            // Both `try` calls are resolved into locals before either stored property is
            // touched -- assigning `clientId` directly here, then having the second `try`
            // throw and fall into `catch` (which assigns `clientId` again), would be a
            // double-initialization of a `let` property, which Swift's definite
            // initialization checker rejects at compile time.
            let resolvedClientId = try SyncConfig.googleOAuthClientId()
            let resolvedClientSecret = try SyncConfig.googleOAuthClientSecret()
            clientId = resolvedClientId
            clientSecret = resolvedClientSecret
            configError = nil
        } catch {
            clientId = nil
            clientSecret = nil
            configError = "Calendar connector isn't configured in this build. See apps/macos/README.md to set up Secrets.xcconfig."
        }

        migrateOldSingleAccountIfNeeded()
        for email in accountStore.emails {
            oauthHelpers[email] = makeHelper(email: email)
        }
        defaultEmail = accountStore.defaultEmail
        // Real event counts are unknown until the next successful round trip -- start
        // every migrated/reloaded account at nil (not 0, which would misleadingly read
        // as "confirmed zero events") rather than blocking init on N network calls; the
        // UI shows a neutral "Connected." for nil until a fresh count arrives.
        accounts = accountStore.emails.map { ConnectedCalendarAccount(email: $0, eventCount: nil) }
    }

    /// Adopts the pre-multi-account single Keychain item (fixed key, no email suffix)
    /// as this connector's first account, if one exists and the new index is still
    /// empty. Decodes the stored token's own ID token to recover the email -- the old
    /// scheme's `CalendarConnectorStatus.connected(email:)` was session-ephemeral UI
    /// state, never persisted, so the email has to be re-derived from the token itself.
    /// Safe to run on every launch: a non-empty index short-circuits immediately.
    private func migrateOldSingleAccountIfNeeded() {
        guard accountStore.emails.isEmpty, let clientId, let clientSecret else { return }
        let oldHelper = GoogleOAuthHelper(
            clientId: clientId, clientSecret: clientSecret, scope: Self.scope,
            keychainAccount: Self.oldFixedKeychainAccount
        )
        guard let idToken = try? oldHelper.storedIdTokenForMigration(),
              let email = GoogleOAuthHelper.decodeJwtEmailClaim(idToken) else {
            return
        }
        guard oldHelper.copyStoredTokens(toKeychainAccount: "google-calendar-connector:\(email)") else {
            log.error("migrateOldSingleAccountIfNeeded: found an old Calendar token but couldn't copy it to the new per-account Keychain slot")
            return
        }
        accountStore.addDuringInit(email)
        // Delete the OLD fixed-key item now that it's been copied and adopted -- if it
        // survived, removing this (the user's only) account later would clear the index
        // but leave the old item in place, and the next launch would silently re-adopt
        // it as a connected, default account the user explicitly removed.
        oldHelper.deleteStoredTokens()
        log.info("Migrated existing Calendar connection to per-account scheme as \(email, privacy: .private)")
    }

    private func makeHelper(email: String) -> GoogleOAuthHelper {
        GoogleOAuthHelper(
            clientId: clientId ?? "", clientSecret: clientSecret ?? "", scope: Self.scope,
            keychainAccount: "google-calendar-connector:\(email)"
        )
    }

    /// Used by `RoutineScheduler` (Task 6) to get the token-refresh helper for one
    /// specific connected account (the one chosen at approval time, or the default for
    /// an auto-approved routine) -- never for an email that isn't in `accounts`.
    func oauthHelper(forEmail email: String) -> GoogleOAuthHelper? {
        oauthHelpers[email]
    }

    /// The "Add Account" button's action. Runs the interactive OAuth consent flow,
    /// decodes the resulting email, and either adopts it as a new connected account or
    /// (if that email is already connected) replaces its stored tokens in place.
    func addAccount(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async {
        guard addAccountState != .connecting else { return }
        guard let clientId, let clientSecret else {
            addAccountState = .error(configError ?? "Calendar connector is not configured")
            return
        }
        addAccountState = .connecting
        // A fresh, not-yet-keyed helper: the real Keychain account it's stored under
        // isn't known until after consent completes and the email is decoded.
        let helper = GoogleOAuthHelper(clientId: clientId, clientSecret: clientSecret, scope: Self.scope, keychainAccount: "google-calendar-connector:pending-\(UUID().uuidString)")
        do {
            try await helper.authorize(presentationContextProvider: presentationContextProvider)
            guard let idToken = try? await helper.validIdToken(), let email = GoogleOAuthHelper.decodeJwtEmailClaim(idToken) else {
                helper.deleteStoredTokens()
                addAccountState = .error("Couldn't determine the connected account's email.")
                return
            }
            // Re-authorizing an already-connected email (the fix for "Reconnect needed")
            // must replace that account's stored tokens, not discard the fresh ones --
            // `accountStore.add` returning false only means "already in the list".
            let isReAdd = accountStore.emails.contains(email)
            if !isReAdd {
                guard await accountStore.add(email) else {
                    helper.deleteStoredTokens()
                    addAccountState = .idle
                    return
                }
            }
            // Re-key from the throwaway "pending-..." slot to the real per-email one.
            guard helper.copyStoredTokens(toKeychainAccount: "google-calendar-connector:\(email)") else {
                if !isReAdd { await accountStore.remove(email) }
                helper.deleteStoredTokens()
                addAccountState = .error("Couldn't save this account's credentials.")
                return
            }
            helper.deleteStoredTokens() // removes the throwaway "pending-..." item
            clearNeedsReconnect(email)
            // From here on, `email` is durably persisted (index + a real per-email
            // Keychain item). A failure past this point must roll both back -- otherwise
            // the account would be invisible this session (never added to `accounts`/
            // `oauthHelpers`) yet reappear as "connected" after the next relaunch, and a
            // retry couldn't fix it since `accountStore.add(email)` would just no-op.
            let realHelper = makeHelper(email: email)
            do {
                let accessToken = try await realHelper.validAccessToken()
                let eventCount = try? await performMcpRoundTrip(accessToken: accessToken)
                oauthHelpers[email] = realHelper
                let account = ConnectedCalendarAccount(email: email, eventCount: eventCount)
                if let index = accounts.firstIndex(where: { $0.email == email }) {
                    accounts[index] = account
                } else {
                    accounts.append(account)
                }
                defaultEmail = accountStore.defaultEmail
                addAccountState = .idle
            } catch {
                // A re-add keeps the tokens just copied in: this failure is about the
                // connector environment, and deleting them would leave a previously
                // connected account worse off than before the attempt.
                if !isReAdd {
                    await accountStore.remove(email)
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

    private func performMcpRoundTrip(accessToken: String) async throws -> Int {
        return try await Task.detached(priority: .userInitiated) {
            guard let nodePath = McpNodeLocator.nodeExecutablePath() else {
                throw CalendarConnectorError.nodeNotFound
            }
            guard let scriptPath = McpNodeLocator.calendarConnectorScriptPath() else {
                throw CalendarConnectorError.connectorScriptNotFound
            }
            guard let host = McpHost() else {
                throw CalendarConnectorError.hostUnavailable
            }
            let args = McpNodeLocator.nodeNeedsEnvWrapper(resolvedPath: nodePath) ? ["node", scriptPath] : [scriptPath]
            guard let connectorId = host.connect(
                command: nodePath, args: args,
                env: [("GOOGLE_CALENDAR_ACCESS_TOKEN", accessToken)]
            ) else {
                throw CalendarConnectorError.connectFailed
            }
            defer { host.disconnect(connectorId: connectorId) }

            guard let tools = host.listTools(connectorId: connectorId), tools.contains(where: { $0.name == "list_events" }) else {
                throw CalendarConnectorError.listToolsFailed
            }

            let iso = ISO8601DateFormatter()
            let now = Date()
            let weekOut = now.addingTimeInterval(7 * 24 * 60 * 60)
            let argsJSON = "{\"time_min\":\"\(iso.string(from: now))\",\"time_max\":\"\(iso.string(from: weekOut))\"}"
            guard let resultText = host.callTool(connectorId: connectorId, toolName: "list_events", argumentsJSON: argsJSON) else {
                throw CalendarConnectorError.callToolFailed
            }
            let eventCount = (try? JSONDecoder().decode(CalendarEventsResponse.self, from: Data(resultText.utf8)))?.items?.count ?? 0
            return eventCount
        }.value
    }

    private func userMessage(for error: Error) -> String {
        if case GoogleOAuthError.signInCancelled = error {
            return "Sign-in cancelled."
        }
        if let connectorError = error as? CalendarConnectorError {
            return connectorError.description
        }
        return "Couldn't connect: \(error.localizedDescription)"
    }
}

private struct CalendarEventsResponse: Decodable {
    let items: [CalendarEvent]?
}

private struct CalendarEvent: Decodable {
    let summary: String?
}

enum CalendarConnectorError: Error, CustomStringConvertible {
    case nodeNotFound
    case connectorScriptNotFound
    case hostUnavailable
    case connectFailed
    case listToolsFailed
    case callToolFailed

    var description: String {
        switch self {
        case .nodeNotFound: return "Node.js isn't installed on this Mac. The Calendar connector needs it to run."
        case .connectorScriptNotFound: return "The Calendar connector's files weren't found in this build."
        case .hostUnavailable: return "Couldn't start the connector host."
        case .connectFailed: return "Couldn't start the Calendar connector process."
        case .listToolsFailed: return "Connected, but the connector didn't report its expected capability."
        case .callToolFailed: return "Connected, but fetching your calendar events failed."
        }
    }
}
