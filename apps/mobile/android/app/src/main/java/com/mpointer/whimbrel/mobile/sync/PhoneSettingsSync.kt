package com.mpointer.whimbrel.mobile.sync

import android.util.Log
import com.mpointer.whimbrel.mobile.core.PulledSettingsField
import com.mpointer.whimbrel.mobile.core.SettingsPulling
import com.mpointer.whimbrel.mobile.secrets.SecretVault

/** The providers Ask can route to. [raw] is the Mac's provider raw value and the `apiKey.<p>` field-key suffix. */
enum class AskProvider(val raw: String) { ANTHROPIC("anthropic"), OPENAI("openai"), GOOGLE("google"), OPENROUTER("openrouter") }

/** SharedPreferences-backed in the app, a map in tests. */
interface PrefStore {
    fun getString(key: String): String?
    fun getLong(key: String): Long?
    fun getBoolean(key: String): Boolean
    fun putString(key: String, v: String)
    fun putLong(key: String, v: Long)
    fun putBoolean(key: String, v: Boolean)
    fun clearPrefix(prefix: String)
}

/** API keys synced from the account, in the vault. The vault rejects `.` in names, so they are `apiKey-<provider>`. */
class PhoneKeyStore(private val vault: SecretVault) {
    private fun name(p: AskProvider) = "apiKey-${p.raw}"

    fun key(p: AskProvider): String? = vault.get(name(p))

    fun save(key: String, p: AskProvider): Boolean = try {
        vault.put(name(p), key)
        true
    } catch (_: Exception) {
        false
    }

    fun delete(p: AskProvider) = vault.delete(name(p))

    fun deleteAll() = AskProvider.entries.forEach(::delete)
}

/**
 * Pull-only settings sync: takes the account's API keys and routing fields. Never pushes.
 * A key is applied only while this phone's toggle for it is on, and the toggle arrives in the same
 * round as the key, so the key lands one round later (the core's `apply_plan`).
 */
class PhoneSettingsSync(
    private val puller: SettingsPulling,
    private val prefs: PrefStore,
    private val keys: PhoneKeyStore,
    private val log: (String) -> Unit = { Log.w("settings-sync", it) },
) {
    /**
     * One round, blocking: call off the main thread. A failure is logged and never thrown (a stray
     * CancellationException would end the caller's loop). [epoch] is [currentWipeEpoch] read when the
     * credentials were captured, so a wipe since then also drops the writes; null reads it now.
     */
    fun pullOnce(credentials: SyncCredentials, epoch: Int? = null) {
        val start = epoch ?: currentWipeEpoch
        try {
            val pulled = puller.iterate(credentials, EMPTY_LOCAL) ?: return log("settings pull failed")
            val relevant = pulled.filterKeys(::handles)
            val timestamps = relevant.keys.mapNotNull { k -> prefs.getLong("${PREFIX}settingsTs.$k")?.let { k to it } }.toMap()
            val toggles = AskProvider.entries.associate { "apiKey.${it.raw}" to prefs.getBoolean("${PREFIX}apiKey.${it.raw}.syncEnabled") }
            val plan = puller.applyPlan(relevant, timestamps, toggles) ?: return log("settings apply plan failed")
            for (field in plan) {
                if (!handles(field.fieldKey)) continue
                synchronized(lock) {
                    if (wipeEpoch != start) return log("wipe during settings pull: dropping the rest")
                    apply(field)
                }
            }
        } catch (e: Exception) {
            log("settings pull error: ${e.javaClass.simpleName}")
        }
    }

    private fun apply(field: PulledSettingsField) {
        val key = field.fieldKey
        val keyOf = AskProvider.entries.firstOrNull { key == "apiKey.${it.raw}" }
        val toggleOf = AskProvider.entries.firstOrNull { key == "apiKey.${it.raw}.syncEnabled" }
        when {
            keyOf != null -> if (field.isDeleted) keys.delete(keyOf) else if (!keys.save(field.value, keyOf)) return
            toggleOf != null -> {
                val on = !field.isDeleted && field.value == "true"
                prefs.putBoolean("${PREFIX}$key", on)
                if (!on) keys.delete(toggleOf)
            }
            else -> prefs.putString(PREFIX + key, if (field.isDeleted) "" else field.value)
        }
        prefs.putLong("${PREFIX}settingsTs.$key", field.updatedAtUnixMs)
    }

    companion object {
        const val PREFIX = "com.mpointer.kestrel.phone."
        private const val EMPTY_LOCAL = """{"fields":{},"api_keys":[]}"""
        private val lock = Any()
        private var wipeEpoch = 0 // guarded by lock

        /** Bumped by every wipe; a pull that started before one drops its remaining writes. */
        val currentWipeEpoch: Int get() = synchronized(lock) { wipeEpoch }

        /** Revoke, wipe, fresh install: forget every synced key, setting and timestamp. Same lock as every apply. */
        fun wipeLocal(prefs: PrefStore, keys: PhoneKeyStore) = synchronized(lock) {
            wipeEpoch++
            keys.deleteAll()
            prefs.clearPrefix(PREFIX)
        }

        private fun handles(key: String) =
            key.startsWith("routing.") || key == "summary.customPromptInstructions" ||
                AskProvider.entries.any { key == "apiKey.${it.raw}" || key == "apiKey.${it.raw}.syncEnabled" }
    }
}
