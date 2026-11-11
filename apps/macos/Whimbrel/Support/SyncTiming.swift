import Foundation

/// Sync cadence and token-renewal timing, shared by the Mac and iPhone apps (this file is
/// compiled into both) so the two can't drift.
enum SyncTiming {
    nonisolated static let syncIntervalSeconds: UInt64 = 60
    nonisolated static let maxBackoffSeconds: UInt64 = 15 * 60

    /// The gap between rounds while a device catches up. A round holds the store for a few
    /// seconds, so this leaves it free most of the time.
    nonisolated static let catchUpIntervalSeconds: UInt64 = 10
    /// How long without a successful round before the app says so.
    nonisolated static let staleAfterSeconds: TimeInterval = 2 * 60 * 60
    /// A round still running after this long counts as stuck.
    nonisolated static let stuckRoundSeconds: TimeInterval = 5 * 60

    /// Normal interval when healthy; doubles per consecutive failure, capped at 15 minutes.
    /// A healthy round that left rows behind comes back sooner.
    nonisolated static func nextDelaySeconds(consecutiveFailures: Int, moreToPull: Bool = false) -> UInt64 {
        if moreToPull, consecutiveFailures == 0 { return catchUpIntervalSeconds }
        return min(syncIntervalSeconds << UInt64(min(consecutiveFailures, 4)), maxBackoffSeconds)
    }

    /// How long without a finished round before the watchdog acts. A loop that is backing off
    /// sleeps up to 15 minutes between rounds, so the limit is the longer of this and the
    /// current backoff plus `overdueSlackSeconds`.
    nonisolated static let roundOverdueSeconds: TimeInterval = 10 * 60
    nonisolated static let overdueSlackSeconds: TimeInterval = 5 * 60
    /// How often the watchdog (and the backlog count) look.
    nonisolated static let watchdogIntervalSeconds: UInt64 = 5 * 60
    /// The floor between attempts to get a stopped sync going again (#590).
    nonisolated static let resumeRetrySeconds: TimeInterval = 30 * 60

    /// Whether a round has gone missing for longer than the loop's own pace explains.
    nonisolated static func roundIsOverdue(now: Date, since: Date?, consecutiveFailures: Int) -> Bool {
        guard let since else { return true }
        let pace = TimeInterval(nextDelaySeconds(consecutiveFailures: consecutiveFailures)) + overdueSlackSeconds
        return now.timeIntervalSince(since) >= max(roundOverdueSeconds, pace)
    }

    /// Sync has gone quiet. Stale means no good round for two hours and a reason: the last round
    /// failed, one is stuck, or none has run for a while (#590: the loop was gone, so nothing
    /// failed and nothing said so). `lastActivityAt` is the newest of the last finished round,
    /// the loop's last start and the last wake, so a Mac that just woke has a few minutes first.
    nonisolated static func isStale(now: Date, lastSuccess: Date?, lastRoundFinishedAt: Date?, lastActivityAt: Date?, roundStartedAt: Date?) -> Bool {
        guard let lastSuccess, now.timeIntervalSince(lastSuccess) >= staleAfterSeconds else { return false }
        if let roundStartedAt, now.timeIntervalSince(roundStartedAt) >= stuckRoundSeconds { return true }
        if let lastRoundFinishedAt, lastRoundFinishedAt > lastSuccess { return true }
        let quietSince = lastActivityAt ?? lastRoundFinishedAt ?? lastSuccess
        return now.timeIntervalSince(quietSince) >= roundOverdueSeconds
    }

    /// Another resume is allowed 30 minutes after the last one. A clock set back allows it.
    nonisolated static func resumeAllowed(now: Date, lastAttemptAt: Date?) -> Bool {
        guard let lastAttemptAt else { return true }
        let elapsed = now.timeIntervalSince(lastAttemptAt)
        return elapsed < 0 || elapsed >= resumeRetrySeconds
    }

    enum WatchdogAction: Equatable {
        case healthy
        /// Status is ready, no round is running and none has finished: replace the loop.
        case restartRound
        /// A round has been running too long. It can't be killed (it waits on the store queue),
        /// and a second round would queue behind it, so only say so.
        case roundStuck
        /// Status left ready (a refused renewal, a device limit): sign in again quietly.
        case resumeStatus
    }

    /// What the 5-minute check and the wake check do. `loopStartedAt` and `lastWakeAt` give a
    /// fresh loop and a Mac that just woke time to finish a round. `interactiveFlowActive` (a
    /// sign-in, a device swap, a fresh start) keeps the resume out of the user's way.
    nonisolated static func watchdogAction(statusIsReady: Bool, statusIsResumable: Bool, hasDeviceKey: Bool, interactiveFlowActive: Bool,
                                           consecutiveFailures: Int, now: Date, lastRoundFinishedAt: Date?, loopStartedAt: Date?,
                                           lastWakeAt: Date?, roundStartedAt: Date?) -> WatchdogAction {
        guard statusIsReady else {
            return statusIsResumable && hasDeviceKey && !interactiveFlowActive ? .resumeStatus : .healthy
        }
        if let roundStartedAt {
            return now.timeIntervalSince(roundStartedAt) >= stuckRoundSeconds ? .roundStuck : .healthy
        }
        let since = [lastRoundFinishedAt, loopStartedAt, lastWakeAt].compactMap { $0 }.max()
        return roundIsOverdue(now: now, since: since, consecutiveFailures: consecutiveFailures) ? .restartRound : .healthy
    }

    /// Changes waiting to send longer than this are worth showing (#590: a Mac sent nothing for four days).
    nonisolated static let backlogStalledSeconds: TimeInterval = 60 * 60

    nonisolated static func backlogIsStalled(pending: Int64, oldest: Date?, now: Date) -> Bool {
        guard pending > 0, let oldest else { return false }
        return now.timeIntervalSince(oldest) >= backlogStalledSeconds
    }

    /// The Worker's Turso token lasts one day (`DEVICE_TOKEN_EXPIRATION`); renew at half,
    /// matching the Windows app's `REAUTH_EVERY`.
    nonisolated static let renewAfterSeconds: TimeInterval = 12 * 60 * 60
    /// Floor between renewal attempts, so a sync that keeps failing for a non-auth
    /// reason (offline, Turso down) costs at most one Worker call per 5 minutes.
    nonisolated static let renewRetrySeconds: TimeInterval = 5 * 60

    nonisolated static func renewalDue(now: Date, lastAuthAt: Date?, lastAttemptAt: Date?, afterFailure: Bool) -> Bool {
        if let lastAttemptAt, now.timeIntervalSince(lastAttemptAt) < renewRetrySeconds {
            return false
        }
        guard let lastAuthAt else { return true }
        return afterFailure || now.timeIntervalSince(lastAuthAt) >= renewAfterSeconds
    }
}
