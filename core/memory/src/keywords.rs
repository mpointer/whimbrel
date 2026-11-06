//! Shared matcher for the per-language keyword tables (`KEYWORDS_*` consts in llm, api).
//! A table is `&[(base_lang, &[keyword])]`; matching uses the union of every row, because
//! the question's language, not the UI setting, decides which words appear.

/// True when `text` contains any keyword in any row. Case-insensitive; curly apostrophes
/// match straight ones. There is no accent folding: list accented forms literally.
/// The `en` row is a plain substring match (today's behaviour: "todays", "summarised" hit);
/// every other row needs whole-word boundaries so short words ("ayer", "hier", "hoy") do not
/// fire inside English words ("layer", "hierarchy").
pub fn any_keyword(text: &str, table: &[(&str, &[&str])]) -> bool {
    let t = text.to_lowercase().replace('\u{2019}', "'");
    table.iter().any(|(lang, kws)| {
        kws.iter().any(|kw| {
            if *lang == "en" {
                t.contains(kw)
            } else {
                contains_word(&t, kw)
            }
        })
    })
}

fn contains_word(t: &str, kw: &str) -> bool {
    t.match_indices(kw).any(|(i, m)| {
        let before = t[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = t[i + m.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: &[(&str, &[&str])] = &[
        ("en", &["today"]),
        ("es", &["ayer"]),
        ("fr", &["aujourd'hui"]),
    ];

    #[test]
    fn en_is_substring_others_are_whole_word() {
        assert!(any_keyword("Today's plan", T));
        assert!(any_keyword("¿Qué pasó AYER?", T));
        assert!(!any_keyword("the player layer", T));
        assert!(any_keyword("Aujourd’hui ?", T));
    }
}
