package com.mpointer.whimbrel.mobile.theme

import android.util.Log
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlin.math.pow

/** The color tokens a theme can set (`json` = the JSON key). Ported from the Mac's Theme.swift. */
enum class ThemeToken(val json: String) {
    backgroundPage("backgroundPage"), backgroundCard("backgroundCard"), border("border"),
    textPrimary("textPrimary"), textSecondary("textSecondary"),
    accentPrimary("accentPrimary"), accentSecondary("accentSecondary"), accentHighlight("accentHighlight"),
    error("error"), warning("warning"), success("success"), link("link"),
    selection("selection"), highlight("highlight"), textOnAccent("textOnAccent"),
}

/** Token to 0xRRGGBB. */
typealias ThemePalette = Map<ThemeToken, Int>

/** Which variants a theme file ships; a one-variant theme serves both appearances. */
enum class ThemeVariants { Both, LightOnly, DarkOnly }

data class Theme(
    val id: String,
    val name: String,
    val author: String,
    /** Both palettes always hold all 15 tokens (missing ones were filled from Default). */
    val light: ThemePalette,
    val dark: ThemePalette,
    val variants: ThemeVariants = ThemeVariants.Both,
)

object ThemeContrast {
    private fun channel(v: Int): Double {
        val c = v / 255.0
        return if (c <= 0.03928) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
    }

    private fun luminance(hex: Int) =
        0.2126 * channel((hex shr 16) and 0xFF) + 0.7152 * channel((hex shr 8) and 0xFF) + 0.0722 * channel(hex and 0xFF)

    /** WCAG contrast ratio, 1..21. */
    fun ratio(a: Int, b: Int): Double {
        val la = luminance(a)
        val lb = luminance(b)
        return (maxOf(la, lb) + 0.05) / (minOf(la, lb) + 0.05)
    }

    const val minimum = 4.5

    private val requiredPairs = listOf(
        ThemeToken.textPrimary to ThemeToken.backgroundPage, ThemeToken.textPrimary to ThemeToken.backgroundCard,
        ThemeToken.textSecondary to ThemeToken.backgroundPage, ThemeToken.textSecondary to ThemeToken.backgroundCard,
        ThemeToken.textPrimary to ThemeToken.selection, ThemeToken.textPrimary to ThemeToken.highlight,
        ThemeToken.textOnAccent to ThemeToken.accentPrimary,
    )

    /** One line per failing pair, e.g. `textOnAccent on accentPrimary = 3.26`. */
    fun failures(p: ThemePalette): List<String> = requiredPairs.mapNotNull { (fg, bg) ->
        val f = p[fg]
        val b = p[bg]
        if (f == null || b == null) return@mapNotNull "${fg.json} on ${bg.json} = missing"
        val r = ratio(f, b)
        if (r < minimum) "${fg.json} on ${bg.json} = ${"%.2f".format(java.util.Locale.ROOT, r)}" else null
    }

    fun passes(p: ThemePalette) = failures(p).isEmpty()
}

object ThemeLoader {
    private val hexColor = Regex("#[0-9a-fA-F]{6}")
    private val json = Json

    private fun warn(msg: String) {
        // android.util.Log is a stub that throws on the JVM unless unit-test defaults are on; never fail a load over a log line.
        runCatching { Log.w("themes", msg) }
    }

    fun isValidId(id: String) = id.isNotEmpty() && id.all { it in 'a'..'z' || it in '0'..'9' || it == '-' }

    /** null when any known token's value is not `#rrggbb`; unknown keys are ignored. Not contrast-checked. */
    private fun variant(raw: JsonObject): ThemePalette? {
        val palette = mutableMapOf<ThemeToken, Int>()
        for (token in ThemeToken.entries) {
            val value = raw[token.json] ?: continue
            val s = (value as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return null
            if (!hexColor.matches(s)) return null
            palette[token] = s.substring(1).toInt(16)
        }
        return palette
    }

    /** A theme file to a complete [Theme], or null when unusable. A variant failing the contrast guard becomes [fallback]'s. */
    fun parse(text: String, fallback: Theme = ThemeDefaults.theme): Theme? {
        val root = runCatching { json.parseToJsonElement(text) }.getOrNull() as? JsonObject
        val id = (root?.get("id") as? JsonPrimitive)?.takeIf { it.isString }?.content?.takeIf(::isValidId)
        val name = (root?.get("name") as? JsonPrimitive)?.takeIf { it.isString }?.content?.takeIf { it.isNotEmpty() }
        if (root == null || id == null || name == null) {
            warn("theme skipped: not a JSON object with a valid id and name")
            return null
        }
        val lightRaw = root["light"] as? JsonObject
        val darkRaw = root["dark"] as? JsonObject
        if (lightRaw == null && darkRaw == null) {
            warn("theme $id skipped: no light or dark variant")
            return null
        }
        val light = lightRaw?.let { variant(it) ?: return null.also { warn("theme $id skipped: a light color is not #rrggbb") } }?.let { fallback.light + it }
        val dark = darkRaw?.let { variant(it) ?: return null.also { warn("theme $id skipped: a dark color is not #rrggbb") } }?.let { fallback.dark + it }
        // A theme that ships one variant serves both appearances.
        var lightPalette = light ?: dark!!
        var darkPalette = dark ?: light!!
        if (!ThemeContrast.passes(lightPalette)) {
            warn("theme $id: light variant fails the contrast guard, using Default's: ${ThemeContrast.failures(lightPalette)}")
            lightPalette = fallback.light
        }
        if (!ThemeContrast.passes(darkPalette)) {
            warn("theme $id: dark variant fails the contrast guard, using Default's: ${ThemeContrast.failures(darkPalette)}")
            darkPalette = fallback.dark
        }
        val variants = if (lightRaw == null) ThemeVariants.DarkOnly else if (darkRaw == null) ThemeVariants.LightOnly else ThemeVariants.Both
        val author = (root["author"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: ""
        return Theme(id, name, author, lightPalette, darkPalette, variants)
    }

    /** Every `*.json` among [files] (file name to text), Default first then by name; broken files and duplicate ids are skipped. */
    fun loadDirectory(files: List<Pair<String, String>>, fallback: Theme = ThemeDefaults.theme): List<Theme> {
        val seen = mutableSetOf(fallback.id)
        val loaded = files.filter { it.first.endsWith(".json") }.sortedBy { it.first }.mapNotNull { (file, text) ->
            parse(text, fallback)?.takeIf { seen.add(it.id) } ?: null.also { warn("theme file skipped: $file") }
        }
        return listOf(fallback) + loaded.sortedBy { it.name }
    }
}
