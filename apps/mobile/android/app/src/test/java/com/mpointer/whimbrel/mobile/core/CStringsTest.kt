package com.mpointer.whimbrel.mobile.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CStringsTest {
    @Test
    fun toCStringAppendsNul() {
        assertArrayEquals(byteArrayOf(97, 98, 99, 0), "abc".toCString())
    }

    @Test
    fun toCStringRoundTripsMultibyte() {
        val text = "😀 Meeting é"
        val bytes = text.toCString()
        assertEquals(text, bytes.copyOf(bytes.size - 1).decodeToString())
        assertEquals(4, "😀".toCString().size - 1)
    }

    @Test
    fun fromCOutputKeepsNullAndDecodesUtf8() {
        assertNull(null.fromCOutput())
        assertEquals("😀", "😀".toByteArray().fromCOutput())
    }
}
