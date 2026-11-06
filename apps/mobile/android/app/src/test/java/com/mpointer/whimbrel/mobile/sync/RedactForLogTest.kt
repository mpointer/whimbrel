package com.mpointer.whimbrel.mobile.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class RedactForLogTest {
    @Test fun urlsAndTokensAreRemovedButTheCauseStays() {
        val raw = "settings sync connection failed: TLS error at libsql://whimbrel-sync-example-me.turso.io with eyJhbGciOiJFZERTQSJ9.eyJleHAiOjE3MDAwMDB9.c2lnbmF0dXJl"
        val out = redactForLog(raw)
        assertFalse(out.contains("turso.io"))
        assertFalse(out.contains("eyJ"))
        assertEquals("settings sync connection failed: TLS error at <url> with <redacted>", out)
    }

    @Test fun anOrdinaryMessageIsUnchanged() {
        val raw = "TLS error: no valid native root CA certificates found (0 invalid) (during connect)"
        assertEquals(raw, redactForLog(raw))
    }
}
