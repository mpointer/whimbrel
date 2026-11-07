package com.mpointer.whimbrel.mobile.update

import com.mpointer.whimbrel.mobile.sync.FakePrefs
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UpdateCheckTest {
    private val release = """{"tag_name":"v0.9.60","html_url":"https://github.com/mpointer/whimbrel/releases/tag/v0.9.60"}"""

    @Test fun comparesDottedVersions() {
        assertTrue(UpdateCheck.isNewer("v0.9.60", "0.9.59"))
        assertTrue(UpdateCheck.isNewer("v0.10.0", "0.9.59"))
        assertTrue(UpdateCheck.isNewer("v1.0", "0.9.59"))
        assertFalse(UpdateCheck.isNewer("v0.9.59", "0.9.59"))
        assertFalse(UpdateCheck.isNewer("v0.9.58", "0.9.59"))
    }

    @Test fun parsesTheLatestRelease() {
        assertEquals(AvailableUpdate("0.9.60", "https://github.com/mpointer/whimbrel/releases/tag/v0.9.60"), UpdateCheck.parse(release))
        assertNull(UpdateCheck.parse("not json"))
    }

    @Test fun showsANewerReleaseAndChecksAtMostDaily() = runTest {
        var now = 1_000_000L
        var fetches = 0
        val prefs = FakePrefs()
        val model = UpdateModel(prefs, "0.9.59", { now }) { fetches++; release }
        model.checkIfDue()
        assertEquals("0.9.60", model.available.value?.version)
        now += 60 * 60 * 1000L
        UpdateModel(prefs, "0.9.59", { now }) { fetches++; release }.apply { checkIfDue(); assertEquals("0.9.60", available.value?.version) }
        assertEquals("one network check a day; the next launch reuses it", 1, fetches)
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
