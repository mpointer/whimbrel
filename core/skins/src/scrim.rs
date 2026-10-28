//! The scrim laid over an image so the text on top still reaches 4.5:1. Ports
//! `SkinImageryView.scrimOpacity`, `blend` and `extremes`.

use crate::contrast::{glass_glow, ratio, relative_luminance, Rgb, MIN_RATIO};

/// Glass's glow, whose text (the greeting and date: primary and secondary) must also read on every
/// colour of the glow laid over the scrimmed image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glow {
    pub accent: Rgb,
    pub text: Vec<Rgb>,
}

/// `over` shown at `opacity` on top of `under`, per sRGB channel.
pub fn blend(under: Rgb, over: Rgb, opacity: f64) -> Rgb {
    let mix = |shift: u32| -> u32 {
        let (u, o) = (
            f64::from((under >> shift) & 0xFF),
            f64::from((over >> shift) & 0xFF),
        );
        ((u * (1.0 - opacity) + o * opacity).round() as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// The smallest opacity (in 1% steps) at which every `text` colour reaches 4.5:1 over the lightest and
/// the darkest colour under the scrim, and with a `glow` its text on every colour of the glow laid over
/// those too; 1 when even a solid scrim falls short.
pub fn scrim_opacity(
    text: &[Rgb],
    scrim: Rgb,
    lightest: Rgb,
    darkest: Rgb,
    glow: Option<&Glow>,
) -> f64 {
    let reads =
        |colours: &[Rgb], surface: Rgb| colours.iter().all(|&c| ratio(c, surface) >= MIN_RATIO);
    for step in 0..=100 {
        let opacity = f64::from(step) / 100.0;
        let ok = [lightest, darkest].into_iter().all(|under| {
            let shown = blend(under, scrim, opacity);
            if !reads(text, shown) {
                return false;
            }
            match glow {
                None => true,
                Some(g) => glass_glow(g.accent, scrim, shown)
                    .into_iter()
                    .all(|c| reads(&g.text, c)),
            }
        });
        if ok {
            return opacity;
        }
    }
    1.0
}

/// The lightest and darkest colours (by luminance) in a 64x64 nearest-neighbour sample of an RGBA
/// image, transparent pixels counting as `page`. An empty image is white and black.
pub fn extremes(rgba: &[u8], width: u32, height: u32, page: Rgb) -> (Rgb, Rgb) {
    const N: u32 = 64;
    if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return (0xFFFFFF, 0x000000);
    }
    let mut lightest = (0u32, -1.0f64);
    let mut darkest = (0u32, 2.0f64);
    for y in 0..N {
        for x in 0..N {
            let sx = ((u64::from(x) * 2 + 1) * u64::from(width) / (u64::from(N) * 2)) as usize;
            let sy = ((u64::from(y) * 2 + 1) * u64::from(height) / (u64::from(N) * 2)) as usize;
            let i = (sy * width as usize + sx) * 4;
            let a = f64::from(rgba[i + 3]) / 255.0;
            let src =
                u32::from(rgba[i]) << 16 | u32::from(rgba[i + 1]) << 8 | u32::from(rgba[i + 2]);
            let hex = blend(page, src, a);
            let l = relative_luminance(hex);
            if l > lightest.1 {
                lightest = (hex, l);
            }
            if l < darkest.1 {
                darkest = (hex, l);
            }
        }
    }
    (lightest.0, darkest.0)
}
