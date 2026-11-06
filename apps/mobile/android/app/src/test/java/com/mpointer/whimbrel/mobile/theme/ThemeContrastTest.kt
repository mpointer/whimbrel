package com.mpointer.whimbrel.mobile.theme

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ThemeContrastTest {
    @Test fun defaultPassesTheGuardAndTextOnAccentIs5Point46InDark() {
        assertTrue(ThemeContrast.failures(ThemeDefaults.light).toString(), ThemeContrast.passes(ThemeDefaults.light))
        assertTrue(ThemeContrast.failures(ThemeDefaults.dark).toString(), ThemeContrast.passes(ThemeDefaults.dark))
        val d = ThemeDefaults.dark
        assertEquals(5.46, ThemeContrast.ratio(d.getValue(ThemeToken.textOnAccent), d.getValue(ThemeToken.accentPrimary)), 0.005)
    }

    @Test fun failuresNameThePair() {
        val bad = ThemeDefaults.dark + (ThemeToken.textOnAccent to 0xFFFFFF)
        assertEquals(listOf("textOnAccent on accentPrimary = 3.26"), ThemeContrast.failures(bad))
        assertEquals(21.0, ThemeContrast.ratio(0xFFFFFF, 0x000000), 1e-9)
    }
}
