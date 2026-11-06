package com.mpointer.whimbrel.mobile.sync

/** When the phone keeps pulling off screen (#572): a first sync is hours of backlog, and sync otherwise stops with the app. */
object CatchUp {
    /** Android 15 allows a dataSync foreground service 6 hours a day. */
    const val maxRunMs = 6 * 60 * 60 * 1000L

    /** Start the catch-up service: approved, and the last round succeeded and left rows behind. */
    fun shouldStart(approved: Boolean, catchingUp: Boolean): Boolean = approved && catchingUp

    /** Keep it: still approved (not signed out or revoked), still behind with no failure, and inside the 6 hours. */
    fun shouldKeepRunning(approved: Boolean, catchingUp: Boolean, startedAtMs: Long, nowMs: Long): Boolean =
        approved && catchingUp && nowMs - startedAtMs < maxRunMs
}
