package com.mpointer.whimbrel.mobile.update

import android.content.SharedPreferences
import java.net.HttpURLConnection
import java.net.URL
import java.util.Calendar
import java.util.TimeZone
import kotlin.coroutines.cancellation.CancellationException
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

    suspend fun fetchLatest(): FetchResult = withContext(Dispatchers.IO) {
        try {
            val c = URL(RELEASES_URL).openConnection() as HttpURLConnection
            c.connectTimeout = 10_000
            c.readTimeout = 10_000
            c.setRequestProperty("Accept", "application/vnd.github+json")
            c.setRequestProperty("User-Agent", "Whimbrel-Android")
            try {
                val code = c.responseCode
                when {
                    code == 200 -> FetchResult.Body(c.inputStream.bufferedReader().readText())
                    code < 100 -> FetchResult.Network // -1: no usable status line, so no answer to report
                    else -> FetchResult.Http(code)
                }
            } finally {
                c.disconnect()
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            // As before, nothing escapes (the old runCatching caught every Throwable). CancellationException is
            // rethrown above; nothing in this block suspends, so that only matters for structured cancellation.
            FetchResult.Network
        }
    }
}

/** What one request for the releases list came back with: the body, an HTTP status other than 200, or no answer at all. */
sealed class FetchResult {
    data class Body(val json: String) : FetchResult()
    data class Http(val code: Int) : FetchResult()
    object Network : FetchResult()
}

/** How the last check attempt ended. CHECKED_EARLIER: only a time is known (written by an older app version). FAILED_* keep the cached update info; only the attempt record changes. */
enum class CheckOutcome { CHECKED_EARLIER, UP_TO_DATE, UPDATE_AVAILABLE, FAILED_NETWORK, FAILED_HTTP, FAILED_UNREADABLE }

/**
 * The last check attempt, success or failure: when, how it ended, the latest version seen (UP_TO_DATE and
 * UPDATE_AVAILABLE) and the HTTP status (FAILED_HTTP). Holds no personal data.
 */
data class LastCheck(val atMs: Long, val outcome: CheckOutcome, val version: String? = null, val httpCode: Int? = null)

/** The English wording of the one-line check status; the app reads it from string resources, tests from a fake. */
interface UpdateStatusStrings {
    fun notChecked(): String
    fun checkedOnly(time: String): String
    fun upToDate(time: String): String
    fun updateAvailable(version: String, time: String): String
    fun network(time: String): String
    fun httpRateLimit(code: Int, time: String): String
    fun httpRefused(code: Int, time: String): String
    fun httpProblem(code: Int, time: String): String
    fun unreadable(time: String): String
}

/** How times are shown: the phone's own 12/24-hour setting and date style in the app, fixed patterns in tests. */
interface UpdateTimeFormats {
    val zone: TimeZone
    fun time(atMs: Long): String
    fun dateTime(atMs: Long): String
}

object UpdateStatus {
    /** Time only for a moment today, date and time for an earlier day (day boundaries in [UpdateTimeFormats.zone]). */
    fun formatTime(atMs: Long, nowMs: Long, formats: UpdateTimeFormats): String {
        fun day(ms: Long) = Calendar.getInstance(formats.zone).apply { timeInMillis = ms }.let { it.get(Calendar.YEAR) * 1000 + it.get(Calendar.DAY_OF_YEAR) }
        return if (day(atMs) == day(nowMs)) formats.time(atMs) else formats.dateTime(atMs)
    }

    fun line(last: LastCheck?, nowMs: Long, strings: UpdateStatusStrings, formats: UpdateTimeFormats): String {
        if (last == null) return strings.notChecked()
        val time = formatTime(last.atMs, nowMs, formats)
        return when (last.outcome) {
            CheckOutcome.CHECKED_EARLIER -> strings.checkedOnly(time)
            CheckOutcome.UP_TO_DATE -> strings.upToDate(time)
            CheckOutcome.UPDATE_AVAILABLE -> strings.updateAvailable(last.version.orEmpty(), time)
            CheckOutcome.FAILED_NETWORK -> strings.network(time)
            CheckOutcome.FAILED_HTTP -> {
                val code = last.httpCode ?: 0
                when {
                    code < 100 -> strings.network(time) // never show "HTTP 0" or "HTTP -1"
                    code == 403 || code == 429 -> strings.httpRateLimit(code, time)
                    code >= 500 -> strings.httpProblem(code, time)
                    else -> strings.httpRefused(code, time)
                }
            }
            CheckOutcome.FAILED_UNREADABLE -> strings.unreadable(time)
        }
    }
}

/** Whether to show "Update available": checks when due, remembers a dismissed version. */
class UpdateModel(
    private val prefs: SharedPreferences,
    private val installed: String,
    private val now: () -> Long = System::currentTimeMillis,
    private val fetch: suspend () -> FetchResult = UpdateCheck::fetchLatest,
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
        val result = fetch()
        val at = now()
        val body = when (result) {
            is FetchResult.Body -> result.json
            is FetchResult.Http -> return record(LastCheck(at, CheckOutcome.FAILED_HTTP, httpCode = result.code))
            FetchResult.Network -> return record(LastCheck(at, CheckOutcome.FAILED_NETWORK))
        }
        val latest = UpdateCheck.parse(body) ?: return record(LastCheck(at, CheckOutcome.FAILED_UNREADABLE))
        val outcome = if (UpdateCheck.isNewer(latest.version, installed)) CheckOutcome.UPDATE_AVAILABLE else CheckOutcome.UP_TO_DATE
        record(LastCheck(at, outcome, version = latest.version))
        prefs.edit().putLong(LAST_CHECK, at).putString(LATEST_VERSION, latest.version).putString(LATEST_APK_KEY, latest.apkUrl).putString(LATEST_URL_KEY, latest.pageUrl).apply()
        _available.value = shown(latest)
    }

    /** The last attempt's record (one prefs write, kept apart from the cache so a failure never clears the cached update). */
    private fun record(c: LastCheck) {
        prefs.edit().putLong(ATTEMPT_MS, c.atMs).putString(ATTEMPT_OUTCOME, c.outcome.name).putString(ATTEMPT_VERSION, c.version).putInt(ATTEMPT_HTTP, c.httpCode ?: 0).apply()
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
        private const val ATTEMPT_MS = "attempt_ms"
        private const val ATTEMPT_OUTCOME = "attempt_outcome"
        private const val ATTEMPT_VERSION = "attempt_version"
        private const val ATTEMPT_HTTP = "attempt_http"

        private const val LATEST_VERSION = "latest_version"
        private const val LATEST_APK_KEY = "latest_apk_url"
        private const val LATEST_URL_KEY = "latest_url"
        private const val DISMISSED = "dismissed_version"
        private const val NOTIFIED = "notified_version"

        /**
         * The last check attempt (foreground or background), or null if none has run yet. An install upgraded from
         * a version that kept only the check time reports that time as CHECKED_EARLIER until its next attempt.
         */
        fun lastCheck(prefs: SharedPreferences): LastCheck? {
            if (!prefs.contains(ATTEMPT_MS)) return if (prefs.contains(LAST_CHECK)) LastCheck(prefs.getLong(LAST_CHECK, 0L), CheckOutcome.CHECKED_EARLIER) else null
            val outcome = runCatching { CheckOutcome.valueOf(prefs.getString(ATTEMPT_OUTCOME, "") ?: "") }.getOrNull() ?: return null
            return LastCheck(prefs.getLong(ATTEMPT_MS, 0L), outcome, prefs.getString(ATTEMPT_VERSION, null), prefs.getInt(ATTEMPT_HTTP, 0).takeIf { it != 0 })
        }
    }
}
