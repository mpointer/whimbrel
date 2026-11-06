package com.mpointer.whimbrel.mobile.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SyncTimingTest {
    private val now = 1_000_000_000L
    private fun ago(seconds: Long) = now - seconds * 1000

    @Test
    fun `cadenceIs60SecondsDoublingTo15MinutesOnFailures`() {
        assertEquals(listOf(60L, 120L, 240L, 480L, 900L, 900L, 900L), (0..6).map { SyncTiming.nextDelaySeconds(it) })
    }

    @Test
    fun `catchUpIs1SecondWhenMoreToPull`() {
        assertEquals(1L, SyncTiming.nextDelaySeconds(0, moreToPull = true))
        assertEquals("a failing round backs off whatever is left", 120L, SyncTiming.nextDelaySeconds(1, moreToPull = true))
    }

    @Test
    fun `staleOnlyWhenFailingOrStuck`() {
        val old = ago(2 * 60 * 60)
        assertFalse("never synced is not stale", SyncTiming.isStale(now, null, 3, null))
        assertFalse("recent success", SyncTiming.isStale(now, ago(60 * 60), 3, null))
        assertFalse("just woke, hasn't failed yet", SyncTiming.isStale(now, old, 0, null))
        assertTrue(SyncTiming.isStale(now, old, 1, null))
        assertFalse(SyncTiming.isStale(now, old, 0, ago(60)))
        assertTrue("a round running 5 minutes is stuck", SyncTiming.isStale(now, old, 0, ago(5 * 60)))
    }

    @Test
    fun `renewalDueAfter12HoursWithA5MinuteRetryFloor`() {
        assertTrue(SyncTiming.renewalDue(now, null, null, afterFailure = false))
        assertFalse(SyncTiming.renewalDue(now, null, ago(60), afterFailure = false))
        assertTrue(SyncTiming.renewalDue(now, null, ago(301), afterFailure = false))
        assertFalse(SyncTiming.renewalDue(now, ago(3_600), ago(3_600), afterFailure = false))
        assertTrue(SyncTiming.renewalDue(now, ago(3_600), ago(3_600), afterFailure = true))
        assertTrue(SyncTiming.renewalDue(now, ago(12 * 3_600), ago(12 * 3_600), afterFailure = false))
    }
}
