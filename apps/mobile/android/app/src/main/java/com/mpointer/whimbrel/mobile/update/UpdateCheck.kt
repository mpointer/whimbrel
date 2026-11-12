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
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** The newest release that ships an Android app file: [apkUrl] downloads it, [pageUrl] is the release notes page. */
data class AvailableUpdate(val version: String, val apkUrl: String, val pageUrl: String)

/**
 * A link-installed app never updates itself, so the phone asks the public repository for its latest release
 * at most every 6 hours: each time the app comes back to the screen, and from a background check that posts a
 * notification once per new version (UpdateCheckWorker). The request carries no personal data.
 * Most releases ship no APK, so it reads the recent releases and takes the newest one that has one.
 */
object UpdateCheck {
    const val RELEASES_URL = "https://api.github.com/repos/mpointer/whimbrel/releases?per_page=20"
    const val CHECK_EVERY_MS = 6 * 60 * 60 * 1000L

    /** True when [latest] (e.g. "v0.9.60") is a higher dotted version than [installed] ("0.9.59"). */
    fun isNewer(latest: String, installed: String): Boolean = compareVersions(latest, installed) > 0

    private fun compareVersions(x: String, y: String): Int {
        fun parts(v: String) = v.trim().removePrefix("v").substringBefore('-').split('.').map { it.toIntOrNull() ?: 0 }
        val a = parts(x)
        val b = parts(y)
        for (i in 0 until maxOf(a.size, b.size)) {
            val d = a.getOrElse(i) { 0 }.compareTo(b.getOrElse(i) { 0 })
            if (d != 0) return d
        }
        return 0
    }

    private val TAG = Regex("""^v?\d+(\.\d+)*(-.*)?$""")

    /**
     * From GitHub's releases list, the highest-versioned published release (no drafts or prereleases) that has an
     * `.apk` asset, or null when there is none or the JSON can't be read. List order is not trusted.
     */
    fun parse(json: String): AvailableUpdate? = runCatching {
        Json.parseToJsonElement(json).jsonArray.mapNotNull { runCatching { candidate(it.jsonObject) }.getOrNull() }
            .reduceOrNull { best, next -> if (compareVersions(next.version, best.version) > 0) next else best }
    }.getOrNull()

    private fun candidate(o: kotlinx.serialization.json.JsonObject): AvailableUpdate? {
        if (o["draft"]?.jsonPrimitive?.booleanOrNull == true || o["prerelease"]?.jsonPrimitive?.booleanOrNull == true) return null
        val tag = o["tag_name"]!!.jsonPrimitive.content
        if (!TAG.matches(tag.trim())) return null
        val apk = o["assets"]!!.jsonArray.map { it.jsonObject }
            .firstOrNull { it["name"]!!.jsonPrimitive.content.endsWith(".apk", ignoreCase = true) } ?: return null
        return AvailableUpdate(tag.removePrefix("v"), apk["browser_download_url"]!!.jsonPrimitive.content, o["html_url"]!!.jsonPrimitive.content)
    }

    suspend fun fetchLatest(): String? = withContext(Dispatchers.IO) {
        runCatching {
            val c = URL(RELEASES_URL).openConnection() as HttpURLConnection
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
        // A cache written before the APK link existed has no usable download URL, so it is checked again.
        val cacheUsable = !prefs.contains(LATEST_VERSION) || prefs.contains(LATEST_APK_KEY)
        if (last != null && cacheUsable && now() - last < UpdateCheck.CHECK_EVERY_MS) {
            _available.value = stored()
            return
        }
        val latest = fetch()?.let(UpdateCheck::parse) ?: return
        prefs.edit().putLong(LAST_CHECK, now()).putString(LATEST_VERSION, latest.version).putString(LATEST_APK_KEY, latest.apkUrl).putString(LATEST_URL_KEY, latest.pageUrl).apply()
        _available.value = shown(latest)
    }

    /** The update to announce in a notification, once per version: null if there is none, it was dismissed
     * with Not now, or this version was already announced. Marks it announced. */
    fun takeNotification(): AvailableUpdate? {
        val u = _available.value ?: return null
        if (prefs.getString(NOTIFIED, null) == u.version) return null
        prefs.edit().putString(NOTIFIED, u.version).apply()
        return u
    }

    fun dismiss() {
        _available.value?.let { prefs.edit().putString(DISMISSED, it.version).apply() }
        _available.value = null
    }

    private fun stored(): AvailableUpdate? {
        val v = prefs.getString(LATEST_VERSION, null) ?: return null
        val apk = prefs.getString(LATEST_APK_KEY, null) ?: return null
        val page = prefs.getString(LATEST_URL_KEY, null) ?: return null
        return shown(AvailableUpdate(v, apk, page))
    }

    private fun shown(u: AvailableUpdate): AvailableUpdate? =
        u.takeIf { UpdateCheck.isNewer(it.version, installed) && prefs.getString(DISMISSED, null) != it.version }

    companion object {
        const val PREFS = "update_check"
        private const val LAST_CHECK = "last_check_ms"
        private const val LATEST_VERSION = "latest_version"
        private const val LATEST_APK_KEY = "latest_apk_url"
        private const val LATEST_URL_KEY = "latest_url"
        private const val DISMISSED = "dismissed_version"
        private const val NOTIFIED = "notified_version"
    }
}
