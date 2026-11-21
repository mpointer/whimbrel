package com.mpointer.whimbrel.mobile.design

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily

/**
 * Hand-written glue over the generated tokens (Tokens.generated.kt): how a fixed color and a type role
 * become Compose values. Skinnable colors stay with MaterialTheme.colorScheme and LocalThemePalette.
 */

/** The 0xRRGGBB variant for an appearance. [highContrast] is the system's high contrast text setting. */
fun WFixedColor.value(dark: Boolean, highContrast: Boolean): Int = when {
    dark && highContrast -> highContrastDark
    highContrast -> highContrastLight
    dark -> this.dark
    else -> light
}

fun WFixedColor.color(dark: Boolean, highContrast: Boolean = false): Color = Color(0xFF000000.toInt() or value(dark, highContrast))

/** A type role in the UI face. Sizes are sp, so the system font scale applies. */
fun WTokens.TypeRole.style(): TextStyle = TextStyle(fontSize = size, lineHeight = lineHeight, fontWeight = weight)

/** A type role in the reading face, for notes and answers. */
fun WTokens.TypeRole.reading(): TextStyle = style().copy(fontFamily = FontFamily.Serif)
