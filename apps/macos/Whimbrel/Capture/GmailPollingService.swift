import Combine
import Foundation
import OSLog

/// Polls Whimbrel's own Gmail OAuth connector for every connected account (mirrors
/// `GmailConnectorState`'s multi-account shape) every 15 minutes, incrementally
/// syncing changed messages into the `items` table via `sync_messages`. Unlike
/// `GoogleCalendarPollingService`, this service has no UI-facing output -- its only
/// job is landing rows in `items` for future consumers (Thread distillation) to read.
/// See docs/superpowers/specs/2026-08-13-gmail-incremental-sync-design.md.
@MainActor
final class GmailPollingService {
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "gmail-polling")
    private var pollTimer: Timer?
    private var accountsCancellable: AnyCancellable?
    private static let pollInterval: TimeInterval = 15 * 60
    private weak var coreStore: CoreMemoryStore?
    private var deviceId: String = ""

    func start(gmailConnectorState: GmailConnectorState, coreStore: CoreMemoryStore?, deviceId: String) {
        self.coreStore = coreStore
        self.deviceId = deviceId
        pollTimer?.invalidate()
        Task { await poll(gmailConnectorState: gmailConnectorState) }
        pollTimer = Timer.scheduledTimer(withTimeInterval: Self.pollInterval, repeats: true) { [weak self, weak gmailConnectorState] _ in
            guard let self, let gmailConnectorState else { return }
            Task { await self.poll(gmailConnectorState: gmailConnectorState) }
        }
        accountsCancellable = gmailConnectorState.$accounts
            .dropFirst()
            .sink { [weak self, weak gmailConnectorState] _ in
                guard let self, let gmailConnectorState else { return }
                Task { await self.poll(gmailConnectorState: gmailConnectorState) }
            }
    }

    func stop() {
        pollTimer?.invalidate()
        pollTimer = nil
        accountsCancellable?.cancel()
        accountsCancellable = nil
    }

    private func poll(gmailConnectorState: GmailConnectorState) async {
        for account in gmailConnectorState.accounts {
            guard let helper = gmailConnectorState.oauthHelper(forEmail: account.email) else {
                log.error("poll: no OAuth helper for \(account.email, privacy: .private) -- account listed but not connected")
                continue
            }
            do {
                let accessToken = try await helper.validAccessToken()
                try await syncAccount(email: account.email, accessToken: accessToken)
                gmailConnectorState.clearNeedsReconnect(account.email)
            } catch {
                switch error {
                case GoogleOAuthError.notAuthorized, GoogleOAuthError.refreshTokenRevoked:
                    gmailConnectorState.markNeedsReconnect(account.email)
                default: break
                }
                log.error("poll: failed for \(account.email, privacy: .private): \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// One account's sync iteration, in the three ADR 0001 phases core/api::connectors
    /// exposes (M3.0 Task 8): `gmailSyncRequest` reads the stored historyId under
    /// `accessQueue`; the `McpHost` round trip runs with no store access;
    /// `applyGmailSync` deletes, upserts, and advances the cursor only if both
    /// succeeded (Gmail never re-reports a change against a newer historyId). A
    /// `needsFullResync` response clears the cursor and retries once with none.
    private func syncAccount(email: String, accessToken: String, isRetryAfterResync: Bool = false) async throws {
        guard let coreStore else {
            log.error("syncAccount: no CoreMemoryStore available for \(email, privacy: .private) -- skipping Gmail sync entirely for this account")
            return
        }
        guard let request = await Self.withStoreAccess({ coreStore.gmailSyncRequest(accountEmail: email) }) else {
            log.error("syncAccount: couldn't read the historyId for \(email, privacy: .private)")
            return
        }
        let resultJSON = try await Self.callConnector(tool: request.tool, argsJSON: request.argsJson, accessToken: accessToken)
        let deviceId = self.deviceId
        guard let outcome = await Self.withStoreAccess({
            coreStore.applyGmailSync(accountEmail: email, deviceId: deviceId, resultJSON: resultJSON)
        }) else {
            log.error("syncAccount: applying the sync result failed for \(email, privacy: .private)")
            return
        }

        if outcome.needsFullResync {
            guard !isRetryAfterResync else {
                log.error("syncAccount: needsFullResync again immediately after a resync retry for \(email, privacy: .private) -- giving up for this poll cycle")
                return
            }
            try await syncAccount(email: email, accessToken: accessToken, isRetryAfterResync: true)
            return
        }
        if !outcome.applied {
            log.error("syncAccount: applying \(outcome.upserted) upsert(s) / \(outcome.deleted) deletion(s) failed for \(email, privacy: .private) -- historyId not advanced, next poll will redo this range")
        }
    }

    private static func withStoreAccess<T>(_ work: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async { continuation.resume(returning: work()) }
        }
    }

    /// The `McpHost` round trip (spawn/connect/callTool/disconnect). Returns the tool's
    /// raw result text.
    private static func callConnector(tool: String, argsJSON: String, accessToken: String) async throws -> String {
        try await Task.detached(priority: .utility) {
            guard let nodePath = McpNodeLocator.nodeExecutablePath() else {
                throw GmailPollingError.nodeNotFound
            }
            guard let scriptPath = McpNodeLocator.gmailConnectorScriptPath() else {
                throw GmailPollingError.connectorScriptNotFound
            }
            guard let host = McpHost() else {
                throw GmailPollingError.hostUnavailable
            }
            let args = McpNodeLocator.nodeNeedsEnvWrapper(resolvedPath: nodePath) ? ["node", scriptPath] : [scriptPath]
            guard let connectorId = host.connect(
                command: nodePath, args: args,
                env: [("GMAIL_ACCESS_TOKEN", accessToken)]
            ) else {
                throw GmailPollingError.connectFailed
            }
            defer { host.disconnect(connectorId: connectorId) }

            guard host.listTools(connectorId: connectorId)?.contains(where: { $0.name == tool }) == true else {
                throw GmailPollingError.listToolsFailed
            }
            guard let resultText = host.callTool(connectorId: connectorId, toolName: tool, argumentsJSON: argsJSON) else {
                let message = host.lastError?.replacingOccurrences(of: #"[^\s@]+@[^\s@]+"#, with: "<email>", options: .regularExpression)
                Logger(subsystem: "com.mpointer.kestrel", category: "gmail-polling")
                    .error("callConnector: \(tool, privacy: .public) failed: \(message ?? "no error text", privacy: .public)")
                throw GmailPollingError.callToolFailed
            }
            return resultText
        }.value
    }
}

private enum GmailPollingError: Error {
    case nodeNotFound
    case connectorScriptNotFound
    case hostUnavailable
    case connectFailed
    case listToolsFailed
    case callToolFailed
}
