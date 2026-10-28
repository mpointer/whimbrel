import AuthenticationServices
import Foundation
import OSLog

struct ConnectedOutlookAccount: Identifiable, Equatable {
    var id: String { email }
    let email: String
    let eventCount: Int?
}

/// Drives the Outlook 365 MCP connector for every connected account. Each account gets
/// its own `MicrosoftOAuthHelper` (own Keychain item, keyed
/// `"outlook365-connector:<email>"`) and its own real proof-of-connection `list_events`
/// round trip via `McpHost`. `ConnectedAccountStore` tracks which emails are connected
/// and which is default; this class owns the live `MicrosoftOAuthHelper` instances and
/// the `list_events`-verified event counts.
///
/// Unlike `CalendarConnectorState`, there is no legacy single-account migration -- this
/// connector never had a pre-multi-account scheme.
@MainActor
final class Outlook365ConnectorState: ObservableObject {
    @Published private(set) var accounts: [ConnectedOutlookAccount] = []
    @Published private(set) var defaultEmail: String?
    @Published private(set) var addAccountState: AddAccountState = .idle

    private var oauthHelpers: [String: MicrosoftOAuthHelper] = [:]
    private let accountStore: ConnectedAccountStore
    private let configError: String?
    private let clientId: String?
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "outlook365-connector")

    private static let scope = "openid email offline_access Calendars.Read Mail.Read"

    init(coreStore: CoreMemoryStore?) {
        accountStore = ConnectedAccountStore(connectorId: "outlook365", store: coreStore)
        do {
            clientId = try SyncConfig.microsoftOAuthClientId()
            configError = nil
        } catch {
            clientId = nil
            configError = "Outlook 365 connector isn't configured in this build. See apps/macos/README.md to set up Secrets.xcconfig."
        }

        for email in accountStore.emails {
            oauthHelpers[email] = makeHelper(email: email)
        }
        defaultEmail = accountStore.defaultEmail
        accounts = accountStore.emails.map { ConnectedOutlookAccount(email: $0, eventCount: nil) }
    }

    private func makeHelper(email: String) -> MicrosoftOAuthHelper {
        MicrosoftOAuthHelper(clientId: clientId ?? "", scope: Self.scope, keychainAccount: "outlook365-connector:\(email)")
    }

    /// Used by future consumers (e.g. wiring Outlook into meeting detection -- out of
    /// scope for this pass) to get the token-refresh helper for one connected account.
    func oauthHelper(forEmail email: String) -> MicrosoftOAuthHelper? {
        oauthHelpers[email]
    }

    /// The "Add Account" button's action. Runs the interactive OAuth consent flow,
    /// decodes the resulting account identifier, and either adopts it as a new
    /// connected account or (if already connected) reports it as a no-op.
    func addAccount(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async {
        guard addAccountState != .connecting else { return }
        guard let clientId else {
            addAccountState = .error(configError ?? "Outlook 365 connector is not configured")
            return
        }
        addAccountState = .connecting
        // A fresh, not-yet-keyed helper: the real Keychain account it's stored under
        // isn't known until after consent completes and the account identifier is
        // decoded.
        let helper = MicrosoftOAuthHelper(
            clientId: clientId, scope: Self.scope,
            keychainAccount: "outlook365-connector:pending-\(UUID().uuidString)"
        )
        do {
            try await helper.authorize(presentationContextProvider: presentationContextProvider)
            guard let idToken = try? await helper.validIdToken(),
                  let email = MicrosoftOAuthHelper.decodeAccountIdentifier(idToken) else {
                helper.deleteStoredTokens()
                addAccountState = .error("Couldn't determine the connected account's email.")
                return
            }
            guard await accountStore.add(email) else {
                helper.deleteStoredTokens()
                addAccountState = .idle
                return // already connected -- silent no-op, matching CalendarConnectorState
            }
            // Re-key from the throwaway "pending-..." slot to the real per-email one.
            guard helper.copyStoredTokens(toKeychainAccount: "outlook365-connector:\(email)") else {
                await accountStore.remove(email)
                helper.deleteStoredTokens()
                addAccountState = .error("Couldn't save this account's credentials.")
                return
            }
            helper.deleteStoredTokens() // removes the throwaway "pending-..." item
            let realHelper = makeHelper(email: email)
            do {
                let accessToken = try await realHelper.validAccessToken()
                let eventCount = try? await performMcpRoundTrip(accessToken: accessToken)
                oauthHelpers[email] = realHelper
                accounts.append(ConnectedOutlookAccount(email: email, eventCount: eventCount))
                defaultEmail = accountStore.defaultEmail
                addAccountState = .idle
            } catch {
                await accountStore.remove(email)
                realHelper.deleteStoredTokens()
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
        Task {
            await accountStore.remove(email)
            defaultEmail = accountStore.defaultEmail
        }
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
                throw Outlook365ConnectorError.nodeNotFound
            }
            guard let scriptPath = McpNodeLocator.outlook365ConnectorScriptPath() else {
                throw Outlook365ConnectorError.connectorScriptNotFound
            }
            guard let host = McpHost() else {
                throw Outlook365ConnectorError.hostUnavailable
            }
            let args = McpNodeLocator.nodeNeedsEnvWrapper(resolvedPath: nodePath) ? ["node", scriptPath] : [scriptPath]
            guard let connectorId = host.connect(
                command: nodePath, args: args,
                env: [("MICROSOFT_ACCESS_TOKEN", accessToken)]
            ) else {
                throw Outlook365ConnectorError.connectFailed
            }
            defer { host.disconnect(connectorId: connectorId) }

            guard let tools = host.listTools(connectorId: connectorId), tools.contains(where: { $0.name == "list_events" }) else {
                throw Outlook365ConnectorError.listToolsFailed
            }

            let iso = ISO8601DateFormatter()
            let now = Date()
            let weekOut = now.addingTimeInterval(7 * 24 * 60 * 60)
            let argsJSON = "{\"time_min\":\"\(iso.string(from: now))\",\"time_max\":\"\(iso.string(from: weekOut))\"}"
            guard let resultText = host.callTool(connectorId: connectorId, toolName: "list_events", argumentsJSON: argsJSON) else {
                throw Outlook365ConnectorError.callToolFailed
            }
            let eventCount = (try? JSONDecoder().decode(OutlookEventsResponse.self, from: Data(resultText.utf8)))?.value?.count ?? 0
            return eventCount
        }.value
    }

    private func userMessage(for error: Error) -> String {
        if case MicrosoftOAuthError.signInCancelled = error {
            return "Sign-in cancelled."
        }
        if let connectorError = error as? Outlook365ConnectorError {
            return connectorError.description
        }
        return "Couldn't connect: \(error.localizedDescription)"
    }
}

/// Microsoft Graph's calendarview response wraps its array in "value", unlike Google
/// Calendar's "items".
private struct OutlookEventsResponse: Decodable {
    let value: [OutlookEvent]?
}

private struct OutlookEvent: Decodable {
    let subject: String?
}

enum Outlook365ConnectorError: Error, CustomStringConvertible {
    case nodeNotFound
    case connectorScriptNotFound
    case hostUnavailable
    case connectFailed
    case listToolsFailed
    case callToolFailed

    var description: String {
        switch self {
        case .nodeNotFound: return "Node.js isn't installed on this Mac. The Outlook 365 connector needs it to run."
        case .connectorScriptNotFound: return "The Outlook 365 connector's files weren't found in this build."
        case .hostUnavailable: return "Couldn't start the connector host."
        case .connectFailed: return "Couldn't start the Outlook 365 connector process."
        case .listToolsFailed: return "Connected, but the connector didn't report its expected capability."
        case .callToolFailed: return "Connected, but fetching your calendar events failed."
        }
    }
}
