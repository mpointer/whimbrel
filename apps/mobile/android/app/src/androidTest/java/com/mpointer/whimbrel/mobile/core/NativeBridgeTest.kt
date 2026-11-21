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

    // ---- Topic links and Ask: each new native is called once, so a missing or misnamed JNI symbol fails here. ----

    private fun <T> withStore(body: (Long) -> T): T {
        val dir = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir
        val file = File(dir, "bridge-${System.nanoTime()}.sqlite")
        try {
            val handle = Native.memoryOpen(file.path)
            assertNotNull(handle.takeIf { it != 0L })
            try {
                return body(handle)
            } finally {
                Native.memoryClose(handle)
            }
        } finally {
            file.delete()
        }
    }

    @Test
    fun aMeetingThisReplicaDoesNotHaveHasNoId() = withStore { handle ->
        assertEquals(-1L, Native.meetingIdForSource(handle, "some-other-mac 😀", 7))
    }

    @Test
    fun modelsResolveToTheCoresDefaults() {
        assertEquals("a real choice is kept", "my-pick", Native.modelResolve("openrouter", "my-pick"))
        assertEquals("OpenRouter has no default", "", Native.modelResolve("openrouter", null))
        assertFalse("the core has a default for Anthropic", Native.modelResolve("anthropic", null).isNullOrEmpty())
    }

    @Test
    fun anAskIsPreparedAndFinalizedOrDiscardedWithoutAnyProvider() = withStore { handle ->
        // No key in the routing: prepare only reads the (empty) replica. Nothing here contacts a provider.
        val routing = """{"sensitive":false,"preferred_provider":"anthropic","preferred_model":"m","custom_instructions":"","use_local_model":false,"output_language":""}"""
        val reply = Native.llmPrepareAsk(handle, "What did we decide? 😀", 5, "[]", routing)
        assertNotNull("prepare answers with an envelope", reply)
        val env = JSONObject(reply!!)
        // A store opened with no models directory, as the phone's is, must still prepare an ask.
        assertEquals("prepare succeeds on a phone store: ${env.optJSONObject("error")}", true, env.optBoolean("ok"))
        val id = env.getJSONObject("data").getString("prepare_id")
        assertFalse(env.getJSONObject("data").getString("prompt").isEmpty())
        val done = JSONObject(Native.llmFinalizeAsk(handle, id, "An answer with no markers.", "m", -1, -1)!!)
        assertEquals(0, done.getJSONArray("citations").length())
        assertEquals("an unknown prepare id has no citations", 0, JSONObject(Native.llmFinalizeAsk(handle, "no-such-id", "x", "m", -1, -1)!!).getJSONArray("citations").length())
        Native.llmDiscardAsk("no-such-id") // a no-op that must not crash
        assertEquals(true, Native.modelNoteAnswer(handle, "anthropic", "asked-model", "asked-model", 1_700_000_000_000L))
    }

    @Test
    fun aStreamAgainstAClosedLocalPortFailsAndIsReleased() {
        assertEquals("a hosted provider with no key is refused", null, Native.inferenceStart("p", "m", "google", null, null))
        assertEquals("an unknown run", null, Native.inferencePoll("no-such-run"))
        assertFalse(Native.inferenceCancel("no-such-run"))
        // Port 9 on this device: nothing listens, so the run fails without leaving the phone.
        val runId = JSONObject(Native.inferenceStart("Say hi", "m", "google", "not-a-real-key", "http://127.0.0.1:9")!!).getString("run_id")
        var terminal: JSONObject? = null
        val deadline = System.currentTimeMillis() + 60_000
        while (terminal == null && System.currentTimeMillis() < deadline) {
            val events = org.json.JSONArray(Native.inferencePoll(runId)!!)
            for (i in 0 until events.length()) if (events.getJSONObject(i).getString("kind") == "terminal") terminal = events.getJSONObject(i)
            if (terminal == null) Thread.sleep(100)
        }
        assertNotNull("the run ends", terminal)
        assertEquals("failed", terminal!!.getJSONObject("outcome").getString("kind"))
        Native.inferenceRelease(runId)
        assertEquals("released", null, Native.inferencePoll(runId))
    }

    // ---- Approving another device ----

    private fun keypair(): Pair<String, String> {
        val k = JSONObject(Native.syncGenerateDeviceKeypair()!!)
        return k.getString("secret_key_base64") to k.getString("public_key_base64")
    }

    @Test
    fun aGroupKeyWrappedForADeviceIsRecoveredOnlyByThatDevice() {
        val groupKey = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="
        val (approverSecret, approverPublic) = keypair()
        val (newSecret, newPublic) = keypair()
        val (strangerSecret, _) = keypair()
        // The approver wraps with its own secret to the new device's public key.
        val wrapped = JSONObject(Native.syncWrapGroupKey(approverSecret, newPublic, groupKey)!!).getString("wrapped_group_key_base64")
        assertFalse(wrapped.isEmpty())
        assertFalse("the wrapped blob does not contain the key", wrapped.contains(groupKey.trimEnd('=')))
        // The new device unwraps with its own secret and the approver's public key.
        val back = JSONObject(Native.syncUnwrapGroupKey(newSecret, approverPublic, wrapped)!!).getString("group_key_base64")
        assertEquals(groupKey, back)
        assertEquals("another device cannot unwrap it", null, Native.syncUnwrapGroupKey(strangerSecret, approverPublic, wrapped))
        assertEquals("a malformed recipient key wraps nothing", null, Native.syncWrapGroupKey(approverSecret, "not-a-key", groupKey))
    }

    @Test
    fun approveAgainstAClosedLocalPortFailsWithAnErrorEnvelope() {
        // Port 9 on this device: nothing listens, so nothing leaves the phone. Never the real Worker.
        val reply = JSONObject(Native.syncWorkerApproveDevice("http://127.0.0.1:9", "not-a-token", "not-a-key", "device-id", "blob")!!)
        assertEquals(true, reply.has("error"))
    }
}
