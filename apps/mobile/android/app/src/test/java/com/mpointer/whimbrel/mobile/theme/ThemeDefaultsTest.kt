package com.mpointer.whimbrel.mobile.theme

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Test

class ThemeDefaultsTest {
    @Test fun defaultsMatchThemesDefaultJson() {
        val root = Json.parseToJsonElement(ThemeTestFiles.read("default.json")).jsonObject
        fun palette(key: String) = root.getValue(key).jsonObject.entries.associate { (k, v) ->
            ThemeToken.entries.single { it.json == k } to v.jsonPrimitive.content.removePrefix("#").toInt(16)
        }
        assertEquals(palette("light"), ThemeDefaults.light)
        assertEquals(palette("dark"), ThemeDefaults.dark)
        assertEquals(root.getValue("id").jsonPrimitive.content, ThemeDefaults.theme.id)
        assertEquals(root.getValue("name").jsonPrimitive.content, ThemeDefaults.theme.name)
        assertEquals(15, ThemeToken.entries.size)
    }
}
