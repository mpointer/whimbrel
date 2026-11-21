package com.mpointer.whimbrel.mobile.update

import com.mpointer.whimbrel.mobile.sync.FakePrefs
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import java.util.TimeZone

class UpdateCheckTest {
    private fun rel(tag: String, apk: Boolean, draft: Boolean = false, pre: Boolean = false): String {
        val v = tag.removePrefix("v")
        val assets = if (apk) {
            """[{"name":"Whimbrel-$v-mac.zip","browser_download_url":"https://example.com/$v.zip"},""" +
                """{"name":"Whimbrel-$v.apk","browser_download_url":"https://github.com/mpointer/whimbrel/releases/download/$tag/Whimbrel-$v.apk"}]"""
        } else {
            """[{"name":"Whimbrel-$v.dmg","browser_download_url":"https://example.com/$v.dmg"}]"""
        }
        return """{"tag_name":"$tag","html_url":"https://github.com/mpointer/whimbrel/releases/tag/$tag","draft":$draft,"prerelease":$pre,"assets":$assets}"""
    }

    private fun list(vararg r: String) = r.joinToString(",", "[", "]")
    private fun apkUrl(v: String) = "https://github.com/mpointer/whimbrel/releases/download/v$v/Whimbrel-$v.apk"
    private fun page(v: String) = "https://github.com/mpointer/whimbrel/releases/tag/v$v"

    private val release = list(rel("v0.9.60", true))
    private fun ok(json: String): FetchResult = FetchResult.Body(json)

    @Test fun comparesDottedVersions() {
        assertTrue(UpdateCheck.isNewer("v0.9.60", "0.9.59"))
        assertTrue(UpdateCheck.isNewer("v0.10.0", "0.9.59"))
        assertTrue(UpdateCheck.isNewer("v1.0", "0.9.59"))
        assertFalse(UpdateCheck.isNewer("v0.9.59", "0.9.59"))
        assertFalse(UpdateCheck.isNewer("v0.9.58", "0.9.59"))
    }

    @Test fun parsesTheNewestApkRelease() {
        assertEquals(AvailableUpdate("0.9.60", apkUrl("0.9.60"), page("0.9.60")), UpdateCheck.parse(release))
        assertNull(UpdateCheck.parse("not json"))
        assertNull(UpdateCheck.parse("""{"tag_name":"v0.9.60"}"""))
    }

    @Test fun usesTheReleasesListEndpoint() {
        assertEquals("https://api.github.com/repos/mpointer/whimbrel/releases?per_page=20", UpdateCheck.RELEASES_URL)
    }

    @Test fun picksNewestReleaseWithApk() {
        val json = list(rel("v0.9.67", false), rel("v0.9.66", false), rel("v0.9.65", true), rel("v0.9.64", false), rel("v0.9.60", true))
        assertEquals(AvailableUpdate("0.9.65", apkUrl("0.9.65"), page("0.9.65")), UpdateCheck.parse(json))
    }

    @Test fun noApkAnywhereMeansNoUpdate() {
        assertNull(UpdateCheck.parse(list(rel("v0.9.67", false), rel("v0.9.66", false))))
        assertNull(UpdateCheck.parse("[]"))
    }

    @Test fun skipsDraftsAndPrereleases() {
        val json = list(rel("v0.9.70", true, draft = true), rel("v0.9.69", true, pre = true), rel("v0.9.65", true))
        assertEquals("0.9.65", UpdateCheck.parse(json)?.version)
    }

    @Test fun ordersBySemverNotListOrder() {
        val json = list(rel("v0.9.9", true), rel("v0.9.10", true), rel("v0.9.2", true))
        assertEquals("0.9.10", UpdateCheck.parse(json)?.version)
    }

    @Test fun olderThanInstalledIsNoUpdate() = runTest {
        val model = UpdateModel(FakePrefs(), "0.9.70", { 1L }) { ok(list(rel("v0.9.67", false), rel("v0.9.65", true))) }
        model.checkIfDue()
        assertNull(model.available.value)
    }

    @Test fun showsANewerReleaseAndChecksAtMostEverySixHours() = runTest {
        var now = 1_000_000L
        var fetches = 0
        val prefs = FakePrefs()
        val model = UpdateModel(prefs, "0.9.59", { now }) { fetches++; ok(release) }
        model.checkIfDue()
        assertEquals("0.9.60", model.available.value?.version)
        now += 5 * 60 * 60 * 1000L
        UpdateModel(prefs, "0.9.59", { now }) { fetches++; ok(release) }.apply { checkIfDue(); assertEquals("0.9.60", available.value?.version) }
        assertEquals("within 6 hours a return reuses the last check", 1, fetches)
        now += 60 * 60 * 1000L
        UpdateModel(prefs, "0.9.59", { now }) { fetches++; ok(release) }.checkIfDue()
        assertEquals("after 6 hours a return checks again", 2, fetches)
    }

    @Test fun announcesEachNewVersionOnce() = runTest {
        var now = 1L
        var releases = release
        val prefs = FakePrefs()
        val first = UpdateModel(prefs, "0.9.59", { now }) { ok(releases) }
        first.checkIfDue()
        assertEquals("0.9.60", first.takeNotification()?.version)
        assertNull("the same version is not announced twice", first.takeNotification())
        now += UpdateCheck.CHECK_EVERY_MS
        UpdateModel(prefs, "0.9.59", { now }) { ok(releases) }.apply { checkIfDue(); assertNull(takeNotification()) }
        releases = list(rel("v0.9.61", true), rel("v0.9.60", true))
        now += UpdateCheck.CHECK_EVERY_MS
        UpdateModel(prefs, "0.9.59", { now }) { ok(releases) }.apply { checkIfDue(); assertEquals("0.9.61", takeNotification()?.version) }
    }

    @Test fun noAnnouncementWhenDismissedOrUpToDate() = runTest {
        val prefs = FakePrefs()
        UpdateModel(prefs, "0.9.59", { 1L }) { ok(release) }.apply { checkIfDue(); dismiss(); assertNull(takeNotification()) }
        UpdateModel(FakePrefs(), "0.9.60", { 1L }) { ok(release) }.apply { checkIfDue(); assertNull(takeNotification()) }
    }

    @Test fun nothingWhenUpToDateOrDismissed() = runTest {
        val prefs = FakePrefs()
        val current = UpdateModel(prefs, "0.9.60", { 1L }) { ok(release) }
        current.checkIfDue()
        assertNull("already on the latest", current.available.value)

        val older = UpdateModel(FakePrefs(), "0.9.59", { 1L }) { ok(release) }
        older.checkIfDue()
        older.dismiss()
        assertNull(older.available.value)
    }

    @Test fun anOfflineCheckShowsNothingAndRetriesLater() = runTest {
        var now = 1L
        val prefs = FakePrefs()
        val model = UpdateModel(prefs, "0.9.59", { now }) { FetchResult.Network }
        model.checkIfDue()
        assertNull(model.available.value)
        now += 1000
        val online = UpdateModel(prefs, "0.9.59", { now }) { ok(release) }
        online.checkIfDue()
        assertEquals("0.9.60", online.available.value?.version)
    }

    private fun last(prefs: FakePrefs) = UpdateModel.lastCheck(prefs)

    @Test fun recordsEachOutcomeWithTheClock() = runTest {
        val prefs = FakePrefs()
        UpdateModel(prefs, "0.9.59", { 100L }) { ok(release) }.checkIfDue()
        assertEquals(LastCheck(100L, CheckOutcome.UPDATE_AVAILABLE, "0.9.60"), last(prefs))
        val p2 = FakePrefs()
        UpdateModel(p2, "0.9.60", { 200L }) { ok(release) }.checkIfDue()
        assertEquals(LastCheck(200L, CheckOutcome.UP_TO_DATE, "0.9.60"), last(p2))
        val p3 = FakePrefs()
        UpdateModel(p3, "0.9.60", { 300L }) { FetchResult.Network }.checkIfDue()
        assertEquals(LastCheck(300L, CheckOutcome.FAILED_NETWORK), last(p3))
        val p4 = FakePrefs()
        UpdateModel(p4, "0.9.60", { 400L }) { FetchResult.Http(403) }.checkIfDue()
        assertEquals(LastCheck(400L, CheckOutcome.FAILED_HTTP, httpCode = 403), last(p4))
        val p5 = FakePrefs()
        UpdateModel(p5, "0.9.60", { 500L }) { ok("not json") }.checkIfDue()
        assertEquals(LastCheck(500L, CheckOutcome.FAILED_UNREADABLE), last(p5))
        val p6 = FakePrefs()
        UpdateModel(p6, "0.9.60", { 600L }) { ok("[]") }.checkIfDue()
        assertEquals("a 200 with no release that has an app file is unreadable", CheckOutcome.FAILED_UNREADABLE, last(p6)?.outcome)
        assertNull("nothing recorded before any attempt", last(FakePrefs()))
    }

    @Test fun aCacheHitLeavesTheLastAttemptAlone() = runTest {
        val prefs = FakePrefs()
        UpdateModel(prefs, "0.9.59", { 1_000L }) { ok(release) }.checkIfDue()
        var fetches = 0
        UpdateModel(prefs, "0.9.59", { 2_000L }) { fetches++; FetchResult.Network }.checkIfDue()
        assertEquals(0, fetches)
        assertEquals(LastCheck(1_000L, CheckOutcome.UPDATE_AVAILABLE, "0.9.60"), last(prefs))
    }

    @Test fun aFailureAfterASuccessKeepsTheCacheAndRetriesOnTheNextReturn() = runTest {
        val prefs = FakePrefs()
        UpdateModel(prefs, "0.9.59", { 1_000L }) { ok(release) }.checkIfDue()
        val later = 1_000L + UpdateCheck.CHECK_EVERY_MS
        val model = UpdateModel(prefs, "0.9.59", { later }) { FetchResult.Http(403) }
        model.checkIfDue()
        assertEquals(LastCheck(later, CheckOutcome.FAILED_HTTP, httpCode = 403), last(prefs))
        // The failed attempt did not move the 6-hour clock, so the next return retries; the cache is intact.
        var fetches = 0
        UpdateModel(prefs, "0.9.59", { later + 1 }) { fetches++; FetchResult.Network }.checkIfDue()
        assertEquals(1, fetches)
        assertEquals("0.9.60", prefs.getString("latest_version", null))
        assertEquals(apkUrl("0.9.60"), prefs.getString("latest_apk_url", null))
    }

    // The background worker calls the same UpdateModel.checkIfDue on the same prefs, so the record is shared.
    @Test fun theBackgroundCheckAndTheAppShareTheRecord() = runTest {
        val prefs = FakePrefs()
        UpdateModel(prefs, "0.9.59", { 10L }) { FetchResult.Network }.checkIfDue() // worker
        assertEquals(CheckOutcome.FAILED_NETWORK, last(prefs)?.outcome)
        UpdateModel(prefs, "0.9.59", { 20L }) { ok(release) }.checkIfDue() // app on return
        assertEquals(LastCheck(20L, CheckOutcome.UPDATE_AVAILABLE, "0.9.60"), last(prefs))
    }

    private object Words : UpdateStatusStrings {
        override fun notChecked() = "not checked yet"
        override fun checkedOnly(time: String) = "checked $time"
        override fun upToDate(time: String) = "up to date ($time)"
        override fun updateAvailable(version: String, time: String) = "$version available ($time)"
        override fun network(time: String) = "couldn't reach GitHub ($time)"
        override fun httpRateLimit(code: Int, time: String) = "refused (rate limit), HTTP $code ($time)"
        override fun httpRefused(code: Int, time: String) = "refused, HTTP $code ($time)"
        override fun httpProblem(code: Int, time: String) = "problem, HTTP $code ($time)"
        override fun unreadable(time: String) = "unreadable ($time)"
    }

    private val utc = TimeZone.getTimeZone("UTC")
    private fun fmt(timePattern: String) = object : UpdateTimeFormats {
        private fun f(p: String) = SimpleDateFormat(p, Locale.US).apply { timeZone = utc }
        override val zone = utc
        override fun time(atMs: Long) = f(timePattern).format(Date(atMs))
        override fun dateTime(atMs: Long) = f("MMM d, $timePattern").format(Date(atMs))
    }
    private val twelve = fmt("h:mm a")
    private val twentyFour = fmt("HH:mm")
    private fun at(day: Int, hour: Int, minute: Int) = Calendar.getInstance(utc).apply { clear(); set(2026, Calendar.NOVEMBER, day, hour, minute) }.timeInMillis
    private val now = at(19, 18, 0)

    @Test fun statusLineForEveryOutcome() {
        fun line(l: LastCheck?) = UpdateStatus.line(l, now, Words, twelve)
        val t = at(19, 14, 10)
        assertEquals("not checked yet", line(null))
        assertEquals("checked 2:10 PM", line(LastCheck(t, CheckOutcome.CHECKED_EARLIER)))
        assertEquals("up to date (2:10 PM)", line(LastCheck(t, CheckOutcome.UP_TO_DATE, "0.9.60")))
        assertEquals("0.9.81 available (2:10 PM)", line(LastCheck(t, CheckOutcome.UPDATE_AVAILABLE, "0.9.81")))
        assertEquals("couldn't reach GitHub (2:10 PM)", line(LastCheck(t, CheckOutcome.FAILED_NETWORK)))
        assertEquals("unreadable (2:10 PM)", line(LastCheck(t, CheckOutcome.FAILED_UNREADABLE)))
    }

    @Test fun httpWordingByStatusClass() {
        fun line(code: Int?) = UpdateStatus.line(LastCheck(now, CheckOutcome.FAILED_HTTP, httpCode = code), now, Words, twelve)
        assertEquals("refused (rate limit), HTTP 403 (6:00 PM)", line(403))
        assertEquals("refused (rate limit), HTTP 429 (6:00 PM)", line(429))
        assertEquals("refused, HTTP 404 (6:00 PM)", line(404))
        assertEquals("problem, HTTP 503 (6:00 PM)", line(503))
        assertEquals("a missing or bad code reads as no answer", "couldn't reach GitHub (6:00 PM)", line(null))
        assertEquals("couldn't reach GitHub (6:00 PM)", line(-1))
    }

    @Test fun timeIsTimeOnlyTodayAndDateAndTimeBefore() {
        assertEquals("2:10 PM", UpdateStatus.formatTime(at(19, 14, 10), now, twelve))
        assertEquals("Nov 18, 2:10 PM", UpdateStatus.formatTime(at(18, 14, 10), now, twelve))
    }

    @Test fun midnightSeparatesToday() {
        val justAfterMidnight = at(19, 0, 30)
        assertEquals("12:05 AM", UpdateStatus.formatTime(at(19, 0, 5), justAfterMidnight, twelve))
        assertEquals("Nov 18, 11:59 PM", UpdateStatus.formatTime(at(18, 23, 59), justAfterMidnight, twelve))
    }

    @Test fun twentyFourHourClockUsesTheInjectedFormat() {
        assertEquals("14:10", UpdateStatus.formatTime(at(19, 14, 10), now, twentyFour))
        assertEquals("Nov 18, 23:59", UpdateStatus.formatTime(at(18, 23, 59), now, twentyFour))
    }

    @Test fun anUpgradedInstallWithOnlyACheckTimeShowsThatTime() {
        val prefs = FakePrefs().also { it.edit().putLong("last_check_ms", 5_000L).apply() }
        assertEquals(LastCheck(5_000L, CheckOutcome.CHECKED_EARLIER), UpdateModel.lastCheck(prefs))
        assertNull(UpdateModel.lastCheck(FakePrefs()))
    }
}
