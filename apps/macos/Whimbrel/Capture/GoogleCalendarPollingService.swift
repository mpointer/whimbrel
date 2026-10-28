import Combine
import Foundation
import OSLog

/// Polls Whimbrel's own Google Calendar OAuth connector for every connected account
/// (multi-account, shipped 2026-08-05) every 5 minutes, so meeting detection works for
/// a connected Google account with no macOS-level Calendar sync required. Mirrors
/// `CalendarConnectorState.performMcpRoundTrip`'s exact spawn/connect/callTool/
/// disconnect `McpHost` pattern -- this class owns no OAuth/account state of its own,
/// it only reads `CalendarConnectorState`'s (read-only, never mutated).
///
/// A failure for one account (expired token, network error, Google API error) logs and
/// keeps that account's last successfully-fetched meetings (`lastKnownGoodMeetings`)
/// rather than blanking its contribution to `oauthMeetings` for the cycle -- a transient
/// failure for one account must never clear or block the rest of the merged meeting
/// list. An account's retained meetings are only dropped once it's no longer in
/// `calendarConnectorState.accounts` (removed by the user), not merely because a poll
/// failed. Also re-polls immediately whenever `calendarConnectorState.accounts` changes,
/// so connecting an account doesn't leave the Schedule panel empty for up to 5 minutes.
@MainActor
final class GoogleCalendarPollingService: ObservableObject {
    @Published private(set) var oauthMeetings: [QualifyingMeeting] = []

    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "google-calendar-polling")
    private var pollTimer: Timer?
    private var accountsCancellable: AnyCancellable?
    private static let pollInterval: TimeInterval = 5 * 60
    private weak var coreStore: CoreMemoryStore?

    /// Retained per account so a transient failure for one account doesn't blank its
    /// contribution to `oauthMeetings` -- only replaced when that account's fetch
    /// actually succeeds. Pruned of any account that's no longer connected, so a
    /// removed account's meetings don't linger forever.
    private var lastKnownGoodMeetings: [String: [QualifyingMeeting]] = [:]

    func start(calendarConnectorState: CalendarConnectorState, coreStore: CoreMemoryStore?) {
        self.coreStore = coreStore
        pollTimer?.invalidate()
        Task { await poll(calendarConnectorState: calendarConnectorState) }
        pollTimer = Timer.scheduledTimer(withTimeInterval: Self.pollInterval, repeats: true) { [weak self, weak calendarConnectorState] _ in
            guard let self, let calendarConnectorState else { return }
            Task { await self.poll(calendarConnectorState: calendarConnectorState) }
        }
        // Re-poll immediately when the connected-account list changes (e.g. right after
        // Add Account or Remove Account), rather than waiting up to 5 minutes for the
        // next scheduled tick -- `dropFirst()` skips the initial value Combine fires
        // immediately on subscribe, since the explicit poll() call above already covers
        // app launch.
        accountsCancellable = calendarConnectorState.$accounts
            .dropFirst()
            .sink { [weak self, weak calendarConnectorState] _ in
                guard let self, let calendarConnectorState else { return }
                Task { await self.poll(calendarConnectorState: calendarConnectorState) }
            }
    }

    func stop() {
        pollTimer?.invalidate()
        pollTimer = nil
        accountsCancellable?.cancel()
        accountsCancellable = nil
    }

    private func poll(calendarConnectorState: CalendarConnectorState) async {
        for account in calendarConnectorState.accounts {
            guard let helper = calendarConnectorState.oauthHelper(forEmail: account.email) else {
                log.error("poll: no OAuth helper for \(account.email, privacy: .private) -- account listed but not connected")
                continue
            }
            do {
                let accessToken = try await helper.validAccessToken()
                try await syncAccount(email: account.email, accessToken: accessToken)
                calendarConnectorState.clearNeedsReconnect(account.email)
            } catch {
                switch error {
                case GoogleOAuthError.notAuthorized, GoogleOAuthError.refreshTokenRevoked:
                    calendarConnectorState.markNeedsReconnect(account.email)
                default: break
                }
                log.error("poll: failed for \(account.email, privacy: .private): \(String(describing: error), privacy: .public) -- keeping last-known-good meetings for this account")
            }
        }
        let connectedEmails = Set(calendarConnectorState.accounts.map(\.email))
        lastKnownGoodMeetings = lastKnownGoodMeetings.filter { connectedEmails.contains($0.key) }
        oauthMeetings = lastKnownGoodMeetings.values.flatMap { $0 }
    }

    /// One account's sync iteration, in the three ADR 0001 phases core/api::connectors
    /// exposes (M3.0 Task 8): `calendarSyncRequest` reads the stored token under
    /// `accessQueue`; the `McpHost` round trip runs with no store access; `applyCalendarSync`
    /// deletes cancelled events, advances the token only after that succeeds, and folds
    /// the incremental result into this account's known meetings. A `needsFullResync`
    /// response clears the stored token and retries once with none, which naturally
    /// produces a full baseline sync and a fresh token.
    private func syncAccount(email: String, accessToken: String, isRetryAfterResync: Bool = false) async throws {
        guard let coreStore else {
            log.error("syncAccount: no CoreMemoryStore available for \(email, privacy: .private) -- skipping this account's Google Calendar sync entirely (its meetings will not appear)")
            return
        }
        // The stored token outlives the app, but `lastKnownGoodMeetings` doesn't: with a
        // token, Google returns only what changed since it, which folded into an empty
        // list after a relaunch reads as an empty day. So each account's first sync in a
        // run starts from no token, a full baseline.
        let needsBaseline = lastKnownGoodMeetings[email] == nil
        guard let request = await Self.withStoreAccess({ () -> CoreConnectorToolCall? in
            if needsBaseline { coreStore.clearCalendarSyncToken(accountEmail: email) }
            return coreStore.calendarSyncRequest(accountEmail: email)
        }) else {
            log.error("syncAccount: couldn't read the sync token for \(email, privacy: .private)")
            return
        }
        let resultJSON = try await Self.callConnector(tool: request.tool, argsJSON: request.argsJson, accessToken: accessToken)
        let existing = lastKnownGoodMeetings[email] ?? []
        let nowUnixMs = Int64(Date().timeIntervalSince1970 * 1000)
        guard let outcome = await Self.withStoreAccess({
            coreStore.applyCalendarSync(accountEmail: email, resultJSON: resultJSON, existing: existing, nowUnixMs: nowUnixMs)
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

        lastKnownGoodMeetings[email] = outcome.meetings
        if outcome.deletionFailed {
            log.error("syncAccount: deleting \(outcome.cancelledCount) cancelled calendar event(s) failed for \(email, privacy: .private) -- not saving the new sync token, so the next poll re-delivers them")
        }
    }

    private static func withStoreAccess<T>(_ work: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async { continuation.resume(returning: work()) }
        }
    }

    /// The `McpHost` round trip (spawn/connect/callTool/disconnect), mirroring
    /// `CalendarConnectorState.performMcpRoundTrip`. Returns the tool's raw result text.
    private static func callConnector(tool: String, argsJSON: String, accessToken: String) async throws -> String {
        try await Task.detached(priority: .utility) {
            guard let nodePath = McpNodeLocator.nodeExecutablePath() else {
                throw GoogleCalendarPollingError.nodeNotFound
            }
            guard let scriptPath = McpNodeLocator.calendarConnectorScriptPath() else {
                throw GoogleCalendarPollingError.connectorScriptNotFound
            }
            guard let host = McpHost() else {
                throw GoogleCalendarPollingError.hostUnavailable
            }
            let args = McpNodeLocator.nodeNeedsEnvWrapper(resolvedPath: nodePath) ? ["node", scriptPath] : [scriptPath]
            guard let connectorId = host.connect(
                command: nodePath, args: args,
                env: [("GOOGLE_CALENDAR_ACCESS_TOKEN", accessToken)]
            ) else {
                throw GoogleCalendarPollingError.connectFailed
            }
            defer { host.disconnect(connectorId: connectorId) }

            guard host.listTools(connectorId: connectorId)?.contains(where: { $0.name == tool }) == true else {
                throw GoogleCalendarPollingError.listToolsFailed
            }
            guard let resultText = host.callTool(connectorId: connectorId, toolName: tool, argumentsJSON: argsJSON) else {
                let message = host.lastError?.replacingOccurrences(of: #"[^\s@]+@[^\s@]+"#, with: "<email>", options: .regularExpression)
                Logger(subsystem: "com.mpointer.kestrel", category: "google-calendar-polling")
                    .error("callConnector: \(tool, privacy: .public) failed: \(message ?? "no error text", privacy: .public)")
                throw GoogleCalendarPollingError.callToolFailed
            }
            return resultText
        }.value
    }
}

private enum GoogleCalendarPollingError: Error {
    case nodeNotFound
    case connectorScriptNotFound
    case hostUnavailable
    case connectFailed
    case listToolsFailed
    case callToolFailed
}
