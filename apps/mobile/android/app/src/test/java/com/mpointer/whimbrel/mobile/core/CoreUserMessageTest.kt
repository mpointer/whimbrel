package com.mpointer.whimbrel.mobile.core

import com.mpointer.whimbrel.mobile.R
import java.io.File
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Test

class CoreUserMessageTest {
    // Resolves ids against the real strings.xml, so the test fails if an entry is missing or reworded.
    private val catalog: Map<String, String> =
        Regex("""<string name="(\w+)">(.*)</string>""")
            .findAll(File("src/main/res/values/strings.xml").readText())
            .associate { it.groupValues[1] to it.groupValues[2].replace("\\'", "'") }

    private val resolve: (Int, Array<Any>) -> String = { id, args ->
        val name = R.string::class.java.fields.first { it.getInt(null) == id }.name
        String.format(catalog.getValue(name), *args)
    }

    @Test
    fun deviceLimitRendersCatalogText() {
        val m = CoreUserMessage("device_limit", null, "english")
        assertEquals("Your free sync plan includes one computer and one phone, and they're already in use.", m.localized(resolve))
    }

    @Test
    fun paramsFillTheCatalogText() {
        val m = CoreUserMessage("server_error", mapOf("status" to JsonPrimitive(502)), "english")
        assertEquals("Sync is temporarily unavailable (server error 502).", m.localized(resolve))
        val d = CoreUserMessage("approve_on_other_device", mapOf("devices" to JsonArray(listOf("A", "B", "C").map(::JsonPrimitive))), "english")
        assertEquals("Approve this computer on A, B, or C; it was active in the last week.", d.localized(resolve))
    }

    @Test
    fun unknownCodeFallsBackToMessage() =
        assertEquals("english", CoreUserMessage("brand_new", null, "english").localized(resolve))

    @Test
    fun nullCodeFallsBackToMessage() =
        assertEquals("english", CoreUserMessage(null, null, "english").localized(resolve))

    @Test
    fun missingParamFallsBackToMessage() {
        assertEquals("english", CoreUserMessage("server_error", null, "english").localized(resolve))
        assertEquals("english", CoreUserMessage("approve_on_other_device", mapOf("devices" to JsonArray(emptyList())), "english").localized(resolve))
    }
}
