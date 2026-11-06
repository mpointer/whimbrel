package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.secrets.FakeCipher
import com.mpointer.whimbrel.mobile.secrets.SecretVault
import java.io.File
import java.io.IOException
import java.nio.file.Files
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

private class WipeMapPrefs(val events: MutableList<String>, val marker: () -> Boolean) : PrefStore {
    val m = mutableMapOf<String, Any>()
    override fun getString(key: String) = m[key] as? String
    override fun getLong(key: String) = m[key] as? Long
    override fun getBoolean(key: String) = m[key] as? Boolean ?: false
    override fun putString(key: String, v: String) { m[key] = v }
    override fun putLong(key: String, v: Long) { m[key] = v }
    override fun putBoolean(key: String, v: Boolean) { m[key] = v }
    var failClear = false
    override fun clearPrefix(prefix: String) {
        if (failClear) throw IllegalStateException("disk")
        events += "clearPrefix(marker pending=${marker()})"
        m.keys.removeAll { it.startsWith(prefix) }
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class WipeWiringTest {
    private val events = mutableListOf<String>()
    private val removal = FakePrefs()
    private val prefs = WipeMapPrefs(events) { removal.getBoolean(StoreRemoval.pendingKey, false) }
    private val keys = PhoneKeyStore(SecretVault(Files.createTempDirectory("v").toFile().resolve("s"), FakeCipher()))
    private val scope = TestScope()
    private val dispatcher = StandardTestDispatcher(scope.testScheduler)
    private val sync = SyncModel(
        openStore = { null as MeetingSyncStore? },
        credentials = object : SyncCredentialSource {
            override fun syncCredentials(): SyncCredentials? = null
            override suspend fun renewIfDue(afterFailure: Boolean, force: Boolean) {}
        },
        scope = scope,
        storeDispatcher = dispatcher,
        sleep = {},
        clock = { 0L },
    )
    private val tmp = Files.createTempDirectory("wipe").toFile()
    private val storeDir = File(tmp, "Sync").apply { mkdirs(); File(this, "memory.sqlite").writeText("x") }
    private val pending get() = removal.getBoolean(StoreRemoval.pendingKey, false)
    private val logs = mutableListOf<String>()
    private val handler = wipeHandler(prefs, removal, keys, sync, storeDir) { logs += it }

    @After
    fun cleanUp() {
        tmp.walkTopDown().forEach { it.setWritable(true) }
        tmp.deleteRecursively()
    }

    @Test
    fun wipeClearsSyncedPrefsAndKeysFirst() {
        prefs.putString(PhoneSettingsSync.PREFIX + "routing.x", "v")
        keys.save("k", AskProvider.OPENAI)
        handler()
        assertEquals(listOf("clearPrefix(marker pending=false)"), events)
        assertTrue(prefs.m.isEmpty())
        assertEquals(null, keys.key(AskProvider.OPENAI))
    }

    @Test
    fun wipeMarksPendingResetsDeletesThenClearsTheMarker() = scope.runTest {
        handler()
        assertTrue("marked before the store thread runs", pending)
        assertTrue(storeDir.exists())
        advanceUntilIdle()
        assertFalse(storeDir.exists())
        assertFalse(pending)
    }

    @Test
    fun failingDeleteLeavesTheMarkerAndFinishIfPendingFinishesIt() = scope.runTest {
        val locked = File(storeDir, "inner").apply { mkdirs(); File(this, "f").writeText("x") }
        storeDir.setWritable(false)
        locked.setWritable(false)
        assertThrows(IOException::class.java) { deleteStoreDir(storeDir) }
        handler()
        advanceUntilIdle()
        assertTrue("a failed delete keeps the marker", pending)
        storeDir.walkTopDown().forEach { it.setWritable(true) }
        StoreRemoval.finishIfPending(removal) { deleteStoreDir(storeDir) }
        assertFalse(storeDir.exists())
        assertFalse(pending)
    }

    @Test
    fun deleteStoreDirOfAMissingDirIsDone() {
        deleteStoreDir(File(tmp, "never-created"))
    }

    @Test
    fun aFailingLocalWipeStillMarksAndResetsTheStore() = scope.runTest {
        prefs.failClear = true
        handler()
        assertTrue(pending)
        assertEquals(listOf("IllegalStateException"), logs.map { it.substringAfterLast(' ') })
        advanceUntilIdle()
        assertFalse("reset still ran and deleted the store", storeDir.exists())
        assertFalse(pending)
    }

    @Test
    fun openIsRefusedWhileARemovalIsPendingAndCannotFinish() {
        StoreRemoval.markPending(removal)
        storeDir.setWritable(false)
        var opened = 0
        val result = openStoreGuarded(removal, storeDir) { opened += 1; "store" }
        assertEquals(null, result)
        assertEquals(0, opened)
        assertTrue(pending)
        storeDir.setWritable(true)
        assertEquals("store", openStoreGuarded(removal, storeDir) { opened += 1; "store" })
        assertEquals(1, opened)
        assertFalse(storeDir.exists())
        assertFalse(pending)
    }

    @Test
    fun openWithoutAMarkerJustOpens() {
        assertEquals("store", openStoreGuarded(removal, storeDir) { "store" })
        assertTrue(storeDir.exists())
    }
}
