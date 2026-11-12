package com.mpointer.whimbrel.mobile.update

import com.mpointer.whimbrel.mobile.sync.FakePrefs
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

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
        val model = UpdateModel(FakePrefs(), "0.9.70", { 1L }) { list(rel("v0.9.67", false), rel("v0.9.65", true)) }
        model.checkIfDue()
        assertNull(model.available.value)
    }

    @Test fun showsANewerReleaseAndChecksAtMostEverySixHours() = runTest {
        var now = 1_000_000L
        var fetches = 0
        val prefs = FakePrefs()
        val model = UpdateModel(prefs, "0.9.59", { now }) { fetches++; release }
        model.checkIfDue()
        assertEquals("0.9.60", model.available.value?.version)
        now += 5 * 60 * 60 * 1000L
        UpdateModel(prefs, "0.9.59", { now }) { fetches++; release }.apply { checkIfDue(); assertEquals("0.9.60", available.value?.version) }
        assertEquals("within 6 hours a return reuses the last check", 1, fetches)
        now += 60 * 60 * 1000L
        UpdateModel(prefs, "0.9.59", { now }) { fetches++; release }.checkIfDue()
        assertEquals("after 6 hours a return checks again", 2, fetches)
    }

    @Test fun announcesEachNewVersionOnce() = runTest {
        var now = 1L
        var releases = release
        val prefs = FakePrefs()
        val first = UpdateModel(prefs, "0.9.59", { now }) { releases }
        first.checkIfDue()
        assertEquals("0.9.60", first.takeNotification()?.version)
        assertNull("the same version is not announced twice", first.takeNotification())
        now += UpdateCheck.CHECK_EVERY_MS
        UpdateModel(prefs, "0.9.59", { now }) { releases }.apply { checkIfDue(); assertNull(takeNotification()) }
        releases = list(rel("v0.9.61", true), rel("v0.9.60", true))
        now += UpdateCheck.CHECK_EVERY_MS
        UpdateModel(prefs, "0.9.59", { now }) { releases }.apply { checkIfDue(); assertEquals("0.9.61", takeNotification()?.version) }
    }

    @Test fun noAnnouncementWhenDismissedOrUpToDate() = runTest {
        val prefs = FakePrefs()
        UpdateModel(prefs, "0.9.59", { 1L }) { release }.apply { checkIfDue(); dismiss(); assertNull(takeNotification()) }
        UpdateModel(FakePrefs(), "0.9.60", { 1L }) { release }.apply { checkIfDue(); assertNull(takeNotification()) }
    }

    @Test fun nothingWhenUpToDateOrDismissed() = runTest {
        val prefs = FakePrefs()
        val current = UpdateModel(prefs, "0.9.60", { 1L }) { release }
        current.checkIfDue()
        assertNull("already on the latest", current.available.value)

        val older = UpdateModel(FakePrefs(), "0.9.59", { 1L }) { release }
        older.checkIfDue()
        older.dismiss()
        assertNull(older.available.value)
    }

    @Test fun anOfflineCheckShowsNothingAndRetriesLater() = runTest {
        var now = 1L
        val prefs = FakePrefs()
        val model = UpdateModel(prefs, "0.9.59", { now }) { null }
        model.checkIfDue()
        assertNull(model.available.value)
        now += 1000
        val online = UpdateModel(prefs, "0.9.59", { now }) { release }
        online.checkIfDue()
        assertEquals("0.9.60", online.available.value?.version)
    }
}
