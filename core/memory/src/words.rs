//! Folded words and word fingerprints (X6 client controls). One folding for every whole-word
//! client-name match (`llm::guard::ExclusionSet`) and for the fingerprints a clean stamp keeps of
//! words that have no stored row (a question, a window's text), so a fingerprint lookup answers
//! "does it name this client" exactly as the text match would.

/// The folded words of `text`, in order: compatibility-decomposed (NFKD) with the combining
/// marks dropped, lowercased, and ß spelled ss. So "José" (precomposed or not), "Jose" and
/// "JOSÉ" are one word, as are "Strauß" and "STRAUSS". Terms and text fold alike.
pub fn folded_words(text: &str) -> Vec<String> {
    let folded: String = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd()
        .normalize(text)
        .chars()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace('ß', "ss");
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether `text` contains any of `terms` as whole folded words ([`folded_words`]), the same match
/// `llm::guard::ExclusionSet` makes. Blank terms match nothing.
pub fn names_any(text: &str, terms: &[String]) -> bool {
    let words = folded_words(text);
    terms.iter().any(|t| {
        let t = folded_words(t);
        !t.is_empty() && words.windows(t.len()).any(|w| w == t.as_slice())
    })
}

/// The byte range in `text` of the first whole-word match of any of `terms` (the same folding and
/// whole-word rule as [`names_any`]), or `None`. Display only: used to show the sentence that names a
/// client on the user's own screen. Each original character is folded alone, so a word's range is the
/// original characters that made it; a mark that folds away stays inside its word.
pub fn first_name_span(text: &str, terms: &[String]) -> Option<(usize, usize)> {
    let terms: Vec<Vec<String>> = terms.iter().map(|t| folded_words(t)).filter(|t| !t.is_empty()).collect();
    if terms.is_empty() {
        return None;
    }
    // (folded word, start byte, end byte) of each word of `text`. ASCII characters (nearly all text) fold
    // without allocating; anything else is folded alone as `folded_words` would.
    let mut words: Vec<(String, usize, usize)> = Vec::new();
    let mut cur: Option<(String, usize, usize)> = None;
    let mut buf = [0u8; 4];
    let push = |f: char, i: usize, end: usize, cur: &mut Option<(String, usize, usize)>, words: &mut Vec<(String, usize, usize)>| {
        if f.is_alphanumeric() {
            match cur.as_mut() {
                Some(c) => {
                    c.0.push(f);
                    c.2 = end;
                }
                None => *cur = Some((f.to_string(), i, end)),
            }
        } else if let Some(c) = cur.take() {
            words.push(c);
        }
    };
    for (i, ch) in text.char_indices() {
        let end = i + ch.len_utf8();
        if ch.is_ascii() {
            push(ch.to_ascii_lowercase(), i, end, &mut cur, &mut words);
            continue;
        }
        let folded: String = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd()
            .normalize(ch.encode_utf8(&mut buf))
            .chars()
            .filter(|c| !is_combining_mark(*c))
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .replace('ß', "ss");
        if folded.is_empty() {
            if let Some(c) = cur.as_mut() {
                c.2 = end;
            }
            continue;
        }
        for f in folded.chars() {
            push(f, i, end, &mut cur, &mut words);
        }
    }
    if let Some(c) = cur.take() {
        words.push(c);
    }
    let mut best: Option<(usize, usize)> = None;
    for t in &terms {
        for w in words.windows(t.len()) {
            if w.iter().map(|x| &x.0).eq(t.iter()) {
                let span = (w[0].1, w[t.len() - 1].2);
                if best.is_none_or(|b| span.0 < b.0) {
                    best = Some(span);
                }
                break;
            }
        }
    }
    best
}

/// The combining-mark blocks NFKD splits accents into (diacritics for Latin, Greek and
/// Cyrillic, their extensions and supplements, and the half marks).
fn is_combining_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

/// The longest run of words a fingerprint covers. A longer name is looked up by its first
/// this-many words ([`term_fingerprint`]).
pub const MAX_FINGERPRINT_WORDS: usize = 4;

/// The fingerprint of a run of folded words.
pub fn fingerprint(words: &[String]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            h = (h ^ u32::from(b' ')).wrapping_mul(0x0100_0193);
        }
        for b in w.bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    }
    h
}

/// Fingerprints of every run of 1 to [`MAX_FINGERPRINT_WORDS`] folded words of `text`, sorted and
/// deduplicated: enough to check later, by name, whether `text` named a client, without keeping
/// its words. A 32-bit fingerprint can collide; a collision only counts as a match (fail closed).
pub fn word_fingerprints(text: &str) -> Vec<u32> {
    let w = folded_words(text);
    let mut out: Vec<u32> = (1..=MAX_FINGERPRINT_WORDS).flat_map(|n| w.windows(n).map(fingerprint).collect::<Vec<_>>()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The fingerprint a term (already folded into words) is looked up by: all its words, or its
/// first [`MAX_FINGERPRINT_WORDS`] for a longer name. Absent means the term is absent; present
/// counts as found (fail closed).
pub fn term_fingerprint(term: &[String]) -> u32 {
    fingerprint(&term[..term.len().min(MAX_FINGERPRINT_WORDS)])
}

/// Whether the text `fingerprints` (sorted, [`word_fingerprints`]) were taken from names any of
/// `terms` (each folded into words).
pub fn fingerprints_name_any(fingerprints: &[u32], terms: &[Vec<String>]) -> bool {
    terms.iter().filter(|t| !t.is_empty()).any(|t| fingerprints.binary_search(&term_fingerprint(t)).is_ok())
}

/// The words of JSON text for name matching (X6 fix round 6, J1-J4): every key and string value,
/// walked through nested objects and arrays, and through string values that are JSON themselves
/// (a sync payload carrying a row's JSON), joined with newlines. Escapes such as `\n` are decoded
/// by the parser, so a name right after one is a whole word. `None` when `raw` is not JSON.
/// Numbers, booleans and nulls are emitted too, as their JSON text. Inherent limit: a name split
/// inside one word (`"Ac" "me"`, or text glued to it on both sides) cannot match, here or anywhere
/// whole-word matching is used.
pub fn json_name_text(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let mut out = Vec::new();
    walk_json(&v, &mut out, 0);
    Some(out.join("\n"))
}

fn walk_json(v: &serde_json::Value, out: &mut Vec<String>, depth: usize) {
    match v {
        serde_json::Value::String(s) => {
            let trimmed = s.trim_start();
            let nested = (trimmed.starts_with('{') || trimmed.starts_with('['))
                && depth < 8
                && serde_json::from_str::<serde_json::Value>(s).is_ok_and(|n| {
                    walk_json(&n, out, depth + 1);
                    true
                });
            if !nested {
                out.push(s.clone());
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| walk_json(x, out, depth + 1)),
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                out.push(k.clone());
                walk_json(x, out, depth + 1);
            }
        }
        other => out.push(other.to_string()),
    }
}

/// The text to name-match for a column that may hold JSON (or text mixed with JSON, or legacy
/// non-JSON text): the decoded JSON when it parses; otherwise the raw text plus a copy with JSON
/// string escapes (`\n \t \r \b \f \" \\ \/ \uXXXX`) decoded, repeatedly for text escaped more than
/// once. Over-matching is the safe direction for every caller (fix round 6, J1-J2).
pub fn name_text(raw: &str) -> String {
    if let Some(text) = json_name_text(raw) {
        return text;
    }
    let mut out = raw.to_string();
    let mut cur = raw.to_string();
    for _ in 0..4 {
        let next = unescape_json(&cur);
        if next == cur {
            break;
        }
        out.push('\n');
        out.push_str(&next);
        cur = next;
    }
    out
}

/// One pass of JSON string-escape decoding; anything that is not a valid escape stays as it is.
fn unescape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('n') | Some('t') | Some('r') | Some('b') | Some('f') => {
                chars.next();
                out.push(' ');
            }
            Some('"') | Some('\\') | Some('/') => {
                out.push(chars.next().unwrap_or(' '));
            }
            Some('u') => {
                let unit = |ahead: &std::iter::Peekable<std::str::Chars<'_>>, skip: usize| -> Option<u32> {
                    let hex: String = ahead.clone().skip(skip).take(4).collect();
                    (hex.len() == 4).then(|| u32::from_str_radix(&hex, 16).ok()).flatten()
                };
                let decoded = match unit(&chars, 1) {
                    // A high surrogate pairs with a following `\uDC00`-`\uDFFF` (fix round 7, F3).
                    Some(hi @ 0xD800..=0xDBFF) => {
                        let rest: String = chars.clone().skip(5).take(2).collect();
                        match (rest == "\\u").then(|| unit(&chars, 7)).flatten() {
                            Some(lo @ 0xDC00..=0xDFFF) => {
                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)).map(|c| (c, 11))
                            }
                            _ => None,
                        }
                    }
                    Some(u) => char::from_u32(u).map(|c| (c, 5)),
                    None => None,
                };
                match decoded {
                    Some((ch, len)) => {
                        for _ in 0..len {
                            chars.next();
                        }
                        out.push(ch);
                    }
                    None => out.push('\\'),
                }
            }
            _ => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_any_matches_whole_folded_words_only() {
        let terms = vec!["Zorblax Labs".to_string(), "Quux".to_string(), "  ".to_string()];
        assert!(names_any("call with ZORBLAX labs today", &terms));
        assert!(names_any("quux: signed", &terms));
        assert!(!names_any("Zorblax only", &terms));
        assert!(!names_any("quuxco", &terms));
        assert!(!names_any("", &terms));
    }

    #[test]
    fn first_name_span_is_the_original_byte_range_of_the_whole_word_match() {
        let t = |s: &str| vec![s.to_string()];
        let text = "Café with JOSÉ García, then Jose again";
        let (a, b) = first_name_span(text, &t("jose garcia")).unwrap();
        assert_eq!(&text[a..b], "JOSÉ García");
        let (a, b) = first_name_span("x Acme-related", &t("Acme")).unwrap();
        assert_eq!(&"x Acme-related"[a..b], "Acme");
        assert_eq!(first_name_span("Acmeville", &t("Acme")), None);
        assert_eq!(first_name_span("Strauß", &t("strauss")).map(|(a, b)| (a, b)), Some((0, "Strauß".len())));
        assert_eq!(first_name_span("anything", &t("  ")), None);
        // Decomposed accent stays inside its word.
        let d = "Jose\u{301} Acme";
        let (a, b) = first_name_span(d, &t("jose")).unwrap();
        assert_eq!(&d[a..b], "Jose\u{301}");
    }

    #[test]
    fn first_name_span_ascii_fast_path_folds_like_folded_words() {
        let text = "Re: ACME-Corp's Q3 (acme corp) - Acme\tCorp";
        let (a, b) = first_name_span(text, &["Acme Corp".to_string()]).unwrap();
        assert_eq!(&text[a..b], "ACME-Corp");
        assert_eq!(first_name_span("acmecorp", &["acme corp".to_string()]), None);
        let mixed = "Zoë and ACME Corp";
        let (a, b) = first_name_span(mixed, &["acme corp".to_string()]).unwrap();
        assert_eq!(&mixed[a..b], "ACME Corp");
    }

    /// Fix round 6: a name right after an escaped newline is a whole word once decoded, in JSON,
    /// in JSON nested in a JSON string, and in text that is not JSON at all.
    #[test]
    fn names_behind_json_escapes_are_whole_words() {
        let has = |text: &str| folded_words(&name_text(text)).iter().any(|w| w == "acme");
        assert!(!folded_words(r#"{"body":"Week.\n\nAcme: signed."}"#).iter().any(|w| w == "acme"), "raw folding misses it");
        assert!(has(r#"{"body":"Week.\n\nAcme: signed."}"#));
        let nested = serde_json::json!({"payload": serde_json::json!({"output_json": r#"{"open_loops":["Ping\nAcme"]}"#}).to_string()}).to_string();
        assert!(has(&nested), "{nested}");
        assert!(has(r#"Summary text {"x":"a\n\tAcme"} more"#), "mixed text and JSON");
        // Fix round 7, F4: real `\u` escapes (the round-6 line had none), in text that is not JSON.
        // Built with an explicit backslash so no tool can turn the escapes into characters.
        let bs = '\\';
        let escaped = format!("plain {bs}u0041cme text");
        assert!(escaped.contains("u0041") && !escaped.contains("Acme"), "the assertion holds an escape: {escaped}");
        assert!(has(&escaped), "\\u escape");
        let jose = format!("note: Jos{bs}u00e9 called");
        assert!(folded_words(&name_text(&jose)).iter().any(|w| w == "jose"), "\\u00e9 decodes to an accented e, folded to e");
        // Fix round 7, F3: a surrogate pair (an emoji) right before the name decodes as one char.
        let emoji = format!("note: {bs}ud83d{bs}ude00Acme called");
        assert!(has(&emoji), "{:?}", name_text(&emoji));
        assert!(name_text(&emoji).contains(char::from_u32(0x1F600).unwrap()));
        assert!(!has(r#"{"body":"Week.\n\nGlobex: signed."}"#), "control");
    }
}
