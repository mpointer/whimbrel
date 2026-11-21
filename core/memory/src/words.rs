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
    terms.iter().flat_map(|t| term_words(t)).any(|t| words.windows(t.len()).any(|w| w == t.as_slice()))
}

/// `text` without its invisible characters ([`invisible`]).
pub fn without_invisibles(text: &str) -> String {
    text.chars().filter(|&c| !invisible(c)).collect()
}

/// The folded words a term is matched by: as written, and with invisible characters dropped
/// ([`invisible`]), so an alias pasted with a soft hyphen or a direction mark (`Ac\u{00AD}me`)
/// matches its plain spelling and the plain alias matches the text with them dropped. Empty
/// variants are left out.
pub fn term_words(term: &str) -> Vec<Vec<String>> {
    let plain: String = term.chars().filter(|&c| !invisible(c)).collect();
    let mut out = vec![folded_words(term)];
    if plain != term {
        let w = folded_words(&plain);
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out.retain(|w| !w.is_empty());
    out
}

/// The byte range in `text` of the first whole-word match of any of `terms` (the same folding and
/// whole-word rule as [`names_any`]), or `None`. Display only: used to show the sentence that names a
/// client on the user's own screen. Each original character is folded alone, so a word's range is the
/// original characters that made it; a mark that folds away stays inside its word.
pub fn first_name_span(text: &str, terms: &[String]) -> Option<(usize, usize)> {
    let terms: Vec<Vec<String>> = terms.iter().map(|t| folded_words(&without_invisibles(t))).filter(|t| !t.is_empty()).collect();
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

/// How many decoding passes [`readings`] makes. Text still decodable after this many is
/// treated as matching every term ([`Readings::undecidable`]): fail closed.
pub const MAX_DECODE_PASSES: usize = 4;

/// How many full decoding passes the matches that delete or file something make
/// (`llm::guard::ExclusionSet::names_in`), with `4 ×` this `+ 2` single steps per chain
/// ([`visit_readings`]): far past [`MAX_DECODE_PASSES`], within [`NAMING_READING_BUDGET`]. Each
/// pass and step is linear, and they stop at the first that changes nothing.
pub const MAX_NAMING_DECODE_PASSES: usize = 64;

/// The fingerprint that stands for "this text could not be fully decoded": present in a text's
/// [`word_fingerprints`], it counts as naming every term ([`fingerprints_name_any`],
/// `llm::guard::ExclusionSet::find_in_fingerprints`), as [`Readings::undecidable`] does for the
/// text match. A real run of words can share it; that only counts as a match (fail closed).
pub const UNDECIDABLE_FINGERPRINT: u32 = u32::MAX;

/// Every form of a text a whole-word client match reads, from one function ([`visit_readings`]),
/// so the text match (`llm::guard::ExclusionSet::find_in`) and the fingerprints a clean stamp keeps
/// ([`word_fingerprints`]) see the same words.
pub struct Readings {
    /// Every reading [`visit_readings`] visits with [`MAX_DECODE_PASSES`].
    pub texts: Vec<String>,
    /// Still decodable at the bound: nothing about it can be ruled out.
    pub undecidable: bool,
}

/// The readings of `text` ([`Readings`]) with [`MAX_DECODE_PASSES`]. Linear in the text.
pub fn readings(text: &str) -> Readings {
    let mut texts = Vec::new();
    let (_, undecidable) = visit_readings(text, MAX_DECODE_PASSES, HOSTED_READING_BUDGET, &mut |t| {
        texts.push(t.to_string());
        false
    });
    Readings { texts, undecidable }
}

/// How much decoding work [`readings`] (the hosted check and the fingerprints) may do: the bytes
/// of every step it decodes or visits, at most this many times the text's length (plus
/// [`READING_BUDGET_FLOOR`]). Text still decoding when it runs out is undecidable (fail closed).
pub const HOSTED_READING_BUDGET: usize = 8;

/// The same budget for what deletes or files (`llm::guard::ExclusionSet::names_in`): one row can
/// never take more than this many linear passes' worth of work; a row still decoding when it runs
/// out reads as not naming the client there (known limit; the hosted check still treats it as the
/// client's).
pub const NAMING_READING_BUDGET: usize = 6;

/// Added to every budget, so short texts can always be decoded through all their levels.
pub const READING_BUDGET_FLOOR: usize = 64 * 1024;

/// Visits every reading of `text`, each followed by its plus-tag reading ([`without_plus_tags`])
/// when it has one, until `visit` returns true. Each distinct reading is visited once. The
/// readings:
/// - the text as written, and with only its invisible characters dropped;
/// - up to `passes` full decoding passes ([`decode_once`]: percent, references, percent again);
/// - four single-operation chains, stepped in turn: two that repeat one operation (percent
///   escapes, or references) until it changes nothing and then switch to the other, and two that
///   switch after every step, one starting with each operation. Every step is a reading, with
///   invisible characters dropped. So any order of one or two decodings a reader might apply is a
///   prefix of some chain: a name glued to a letter decoded later (`Ac%6De%2541`,
///   `Ac%26%23109%3Be%2541`) is still read alone.
///
/// Each chain is bounded at `4 × passes + 2` steps, and all of it at `budget × len + floor` bytes
/// of decoded or visited text. Returns (stopped by `visit`, undecidable): still decoding at a bound
/// makes the text undecidable. Keeps only each chain's current text, so memory stays linear.
pub fn visit_readings(text: &str, passes: usize, budget: usize, visit: &mut dyn FnMut(&str) -> bool) -> (bool, bool) {
    let limit = budget.saturating_mul(text.len()).saturating_add(READING_BUDGET_FLOOR);
    let mut spent = 0usize;
    let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let hash = |t: &str| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    };
    // Visits `t` (and its plus-tag reading) unless already visited; true when `visit` stopped.
    let mut seen = |t: &str, spent: &mut usize, visit: &mut dyn FnMut(&str) -> bool| -> bool {
        *spent += t.len();
        if !visited.insert(hash(t)) {
            return false;
        }
        visit(t) || without_plus_tags(t).is_some_and(|u| {
            *spent += u.len();
            visited.insert(hash(&u)) && visit(&u)
        })
    };
    if seen(text, &mut spent, visit) {
        return (true, false);
    }
    if text.chars().any(invisible) {
        let plain: String = text.chars().filter(|&c| !invisible(c)).collect();
        if seen(&plain, &mut spent, visit) {
            return (true, false);
        }
    }
    let mut undecidable = false;
    let mut cur = text.to_string();
    let mut decided = false;
    for _ in 0..passes {
        if spent > limit {
            break;
        }
        match decode_once(&cur) {
            Some(next) => {
                if seen(&next, &mut spent, visit) {
                    return (true, false);
                }
                cur = next;
            }
            None => {
                decided = true;
                break;
            }
        }
    }
    if !decided && decode_once(&cur).is_some() {
        undecidable = true;
    }
    if text.contains(['%', '&']) {
        // (current text, percent next, switches after every step, idle steps, settled)
        let mut chains: Vec<(String, bool, bool, u8, bool)> =
            [(true, false), (false, false), (true, true), (false, true)].iter().map(|&(p, strict)| (text.to_string(), p, strict, 0, false)).collect();
        for _ in 0..4 * passes + 2 {
            let mut active = false;
            for chain in chains.iter_mut().filter(|c| !c.4) {
                if spent > limit {
                    return (false, true);
                }
                active = true;
                let (cur, percent, strict, idle, settled) = chain;
                let mut changed = false;
                let bytes = if *percent { percent_bytes(cur.as_bytes(), &mut changed) } else { entity_bytes(cur.as_bytes(), &mut changed) };
                spent += bytes.len();
                let next: String = String::from_utf8_lossy(&bytes).chars().filter(|&c| !invisible(c)).collect();
                if next != *cur {
                    if seen(&next, &mut spent, visit) {
                        return (true, false);
                    }
                    *cur = next;
                    *idle = 0;
                    if *strict {
                        *percent = !*percent;
                    }
                } else {
                    *percent = !*percent;
                    *idle += 1;
                    *settled = *idle == 2;
                }
            }
            if !active {
                break;
            }
        }
        undecidable |= chains.iter().any(|c| !c.4);
    }
    (false, undecidable || spent > limit)
}

/// Characters that render as nothing and can sit inside a name or an address: the Unicode
/// Default_Ignorable_Code_Point set (soft hyphen, combining grapheme joiner, Arabic letter mark,
/// Hangul fillers, Khmer and Mongolian format characters, zero-width space, joiners and direction
/// marks, bidi embeddings, overrides and isolates, invisible operators, variation selectors, the
/// byte-order mark, shorthand format controls, musical format characters, tag characters).
/// Dropping them only adds a reading ([`readings`] keeps the text as written), so text with an
/// emoji joiner or a variation selector still matches as it did.
pub fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// One decoding pass over `text`, or `None` when nothing in it decodes. As bytes: percent escapes
/// (so `%40` is `@` and `%EF%BC%A0` is a fullwidth `＠`), then HTML character references (numeric,
/// any number of leading zeros, `;` optional, 128-159 read as Windows-1252 as browsers do; the
/// named ones in [`named_entity`], `;` optional), then percent escapes again (so a UTF-8 sequence
/// split across the two, `%C3&#x25;A9`, is one `é`). Then, as characters, bytes that are not UTF-8
/// become U+FFFD and invisible characters ([`invisible`]) are dropped. Never panics; linear.
pub fn decode_once(text: &str) -> Option<String> {
    if !text.contains(['%', '&']) && !text.chars().any(invisible) {
        return None;
    }
    let mut changed = false;
    let bytes = percent_bytes(text.as_bytes(), &mut changed);
    let bytes = entity_bytes(&bytes, &mut changed);
    let bytes = percent_bytes(&bytes, &mut changed);
    let decoded = String::from_utf8_lossy(&bytes);
    let out: String = decoded
        .chars()
        .filter(|&c| {
            let drop = invisible(c);
            changed |= drop;
            !drop
        })
        .collect();
    changed.then_some(out)
}

/// `bytes` with every `%XX` escape decoded to its byte.
fn percent_bytes(bytes: &[u8], changed: &mut bool) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                *changed = true;
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// `bytes` with every HTML character reference ([`entity_at`]) replaced by its UTF-8.
fn entity_bytes(bytes: &[u8], changed: &mut bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut buf = [0u8; 4];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some((c, next)) = entity_at(bytes, i) {
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                *changed = true;
                i = next;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Windows-1252's characters for 0x80-0x9F, as an HTML reader maps numeric references in that
/// range (`&#138;` is `Š`); `None` where Windows-1252 has none.
fn windows_1252(n: u32) -> Option<char> {
    const TABLE: [u32; 32] = [
        0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, 0, 0x2018,
        0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
    ];
    let v = *TABLE.get(n.checked_sub(0x80)? as usize)?;
    (v != 0).then(|| char::from_u32(v)).flatten()
}

/// The named references decoded: `&commat; &period; &plus;` (the address punctuation), `&amp;
/// &nbsp; &lt; &gt; &quot;`, and the Latin-1 letters in either case (`&eacute;`, `&Eacute;`, `&szlig;`).
fn named_entity(name: &[u8]) -> Option<char> {
    Some(match name {
        b"commat" => '@',
        b"period" => '.',
        b"plus" => '+',
        b"amp" | b"AMP" => '&',
        b"nbsp" => ' ',
        // A soft hyphen: decoded, then dropped as invisible.
        b"shy" => '\u{00AD}',
        b"lt" | b"LT" => '<',
        b"gt" | b"GT" => '>',
        b"quot" | b"QUOT" => '"',
        b"szlig" => 'ß',
        b"aacute" => 'á', b"Aacute" => 'Á', b"agrave" => 'à', b"Agrave" => 'À', b"acirc" => 'â', b"Acirc" => 'Â',
        b"atilde" => 'ã', b"Atilde" => 'Ã', b"auml" => 'ä', b"Auml" => 'Ä', b"aring" => 'å', b"Aring" => 'Å',
        b"aelig" => 'æ', b"AElig" => 'Æ', b"ccedil" => 'ç', b"Ccedil" => 'Ç',
        b"eacute" => 'é', b"Eacute" => 'É', b"egrave" => 'è', b"Egrave" => 'È', b"ecirc" => 'ê', b"Ecirc" => 'Ê',
        b"euml" => 'ë', b"Euml" => 'Ë',
        b"iacute" => 'í', b"Iacute" => 'Í', b"igrave" => 'ì', b"Igrave" => 'Ì', b"icirc" => 'î', b"Icirc" => 'Î',
        b"iuml" => 'ï', b"Iuml" => 'Ï',
        b"ntilde" => 'ñ', b"Ntilde" => 'Ñ',
        b"oacute" => 'ó', b"Oacute" => 'Ó', b"ograve" => 'ò', b"Ograve" => 'Ò', b"ocirc" => 'ô', b"Ocirc" => 'Ô',
        b"otilde" => 'õ', b"Otilde" => 'Õ', b"ouml" => 'ö', b"Ouml" => 'Ö', b"oslash" => 'ø', b"Oslash" => 'Ø',
        b"uacute" => 'ú', b"Uacute" => 'Ú', b"ugrave" => 'ù', b"Ugrave" => 'Ù', b"ucirc" => 'û', b"Ucirc" => 'Û',
        b"uuml" => 'ü', b"Uuml" => 'Ü',
        b"yacute" => 'ý', b"Yacute" => 'Ý', b"yuml" => 'ÿ',
        b"eth" => 'ð', b"ETH" => 'Ð', b"thorn" => 'þ', b"THORN" => 'Þ',
        _ => return None,
    })
}

/// The character the reference starting at `bytes[at]` (an `&`) stands for, and the index after
/// it. Looks at most a few bytes ahead (the digits of a numeric reference after its leading
/// zeros run once; a name at most 6 letters), so a pass stays linear.
fn entity_at(bytes: &[u8], at: usize) -> Option<(char, usize)> {
    let mut i = at + 1;
    if bytes.get(i) == Some(&b'#') {
        i += 1;
        let radix = if matches!(bytes.get(i), Some(b'x' | b'X')) {
            i += 1;
            16
        } else {
            10
        };
        let start = i;
        while i < bytes.len() && bytes[i] == b'0' {
            i += 1;
        }
        let digits_from = i;
        while i < bytes.len() && (bytes[i] as char).is_digit(radix) && i - digits_from <= 7 {
            i += 1;
        }
        if i == start || i - digits_from > 7 {
            return None;
        }
        let n = std::str::from_utf8(&bytes[digits_from..i]).ok()?;
        let n = if n.is_empty() { 0 } else { u32::from_str_radix(n, radix).ok()? };
        let value = windows_1252(n).or_else(|| char::from_u32(n))?;
        let next = if bytes.get(i) == Some(&b';') { i + 1 } else { i };
        return Some((value, next));
    }
    // A name: the longest known one, with or without its `;`, as browsers read the legacy names.
    let end = (i..bytes.len().min(i + 6)).take_while(|&j| bytes[j].is_ascii_alphanumeric()).last()? + 1;
    (i + 1..=end).rev().find_map(|j| {
        let value = named_entity(&bytes[i..j])?;
        Some((value, if bytes.get(j) == Some(&b';') { j + 1 } else { j }))
    })
}

/// A mailbox character: what a plus tag and the local part around it are made of.
fn mailbox_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// `text` with the `+tag` of every plus-addressed mailbox dropped, or `None` when it has none. A
/// tag is a `+` after a mailbox character that runs, over mailbox characters, up to an `@` (or a
/// fullwidth `＠`): `wile+pricing@coyote.example` reads `wile@coyote.example`. Nothing else in
/// the text changes, so a term can only match where the address without its tag is one.
pub fn without_plus_tags(text: &str) -> Option<String> {
    if !text.contains('+') {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '+' && i > 0 && mailbox_char(chars[i - 1]) {
            let mut j = i + 1;
            while j < chars.len() && mailbox_char(chars[j]) {
                j += 1;
            }
            if j > i + 1 && j < chars.len() && matches!(chars[j], '@' | '\u{ff20}') {
                changed = true;
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    changed.then_some(out)
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
///
/// Taken over every [`readings`] of `text` (decoded, plus tags read off), the same forms
/// `llm::guard::ExclusionSet::find_in` matches, so a fingerprint lookup answers as the text match
/// would; text that cannot be fully decoded carries [`UNDECIDABLE_FINGERPRINT`].
pub fn word_fingerprints(text: &str) -> Vec<u32> {
    let r = readings(text);
    let mut out: Vec<u32> = Vec::new();
    for t in &r.texts {
        let w = folded_words(t);
        out.extend((1..=MAX_FINGERPRINT_WORDS).flat_map(|n| w.windows(n).map(fingerprint).collect::<Vec<_>>()));
    }
    if r.undecidable {
        out.push(UNDECIDABLE_FINGERPRINT);
    }
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
    let terms: Vec<&Vec<String>> = terms.iter().filter(|t| !t.is_empty()).collect();
    !terms.is_empty()
        && (fingerprints.binary_search(&UNDECIDABLE_FINGERPRINT).is_ok()
            || terms.iter().any(|t| fingerprints.binary_search(&term_fingerprint(t)).is_ok()))
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

    /// Review 4, M3: the "could not be decoded" marker names every term in a fingerprint lookup,
    /// and nothing when there is no term.
    #[test]
    fn the_undecidable_marker_names_every_term_and_only_when_there_is_one() {
        let deep = format!("R&{}D", "amp;".repeat(MAX_DECODE_PASSES + 1));
        assert!(readings(&deep).undecidable);
        let fps = word_fingerprints(&deep);
        assert!(fps.binary_search(&UNDECIDABLE_FINGERPRINT).is_ok());
        assert!(fingerprints_name_any(&fps, &[folded_words("Acme")]));
        assert!(!fingerprints_name_any(&fps, &[]));
        assert!(!fingerprints_name_any(&word_fingerprints("R&D digest"), &[folded_words("Acme")]));
    }

    /// Review 4, I1: default-ignorable characters are dropped in a reading; text as written stays a
    /// reading, so emoji sequences and Hangul still read as they did.
    #[test]
    fn default_ignorables_are_dropped_and_ordinary_text_is_kept() {
        for c in ['\u{00AD}', '\u{034F}', '\u{061C}', '\u{115F}', '\u{1160}', '\u{17B4}', '\u{180E}', '\u{200E}', '\u{202E}', '\u{2066}', '\u{3164}', '\u{FE0F}', '\u{FEFF}', '\u{FFA0}', '\u{FFF0}', '\u{1BCA0}', '\u{1D173}', '\u{E0001}', '\u{E0069}'] {
            assert!(invisible(c), "{:X}", c as u32);
            assert_eq!(decode_once(&format!("Ac{c}me")), Some("Acme".to_string()), "{:X}", c as u32);
        }
        for c in ['a', '한', '\u{1F468}', '@', ' ', '\u{0301}'] {
            assert!(!invisible(c));
        }
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} family";
        let r = readings(family);
        assert!(!r.undecidable && r.texts.contains(&family.to_string()), "the text as written stays a reading");
        assert_eq!(decode_once("한국어 텍스트"), None, "Hangul text is not touched");
        assert!(names_any("회의 한국어 텍스트", &["한국어".to_string()]));
    }

    /// Review 4, M1 and M2: named references (Latin-1 letters in either case, the address
    /// punctuation, `;` optional), numeric references 128-159 read as Windows-1252, and a UTF-8
    /// sequence split across a percent escape and a reference.
    #[test]
    fn references_decode_as_browsers_read_them() {
        assert_eq!(decode_once("Nestl&eacute; &Eacute;cole &szlig; &Uuml;ber &ntilde; &aring;"), Some("Nestlé École ß Über ñ å".to_string()));
        assert_eq!(decode_once("Nestl&eacute and &amp more"), Some("Nestlé and & more".to_string()));
        assert_eq!(decode_once("a&commatb&periodc&plusd"), Some("a@b.c+d".to_string()));
        assert_eq!(decode_once("&#138;koda &#x8A;koda &#128; &#352;"), Some("Škoda Škoda € Š".to_string()));
        assert_eq!(decode_once("Jos%C3&#x25;A9"), Some("José".to_string()));
        assert_eq!(decode_once("&unknown; &ampx"), Some("&unknown; &x".to_string()));
        assert_eq!(decode_once("no refs at all"), None);
    }

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
