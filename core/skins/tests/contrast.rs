//! Contrast, palette parsing and repair, the archetype pairs and the scrim. Expected numbers that
//! are literals were worked out in a separate Python implementation of the same formulas.

use skins::contrast::{self, composite, glass_glow, ratio, MIN_RATIO};
use skins::format::Archetype;
use skins::palette::{default_palette, load_palettes, parse_palette, Token, Variant, Variants};
use skins::repair::{
    all_pairs, archetype_pairs, hsl, pair_failures, pairs_pass, repair, repair_variant, Pair,
};
use skins::scrim::{blend, extremes, scrim_opacity, Glow};
use skins::{load_bundled, Palette};
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bundled_palettes() -> Vec<Palette> {
    let fallback = default_palette();
    let mut all = load_palettes(&repo().join("themes"), &fallback);
    all.extend(
        load_palettes(&repo().join("skins/palettes"), &fallback)
            .into_iter()
            .skip(1),
    );
    all
}

fn approx(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} is not within {tol} of {b}");
}

// --- contrast maths

#[test]
fn ratio_matches_known_values() {
    approx(ratio(0xFFFFFF, 0x000000), 21.0, 1e-9);
    approx(ratio(0x000000, 0xFFFFFF), 21.0, 1e-9);
    approx(ratio(0x808080, 0x808080), 1.0, 1e-12);
    approx(ratio(0xFFB612, 0xFFFFFF), 1.757040764779017, 1e-9);
    approx(ratio(0xFFFFFF, 0x1C1712), 17.786537408714604, 1e-9);
}

#[test]
fn composite_and_blend_round_to_the_nearest_channel() {
    assert_eq!(blend(0x000000, 0xFFFFFF, 0.5), 0x808080);
    assert_eq!(composite(0xFF0000, 0.1, 0x0000FF), 0x1A00E6);
    assert_eq!(composite(0x123456, 1.0, 0xFFFFFF), 0x123456);
    assert_eq!(composite(0x123456, 0.0, 0xABCDEF), 0xABCDEF);
}

#[test]
fn glass_glow_runs_from_accent_tint_to_the_page() {
    let glow = glass_glow(0xC97B54, 0x1C1712, 0x1C1712);
    let want = [
        0x503526, 0x563828, 0x593A29, 0x5A3B29, 0x583A29, 0x553827, 0x4E3425, 0x452F22, 0x3A281D,
        0x2C2019, 0x1C1712,
    ];
    assert_eq!(glow, want);
}

#[test]
fn the_shared_constants_match_the_macs_styles() {
    // The Mac's GlassStyle.cardOpacity / glowOpacity and FieldStyle stripe / line opacities.
    assert_eq!(contrast::GLASS_COMPOSITE, 0.9);
    assert_eq!(contrast::GLASS_GLOW_OPACITY, 0.3);
    assert_eq!(contrast::GLASS_GLOW_STEPS, 11);
    assert_eq!(contrast::FIELD_STRIPE_OPACITY, 0.10);
    assert_eq!(contrast::FIELD_LINE_OPACITY, 0.45);
}

// --- palettes

#[test]
fn the_default_palette_mirrors_themes_default_json() {
    let fallback = default_palette();
    let from_file = parse_palette(
        &std::fs::read(repo().join("themes/default.json")).unwrap(),
        &fallback,
    )
    .unwrap();
    assert_eq!(from_file, fallback);
    assert_eq!(fallback.light.0.len(), 15);
    assert_eq!(fallback.dark.0.len(), 15);
}

fn invalid(name: &str) -> Option<Palette> {
    parse_palette(
        &std::fs::read(repo().join("themes/invalid").join(name)).unwrap(),
        &default_palette(),
    )
}

#[test]
fn unusable_palette_files_are_skipped() {
    for f in [
        "not-json.json",
        "no-id.json",
        "no-variants.json",
        "bad-hex.json",
    ] {
        assert!(invalid(f).is_none(), "{f}");
    }
}

#[test]
fn a_failing_variant_is_replaced_by_the_fallbacks() {
    let p = invalid("low-contrast.json").unwrap();
    let d = default_palette();
    assert_eq!(p.light, d.light, "the failing light variant falls back");
    assert_ne!(p.dark, d.dark, "the passing dark variant is kept");
    assert_eq!(p.variants, Variants::Both);
}

#[test]
fn one_variant_serves_both_appearances_with_missing_tokens_from_default() {
    let d = default_palette();
    let p = invalid("missing-tokens.json").unwrap();
    assert_eq!(p.variants, Variants::LightOnly);
    assert_eq!(p.light, p.dark, "a light-only palette serves both");
    assert_eq!(
        p.light.get(Token::BackgroundPage),
        d.light.get(Token::BackgroundPage)
    );
    assert_ne!(
        p.light.get(Token::AccentPrimary),
        d.light.get(Token::AccentPrimary)
    );

    let s = invalid("single-variant.json").unwrap();
    assert_eq!(s.variants, Variants::DarkOnly);
    assert_eq!(s.light, s.dark);
    for token in Token::ALL {
        assert!(s.dark.get(token).is_some(), "{token:?}");
    }
}

#[test]
fn a_legacy_twelve_token_palette_gets_the_three_new_tokens_from_default() {
    let d = default_palette();
    let p = invalid("legacy-12-token.json").unwrap();
    for token in [Token::Selection, Token::Highlight, Token::TextOnAccent] {
        assert_eq!(p.light.get(token), d.light.get(token), "{token:?}");
    }
}

#[test]
fn a_palette_serialises_to_its_file_shape_and_parses_back() {
    let fallback = default_palette();
    for p in bundled_palettes() {
        let json = p.to_json();
        assert_eq!(
            json["light"].is_object(),
            p.variants != Variants::DarkOnly,
            "{}",
            p.id
        );
        assert_eq!(
            json["dark"].is_object(),
            p.variants != Variants::LightOnly,
            "{}",
            p.id
        );
        let again = parse_palette(&serde_json::to_vec(&json).unwrap(), &fallback).unwrap();
        assert_eq!(again, p, "{}", p.id);
    }
    assert_eq!(
        default_palette().to_json()["light"]["backgroundPage"],
        "#F7F2E7"
    );
}

#[test]
fn the_bundled_palette_directories_load() {
    let all = bundled_palettes();
    assert_eq!(all[0].id, "default");
    assert!(all.len() >= 15, "{} palettes", all.len());
    let ids: std::collections::BTreeSet<_> = all.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids.len(), all.len(), "duplicate palette ids");
    for id in ["slate", "sepia", "high-contrast", "amber", "paper", "ice"] {
        assert!(ids.contains(id), "{id}");
    }
}

// --- the archetype pairs

#[test]
fn every_bundled_skin_palette_passes_its_archetype() {
    let palettes = bundled_palettes();
    for skin in load_bundled(&repo().join("skins")) {
        let pairs = archetype_pairs(skin.today.archetype);
        for id in &skin.palettes {
            let p = palettes
                .iter()
                .find(|p| &p.id == id)
                .unwrap_or_else(|| panic!("{}: palette {id} is not bundled", skin.id));
            for (label, v) in [("light", &p.light), ("dark", &p.dark)] {
                let failures = pair_failures(v, &pairs);
                assert!(
                    failures.is_empty(),
                    "{} / {id} {label} on {:?}: {failures:?}",
                    skin.id,
                    skin.today.archetype
                );
            }
        }
    }
}

#[test]
fn the_pair_lists_have_the_expected_sizes() {
    let sizes = |a| archetype_pairs(a).len();
    assert_eq!(sizes(Archetype::Standard), 11);
    assert_eq!(sizes(Archetype::Terminal), 11);
    assert_eq!(sizes(Archetype::Almanac), 11);
    assert_eq!(sizes(Archetype::Reference), 13);
    assert_eq!(sizes(Archetype::Glass), 9 + 45 + 22);
    assert_eq!(sizes(Archetype::Field), 13);
    assert_eq!(all_pairs().len(), 84, "each distinct pair once");
}

#[test]
fn glass_checks_the_greeting_text_on_every_glow_step() {
    let glass = archetype_pairs(Archetype::Glass);
    let glow: Vec<&Pair> = glass
        .iter()
        .filter(|p| p.name.starts_with("glass glow "))
        .collect();
    assert_eq!(glow.len(), 22);
    assert!(glow
        .iter()
        .all(|p| matches!(p.text, Token::TextPrimary | Token::TextSecondary)));
}

#[test]
fn default_fails_glass_then_repairs() {
    let d = default_palette();
    let glass = archetype_pairs(Archetype::Glass);
    let before = pair_failures(&d.light, &glass);
    assert!(!before.is_empty());
    // The glow's worst step is about 3.38:1 (the Mac's own test comment says about 3.4).
    assert!(
        before
            .iter()
            .any(|l| l.starts_with("textSecondary on glass glow") && l.ends_with("= 3.38")),
        "{before:?}"
    );
    let (fixed, changed) = repair_variant(&d.light, &glass);
    assert!(
        pairs_pass(&fixed, &glass),
        "{:?}",
        pair_failures(&fixed, &glass)
    );
    assert!(changed.contains(&Token::TextSecondary));
    let (fixed_dark, _) = repair_variant(&d.dark, &glass);
    assert!(pairs_pass(&fixed_dark, &glass));
}

// --- repair

/// SplitMix64, so the palettes are the same on every run.
struct Seeded(u64);
impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn colour(&mut self) -> u32 {
        (self.next() % 0x0100_0000) as u32
    }
}

/// 8-bit chroma (max - min channel); below about 120 the hue of a rounded colour can drift over 1 degree.
fn chroma(c: u32) -> u32 {
    let ch = [(c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF];
    ch.iter().max().unwrap() - ch.iter().min().unwrap()
}

fn random_variant(rng: &mut Seeded) -> Variant {
    Variant(Token::ALL.into_iter().map(|t| (t, rng.colour())).collect())
}

#[test]
fn repair_always_passes_and_reports_what_it_changed() {
    let mut rng = Seeded(20_261_022);
    let all = all_pairs();
    let standard = archetype_pairs(Archetype::Standard);
    for n in 0..500 {
        let v = random_variant(&mut rng);
        let (fixed, changed) = repair_variant(&v, &all);
        assert!(
            pairs_pass(&fixed, &all),
            "palette {n}: {:?}",
            pair_failures(&fixed, &all)
        );
        assert!(pairs_pass(&fixed, &standard), "palette {n} on Standard");
        for token in Token::ALL {
            if !changed.contains(&token) {
                assert_eq!(
                    fixed.get(token),
                    v.get(token),
                    "palette {n}: {token:?} changed without being reported"
                );
            }
        }
    }
}

#[test]
fn repair_keeps_hue() {
    let mut rng = Seeded(20_261_021);
    let all = all_pairs();
    let mut checked = 0;
    for _ in 0..500 {
        let v = random_variant(&mut rng);
        let (fixed, changed) = repair_variant(&v, &all);
        for token in changed {
            let (before, after) = (v.get(token).unwrap(), fixed.get(token).unwrap());
            if chroma(before) >= 120 && chroma(after) >= 120 {
                let diff = (hsl(before).0 - hsl(after).0).abs();
                assert!(diff.min(360.0 - diff) <= 1.0, "{token:?} hue moved {diff}");
                checked += 1;
            }
        }
    }
    assert!(
        checked > 50,
        "too few changed colours kept enough chroma to measure hue: {checked}"
    );
}

#[test]
fn steelers_gold_on_white_is_repaired_on_every_pair() {
    let mut v = default_palette().light;
    v.set(Token::BackgroundPage, 0xFFFFFF);
    v.set(Token::BackgroundCard, 0xFFFFFF);
    v.set(Token::AccentPrimary, 0xFFB612);
    let all = all_pairs();
    assert!(!pairs_pass(&v, &all));
    let (fixed, changed) = repair_variant(&v, &all);
    assert!(pair_failures(&fixed, &all).is_empty());
    assert!(changed.contains(&Token::AccentPrimary));
    let gold = fixed.get(Token::AccentPrimary).unwrap();
    for bg in [Token::BackgroundPage, Token::BackgroundCard] {
        assert!(ratio(gold, fixed.get(bg).unwrap()) >= MIN_RATIO);
    }
    let diff = (hsl(gold).0 - hsl(0xFFB612).0).abs();
    assert!(diff.min(360.0 - diff) <= 2.0, "still gold");
}

#[test]
fn a_passing_palette_is_unchanged() {
    // Default passes the theme guard (not every archetype: its secondary text on Glass's glow is about 3.4:1).
    let guard: Vec<Pair> = contrast::REQUIRED_PAIRS
        .iter()
        .map(|&(t, b)| Pair::plain(t, b))
        .collect();
    let d = default_palette();
    for v in [&d.light, &d.dark] {
        let (fixed, changed) = repair_variant(v, &guard);
        assert!(changed.is_empty());
        assert_eq!(&fixed, v);
    }
}

#[test]
fn repair_of_a_palette_repairs_both_variants_and_keeps_the_rest() {
    let d = default_palette();
    let (fixed, changed) = repair(&d, &archetype_pairs(Archetype::Glass));
    assert_eq!(
        (fixed.id.as_str(), fixed.name.as_str(), fixed.variants),
        ("default", "Default", Variants::Both)
    );
    assert!(pairs_pass(&fixed.light, &archetype_pairs(Archetype::Glass)));
    assert!(pairs_pass(&fixed.dark, &archetype_pairs(Archetype::Glass)));
    assert!(!changed.is_empty());
    assert_ne!(fixed.light, d.light);
}

#[test]
fn surfaces_at_both_ends_move_the_surface() {
    // The text must read on white, black and mid grey at once, which no lightness of it does. Black suits
    // two of the three, so it goes to black and the surface it still fails on (the black one) lightens.
    let mut v = default_palette().light;
    v.set(Token::BackgroundPage, 0xFFFFFF);
    v.set(Token::BackgroundCard, 0x000000);
    v.set(Token::Selection, 0x808080);
    v.set(Token::TextPrimary, 0x808080);
    let pairs: Vec<Pair> = [
        Token::BackgroundPage,
        Token::BackgroundCard,
        Token::Selection,
    ]
    .into_iter()
    .map(|bg| Pair::plain(Token::TextPrimary, bg))
    .collect();
    let (fixed, changed) = repair_variant(&v, &pairs);
    assert!(
        pairs_pass(&fixed, &pairs),
        "{:?}",
        pair_failures(&fixed, &pairs)
    );
    assert_eq!(fixed.get(Token::TextPrimary), Some(0x000000));
    assert!(changed.contains(&Token::BackgroundCard), "{changed:?}");
    assert_eq!(
        fixed.get(Token::BackgroundPage),
        Some(0xFFFFFF),
        "a surface that already reads stays"
    );
    assert!(fixed.get(Token::BackgroundCard).unwrap() > 0x000000);
}

// --- scrim

fn glow(accent: u32, text: &[u32]) -> Glow {
    Glow {
        accent,
        text: text.to_vec(),
    }
}

#[test]
fn scrim_matches_the_reference_values() {
    let cases: [(u32, u32, f64, f64); 3] = [
        (0x111111, 0xFAFAFA, 0.5, 0.38),
        (0xF2F2F2, 0x101010, 0.61, 0.47),
        (0x1A1A1A, 0xF4EDE0, 0.55, 0.44),
    ];
    for (text, page, over_white_black, over_mid) in cases {
        approx(
            scrim_opacity(&[text], page, 0xFFFFFF, 0x000000, None),
            over_white_black,
            0.005,
        );
        approx(
            scrim_opacity(&[text], page, 0xC0C0C0, 0x303030, None),
            over_mid,
            0.005,
        );
    }
    approx(
        scrim_opacity(&[0x111111, 0x444444], 0xFAFAFA, 0xFFFFFF, 0x000000, None),
        0.71,
        0.005,
    );
}

#[test]
fn scrim_caps_at_one_when_contrast_is_unreachable() {
    assert_eq!(
        scrim_opacity(&[0x808080], 0x808080, 0xFFFFFF, 0x000000, None),
        1.0
    );
}

#[test]
fn glass_glow_text_raises_the_scrim() {
    let none = scrim_opacity(&[0xF0F0F0], 0x101010, 0xFFFFFF, 0x000000, None);
    let dim = scrim_opacity(
        &[0xF0F0F0],
        0x101010,
        0xFFFFFF,
        0x000000,
        Some(&glow(0xA0A0A0, &[0xF0F0F0])),
    );
    let bright = scrim_opacity(
        &[0xF0F0F0],
        0x101010,
        0xFFFFFF,
        0x000000,
        Some(&glow(0xE0E0E0, &[0xF0F0F0])),
    );
    approx(none, 0.61, 0.005);
    approx(dim, 0.71, 0.005);
    approx(bright, 0.85, 0.005);
}

#[test]
fn scrim_is_the_smallest_opacity_that_reads() {
    let mut rng = Seeded(7);
    for _ in 0..100 {
        let (text, page) = (rng.colour(), rng.colour());
        let (a, b) = (rng.colour(), rng.colour());
        let op = scrim_opacity(&[text], page, a, b, None);
        assert!((0.0..=1.0).contains(&op));
        if op < 1.0 {
            for under in [a, b] {
                assert!(ratio(text, blend(under, page, op)) >= MIN_RATIO);
            }
            if op > 0.0 {
                let less = op - 0.01;
                let worst = [a, b]
                    .iter()
                    .map(|&u| ratio(text, blend(u, page, less)))
                    .fold(f64::MAX, f64::min);
                assert!(worst < MIN_RATIO, "{op} is not the smallest");
            }
        }
    }
}

fn solid(rgba: [u8; 4], w: u32, h: u32) -> Vec<u8> {
    (0..w * h).flat_map(|_| rgba).collect()
}

#[test]
fn extremes_of_solid_images() {
    assert_eq!(
        extremes(&solid([255, 255, 255, 255], 8, 8), 8, 8, 0x808080).0,
        0xFFFFFF
    );
    assert_eq!(
        extremes(&solid([0, 0, 0, 255], 8, 8), 8, 8, 0x808080).1,
        0x000000
    );
    assert_eq!(
        extremes(&solid([0x12, 0x34, 0x56, 255], 3, 5), 3, 5, 0).0,
        0x123456
    );
}

#[test]
fn extremes_find_both_ends_of_a_gradient() {
    let (w, h) = (100u32, 10u32);
    let mut px = Vec::new();
    for _y in 0..h {
        for x in 0..w {
            let v = (x * 255 / (w - 1)) as u8;
            px.extend([v, v, v, 255]);
        }
    }
    let (light, dark) = extremes(&px, w, h, 0x808080);
    assert!(light >> 16 >= 0xFA, "{light:06x}");
    assert!(dark >> 16 <= 0x05, "{dark:06x}");
}

#[test]
fn transparent_pixels_count_as_the_page() {
    let (light, dark) = extremes(&solid([255, 255, 255, 0], 4, 4), 4, 4, 0x203040);
    assert_eq!((light, dark), (0x203040, 0x203040));
    // Half-transparent white over black is mid grey.
    let (l, _) = extremes(&solid([255, 255, 255, 128], 4, 4), 4, 4, 0x000000);
    assert_eq!(l, 0x808080);
}

#[test]
fn extremes_of_an_empty_or_short_buffer_are_white_and_black() {
    assert_eq!(extremes(&[], 0, 0, 0x123456), (0xFFFFFF, 0x000000));
    assert_eq!(extremes(&[1, 2, 3], 4, 4, 0x123456), (0xFFFFFF, 0x000000));
}

#[test]
fn a_small_image_is_sampled_whole() {
    // 2x2: white top-left, black bottom-right; nearest-neighbour sampling reaches every pixel.
    let px = [
        255, 255, 255, 255, 10, 20, 30, 255, 40, 50, 60, 255, 0, 0, 0, 255,
    ];
    let (light, dark) = extremes(&px, 2, 2, 0x808080);
    assert_eq!((light, dark), (0xFFFFFF, 0x000000));
}

// --- the guard pairs and the dark variant

#[test]
fn every_required_pair_is_guarded() {
    for (fg, bg) in contrast::REQUIRED_PAIRS {
        let mut v = default_palette().light;
        assert!(contrast::failures(&v).is_empty());
        v.set(fg, v.get(bg).unwrap());
        assert!(
            contrast::failures(&v).contains(&(fg, bg)),
            "{fg:?} on {bg:?}"
        );
        let line = format!("{} on {} = 1.00", fg.name(), bg.name());
        assert!(
            contrast::failure_lines(&v).contains(&line),
            "{line} in {:?}",
            contrast::failure_lines(&v)
        );
    }
    let mut missing = default_palette().light;
    missing.0.remove(&Token::TextOnAccent);
    assert_eq!(
        contrast::failure_lines(&missing),
        ["textOnAccent on accentPrimary = missing"]
    );
}

#[test]
fn a_failing_dark_variant_is_replaced_by_the_fallbacks() {
    let d = default_palette();
    let mut dark = d.dark.clone();
    dark.set(Token::TextPrimary, dark.get(Token::BackgroundPage).unwrap());
    let json = serde_json::json!({
        "id": "bad-dark", "name": "Bad dark",
        "light": d.light, "dark": dark,
    });
    let p = parse_palette(&serde_json::to_vec(&json).unwrap(), &d).unwrap();
    assert_eq!(p.dark, d.dark, "the failing dark variant falls back");
    assert_eq!(p.light, d.light);
    assert_eq!(p.variants, Variants::Both);
}

// --- golden repairs, for the Mac to run too

fn variant_from_json(v: &serde_json::Value) -> Variant {
    Variant(
        Token::ALL
            .into_iter()
            .map(|t| {
                (
                    t,
                    u32::from_str_radix(&v[t.name()].as_str().unwrap()[1..], 16).unwrap(),
                )
            })
            .collect(),
    )
}

fn pairs_named(name: &str) -> Vec<Pair> {
    match name {
        "all" => all_pairs(),
        "glass" => archetype_pairs(Archetype::Glass),
        "field" => archetype_pairs(Archetype::Field),
        "standard" => archetype_pairs(Archetype::Standard),
        other => panic!("unknown pair set {other}"),
    }
}

/// The palettes the golden file covers: random ones across the pair sets, Steelers gold, and Default on Glass.
fn golden_inputs() -> Vec<(String, &'static str, Variant)> {
    let mut rng = Seeded(1022);
    let mut cases = Vec::new();
    for n in 0..30 {
        cases.push((format!("random-{n}"), "all", random_variant(&mut rng)));
    }
    for (n, set) in ["glass", "field", "standard"].into_iter().enumerate() {
        for k in 0..4 {
            cases.push((
                format!("random-{set}-{k}"),
                set,
                random_variant(&mut Seeded(500 + (n * 10 + k) as u64)),
            ));
        }
    }
    let mut gold = default_palette().light;
    gold.set(Token::BackgroundPage, 0xFFFFFF);
    gold.set(Token::BackgroundCard, 0xFFFFFF);
    gold.set(Token::AccentPrimary, 0xFFB612);
    cases.push(("steelers-gold-on-white".into(), "all", gold));
    cases.push((
        "default-light-on-glass".into(),
        "glass",
        default_palette().light,
    ));
    cases.push((
        "default-dark-on-glass".into(),
        "glass",
        default_palette().dark,
    ));
    cases
}

#[test]
fn repair_matches_the_golden_file() {
    let path = repo().join("skins/testdata/repair/palette-repair.json");
    let mut produced = Vec::new();
    for (name, set, input) in golden_inputs() {
        let (fixed, changed) = repair_variant(&input, &pairs_named(set));
        produced.push(serde_json::json!({
            "name": name, "pairs": set, "input": input, "expected": fixed,
            "changed": changed.iter().map(|t| t.name()).collect::<Vec<_>>(),
        }));
    }
    let file = serde_json::json!({ "cases": produced });
    if std::env::var("WHIMBREL_UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap() + "\n").unwrap();
    }
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("golden file")).unwrap();
    assert_eq!(
        stored, file,
        "repair output changed; rerun with WHIMBREL_UPDATE_GOLDEN=1 only if that is intended"
    );
    // And the stored file itself is a repair the Mac can verify: every expected variant passes its pair set.
    for case in stored["cases"].as_array().unwrap() {
        let fixed = variant_from_json(&case["expected"]);
        assert!(
            pairs_pass(&fixed, &pairs_named(case["pairs"].as_str().unwrap())),
            "{}",
            case["name"]
        );
    }
}
