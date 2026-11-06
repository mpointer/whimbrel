package com.mpointer.whimbrel.mobile.core

import com.mpointer.whimbrel.mobile.sync.SyncCredentials
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CoreSettingsPullerTest {
    private val native = FakeSyncNative()
    private val puller = CoreSettingsPuller(native)
    private val creds = SyncCredentials("tok", "libsql://db", "key", "dev")

    @Test
    fun settingsIterationDecodesPulled() {
        native.iterationReply = """{"pushed_count":2,"pulled":{"theme":{"value":"dark","updated_at_unix_ms":5,"is_deleted":false}}}"""
        assertEquals(mapOf("theme" to PulledSettingsField("", "dark", 5, false)), puller.iterate(creds, "{}"))
    }

    @Test
    fun applyPlanDecodesFields() {
        native.planReply = """{"fields":[{"field_key":"theme","value":"dark","updated_at_unix_ms":5,"is_deleted":false}],"hidden_episode_marks":{}}"""
        val out = puller.applyPlan(
            mapOf("theme" to PulledSettingsField("", "dark", 5, false)), mapOf("theme" to 1L), mapOf("apiKey.openai" to true),
        )
        assertEquals(listOf(PulledSettingsField("theme", "dark", 5, false)), out)
        assertEquals(listOf("""{"theme":{"value":"dark","updated_at_unix_ms":5,"is_deleted":false}}""", """{"theme":1}""", """{"apiKey.openai":true}"""), native.planArgs)
    }

    @Test
    fun settingsFailureIsNull() {
        assertNull(puller.iterate(creds, "{}"))
        native.iterationReply = "garbage"
        assertNull(puller.iterate(creds, "{}"))
        assertNull(puller.applyPlan(emptyMap(), emptyMap(), emptyMap()))
    }
}
