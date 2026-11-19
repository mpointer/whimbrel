//! The five Home element choices a skin makes in `today.options` (docs/ui-redesign/skins.md,
//! "Converting old skins"). A part with no key takes its value from the skin's old `archetype`; a key
//! with a value this list does not know takes the default for that part (the Standard row).

use crate::format::Archetype;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextMeeting {
    Card,
    Scoreboard,
    Infobox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeline {
    Line,
    Ruled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Labels {
    Plain,
    Bracketed,
    SmallCaps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Field,
    Prompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actions {
    Buttons,
    Links,
}

/// The five choices, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementChoices {
    pub next_meeting: NextMeeting,
    pub timeline: Timeline,
    pub labels: Labels,
    pub ask: Ask,
    pub actions: Actions,
}

impl Default for ElementChoices {
    fn default() -> Self {
        ElementChoices {
            next_meeting: NextMeeting::Card,
            timeline: Timeline::Line,
            labels: Labels::Plain,
            ask: Ask::Field,
            actions: Actions::Buttons,
        }
    }
}

/// Every key and its allowed values, in the order the spec lists them. The catalogue is built from this.
pub const ELEMENT_KEYS: [(&str, &[&str]); 5] = [
    ("nextMeeting", &["card", "scoreboard", "infobox"]),
    ("timeline", &["line", "ruled"]),
    ("labels", &["plain", "bracketed", "smallCaps"]),
    ("ask", &["field", "prompt"]),
    ("actions", &["buttons", "links"]),
];

impl ElementChoices {
    /// What an old skin with no element keys draws, by its `archetype`.
    pub fn from_archetype(archetype: Archetype) -> ElementChoices {
        let d = ElementChoices::default();
        match archetype {
            Archetype::Standard | Archetype::Glass => d,
            Archetype::Field => ElementChoices { next_meeting: NextMeeting::Scoreboard, ..d },
            Archetype::Terminal => ElementChoices { labels: Labels::Bracketed, ask: Ask::Prompt, ..d },
            Archetype::Almanac => ElementChoices { timeline: Timeline::Ruled, labels: Labels::SmallCaps, ..d },
            Archetype::Reference => ElementChoices {
                next_meeting: NextMeeting::Infobox,
                timeline: Timeline::Ruled,
                actions: Actions::Links,
                ..d
            },
        }
    }

    /// Reads the five keys from `today.options`. A missing key takes the archetype's value for that
    /// part; a present key with an unknown value takes the default for that part; other keys are ignored.
    pub fn from_options(options: &BTreeMap<String, String>, archetype: Archetype) -> ElementChoices {
        let base = ElementChoices::from_archetype(archetype);
        let d = ElementChoices::default();
        ElementChoices {
            next_meeting: match options.get("nextMeeting").map(String::as_str) {
                None => base.next_meeting,
                Some("card") => NextMeeting::Card,
                Some("scoreboard") => NextMeeting::Scoreboard,
                Some("infobox") => NextMeeting::Infobox,
                Some(_) => d.next_meeting,
            },
            timeline: match options.get("timeline").map(String::as_str) {
                None => base.timeline,
                Some("line") => Timeline::Line,
                Some("ruled") => Timeline::Ruled,
                Some(_) => d.timeline,
            },
            labels: match options.get("labels").map(String::as_str) {
                None => base.labels,
                Some("plain") => Labels::Plain,
                Some("bracketed") => Labels::Bracketed,
                Some("smallCaps") => Labels::SmallCaps,
                Some(_) => d.labels,
            },
            ask: match options.get("ask").map(String::as_str) {
                None => base.ask,
                Some("field") => Ask::Field,
                Some("prompt") => Ask::Prompt,
                Some(_) => d.ask,
            },
            actions: match options.get("actions").map(String::as_str) {
                None => base.actions,
                Some("buttons") => Actions::Buttons,
                Some("links") => Actions::Links,
                Some(_) => d.actions,
            },
        }
    }

    /// All the variants at once, for a repair that must cover any skin.
    pub fn every_variant() -> Vec<ElementChoices> {
        let d = ElementChoices::default();
        vec![
            d,
            ElementChoices { next_meeting: NextMeeting::Scoreboard, ..d },
            ElementChoices { next_meeting: NextMeeting::Infobox, ..d },
            ElementChoices { timeline: Timeline::Ruled, ..d },
            ElementChoices { labels: Labels::Bracketed, ..d },
            ElementChoices { labels: Labels::SmallCaps, ..d },
            ElementChoices { ask: Ask::Prompt, ..d },
            ElementChoices { actions: Actions::Links, ..d },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn every_archetype_row_matches_the_table() {
        let row = |a: Archetype| {
            let c = ElementChoices::from_options(&BTreeMap::new(), a);
            (c.next_meeting, c.timeline, c.labels, c.ask, c.actions)
        };
        use Actions::Buttons as Bt;
        use NextMeeting as N;
        use Timeline as T;
        use Labels as L;
        use Ask as A;
        assert_eq!(row(Archetype::Standard), (N::Card, T::Line, L::Plain, A::Field, Bt));
        assert_eq!(row(Archetype::Field), (N::Scoreboard, T::Line, L::Plain, A::Field, Bt));
        assert_eq!(row(Archetype::Terminal), (N::Card, T::Line, L::Bracketed, A::Prompt, Bt));
        assert_eq!(row(Archetype::Almanac), (N::Card, T::Ruled, L::SmallCaps, A::Field, Bt));
        assert_eq!(
            row(Archetype::Reference),
            (N::Infobox, T::Ruled, L::Plain, A::Field, Actions::Links)
        );
        assert_eq!(row(Archetype::Glass), (N::Card, T::Line, L::Plain, A::Field, Bt));
    }

    #[test]
    fn options_win_over_the_archetype() {
        let o = opts(&[
            ("nextMeeting", "scoreboard"),
            ("timeline", "ruled"),
            ("labels", "smallCaps"),
            ("ask", "prompt"),
            ("actions", "links"),
        ]);
        let c = ElementChoices::from_options(&o, Archetype::Standard);
        assert_eq!(
            c,
            ElementChoices {
                next_meeting: NextMeeting::Scoreboard,
                timeline: Timeline::Ruled,
                labels: Labels::SmallCaps,
                ask: Ask::Prompt,
                actions: Actions::Links,
            }
        );
    }

    #[test]
    fn unknown_values_take_the_default_and_unknown_keys_are_ignored() {
        let o = opts(&[("nextMeeting", "hologram"), ("labels", "Bracketed"), ("colour", "red"), ("ask", "")]);
        // The archetype would say infobox, bracketed-free reference; unknown values still mean the default.
        let c = ElementChoices::from_options(&o, Archetype::Reference);
        assert_eq!(c.next_meeting, NextMeeting::Card);
        assert_eq!(c.labels, Labels::Plain);
        assert_eq!(c.ask, Ask::Field);
        // Parts with no key still come from the archetype.
        assert_eq!(c.timeline, Timeline::Ruled);
        assert_eq!(c.actions, Actions::Links);
    }

    #[test]
    fn partial_options_fill_the_rest_from_the_archetype() {
        let c = ElementChoices::from_options(&opts(&[("labels", "plain")]), Archetype::Terminal);
        assert_eq!(c.labels, Labels::Plain);
        assert_eq!(c.ask, Ask::Prompt);
    }

    #[test]
    fn the_key_table_lists_every_value_the_reader_accepts() {
        for (key, values) in ELEMENT_KEYS {
            for v in values {
                let c = ElementChoices::from_options(&opts(&[(key, v)]), Archetype::Standard);
                let again = ElementChoices::from_options(&opts(&[(key, v)]), Archetype::Reference);
                // An accepted value does not depend on the archetype.
                let part = |c: ElementChoices| match key {
                    "nextMeeting" => format!("{:?}", c.next_meeting),
                    "timeline" => format!("{:?}", c.timeline),
                    "labels" => format!("{:?}", c.labels),
                    "ask" => format!("{:?}", c.ask),
                    _ => format!("{:?}", c.actions),
                };
                assert_eq!(part(c), part(again), "{key}={v}");
            }
        }
    }
}
