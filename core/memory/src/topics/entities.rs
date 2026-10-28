//! Pure helpers that turn headers, titles and OCR text into topic entities
//! (people / orgs / phrases). No I/O, std only.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Person,
    Org,
    Phrase,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Person => "person",
            EntityKind::Org => "org",
            EntityKind::Phrase => "phrase",
        }
    }
}

const FREE_MAIL: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "yahoo.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
    "gmx.com",
    "fastmail.com",
];

const TWO_PART_SUFFIXES: &[&str] = &["co.uk", "com.au", "co.jp", "co.nz", "com.br", "co.in"];

/// Phrases that look like titles but carry no topic signal (lowercase).
pub const GENERIC_PHRASES: &[&str] = &[
    "zoom meeting",
    "microsoft teams",
    "google chrome",
    "google meet",
    "google calendar",
    "google docs",
    "google drive",
    "meeting id",
    "join meeting",
    "teams meeting",
    "calendar invite",
    "microsoft outlook",
    "microsoft word",
    "microsoft excel",
    "microsoft powerpoint",
    "screen sharing",
    "quick look",
    "system settings",
];

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "of", "in", "on", "at", "to", "for", "with", "from",
    "by", "via", "as", "is", "are", "was", "were", "be", "this", "that", "these", "those", "it",
    "its", "if", "we", "you", "i", "he", "she", "they", "our", "your", "my", "re", "fwd", "fw",
    "hi", "hello", "dear", "thanks", "thank", "please", "not", "no", "yes", "so", "then",
];

// Full names only: short forms (may, mar, sun, sat, wed, mon, dec, jan...) are also names/words.
const DAYS_MONTHS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

pub fn is_free_mail(domain: &str) -> bool {
    FREE_MAIL.contains(&domain.trim().to_lowercase().as_str())
}

/// Registrable domain, or `None` for free-mail / single-label domains.
pub fn org_key_from_domain(domain: &str) -> Option<String> {
    let d = domain.trim().trim_matches('.').to_lowercase();
    if d.is_empty() || is_free_mail(&d) {
        return None;
    }
    let labels: Vec<&str> = d.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty()) {
        return None;
    }
    let last_two = labels[labels.len() - 2..].join(".");
    let keep = if TWO_PART_SUFFIXES.contains(&last_two.as_str()) {
        3
    } else {
        2
    };
    let start = labels.len().saturating_sub(keep);
    let key = labels[start..].join(".");
    (!is_free_mail(&key)).then_some(key)
}

/// Lowercase, strip punctuation, collapse whitespace.
pub fn person_key(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match (c, c == '\\') {
            (_, true) => out.extend(it.next()),
            _ => out.push(c),
        }
    }
    out
}

/// Parse an RFC-5322-ish address list (`"Name" <a@b>`, bare `a@b`, comma/semicolon separated).
pub fn parse_addresses(raw: &str) -> Vec<Address> {
    let mut parts = Vec::new();
    let (mut cur, mut in_quote, mut in_angle) = (String::new(), false, false);
    let (mut escaped, mut comment) = (false, 0usize);
    for c in raw.chars() {
        if escaped {
            escaped = false;
            cur.push(c);
            continue;
        }
        if comment > 0 || (c == '(' && !in_quote) {
            match c {
                '(' => comment += 1,
                ')' => comment -= 1,
                _ => {}
            }
            continue;
        }
        match c {
            '\\' if in_quote => {
                escaped = true;
                cur.push(c);
            }
            '"' => {
                in_quote = !in_quote;
                cur.push(c);
            }
            '<' if !in_quote => {
                in_angle = true;
                cur.push(c);
            }
            '>' if !in_quote => {
                in_angle = false;
                cur.push(c);
            }
            ',' | ';' if !in_quote && !in_angle => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);

    parts
        .iter()
        .filter_map(|p| {
            let p = p.trim();
            let (name, email) = match (p.rfind('<'), p.rfind('>')) {
                (Some(l), Some(r)) if l < r => (p[..l].trim(), p[l + 1..r].trim()),
                _ => ("", p),
            };
            let email = email.to_lowercase();
            let email = email.strip_prefix("mailto:").unwrap_or(&email).to_string();
            let valid = email.contains('@')
                && !email.starts_with('@')
                && !email.ends_with('@')
                && !email.chars().any(char::is_whitespace);
            if !valid {
                return None;
            }
            let name = name.strip_prefix('"').unwrap_or(name);
            let name = name.strip_suffix('"').unwrap_or(name);
            let name = unescape(name);
            let name = name.trim();
            Some(Address {
                name: (!name.is_empty()).then(|| name.to_string()),
                email,
            })
        })
        .collect()
}

/// Candidate topic phrases: 2-4 capitalized / ALLCAPS word runs and quoted titles.
/// Lowercased, de-duplicated, in order of first appearance, at most `max`.
pub fn extract_phrases(text: &str, max: usize) -> Vec<String> {
    let mut found: Vec<(usize, String)> = Vec::new();

    // Quoted titles.
    let mut open: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match (c, open) {
            ('"' | '\u{201C}', None) => open = Some(i + c.len_utf8()),
            ('"' | '\u{201D}', Some(s)) => {
                let t = text[s..i]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
                if (2..=80).contains(&t.chars().count()) && !GENERIC_PHRASES.contains(&t.as_str()) {
                    found.push((s, t));
                }
                open = None;
            }
            _ => {}
        }
    }

    // Capitalized runs.
    let mut run: Vec<String> = Vec::new();
    let mut run_pos = 0;
    let flush = |run: &mut Vec<String>, pos: usize, found: &mut Vec<(usize, String)>| {
        // Trim leading stopwords and day/month tokens at either edge.
        let is_dm = |w: &String| DAYS_MONTHS.contains(&w.as_str());
        let mut lo = 0;
        while lo < run.len() && (STOPWORDS.contains(&run[lo].as_str()) || is_dm(&run[lo])) {
            lo += 1;
        }
        let mut hi = run.len();
        while hi > lo && is_dm(&run[hi - 1]) {
            hi -= 1;
        }
        if (2..=4).contains(&(hi - lo)) {
            let p = run[lo..hi].join(" ");
            if !GENERIC_PHRASES.contains(&p.as_str()) {
                found.push((pos, p));
            }
        }
        run.clear();
    };
    for word in text.split_whitespace() {
        let pos = word.as_ptr() as usize - text.as_ptr() as usize;
        let core = word.trim_matches(|c: char| !c.is_alphanumeric());
        let first = core.chars().next();
        let allcaps = core.chars().count() >= 3
            && core.chars().any(char::is_uppercase)
            && core.chars().all(|c| c.is_uppercase() || c.is_ascii_digit());
        let capitalized = first.is_some_and(|c| c.is_uppercase());
        if capitalized || allcaps {
            if run.is_empty() {
                run_pos = pos;
            }
            run.push(core.to_lowercase());
            if word.ends_with(|c: char| ",.;:!?)".contains(c)) {
                flush(&mut run, run_pos, &mut found);
            }
        } else {
            flush(&mut run, run_pos, &mut found);
        }
    }
    flush(&mut run, run_pos, &mut found);

    found.sort_by_key(|(p, _)| *p);
    let mut out: Vec<String> = Vec::new();
    for (_, p) in found {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.truncate(max);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_addresses_handles_names_bare_and_lists() {
        let a = parse_addresses("\"Lee, Sarah\" <Sarah@Acme.com>, bob@x.io; Carol <c@y.org>");
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].name.as_deref(), Some("Lee, Sarah"));
        assert_eq!(a[0].email, "sarah@acme.com");
        assert_eq!(a[1].name, None);
        assert_eq!(a[1].email, "bob@x.io");
        assert_eq!(a[2].name.as_deref(), Some("Carol"));
    }

    #[test]
    fn org_key_skips_free_mail_and_strips_subdomains() {
        assert_eq!(org_key_from_domain("gmail.com"), None);
        assert_eq!(
            org_key_from_domain("eng.acme.com").as_deref(),
            Some("acme.com")
        );
        assert_eq!(
            org_key_from_domain("mail.acme.co.uk").as_deref(),
            Some("acme.co.uk")
        );
        assert!(is_free_mail("gmail.com"));
    }

    #[test]
    fn person_key_normalizes() {
        assert_eq!(person_key("  Sarah   LEE. "), "sarah lee");
    }

    #[test]
    fn extract_phrases_finds_titles_not_noise() {
        let p = extract_phrases(
            "Q3 Pricing Review with Acme Corp on Monday via Zoom Meeting",
            10,
        );
        assert!(p.contains(&"q3 pricing review".to_string()), "{p:?}");
        assert!(p.contains(&"acme corp".to_string()), "{p:?}");
        assert!(!p.contains(&"monday".to_string()));
        assert!(!p.contains(&"zoom meeting".to_string()));
        let q = extract_phrases("see \"Board Deck v2\" and Acme Corp, acme corp", 10);
        assert!(q.contains(&"board deck v2".to_string()), "{q:?}");
        assert_eq!(q.iter().filter(|s| *s == "acme corp").count(), 1);
    }

    #[test]
    fn leading_stopwords_are_stripped_not_dropped() {
        let p = extract_phrases("The Acme Corp signed", 10);
        assert_eq!(p, vec!["acme corp".to_string()]);
    }

    #[test]
    fn day_month_trimmed_at_edges_only_and_ambiguous_kept() {
        assert!(extract_phrases("met Sarah May today", 10).contains(&"sarah may".to_string()));
        assert!(extract_phrases("met Sun Microsystems today", 10)
            .contains(&"sun microsystems".to_string()));
        assert_eq!(
            extract_phrases("Acme Corp Monday", 10),
            vec!["acme corp".to_string()]
        );
        assert!(extract_phrases("see Q3 Monday Review notes", 10)
            .contains(&"q3 monday review".to_string()));
    }

    #[test]
    fn org_key_rechecks_free_mail_after_reduction() {
        assert_eq!(org_key_from_domain("mail.yahoo.com"), None);
    }

    #[test]
    fn digits_are_not_allcaps() {
        // "2024" breaks the run; "Acme" and "Review" are single words -> nothing.
        assert!(extract_phrases("Acme 2024 Review", 10).is_empty());
        assert!(extract_phrases("see NDA 2024 draft", 10).is_empty());
        assert!(extract_phrases("2024 2025 2026", 10).is_empty());
    }

    #[test]
    fn parse_addresses_escaped_quotes_and_comments() {
        let a = parse_addresses(r#""O\"Brien, Pat" <x@y.com>"#);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name.as_deref(), Some("O\"Brien, Pat"));
        assert_eq!(a[0].email, "x@y.com");
        let b = parse_addresses("a@b.com (Alice), c@d.com");
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].email, "a@b.com");
    }

    #[test]
    fn parse_addresses_strips_mailto() {
        let a = parse_addresses("MailTo:Sarah@Acme.com, Bob <mailto:bob@x.io>");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].email, "sarah@acme.com");
        assert_eq!(a[1].email, "bob@x.io");
        assert_eq!(a[1].name.as_deref(), Some("Bob"));
    }
}
