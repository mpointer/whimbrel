//! WCAG contrast and the colour maths the Glass and Field archetypes share with it. Ports
//! `ThemeContrast` (Theme.swift) and the numbers in `GlassStyle` and `FieldStyle`. Colours are
//! `0xRRGGBB`.

use crate::palette::{Token, Variant};

/// A colour as `0xRRGGBB`.
pub type Rgb = u32;

/// Every text colour must reach this ratio on every surface it is drawn on.
pub const MIN_RATIO: f64 = 4.5;

/// How much of `backgroundCard` covers Glass's material. The tint is mostly opaque, so text contrast
/// is set by the palette, not the backdrop. (The Mac's `GlassStyle.cardOpacity`.)
pub const GLASS_COMPOSITE: f64 = 0.9;
/// Strength of `accentPrimary` at the top of Glass's gradient (it fades to `backgroundPage`).
pub const GLASS_GLOW_OPACITY: f64 = 0.3;
/// The glow's colours, top to bottom in tenths: 0..=10.
pub const GLASS_GLOW_STEPS: usize = 11;
/// Strength of the `accentPrimary` mowing stripes over Field's pitch (`backgroundCard`).
pub const FIELD_STRIPE_OPACITY: f64 = 0.10;
/// Strength of the `border` chalk lines over Field's pitch. Labels may cross one.
pub const FIELD_LINE_OPACITY: f64 = 0.45;

fn channel(value: u32) -> f64 {
    let c = f64::from(value) / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn relative_luminance(hex: Rgb) -> f64 {
    0.2126 * channel((hex >> 16) & 0xFF)
        + 0.7152 * channel((hex >> 8) & 0xFF)
        + 0.0722 * channel(hex & 0xFF)
}

/// WCAG contrast ratio, 1..=21.
pub fn ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `fg` at `alpha` over opaque `bg`, per sRGB channel.
pub fn composite(fg: Rgb, alpha: f64, bg: Rgb) -> Rgb {
    let mix = |shift: u32| -> u32 {
        (f64::from((fg >> shift) & 0xFF) * alpha + f64::from((bg >> shift) & 0xFF) * (1.0 - alpha))
            .round() as u32
    };
    mix(16) << 16 | mix(8) << 8 | mix(0)
}

/// Glass's glow, top to bottom in tenths, over opaque `under` (the page, or a scrimmed background
/// image): `accent` at `GLASS_GLOW_OPACITY` fading to opaque `page`, interpolated unpremultiplied.
pub fn glass_glow(accent: Rgb, page: Rgb, under: Rgb) -> Vec<Rgb> {
    (0..GLASS_GLOW_STEPS)
        .map(|step| {
            let t = step as f64 / 10.0;
            let colour = composite(page, t, accent);
            composite(
                colour,
                GLASS_GLOW_OPACITY + (1.0 - GLASS_GLOW_OPACITY) * t,
                under,
            )
        })
        .collect()
}

/// (foreground, background) pairs every variant must keep readable.
pub const REQUIRED_PAIRS: [(Token, Token); 7] = [
    (Token::TextPrimary, Token::BackgroundPage),
    (Token::TextPrimary, Token::BackgroundCard),
    (Token::TextSecondary, Token::BackgroundPage),
    (Token::TextSecondary, Token::BackgroundCard),
    (Token::TextPrimary, Token::Selection),
    (Token::TextPrimary, Token::Highlight),
    (Token::TextOnAccent, Token::AccentPrimary),
];

/// The required pairs a variant fails, in order.
pub fn failures(v: &Variant) -> Vec<(Token, Token)> {
    REQUIRED_PAIRS
        .into_iter()
        .filter(|&(fg, bg)| match (v.get(fg), v.get(bg)) {
            (Some(f), Some(b)) => ratio(f, b) < MIN_RATIO,
            _ => true,
        })
        .collect()
}

/// One line per failing pair, e.g. `textOnAccent on accentPrimary = 3.26`.
pub fn failure_lines(v: &Variant) -> Vec<String> {
    REQUIRED_PAIRS
        .into_iter()
        .filter_map(|(fg, bg)| match (v.get(fg), v.get(bg)) {
            (Some(f), Some(b)) => {
                let r = ratio(f, b);
                (r < MIN_RATIO).then(|| format!("{} on {} = {r:.2}", fg.name(), bg.name()))
            }
            _ => Some(format!("{} on {} = missing", fg.name(), bg.name())),
        })
        .collect()
}

pub fn passes(v: &Variant) -> bool {
    failures(v).is_empty()
}
