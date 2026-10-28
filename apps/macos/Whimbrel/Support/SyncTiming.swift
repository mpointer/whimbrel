import Foundation

/// Sync cadence and token-renewal timing, shared by the Mac and iPhone apps (this file is
/// compiled into both) so the two can't drift.
enum SyncTiming {
    nonisolated static let syncIntervalSeconds: UInt64 = 60
    nonisolated static let maxBackoffSeconds: UInt64 = 15 * 60

    /// Normal interval when healthy; doubles per consecutive failure, capped at 15 minutes.
    nonisolated static func nextDelaySeconds(consecutiveFailures: Int) -> UInt64 {
        min(syncIntervalSeconds << UInt64(min(consecutiveFailures, 4)), maxBackoffSeconds)
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
