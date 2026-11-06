package com.mpointer.whimbrel.mobile.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** The tokens Material has no slot for, read as `LocalThemePalette.current`. */
data class ExtraColors(
    val success: Color, val warning: Color, val link: Color,
    val highlight: Color, val selection: Color, val accentSecondary: Color, val accentHighlight: Color,
)

val LocalThemePalette = staticCompositionLocalOf {
    ExtraColors(Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified)
}

private fun ThemePalette.color(t: ThemeToken) = Color(0xFF000000.toInt() or getValue(t))

internal fun ThemePalette.toColorScheme(dark: Boolean): ColorScheme {
    fun c(t: ThemeToken) = color(t)
    val base = if (dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        background = c(ThemeToken.backgroundPage), onBackground = c(ThemeToken.textPrimary),
        surface = c(ThemeToken.backgroundPage), onSurface = c(ThemeToken.textPrimary),
        surfaceVariant = c(ThemeToken.backgroundCard), onSurfaceVariant = c(ThemeToken.textSecondary),
        surfaceContainerLowest = c(ThemeToken.backgroundPage), surfaceContainerLow = c(ThemeToken.backgroundCard),
        surfaceContainer = c(ThemeToken.backgroundCard), surfaceContainerHigh = c(ThemeToken.backgroundCard),
        surfaceContainerHighest = c(ThemeToken.backgroundCard),
        primary = c(ThemeToken.accentPrimary), onPrimary = c(ThemeToken.textOnAccent),
        secondary = c(ThemeToken.accentSecondary), tertiary = c(ThemeToken.accentHighlight),
        error = c(ThemeToken.error),
        outline = c(ThemeToken.border), outlineVariant = c(ThemeToken.border),
        // Every slot a component can draw from comes from a Whimbrel token, so no Material default (purple) shows.
        onSecondary = c(ThemeToken.textOnAccent), onTertiary = c(ThemeToken.textOnAccent), onError = c(ThemeToken.textOnAccent),
        primaryContainer = c(ThemeToken.selection), onPrimaryContainer = c(ThemeToken.textPrimary),
        secondaryContainer = c(ThemeToken.selection), onSecondaryContainer = c(ThemeToken.textPrimary),
        tertiaryContainer = c(ThemeToken.highlight), onTertiaryContainer = c(ThemeToken.textPrimary),
        // contentColorFor(backgroundCard) lands here first (cards, sheets share that colour), so this is the default text colour on them.
        errorContainer = c(ThemeToken.backgroundCard), onErrorContainer = c(ThemeToken.textPrimary),
        surfaceTint = c(ThemeToken.accentPrimary), inversePrimary = c(ThemeToken.accentPrimary),
        inverseSurface = c(ThemeToken.textPrimary), inverseOnSurface = c(ThemeToken.backgroundPage),
        surfaceBright = c(ThemeToken.backgroundCard), surfaceDim = c(ThemeToken.backgroundPage),
    )
}

/** Wraps [content] in the selected theme's Material 3 scheme; a one-variant theme already serves both appearances. */
@Composable
fun WhimbrelTheme(store: ThemeStore, content: @Composable () -> Unit) {
    val themes by store.themes.collectAsState()
    val id by store.selectedId.collectAsState()
    val theme = themes.firstOrNull { it.id == id } ?: ThemeDefaults.theme
    val dark = isSystemInDarkTheme()
    val palette = if (dark) theme.dark else theme.light
    val extras = ExtraColors(
        success = palette.color(ThemeToken.success), warning = palette.color(ThemeToken.warning), link = palette.color(ThemeToken.link),
        highlight = palette.color(ThemeToken.highlight), selection = palette.color(ThemeToken.selection),
        accentSecondary = palette.color(ThemeToken.accentSecondary), accentHighlight = palette.color(ThemeToken.accentHighlight),
    )
    CompositionLocalProvider(LocalThemePalette provides extras) {
        MaterialTheme(colorScheme = palette.toColorScheme(dark), content = content)
    }
}
