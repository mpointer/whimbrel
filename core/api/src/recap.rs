//! A meeting's recap: the summary rendered for copying (plain text, Markdown, Slack) or for
//! an email to the attendees, who it goes to, and the `mailto:` draft used when Gmail isn't
//! connected. Pure functions: the apps pass the summary as shown (it may have been edited)
//! and their translated labels, so Mac and Windows produce the same text.
//!
//! A recap never carries the transcript, the user's notes or any link but the footer's.

use memory::speaker_names::Attendee;
use serde::{Deserialize, Serialize};

/// The site the email footer links to.
pub const SITE_URL: &str = "https://whimbrelapp.com";

/// A `mailto:` URL longer than this may be cut off by the mail app or the OS; the body then
/// goes to the clipboard instead.
pub const MAILTO_MAX_CHARS: usize = 1_800;

/// The summary as the meeting page shows it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecapSummary {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub tldr: String,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub action_items: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

/// The app's translated section headings and footer text ("Sent with Whimbrel").
#[derive(Debug, Clone, Deserialize)]
pub struct RecapLabels {
    pub decisions: String,
    pub action_items: String,
    pub open_questions: String,
    pub footer: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecapFormat {
    /// What Copy gives today: the TL;DR, then "Heading:" blocks of "• " bullets.
    Plain,
    Markdown,
    Slack,
    /// Plain text with the footer, for Gmail's plain-text send or a mail app.
    Email,
}

impl RecapFormat {
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

fn clean(items: &[String]) -> Vec<String> {
    items.iter().map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()).collect()
}

/// Slack treats `&`, `<` and `>` as control characters; everything else is literal enough.
fn slack_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The recap text in `format`. Empty sections are left out; an empty summary renders as "".
pub fn render(summary: &RecapSummary, labels: &RecapLabels, format: RecapFormat) -> String {
    let tldr = summary.tldr.trim();
    let title = summary.title.trim();
    let sections = [
        (labels.decisions.trim(), clean(&summary.decisions)),
        (labels.action_items.trim(), clean(&summary.action_items)),
        (labels.open_questions.trim(), clean(&summary.open_questions)),
    ];
    let mut parts: Vec<String> = Vec::new();
    match format {
        RecapFormat::Plain | RecapFormat::Email => {
            if !tldr.is_empty() {
                parts.push(tldr.to_string());
            }
            for (heading, items) in &sections {
                if !items.is_empty() {
                    let bullets: Vec<String> = items.iter().map(|i| format!("• {i}")).collect();
                    parts.push(format!("{heading}:\n{}", bullets.join("\n")));
                }
            }
        }
        RecapFormat::Markdown => {
            if !title.is_empty() {
                parts.push(format!("## {title}"));
            }
            if !tldr.is_empty() {
                parts.push(tldr.to_string());
            }
            for (heading, items) in &sections {
                if !items.is_empty() {
                    let bullets: Vec<String> = items.iter().map(|i| format!("- {i}")).collect();
                    parts.push(format!("### {heading}\n{}", bullets.join("\n")));
                }
            }
        }
        RecapFormat::Slack => {
            if !title.is_empty() {
                parts.push(format!("*{}*", slack_escape(title)));
            }
            if !tldr.is_empty() {
                parts.push(slack_escape(tldr));
            }
            for (heading, items) in &sections {
                if !items.is_empty() {
                    let bullets: Vec<String> = items.iter().map(|i| format!("• {}", slack_escape(i))).collect();
                    parts.push(format!("*{}*\n{}", slack_escape(heading), bullets.join("\n")));
                }
            }
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    let mut text = parts.join("\n\n");
    if format == RecapFormat::Email {
        text.push_str(&format!("\n\n--\n{}: {SITE_URL}", labels.footer.trim()));
    }
    text
}

/// An address safe to put in a `To:` header or a `mailto:` list: one `@`, no spaces,
/// separators, angle brackets or line breaks.
pub fn is_sendable_address(email: &str) -> bool {
    let e = email.trim();
    e.len() <= 254
        && e.matches('@').count() == 1
        && !e.starts_with('@')
        && !e.ends_with('@')
        && !e.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | ';' | '<' | '>' | '"' | '(' | ')'))
}

/// Who a recap can go to: the invitees with a usable address, once each (case-insensitive),
/// minus the user's own addresses. Order is kept.
pub fn recipients(attendees: &[Attendee], self_emails: &[String]) -> Vec<Attendee> {
    let mut seen: Vec<String> = self_emails.iter().map(|e| e.trim().to_lowercase()).collect();
    let mut out = Vec::new();
    for a in attendees {
        let Some(email) = a.email.as_deref().map(str::trim).filter(|e| is_sendable_address(e)) else { continue };
        let key = email.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(Attendee { name: a.name.trim().to_string(), email: Some(email.to_string()) });
    }
    out
}

/// A subject line safe for a header: line breaks and runs of spaces become single spaces.
pub fn header_safe(subject: &str) -> String {
    subject.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MailtoDraft {
    pub url: String,
    /// `false`: the body was too long for the URL, so the app copies it to the clipboard
    /// and the draft carries `paste_hint` instead.
    pub body_in_url: bool,
}

/// RFC 3986 percent-encoding of everything but the unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `mailto:` draft for a mail app. Unusable addresses are dropped. Line breaks in the
/// body are sent as CRLF, as RFC 6068 asks.
pub fn mailto(to: &[String], subject: &str, body: &str, paste_hint: &str) -> MailtoDraft {
    let to: Vec<String> = to.iter().map(|e| e.trim()).filter(|e| is_sendable_address(e)).map(encode).collect();
    let crlf = |s: &str| s.replace("\r\n", "\n").replace('\n', "\r\n");
    let base = format!("mailto:{}?subject={}", to.join(","), encode(&header_safe(subject)));
    let full = format!("{base}&body={}", encode(&crlf(body)));
    if full.len() <= MAILTO_MAX_CHARS {
        return MailtoDraft { url: full, body_in_url: true };
    }
    let hint = paste_hint.trim();
    let url = if hint.is_empty() { base } else { format!("{base}&body={}", encode(hint)) };
    MailtoDraft { url, body_in_url: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> RecapLabels {
        RecapLabels {
            decisions: "Decisions".into(),
            action_items: "Action items".into(),
            open_questions: "Open questions".into(),
            footer: "Sent with Whimbrel".into(),
        }
    }

    fn summary() -> RecapSummary {
        RecapSummary {
            title: "Q3 planning".into(),
            tldr: "We agreed the launch plan.".into(),
            decisions: vec!["Ship on the 20th".into()],
            action_items: vec!["Ben: send the deck (Fri)".into(), "  ".into()],
            open_questions: vec![],
        }
    }

    #[test]
    fn plain_matches_what_copy_gives_today_and_leaves_out_empty_sections() {
        assert_eq!(
            render(&summary(), &labels(), RecapFormat::Plain),
            "We agreed the launch plan.\n\nDecisions:\n• Ship on the 20th\n\nAction items:\n• Ben: send the deck (Fri)"
        );
    }

    #[test]
    fn markdown_has_a_title_headings_and_dashes() {
        assert_eq!(
            render(&summary(), &labels(), RecapFormat::Markdown),
            "## Q3 planning\n\nWe agreed the launch plan.\n\n### Decisions\n- Ship on the 20th\n\n### Action items\n- Ben: send the deck (Fri)"
        );
    }

    #[test]
    fn slack_bolds_headings_and_escapes_its_control_characters() {
        let s = RecapSummary { title: "R&D".into(), tldr: "a <b> c".into(), decisions: vec!["x & y".into()], ..Default::default() };
        assert_eq!(render(&s, &labels(), RecapFormat::Slack), "*R&amp;D*\n\na &lt;b&gt; c\n\n*Decisions*\n• x &amp; y");
    }

    #[test]
    fn email_ends_with_the_footer_and_its_link() {
        let text = render(&summary(), &labels(), RecapFormat::Email);
        assert!(text.starts_with("We agreed the launch plan.\n\nDecisions:"));
        assert!(text.ends_with("\n\n--\nSent with Whimbrel: https://whimbrelapp.com"), "{text}");
        assert!(!text.contains("Q3 planning"), "the title is the subject, not the body");
    }

    #[test]
    fn an_empty_summary_renders_nothing_not_even_a_footer() {
        for f in [RecapFormat::Plain, RecapFormat::Markdown, RecapFormat::Slack, RecapFormat::Email] {
            assert_eq!(render(&RecapSummary { title: String::new(), ..Default::default() }, &labels(), f), "");
        }
    }

    #[test]
    fn formats_parse_by_name() {
        assert_eq!(RecapFormat::parse("slack"), Some(RecapFormat::Slack));
        assert_eq!(RecapFormat::parse("email"), Some(RecapFormat::Email));
        assert_eq!(RecapFormat::parse("html"), None);
    }

    #[test]
    fn recipients_skip_the_user_duplicates_and_unusable_addresses() {
        let a = |n: &str, e: Option<&str>| Attendee { name: n.into(), email: e.map(Into::into) };
        let attendees = [
            a("Me", Some("Mike@Example.com")),
            a("Ana", Some("ana@x.com")),
            a("Ana again", Some("ANA@x.com")),
            a("Bo", None),
            a("Room", Some("bad address@x.com")),
            a("Inject", Some("eve@x.com\r\nBcc: z@z.com")),
            a("Cy", Some(" cy@y.org ")),
        ];
        let out = recipients(&attendees, &["mike@example.com".into()]);
        assert_eq!(out, vec![a("Ana", Some("ana@x.com")), a("Cy", Some("cy@y.org"))]);
    }

    #[test]
    fn mailto_encodes_everything_and_uses_crlf() {
        let d = mailto(&["ana@x.com".into(), "b+c@y.org".into(), "no good".into()], "Recap: Q3 & more\r\nBcc: x", "Line 1\nLine 2", "Paste");
        assert!(d.body_in_url);
        assert_eq!(d.url, "mailto:ana%40x.com,b%2Bc%40y.org?subject=Recap%3A%20Q3%20%26%20more%20Bcc%3A%20x&body=Line%201%0D%0ALine%202");
    }

    #[test]
    fn a_long_body_goes_to_the_clipboard_and_the_draft_carries_the_hint() {
        let body = "word ".repeat(600);
        let d = mailto(&["ana@x.com".into()], "Recap", &body, "Paste the recap here.");
        assert!(!d.body_in_url);
        assert_eq!(d.url, "mailto:ana%40x.com?subject=Recap&body=Paste%20the%20recap%20here.");
        assert!(d.url.len() <= MAILTO_MAX_CHARS);
    }
}
