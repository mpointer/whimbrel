package com.mpointer.whimbrel.mobile.update

import android.content.SharedPreferences
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** A newer release on the public release page. */
data class AvailableUpdate(val version: String, val pageUrl: String)

/**
 * A link-installed app never updates itself, so the phone asks the public repository for its latest release
 * at most once a day, only while the app is open. The request carries no personal data.
 */
object UpdateCheck {
    const val LATEST_URL = "https://api.github.com/repos/mpointer/whimbrel/releases/latest"
    const val CHECK_EVERY_MS = 24 * 60 * 60 * 1000L

    /** True when [latest] (e.g. "v0.9.60") is a higher dotted version than [installed] ("0.9.59"). */
    fun isNewer(latest: String, installed: String): Boolean {
        fun parts(v: String) = v.trim().removePrefix("v").substringBefore('-').split('.').map { it.toIntOrNull() ?: 0 }
        val a = parts(latest)
        val b = parts(installed)
        for (i in 0 until maxOf(a.size, b.size)) {
            val d = a.getOrElse(i) { 0 } - b.getOrElse(i) { 0 }
            if (d != 0) return d > 0
        }
        return false
    }

    /** The release in GitHub's "latest release" JSON, or null when it can't be read. */
    fun parse(json: String): AvailableUpdate? = runCatching {
        val o = Json.parseToJsonElement(json).jsonObject
        AvailableUpdate(o["tag_name"]!!.jsonPrimitive.content.removePrefix("v"), o["html_url"]!!.jsonPrimitive.content)
    }.getOrNull()

    suspend fun fetchLatest(): String? = withContext(Dispatchers.IO) {
        runCatching {
            val c = URL(LATEST_URL).openConnection() as HttpURLConnection
            c.connectTimeout = 10_000
            c.readTimeout = 10_000
            c.setRequestProperty("Accept", "application/vnd.github+json")
            c.setRequestProperty("User-Agent", "Whimbrel-Android")
            try {
                if (c.responseCode == 200) c.inputStream.bufferedReader().readText() else null
            } finally {
                c.disconnect()
            }
        }.getOrNull()
    }
}

/** Whether to show "Update available": checks when due, remembers a dismissed version. */
class UpdateModel(
    private val prefs: SharedPreferences,
    private val installed: String,
    private val now: () -> Long = System::currentTimeMillis,
    private val fetch: suspend () -> String? = UpdateCheck::fetchLatest,
) {
    private val _available = MutableStateFlow<AvailableUpdate?>(null)
    val available: StateFlow<AvailableUpdate?> = _available.asStateFlow()

    suspend fun checkIfDue() {
        val last = if (prefs.contains(LAST_CHECK)) prefs.getLong(LAST_CHECK, 0L) else null
        if (last != null && now() - last < UpdateCheck.CHECK_EVERY_MS) {
            _available.value = stored()
            return
        }
        val latest = fetch()?.let(UpdateCheck::parse) ?: return
        prefs.edit().putLong(LAST_CHECK, now()).putString(LATEST_VERSION, latest.version).putString(LATEST_URL_KEY, latest.pageUrl).apply()
        _available.value = shown(latest)
    }

    fun dismiss() {
        _available.value?.let { prefs.edit().putString(DISMISSED, it.version).apply() }
        _available.value = null
    }

    private fun stored(): AvailableUpdate? {
        val v = prefs.getString(LATEST_VERSION, null) ?: return null
        val u = prefs.getString(LATEST_URL_KEY, null) ?: return null
        return shown(AvailableUpdate(v, u))
    }

    private fun shown(u: AvailableUpdate): AvailableUpdate? =
        u.takeIf { UpdateCheck.isNewer(it.version, installed) && prefs.getString(DISMISSED, null) != it.version }

    companion object {
        const val PREFS = "update_check"
        private const val LAST_CHECK = "last_check_ms"
        private const val LATEST_VERSION = "latest_version"
        private const val LATEST_URL_KEY = "latest_url"
        private const val DISMISSED = "dismissed_version"
    }
}
