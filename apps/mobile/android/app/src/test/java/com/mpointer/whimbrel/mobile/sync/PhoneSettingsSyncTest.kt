package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.core.PulledSettingsField
import com.mpointer.whimbrel.mobile.core.SettingsPulling
import com.mpointer.whimbrel.mobile.secrets.FakeCipher
import com.mpointer.whimbrel.mobile.secrets.SecretVault
import java.nio.file.Files
import kotlinx.coroutines.CancellationException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private class MapPrefs : PrefStore {
    val m = mutableMapOf<String, Any>()
    override fun getString(key: String) = m[key] as? String
    override fun getLong(key: String) = m[key] as? Long
    override fun getBoolean(key: String) = m[key] as? Boolean ?: false
    override fun putString(key: String, v: String) { m[key] = v }
    override fun putLong(key: String, v: Long) { m[key] = v }
    override fun putBoolean(key: String, v: Boolean) { m[key] = v }
    override fun clearPrefix(prefix: String) { m.keys.removeAll { it.startsWith(prefix) } }
}

/** Scripted core: one pulled delta per round and a plan honouring the key rule, like `apply_plan`. */
private class FakePuller : SettingsPulling {
    var onApplyPlan: () -> Unit = {}
    var failIterate: Throwable? = null
    var rounds = ArrayDeque<Map<String, PulledSettingsField>>()
    val localJsons = mutableListOf<String>()
    val toggles = mutableListOf<Map<String, Boolean>>()

    override fun iterate(c: SyncCredentials, localJson: String): Map<String, PulledSettingsField>? {
        failIterate?.let { throw it }
        localJsons += localJson
        return rounds.removeFirstOrNull() ?: emptyMap()
    }

    override fun applyPlan(
        pulled: Map<String, PulledSettingsField>,
        localTimestamps: Map<String, Long>,
        apiKeySyncEnabled: Map<String, Boolean>,
    ): List<PulledSettingsField> {
        toggles += apiKeySyncEnabled
        onApplyPlan()
        return pulled.filter { (k, f) ->
            if (k.startsWith("apiKey.") && !k.endsWith(".syncEnabled")) apiKeySyncEnabled[k] == true
            else f.updatedAtUnixMs > (localTimestamps[k] ?: 0)
        }.map { (k, f) -> f.copy(fieldKey = k) }
    }
}

class PhoneSettingsSyncTest {
    private val creds = SyncCredentials("t", "u", "k", "phone")
    private val p = "com.mpointer.kestrel.phone."
    private val prefs = MapPrefs()
    private val keys = PhoneKeyStore(SecretVault(Files.createTempDirectory("v").toFile().resolve("s"), FakeCipher()))
    private val puller = FakePuller()
    private val logs = mutableListOf<String>()
    private val sync = PhoneSettingsSync(puller, prefs, keys) { logs += it }

    private fun field(v: String, at: Long = 100, deleted: Boolean = false) = PulledSettingsField("", v, at, deleted)

    @Test
    fun toggleThenKeyArrivesNextRound() {
        val delta = mapOf("apiKey.anthropic.syncEnabled" to field("true"), "apiKey.anthropic" to field("sk-x"))
        puller.rounds = ArrayDeque(listOf(delta, delta))
        sync.pullOnce(creds)
        assertNull(keys.key(AskProvider.ANTHROPIC))
        assertEquals(false, puller.toggles[0]["apiKey.anthropic"])
        sync.pullOnce(creds)
        assertEquals(true, puller.toggles[1]["apiKey.anthropic"])
        assertEquals("sk-x", keys.key(AskProvider.ANTHROPIC))
        assertTrue(prefs.getBoolean(p + "apiKey.anthropic.syncEnabled"))
        assertEquals(100L, prefs.getLong(p + "settingsTs.apiKey.anthropic"))
    }

    @Test
    fun deletedKeyFieldRemovesTheStoredKey() {
        prefs.putBoolean(p + "apiKey.openai.syncEnabled", true)
        keys.save("sk-old", AskProvider.OPENAI)
        puller.rounds = ArrayDeque(listOf(mapOf("apiKey.openai" to field("", 200, true))))
        sync.pullOnce(creds)
        assertNull(keys.key(AskProvider.OPENAI))
    }

    @Test
    fun routingFieldsLandInPrefs() {
        puller.rounds = ArrayDeque(listOf(mapOf(
            "routing.preferredProvider" to field("anthropic"),
            "routing.globalDailyCapCents" to field(""),
            "summary.customPromptInstructions" to field("be brief"),
        )))
        sync.pullOnce(creds)
        assertEquals("anthropic", prefs.getString(p + "routing.preferredProvider"))
        assertEquals("", prefs.getString(p + "routing.globalDailyCapCents"))
        assertEquals("be brief", prefs.getString(p + "summary.customPromptInstructions"))
    }

    @Test
    fun neverPushes() {
        sync.pullOnce(creds)
        assertEquals("""{"fields":{},"api_keys":[]}""", puller.localJsons.single())
    }

    @Test
    fun unknownFieldIsIgnoredAndTimestampUntouched() {
        puller.rounds = ArrayDeque(listOf(mapOf("episode.hidden.1" to field("true"), "profile.name" to field("x"))))
        sync.pullOnce(creds)
        assertTrue(prefs.m.isEmpty())
    }

    @Test
    fun toggleOffDeletesKey() {
        prefs.putBoolean(p + "apiKey.google.syncEnabled", true)
        keys.save("sk-g", AskProvider.GOOGLE)
        puller.rounds = ArrayDeque(listOf(mapOf("apiKey.google.syncEnabled" to field("false", 300))))
        sync.pullOnce(creds)
        assertNull(keys.key(AskProvider.GOOGLE))
        assertFalse(prefs.getBoolean(p + "apiKey.google.syncEnabled"))
    }

    @Test
    fun wipeClearsKeysPrefsAndTimestamps() {
        keys.save("sk-x", AskProvider.ANTHROPIC)
        puller.rounds = ArrayDeque(listOf(mapOf("routing.preferredProvider" to field("openai"), "apiKey.anthropic.syncEnabled" to field("true"))))
        sync.pullOnce(creds)
        prefs.putString("unrelated", "keep")
        PhoneSettingsSync.wipeLocal(prefs, keys)
        assertNull(keys.key(AskProvider.ANTHROPIC))
        assertEquals(mapOf<String, Any>("unrelated" to "keep"), prefs.m)
    }

    @Test
    fun wipeDuringPullDropsItsWrites() {
        prefs.putBoolean(p + "apiKey.anthropic.syncEnabled", true)
        puller.onApplyPlan = { PhoneSettingsSync.wipeLocal(prefs, keys) }
        puller.rounds = ArrayDeque(listOf(mapOf("apiKey.anthropic" to field("sk-x"), "routing.preferredProvider" to field("openai"))))
        sync.pullOnce(creds)
        assertNull(keys.key(AskProvider.ANTHROPIC))
        assertTrue(prefs.m.isEmpty())
    }

    @Test
    fun wipeAfterCredentialsCapturedDropsWrites() {
        val epoch = PhoneSettingsSync.currentWipeEpoch
        PhoneSettingsSync.wipeLocal(prefs, keys)
        prefs.putBoolean(p + "apiKey.anthropic.syncEnabled", true)
        puller.rounds = ArrayDeque(listOf(mapOf("apiKey.anthropic" to field("sk-x"))))
        sync.pullOnce(creds, epoch)
        assertNull(keys.key(AskProvider.ANTHROPIC))
        assertNull(prefs.getLong(p + "settingsTs.apiKey.anthropic"))
    }

    @Test
    fun failuresAreLoggedNotThrownIncludingCancellation() {
        puller.failIterate = IllegalStateException("boom")
        sync.pullOnce(creds)
        puller.failIterate = CancellationException("stray")
        sync.pullOnce(creds)
        assertEquals(2, logs.size)
    }
}
