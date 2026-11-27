//! Richer skins row 12: Classic, High Contrast and Hypermodern choose their Home elements in
//! `today.options` (docs/superpowers/plans/2026-11-25-richer-skins.md, section 7). The files are
//! `skins/whimbrel.json`, `skins/high-contrast.json` and `skins/hypermodern.json`.

use skins::elements::{Actions, Ask, ElementChoices, Labels, NextMeeting, Timeline};
use skins::format::Archetype;
use skins::palette::{default_palette, load_palettes};
use skins::repair::{archetype_pairs, pair_failures, redesign_pairs, repair_for};
use skins::{load_bundled, parse_skin, whimbrel_default, Palette, Skin};
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bundled_palettes() -> Vec<Palette> {
    let fallback = default_palette();
    let mut all = load_palettes(&repo().join("themes"), &fallback);
    all.extend(load_palettes(&repo().join("skins/palettes"), &fallback).into_iter().skip(1));
    all
}

fn skin(id: &str) -> Skin {
    load_bundled(&repo().join("skins")).into_iter().find(|s| s.id == id).unwrap_or_else(|| panic!("{id} is bundled"))
}

fn choices(s: &Skin) -> ElementChoices {
    ElementChoices::from_options(&s.today.options, s.today.archetype)
}

fn expected(id: &str) -> ElementChoices {
    match id {
        "whimbrel" => ElementChoices {
            next_meeting: NextMeeting::Infobox,
            timeline: Timeline::Line,
            labels: Labels::SmallCaps,
            ask: Ask::Field,
            actions: Actions::Buttons,
        },
        "high-contrast" => ElementChoices {
            next_meeting: NextMeeting::Infobox,
            timeline: Timeline::Ruled,
            labels: Labels::Plain,
            ask: Ask::Field,
            actions: Actions::Buttons,
        },
        "hypermodern" => ElementChoices {
            next_meeting: NextMeeting::Scoreboard,
            timeline: Timeline::Line,
            labels: Labels::Plain,
            ask: Ask::Prompt,
            actions: Actions::Buttons,
        },
        _ => unreachable!(),
    }
}

const THREE: [&str; 3] = ["whimbrel", "high-contrast", "hypermodern"];

#[test]
fn the_three_skins_resolve_to_the_elements_in_the_plan_and_differ_from_the_default() {
    for id in THREE {
        let s = skin(id);
        assert_eq!(choices(&s), expected(id), "{id}");
        assert_ne!(choices(&s), ElementChoices::default(), "{id} still looks like the default");
        // All five keys are written out, as thread.json does.
        let keys: Vec<&str> = s.today.options.keys().map(String::as_str).collect();
        assert_eq!(keys, ["actions", "ask", "labels", "nextMeeting", "timeline"], "{id}");
    }
}

#[test]
fn the_archetype_stays_so_an_older_app_draws_its_old_layout() {
    assert_eq!(skin("whimbrel").today.archetype, Archetype::Standard);
    assert_eq!(skin("high-contrast").today.archetype, Archetype::Standard);
    assert_eq!(skin("hypermodern").today.archetype, Archetype::Glass);
}

#[test]
fn the_three_skins_load_with_no_repair() {
    for id in THREE {
        let bytes = std::fs::read(repo().join(format!("skins/{id}.json"))).unwrap();
        let (_, repairs) = parse_skin(&bytes, &whimbrel_default()).expect("parses");
        assert!(repairs.is_empty(), "{id}: {repairs:?}");
    }
}

#[test]
fn redesign_pairs_pass_on_every_palette_of_the_three_as_shipped() {
    let palettes = bundled_palettes();
    for id in THREE {
        let s = skin(id);
        let pairs = redesign_pairs(&[choices(&s)]);
        assert!(!s.palettes.is_empty(), "{id}");
        for pid in &s.palettes {
            let p = palettes.iter().find(|p| &p.id == pid).unwrap_or_else(|| panic!("{id}: palette {pid} is bundled"));
            for (mode, v) in [("light", &p.light), ("dark", &p.dark)] {
                let failures = pair_failures(v, &pairs);
                assert!(failures.is_empty(), "{id} on {pid} {mode}: {failures:?}");
            }
        }
    }
}

#[test]
fn redesign_pairs_pass_on_every_palette_of_the_three_after_repair_and_repair_changes_nothing() {
    let palettes = bundled_palettes();
    for id in THREE {
        let s = skin(id);
        let c = [choices(&s)];
        for pid in &s.palettes {
            let p = palettes.iter().find(|p| &p.id == pid).unwrap();
            let (fixed, changed) = repair_for(p, &archetype_pairs(Archetype::Standard), Some(&c));
            assert!(changed.is_empty(), "{id} on {pid} needed repair: {changed:?}");
            let pairs = redesign_pairs(&c);
            for v in [&fixed.light, &fixed.dark] {
                assert!(pair_failures(v, &pairs).is_empty(), "{id} on {pid}");
            }
        }
    }
}

#[test]
fn the_thread_skin_is_unchanged() {
    let s = skin("thread");
    assert_eq!(choices(&s), ElementChoices::default());
    let options: Vec<(&str, &str)> = s.today.options.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(options, [("actions", "buttons"), ("ask", "field"), ("labels", "plain"), ("nextMeeting", "card"), ("timeline", "line")]);
    assert_eq!(s.today.archetype, Archetype::Standard);
}

/// A copy of each skin as an older Whimbrel wrote it (empty options, then no `today.options` at all, then no
/// `today` at all), which sync can still hand to a newer app, loads without a repair and draws its old Home.
#[test]
fn an_old_synced_copy_of_each_still_loads_and_draws_its_old_home() {
    for id in THREE {
        let current: serde_json::Value = serde_json::from_slice(&std::fs::read(repo().join(format!("skins/{id}.json"))).unwrap()).unwrap();
        let archetype = current["today"]["archetype"].as_str().unwrap().to_string();
        let old_today: [Option<serde_json::Value>; 3] = [
            Some(serde_json::json!({ "archetype": archetype, "options": {} })),
            Some(serde_json::json!({ "archetype": archetype })),
            None,
        ];
        for today in old_today {
            let mut old = current.clone();
            match &today {
                Some(t) => old["today"] = t.clone(),
                None => {
                    old.as_object_mut().unwrap().remove("today");
                }
            }
            let (s, repairs) = parse_skin(&serde_json::to_vec(&old).unwrap(), &whimbrel_default()).unwrap_or_else(|| panic!("{id} loads"));
            assert!(repairs.is_empty(), "{id}: {repairs:?}");
            assert_eq!(s.id, id);
            assert!(s.today.options.is_empty(), "{id}: an old copy carries no element keys");
            let want = if today.is_some() {
                ElementChoices::from_archetype(s.today.archetype)
            } else {
                ElementChoices::from_archetype(whimbrel_default().today.archetype)
            };
            assert_eq!(choices(&s), want, "{id} {today:?}");
        }
    }
}

/// The Windows infobox draws its "Next, in N min" label as `textPrimary` on `selection` (system.css
/// `.w-next-head`; the e2e matrix caught `accentPrimary` there at 4.19:1 on Default). That pair is a required
/// one, so it must reach 4.5:1 on every bundled palette. The Mac draws the label as `textSecondary` on the
/// page, which the infobox element pairs cover.
#[test]
fn the_infobox_label_pairs_reach_4_5_on_every_bundled_palette() {
    use skins::palette::Token;
    use skins::repair::Pair;
    let pairs = [
        Pair::plain(Token::TextPrimary, Token::Selection),
        Pair::plain(Token::TextSecondary, Token::BackgroundPage),
    ];
    for p in bundled_palettes() {
        for (mode, v) in [("light", &p.light), ("dark", &p.dark)] {
            let failures = pair_failures(v, &pairs);
            assert!(failures.is_empty(), "{} {mode}: {failures:?}", p.id);
        }
    }
}
