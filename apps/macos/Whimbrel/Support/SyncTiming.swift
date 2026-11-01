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

    /// Sync has gone quiet and is failing or stuck. A Mac that just woke hasn't failed yet, so
    /// it isn't stale however old its last success is.
    nonisolated static func isStale(now: Date, lastSuccess: Date?, consecutiveFailures: Int, roundStartedAt: Date?) -> Bool {
        guard let lastSuccess, now.timeIntervalSince(lastSuccess) >= staleAfterSeconds else { return false }
        let stuck = roundStartedAt.map { now.timeIntervalSince($0) >= stuckRoundSeconds } ?? false
        return consecutiveFailures > 0 || stuck
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
