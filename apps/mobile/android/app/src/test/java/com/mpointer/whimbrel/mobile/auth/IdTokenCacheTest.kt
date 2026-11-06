package com.mpointer.whimbrel.mobile.auth

import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class IdTokenCacheTest {
    private var clock = 1_000_000L
    private var fetches = 0
    private var failNext = false

    // Token "t<n>" expires at exp = 2_000 s (2_000_000 ms); the cache refreshes 60 s early.
    private fun cache(delayMs: Long = 0) = IdTokenCache(
        now = { clock },
        fetch = {
            fetches++
            delay(delayMs)
            if (failNext) { failNext = false; error("boom") }
            "t$fetches"
        },
        expSeconds = { 2_000L },
    )

    @Test fun cachedUntilSixtySecondsBeforeExp() = runTest {
        val c = cache()
        assertEquals("t1", c.get())
        clock = 2_000_000L - 60_001
        assertEquals("t1", c.get())
        clock = 2_000_000L - 60_000
        assertEquals("t2", c.get())
        assertEquals(2, fetches)
    }

    @Test fun aSeededTokenIsServedWithoutAFetch() = runTest {
        val c = cache()
        c.seed("interactive")
        assertEquals("interactive", c.get())
        assertEquals(0, fetches)
    }

    @Test fun tenConcurrentCallsShareOneFetch() = runTest {
        val c = cache(delayMs = 100)
        val tokens = (1..10).map { async { c.get() } }.awaitAll()
        assertEquals(List(10) { "t1" }, tokens)
        assertEquals(1, fetches)
    }

    @Test fun clearForcesAFetch() = runTest {
        val c = cache()
        c.get(); c.clear()
        assertEquals("t2", c.get())
    }

    @Test fun failingFetchIsNotCachedAndNextGetRetries() = runTest {
        val c = cache()
        failNext = true
        assertThrows(IllegalStateException::class.java) { kotlinx.coroutines.runBlocking { c.get() } }
        assertEquals("t2", c.get())
        assertEquals(2, fetches)
    }

    @Test fun clearDuringInFlightFetchDropsItsToken() = runTest {
        val c = cache(delayMs = 100)
        val first = async { c.get() }
        delay(10)
        c.clear()
        first.await()
        assertEquals("t2", c.get())
        assertEquals(2, fetches)
    }
}
