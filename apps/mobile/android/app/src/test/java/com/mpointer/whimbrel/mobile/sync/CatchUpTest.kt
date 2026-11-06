package com.mpointer.whimbrel.mobile.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CatchUpTest {
    @Test
    fun startsOnlyWhenApprovedAndBehind() {
        assertTrue(CatchUp.shouldStart(approved = true, catchingUp = true))
        assertFalse("caught up", CatchUp.shouldStart(approved = true, catchingUp = false))
        assertFalse("signed out or revoked", CatchUp.shouldStart(approved = false, catchingUp = true))
    }

    @Test
    fun stopsWhenCaughtUpFailingSignedOutOrAfterSixHours() {
        val start = 1_000_000L
        assertTrue(CatchUp.shouldKeepRunning(true, true, start, start + CatchUp.maxRunMs - 1))
        assertFalse("nothing more to pull, or a failed round", CatchUp.shouldKeepRunning(true, false, start, start))
        assertFalse("sign-out or revoke", CatchUp.shouldKeepRunning(false, true, start, start))
        assertFalse("Android 15's 6-hour limit", CatchUp.shouldKeepRunning(true, true, start, start + CatchUp.maxRunMs))
    }
}
