package com.mpointer.whimbrel.mobile.sync

/** Sync cadence and token-renewal timing, ported from the Mac's `SyncTiming.swift` (shared there with the iPhone). Times are wall-clock ms. */
object SyncTiming {
    const val syncIntervalSeconds = 60L
    const val maxBackoffSeconds = 900L

    /** The gap between rounds while a device catches up (#572: the round itself is the throttle). */
    const val catchUpIntervalSeconds = 1L

    /** How long without a successful round before the app says so. */
    const val staleAfterSeconds = 7200L

    /** A round still running after this long counts as stuck. */
    const val stuckRoundSeconds = 300L

    /** The Worker's Turso token lasts one day; renew at half. */
    const val renewAfterSeconds = 43_200L

    /** Floor between renewal attempts, so a sync failing for a non-auth reason costs at most one Worker call per 5 minutes. */
    const val renewRetrySeconds = 300L

    /** 60 s when healthy, doubling per consecutive failure to 15 minutes; a healthy round that left rows behind comes back sooner. */
    fun nextDelaySeconds(consecutiveFailures: Int, moreToPull: Boolean = false): Long {
        if (moreToPull && consecutiveFailures == 0) return catchUpIntervalSeconds
        return minOf(syncIntervalSeconds shl consecutiveFailures.coerceIn(0, 4), maxBackoffSeconds)
    }

    /** Sync has gone quiet and is failing or stuck. A phone that just woke hasn't failed yet, so it isn't stale. */
    fun isStale(nowMs: Long, lastSuccessMs: Long?, consecutiveFailures: Int, roundStartedAtMs: Long?): Boolean {
        if (lastSuccessMs == null || nowMs - lastSuccessMs < staleAfterSeconds * 1000) return false
        val stuck = roundStartedAtMs != null && nowMs - roundStartedAtMs >= stuckRoundSeconds * 1000
        return consecutiveFailures > 0 || stuck
    }

    fun renewalDue(nowMs: Long, lastAuthAtMs: Long?, lastAttemptAtMs: Long?, afterFailure: Boolean): Boolean {
        if (lastAttemptAtMs != null && nowMs - lastAttemptAtMs < renewRetrySeconds * 1000) return false
        if (lastAuthAtMs == null) return true
        return afterFailure || nowMs - lastAuthAtMs >= renewAfterSeconds * 1000
    }
}
