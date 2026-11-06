package com.mpointer.whimbrel.mobile.core

import androidx.test.platform.app.InstrumentationRegistry
import com.mpointer.whimbrel.mobile.BuildConfig
import java.io.File
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Test

class NativeBridgeTest {
    @Test
    fun coreVersionMatchesTheAppVersion() = assertEquals(BuildConfig.VERSION_NAME, Native.coreVersion())

    @Test
    fun meetingTitleRoundTripsByteExact() {
        val dir = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir
        val file = File(dir, "bridge-${System.nanoTime()}.sqlite")
        val title = "Retro 😀 café"
        val meeting = JSONObject()
            .put("device_id", "android-test")
            .put("start_unix_ms", 1_700_000_000_000L)
            .put("end_unix_ms", JSONObject.NULL)
            .put("title", title)
            .put("attendees", org.json.JSONArray())
            .put("trigger", "calendar")
            .put("audio_path", JSONObject.NULL)
            .put("status", "recording")
        try {
            val handle = Native.memoryOpen(file.path)
            assertNotNull(handle.takeIf { it != 0L })
            try {
                val id = JSONObject(Native.memoryInsertMeeting(handle, meeting.toString())!!).getLong("id")
                val back = JSONObject(Native.memoryGetMeeting(handle, id)!!)
                assertEquals(title, back.getString("title"))
            } finally {
                Native.memoryClose(handle)
            }
        } finally {
            file.delete()
        }
    }

    @Test
    fun pullOnlyWithBadCredentialsFailsAndReportsAnError() {
        val dir = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir
        val file = File(dir, "bridge-${System.nanoTime()}.sqlite")
        try {
            val handle = Native.memoryOpen(file.path)
            assertNotNull(handle.takeIf { it != 0L })
            try {
                val ok = Native.syncContentPullOnly(handle, "libsql://nonexistent.invalid", "bad-token", "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=", "android-test", 14)
                assertFalse(ok)
                assertNotNull(Native.syncContentLastError())
            } finally {
                Native.memoryClose(handle)
            }
        } finally {
            file.delete()
        }
    }
}
