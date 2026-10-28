//! Palette repair and the contrast pairs each Today archetype draws. Ports `PaletteRepair` and
//! `ArchetypeContrast` (PaletteRepair.swift).
//!
//! A palette is repaired, not replaced: each text colour of a failing pair moves its HSL lightness in
//! 2% steps (hue and saturation kept) to the nearest value that passes on all of its surfaces. When
//! none does (its surfaces sit at both ends), the text goes to the end that suits most of them and
//! each surface still failing has its base colour step away from it the same way.

use crate::contrast::{
    composite, glass_glow, ratio, Rgb, FIELD_LINE_OPACITY, FIELD_STRIPE_OPACITY, GLASS_COMPOSITE,
    GLASS_GLOW_STEPS, MIN_RATIO, REQUIRED_PAIRS,
};
use crate::format::Archetype;
use crate::palette::{Palette, Token, Variant};
use std::sync::Arc;

const STEP: f64 = 0.02;
/// Rounds of repair; a colour moved for one pair can break another (the accent is text and a surface).
const ROUNDS: usize = 12;

type Surface = Arc<dyn Fn(&Variant) -> Option<Rgb> + Send + Sync>;

/// A text colour and the surface it is drawn on.
#[derive(Clone)]
pub struct Pair {
    pub text: Token,
    /// The palette colour the surface is built on: the one a repair may move.
    pub base: Token,
    pub name: String,
    surface: Surface,
}

impl Pair {
    pub fn surface(&self, v: &Variant) -> Option<Rgb> {
        (self.surface)(v)
    }

    fn new(
        text: Token,
        base: Token,
        name: impl Into<String>,
        f: impl Fn(&Variant) -> Option<Rgb> + Send + Sync + 'static,
    ) -> Pair {
        Pair {
            text,
            base,
            name: name.into(),
            surface: Arc::new(f),
        }
    }

    /// `text` on the plain `background` colour.
    pub fn plain(text: Token, background: Token) -> Pair {
        Pair::new(text, background, background.name(), move |v| {
            v.get(background)
        })
    }
}

fn on(texts: &[Token], backgrounds: &[Token]) -> Vec<Pair> {
    texts
        .iter()
        .flat_map(|&t| backgrounds.iter().map(move |&b| Pair::plain(t, b)))
        .collect()
}

/// The pairs every archetype draws: the required pairs, and `error` on the page and cards.
fn common() -> Vec<Pair> {
    let mut pairs: Vec<Pair> = REQUIRED_PAIRS
        .iter()
        .map(|&(t, b)| Pair::plain(t, b))
        .collect();
    pairs.extend(on(
        &[Token::Error],
        &[Token::BackgroundPage, Token::BackgroundCard],
    ));
    pairs
}

fn glow_of(v: &Variant) -> Option<Vec<Rgb>> {
    let (accent, page) = (v.get(Token::AccentPrimary)?, v.get(Token::BackgroundPage)?);
    Some(glass_glow(accent, page, page))
}

/// Glass: the card is `backgroundCard` at 0.9 over a material that shows any backdrop (white and black
/// bound it); the greeting and date sit straight on the glow.
fn glass() -> Vec<Pair> {
    type Backdrop = Arc<dyn Fn(&Variant, usize) -> Option<Rgb> + Send + Sync>;
    let backdrops: Vec<(&str, usize, Backdrop)> = vec![
        ("page", 1, Arc::new(|v, _| v.get(Token::BackgroundPage))),
        (
            "glow",
            GLASS_GLOW_STEPS,
            Arc::new(|v, i| glow_of(v).map(|g| g[i])),
        ),
        ("accent", 1, Arc::new(|v, _| v.get(Token::AccentPrimary))),
        ("white", 1, Arc::new(|_, _| Some(0xFFFFFF))),
        ("black", 1, Arc::new(|_, _| Some(0))),
    ];
    let mut pairs = Vec::new();
    for (label, count, colour) in &backdrops {
        for i in 0..*count {
            for text in [Token::TextPrimary, Token::TextSecondary, Token::Error] {
                let name = if *count == 1 {
                    format!("glass card over {label}")
                } else {
                    format!("glass card over {label} {i}")
                };
                let colour = colour.clone();
                pairs.push(Pair::new(text, Token::BackgroundCard, name, move |v| {
                    let under = colour(v, i)?;
                    Some(composite(
                        v.get(Token::BackgroundCard)?,
                        GLASS_COMPOSITE,
                        under,
                    ))
                }));
            }
        }
    }
    for i in 0..GLASS_GLOW_STEPS {
        for text in [Token::TextPrimary, Token::TextSecondary] {
            pairs.push(Pair::new(
                text,
                Token::BackgroundPage,
                format!("glass glow {i}"),
                move |v| glow_of(v).map(|g| g[i]),
            ));
        }
    }
    pairs
}

/// Field: labels on the pitch (`backgroundCard` under accent stripes), where a chalk line is the worst case.
fn field() -> Vec<Pair> {
    let stripe = |v: &Variant| -> Option<Rgb> {
        Some(composite(
            v.get(Token::AccentPrimary)?,
            FIELD_STRIPE_OPACITY,
            v.get(Token::BackgroundCard)?,
        ))
    };
    let mut pairs = Vec::new();
    for text in [Token::TextPrimary, Token::TextSecondary] {
        pairs.push(Pair::new(
            text,
            Token::BackgroundCard,
            "pitch stripe",
            stripe,
        ));
        // The chalk line dominates this surface, so a repair moves it.
        pairs.push(Pair::new(
            text,
            Token::Border,
            "pitch stripe+line",
            move |v| {
                Some(composite(
                    v.get(Token::Border)?,
                    FIELD_LINE_OPACITY,
                    stripe(v)?,
                ))
            },
        ));
    }
    pairs
}

/// Every text-on-surface pair Today draws for `archetype`.
pub fn archetype_pairs(archetype: Archetype) -> Vec<Pair> {
    let mut pairs = common();
    match archetype {
        Archetype::Standard | Archetype::Almanac | Archetype::Terminal => {
            pairs.extend(on(
                &[Token::AccentPrimary],
                &[Token::BackgroundPage, Token::BackgroundCard],
            ));
        }
        Archetype::Reference => {
            pairs.extend(on(
                &[Token::AccentPrimary, Token::Link],
                &[Token::BackgroundPage, Token::BackgroundCard],
            ));
        }
        Archetype::Glass => pairs.extend(glass()),
        Archetype::Field => pairs.extend(field()),
    }
    pairs
}

/// Every archetype's pairs, once each (a user skin can take any archetype, so a repair covers all).
pub fn all_pairs() -> Vec<Pair> {
    let order = [
        Archetype::Standard,
        Archetype::Terminal,
        Archetype::Glass,
        Archetype::Field,
        Archetype::Almanac,
        Archetype::Reference,
    ];
    let mut seen = std::collections::BTreeSet::new();
    order
        .into_iter()
        .flat_map(archetype_pairs)
        .filter(|p| seen.insert(format!("{} on {}", p.text.name(), p.name)))
        .collect()
}

/// One line per failing pair, e.g. `accentPrimary on backgroundPage = 1.73`.
pub fn pair_failures(v: &Variant, pairs: &[Pair]) -> Vec<String> {
    pairs
        .iter()
        .filter_map(|pair| match (v.get(pair.text), pair.surface(v)) {
            (Some(text), Some(surface)) => {
                let r = ratio(text, surface);
                (r < MIN_RATIO).then(|| format!("{} on {} = {r:.2}", pair.text.name(), pair.name))
            }
            _ => Some(format!("{} on {} = missing", pair.text.name(), pair.name)),
        })
        .collect()
}

pub fn pairs_pass(v: &Variant, pairs: &[Pair]) -> bool {
    pair_failures(v, pairs).is_empty()
}

fn passes_pair(text: Rgb, pair: &Pair, v: &Variant) -> bool {
    pair.surface(v)
        .map(|s| ratio(text, s) >= MIN_RATIO)
        .unwrap_or(true)
}

/// Repairs one variant so it passes `pairs`. Also returns the tokens that moved.
pub fn repair_variant(variant: &Variant, pairs: &[Pair]) -> (Variant, Vec<Token>) {
    let mut p = variant.clone();
    let mut changed: Vec<Token> = Vec::new();
    fn mark(t: Token, changed: &mut Vec<Token>) {
        if !changed.contains(&t) {
            changed.push(t);
        }
    }

    let mut texts: Vec<Token> = Vec::new();
    for pair in pairs {
        if !texts.contains(&pair.text) {
            texts.push(pair.text);
        }
    }
    for _ in 0..ROUNDS {
        if pairs_pass(&p, pairs) {
            break;
        }
        for &text in &texts {
            let Some(colour) = p.get(text) else { continue };
            let mine: Vec<&Pair> = pairs.iter().filter(|x| x.text == text).collect();
            if mine.iter().all(|x| passes_pair(colour, x, &p)) {
                continue;
            }
            // Hue and saturation always come from the requested colour, so repeated moves never drift it.
            let requested = variant.get(text).unwrap_or(colour);
            if let Some(fixed) = nearest(requested, |c| mine.iter().all(|x| passes_pair(c, x, &p)))
            {
                p.set(text, fixed);
                mark(text, &mut changed);
                continue;
            }
            let (black, white) = (
                with_lightness(requested, 0.0),
                with_lightness(requested, 1.0),
            );
            let dark_text = mine.iter().filter(|x| passes_pair(black, x, &p)).count()
                >= mine.iter().filter(|x| passes_pair(white, x, &p)).count();
            let end = if dark_text { black } else { white };
            p.set(text, end);
            mark(text, &mut changed);
            for pair in &mine {
                if passes_pair(end, pair, &p) {
                    continue;
                }
                let Some(b) = p.get(pair.base) else { continue };
                let hue_from = variant.get(pair.base).unwrap_or(b);
                let moved = away(b, hue_from, !dark_text, |candidate| {
                    let mut q = p.clone();
                    q.set(pair.base, candidate);
                    passes_pair(end, pair, &q)
                });
                p.set(pair.base, moved);
                mark(pair.base, &mut changed);
            }
        }
    }
    (p, changed)
}

/// Repairs both variants of a palette. The tokens that moved in either are returned once each.
pub fn repair(palette: &Palette, pairs: &[Pair]) -> (Palette, Vec<Token>) {
    let (light, mut changed) = repair_variant(&palette.light, pairs);
    let (dark, more) = repair_variant(&palette.dark, pairs);
    for t in more {
        if !changed.contains(&t) {
            changed.push(t);
        }
    }
    (
        Palette {
            light,
            dark,
            ..palette.clone()
        },
        changed,
    )
}

/// The colour at the nearest lightness (2% steps either way) that satisfies `ok`, or `None`.
fn nearest(colour: Rgb, ok: impl Fn(Rgb) -> bool) -> Option<Rgb> {
    let l = hsl(colour).2;
    for k in 1..=((1.0 / STEP) as i32) {
        for target in [l - f64::from(k) * STEP, l + f64::from(k) * STEP] {
            let c = with_lightness(colour, target.clamp(0.0, 1.0));
            if ok(c) {
                return Some(c);
            }
        }
    }
    None
}

/// Steps `colour`'s lightness toward black (`darker`) or white, in `hue`'s hue and saturation, until
/// `ok`; the end itself always passes against the opposite end.
fn away(colour: Rgb, hue: Rgb, darker: bool, ok: impl Fn(Rgb) -> bool) -> Rgb {
    let mut l = hsl(colour).2;
    loop {
        l = if darker {
            (l - STEP).max(0.0)
        } else {
            (l + STEP).min(1.0)
        };
        let c = with_lightness(hue, l);
        if ok(c) || l == 0.0 || l == 1.0 {
            return c;
        }
    }
}

pub fn with_lightness(colour: Rgb, l: f64) -> Rgb {
    let (h, s, _) = hsl(colour);
    from_hsl(h, s, l)
}

/// (hue 0..360, saturation 0..1, lightness 0..1).
pub fn hsl(colour: Rgb) -> (f64, f64, f64) {
    let r = f64::from((colour >> 16) & 0xFF) / 255.0;
    let g = f64::from((colour >> 8) & 0xFF) / 255.0;
    let b = f64::from(colour & 0xFF) / 255.0;
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let (l, d) = ((mx + mn) / 2.0, mx - mn);
    if d <= 0.0 {
        return (0.0, 0.0, l);
    }
    let s = (d / (1.0 - (2.0 * l - 1.0).abs())).min(1.0);
    let mut h = if mx == r {
        (g - b) / d
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h *= 60.0;
    if h < 0.0 {
        h += 360.0;
    }
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as i64 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| -> u32 { ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32 };
    byte(r) << 16 | byte(g) << 8 | byte(b)
}
