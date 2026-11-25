import SwiftUI

/// Which colors a screen draws for a picked skin, the light or dark appearance and the system's contrast setting.
///
/// While the system asks for more contrast (macOS Increase Contrast) every screen draws Thread's high contrast palette, whatever
/// skin is picked, and the stored skin choice is not touched. It is Thread's own high contrast palette (`WThreadPalette`) rather
/// than the bundled High Contrast skin's, because Thread's buttons, borders and focus ring already draw from it: text, backgrounds
/// and those parts then agree.
public enum WContrastPalette {
    /// The 15 skinnable colors to draw: `skin` as picked, or Thread's high contrast palette for `colorScheme` when `contrast` is
    /// `.increased`.
    public static func palette(skin: [String: UInt32], colorScheme: ColorScheme, contrast: ColorSchemeContrast) -> [String: UInt32] {
        increased(contrast) ? WVariant(colorScheme: colorScheme, contrast: contrast).threadPalette : skin
    }

    /// One color: `skin` (the picked skin's value for `token`), or Thread's high contrast value for `token` when `increased`.
    /// A token Thread does not have keeps `skin`.
    public static func hex(token: String, skin: UInt32, isDark: Bool, increased: Bool) -> UInt32 {
        guard increased else { return skin }
        let variant: WVariant = isDark ? .highContrastDark : .highContrastLight
        return variant.threadPalette[token] ?? skin
    }

    private static func increased(_ contrast: ColorSchemeContrast) -> Bool { contrast == .increased }
}
