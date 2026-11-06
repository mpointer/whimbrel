package com.mpointer.whimbrel.mobile.theme

import com.mpointer.whimbrel.mobile.theme.ThemeToken.*

/** The Default theme in code, so the app has a complete palette even if the bundled files are missing. ThemeDefaultsTest pins it to themes/default.json. */
object ThemeDefaults {
    val light: ThemePalette = mapOf(
        backgroundPage to 0xF7F2E7, backgroundCard to 0xFFFFFF, border to 0xE4D9C2,
        textPrimary to 0x241E19, textSecondary to 0x6B5F4F,
        accentPrimary to 0xA8502E, accentSecondary to 0x55606E, accentHighlight to 0xD9A441,
        error to 0xB3261E, warning to 0x9A5B00, success to 0x3B7A3B, link to 0x2E5A87,
        selection to 0xEAE1CD, highlight to 0xF6E8D0, textOnAccent to 0xFFFFFF,
    )

    val dark: ThemePalette = mapOf(
        backgroundPage to 0x1C1712, backgroundCard to 0x262019, border to 0x3A322B,
        textPrimary to 0xEDE6D9, textSecondary to 0xB7AB9A,
        accentPrimary to 0xC97B54, accentSecondary to 0x7C8794, accentHighlight to 0xE8B968,
        error to 0xF2877E, warning to 0xE0A04A, success to 0x7CC47C, link to 0x8FB4DC,
        selection to 0x312A24, highlight to 0x57462D,
        // White is 3.26:1 on the accent, below the 4.5:1 guard; near-black is 5.46:1.
        textOnAccent to 0x1C1712,
    )

    val theme = Theme(id = "default", name = "Default", author = "Whimbrel", light = light, dark = dark)
}
