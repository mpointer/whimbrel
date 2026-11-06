package com.mpointer.whimbrel.mobile.sync

import android.content.SharedPreferences
import java.io.FileNotFoundException
import java.io.IOException
import java.nio.file.NoSuchFileException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Map-backed [SharedPreferences]; edits land on commit or apply. */
internal class FakePrefs : SharedPreferences {
    val values = mutableMapOf<String, Any?>()
    var commits = 0

    override fun getAll(): Map<String, *> = values.toMap()
    override fun getString(key: String, defValue: String?) = values[key] as String? ?: defValue
    override fun getStringSet(key: String, defValues: Set<String>?) = @Suppress("UNCHECKED_CAST") (values[key] as Set<String>? ?: defValues)
    override fun getInt(key: String, defValue: Int) = values[key] as Int? ?: defValue
    override fun getLong(key: String, defValue: Long) = values[key] as Long? ?: defValue
    override fun getFloat(key: String, defValue: Float) = values[key] as Float? ?: defValue
    override fun getBoolean(key: String, defValue: Boolean) = values[key] as Boolean? ?: defValue
    override fun contains(key: String) = key in values
    override fun registerOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener) {}
    override fun unregisterOnSharedPreferenceChangeListener(listener: SharedPreferences.OnSharedPreferenceChangeListener) {}

    override fun edit(): SharedPreferences.Editor = object : SharedPreferences.Editor {
        val pending = mutableMapOf<String, Any?>()
        val removed = mutableSetOf<String>()
        override fun putString(key: String, value: String?) = apply { pending[key] = value }
        override fun putStringSet(key: String, values: Set<String>?) = apply { pending[key] = values }
        override fun putInt(key: String, value: Int) = apply { pending[key] = value }
        override fun putLong(key: String, value: Long) = apply { pending[key] = value }
        override fun putFloat(key: String, value: Float) = apply { pending[key] = value }
        override fun putBoolean(key: String, value: Boolean) = apply { pending[key] = value }
        override fun remove(key: String) = apply { removed += key }
        override fun clear() = apply { removed += values.keys }
        override fun commit(): Boolean {
            removed.forEach { values.remove(it) }
            values.putAll(pending)
            commits += 1
            return true
        }
        override fun apply() { removed.forEach { values.remove(it) }; values.putAll(pending) }
    }
}

class StoreRemovalTest {
    private val prefs = FakePrefs()
    private fun pending() = prefs.getBoolean(StoreRemoval.pendingKey, false)

    @Test
    fun markIsCommittedSynchronously() {
        StoreRemoval.markPending(prefs)
        assertTrue(pending())
        assertEquals("written with commit, so it survives a kill right after", 1, prefs.commits)
    }

    @Test
    fun successClearsTheMarker() {
        StoreRemoval.markPending(prefs)
        var removals = 0
        StoreRemoval.finish(prefs) { removals += 1 }
        assertEquals(1, removals)
        assertFalse(pending())
    }

    @Test
    fun missingCountsAsDone() {
        for (gone in listOf(FileNotFoundException(), NoSuchFileException("x"))) {
            StoreRemoval.markPending(prefs)
            StoreRemoval.finish(prefs) { throw gone }
            assertFalse(pending())
        }
    }

    @Test
    fun anyOtherFailureKeepsTheMarkerForNextLaunch() {
        StoreRemoval.markPending(prefs)
        StoreRemoval.finish(prefs) { throw IOException("busy") }
        assertTrue(pending())
        StoreRemoval.finish(prefs) { throw IllegalStateException("odd") }
        assertTrue(pending())
        var removals = 0
        StoreRemoval.finishIfPending(prefs) { removals += 1 }
        assertEquals("a wipe cut short finishes at next launch", 1, removals)
        assertFalse(pending())
    }

    @Test
    fun finishIfPendingDoesNothingWithoutAMarker() {
        var removals = 0
        StoreRemoval.finishIfPending(prefs) { removals += 1 }
        assertEquals(0, removals)
    }
}
