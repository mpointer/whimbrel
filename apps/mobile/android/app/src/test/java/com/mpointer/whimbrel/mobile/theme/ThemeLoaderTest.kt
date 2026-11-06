package com.mpointer.whimbrel.mobile.theme

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ThemeLoaderTest {
    private fun load(name: String) = ThemeLoader.parse(ThemeTestFiles.invalid(name))

    @Test fun unusableFilesAreNull() {
        for (n in listOf("not-json", "no-id", "no-variants", "bad-hex")) assertNull(n, load(n))
    }

    @Test fun lowContrastLightFallsBackToDefaultsLightAndKeepsItsDark() {
        val t = load("low-contrast")!!
        assertEquals(ThemeDefaults.light, t.light)
        assertEquals(0x000000, t.dark[ThemeToken.backgroundPage])
        assertEquals(0xFFFFFF, t.dark[ThemeToken.textPrimary])
        assertEquals(ThemeVariants.Both, t.variants)
    }

    @Test fun missingTokensOverrideOnlyAccentPrimaryAndServeBothAppearances() {
        val t = load("missing-tokens")!!
        assertEquals(ThemeDefaults.light + (ThemeToken.accentPrimary to 0x336699), t.light)
        assertEquals(t.light, t.dark)
        assertEquals(ThemeVariants.LightOnly, t.variants)
    }

    @Test fun singleDarkVariantServesLightMode() {
        val t = load("single-variant")!!
        assertEquals(t.dark, t.light)
        assertEquals(0x101010, t.light[ThemeToken.backgroundPage])
        assertEquals(ThemeDefaults.dark[ThemeToken.accentPrimary], t.light[ThemeToken.accentPrimary])
        assertEquals(ThemeVariants.DarkOnly, t.variants)
    }

    @Test fun legacyThemeTakesNewTokensFromDefault() {
        val t = load("legacy-12-token")!!
        assertEquals(0x1F6F4A, t.light[ThemeToken.accentPrimary])
        for (k in listOf(ThemeToken.selection, ThemeToken.highlight, ThemeToken.textOnAccent)) assertEquals(ThemeDefaults.light[k], t.light[k])
        assertEquals(ThemeToken.entries.size, t.light.size)
    }

    @Test fun idRule() {
        assertTrue(ThemeLoader.isValidId("a-b-9"))
        for (bad in listOf("", "A", "a_b", "a b", "../x")) assertFalse(bad, ThemeLoader.isValidId(bad))
    }

    @Test fun directorySkipsDuplicatesAndDefaultIdAndSortsByName() {
        fun f(id: String, name: String) = """{"id":"$id","name":"$name","light":{"accentPrimary":"#336699"}}"""
        val files = listOf(
            "b.json" to f("zed", "Zed"), "a.json" to f("alpha", "Alpha"), "c.json" to f("alpha", "Alpha again"),
            "d.json" to f("default", "Impostor"), "e.json" to "{ nope", "readme.md" to "ignored",
        )
        val themes = ThemeLoader.loadDirectory(files)
        assertEquals(listOf("default", "alpha", "zed"), themes.map { it.id })
        assertEquals("Default", themes[0].name)
        assertNotNull(themes[1])
    }
}
