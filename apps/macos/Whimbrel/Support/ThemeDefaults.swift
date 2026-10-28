import Foundation

/// The Default theme in code, so the app has a complete palette even if `themes/default.json`
/// is missing from the bundle. `ThemeDefaultsTests` pins this and `themes/default.json` to
/// the same values.
enum ThemeDefaults {
    static let light: ThemePalette = [
        .backgroundPage: 0xF7F2E7, .backgroundCard: 0xFFFFFF, .border: 0xE4D9C2,
        .textPrimary: 0x241E19, .textSecondary: 0x6B5F4F,
        .accentPrimary: 0xA8502E, .accentSecondary: 0x55606E, .accentHighlight: 0xD9A441,
        .error: 0xB3261E, .warning: 0x9A5B00, .success: 0x3B7A3B, .link: 0x2E5A87,
        .selection: 0xEAE1CD, .highlight: 0xF6E8D0, .textOnAccent: 0xFFFFFF,
    ]

    static let dark: ThemePalette = [
        .backgroundPage: 0x1C1712, .backgroundCard: 0x262019, .border: 0x3A322B,
        .textPrimary: 0xEDE6D9, .textSecondary: 0xB7AB9A,
        .accentPrimary: 0xC97B54, .accentSecondary: 0x7C8794, .accentHighlight: 0xE8B968,
        .error: 0xF2877E, .warning: 0xE0A04A, .success: 0x7CC47C, .link: 0x8FB4DC,
        .selection: 0x312A24, .highlight: 0x57462D,
        // Today this is white (3.26:1 on the accent, below the 4.5:1 guard); near-black is 5.46:1.
        .textOnAccent: 0x1C1712,
    ]

    static let theme = Theme(id: "default", name: "Default", author: "Whimbrel", light: light, dark: dark)
}
