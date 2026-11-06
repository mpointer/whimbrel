package com.mpointer.whimbrel.mobile.theme

import com.mpointer.whimbrel.mobile.sync.PrefStore
import org.junit.Assert.assertEquals
import org.junit.Test

class ThemeStoreTest {
    private class MapPrefs(val m: MutableMap<String, String> = mutableMapOf()) : PrefStore {
        override fun getString(key: String) = m[key]
        override fun getLong(key: String): Long? = null
        override fun getBoolean(key: String) = false
        override fun putString(key: String, v: String) { m[key] = v }
        override fun putLong(key: String, v: Long) {}
        override fun putBoolean(key: String, v: Boolean) {}
        override fun clearPrefix(prefix: String) {}
    }

    private val sepia = ThemeDefaults.theme.copy(id = "sepia", name = "Sepia")

    @Test fun selectionPersistsAndUnknownStoredIdFallsBackToDefault() {
        val prefs = MapPrefs()
        val s = ThemeStore(prefs)
        s.install(listOf(sepia))
        s.select("sepia")
        assertEquals("sepia", prefs.m["com.mpointer.kestrel.themeId"])
        assertEquals("sepia", ThemeStore(prefs).also { it.install(listOf(sepia)) }.selectedId.value)
        // The stored theme is gone: Default, and the store still remembers the choice.
        val fresh = ThemeStore(prefs)
        assertEquals("default", fresh.selectedId.value)
        fresh.install(listOf(sepia))
        assertEquals("sepia", fresh.selectedId.value)
    }

    @Test fun unknownSelectIsIgnoredAndDefaultStaysFirst() {
        val s = ThemeStore(MapPrefs())
        s.install(listOf(sepia, ThemeDefaults.theme.copy(name = "Impostor")))
        s.select("nope")
        assertEquals("default", s.selectedId.value)
        assertEquals(listOf("default", "sepia"), s.themes.value.map { it.id })
        assertEquals("Default", s.themes.value[0].name)
    }
}
