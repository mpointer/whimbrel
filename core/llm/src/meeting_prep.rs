//! Meeting Prep's brief: the prompt, the model's structured answer, and the check that
//! every claim cites only sources it was given. See
//! docs/superpowers/specs/2026-10-06-meeting-prep-design.md.

use std::collections::HashSet;

use memory::prep::{same_person, PrepContext, PrepSourceKind};
use serde::{Deserialize, Serialize};

use super::{extract_json_object, LlmClient, LlmError, TokenUsage};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub text: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttendeeNote {
    pub who: String,
    pub note: String,
    pub sources: Vec<String>,
    pub last_interaction_unix_ms: Option<i64>,
}

/// A source's metadata for a citation chip (the text itself is not stored).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BriefSource {
    pub id: String,
    pub kind: PrepSourceKind,
    pub title: String,
    pub ts_unix_ms: i64,
    pub meeting_id: Option<i64>,
    pub thread_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Brief {
    pub summary: Claim,
    pub attendees: Vec<AttendeeNote>,
    pub open_items: Vec<Claim>,
    pub open_questions: Vec<Claim>,
    pub sources: Vec<BriefSource>,
}

/// What the model is asked to return, before the citations are checked.
#[derive(Debug, Deserialize, Default)]
struct RawBrief {
    #[serde(default)]
    summary: Option<Claim>,
    #[serde(default)]
    attendees: Vec<RawNote>,
    #[serde(default)]
    open_items: Vec<Claim>,
    #[serde(default)]
    open_questions: Vec<Claim>,
}

#[derive(Debug, Deserialize)]
struct RawNote {
    who: String,
    note: String,
    #[serde(default)]
    sources: Vec<String>,
}

pub fn build_prompt(title: &str, attendees: &[String], context: &PrepContext, profile: &str) -> String {
    let mut prompt = format!(
        "You prepare a short pre-meeting brief. Meeting: \"{title}\". Attendees: {}.\n\n\
         Use ONLY the numbered sources below. Cite the source ids that support every claim. \
         Do not write anything you cannot cite. Respond with ONLY a JSON object, no markdown \
         fences, matching exactly:\n\
         {{\"summary\":{{\"text\":\"...\",\"sources\":[\"M1\"]}},\
         \"attendees\":[{{\"who\":\"...\",\"note\":\"...\",\"sources\":[\"E1\"]}}],\
         \"open_items\":[{{\"text\":\"...\",\"sources\":[\"M1\"]}}],\
         \"open_questions\":[{{\"text\":\"...\",\"sources\":[\"M1\"]}}]}}\n\n",
        attendees.join(", ")
    );
    let about = crate::profile::profile_section(profile);
    if !about.is_empty() {
        // Background about the user, not a source: it cannot be cited.
        prompt.push_str(&about);
        prompt.push_str("\n\n");
    }
    prompt.push_str("Sources:\n");
    for s in &context.sources {
        prompt.push_str(&format!("[{}] ({:?}, with {}) {}\n", s.id, s.kind, s.attendee, s.text));
    }
    prompt
}

fn brief_sources(context: &PrepContext) -> Vec<BriefSource> {
    context
        .sources
        .iter()
        .map(|s| BriefSource {
            id: s.id.clone(),
            kind: s.kind,
            title: s.title.clone(),
            ts_unix_ms: s.ts_unix_ms,
            meeting_id: s.meeting_id,
            thread_id: s.thread_id,
        })
        .collect()
}

fn last_interaction_for(context: &PrepContext, who: &str) -> Option<i64> {
    context.last_interaction.iter().find(|l| same_person(&l.attendee, who)).and_then(|l| l.ts_unix_ms)
}

/// A claim stands only if it cites at least one source and every id it cites was supplied.
fn cites_only(sources: &[String], valid: &HashSet<&str>) -> bool {
    !sources.is_empty() && sources.iter().all(|id| valid.contains(id.as_str()))
}

pub fn parse_brief(text: &str, _title: &str, _attendees: &[String], context: &PrepContext) -> Result<Brief, LlmError> {
    let json = extract_json_object(text)
        .ok_or_else(|| LlmError::Provider(format!("the model's answer contained no JSON object: {text:?}")))?;
    let raw: RawBrief = serde_json::from_str(json)
        .map_err(|e| LlmError::Provider(format!("failed to parse the meeting brief JSON: {e}")))?;
    let valid: HashSet<&str> = context.sources.iter().map(|s| s.id.as_str()).collect();

    let keep = |claims: Vec<Claim>| claims.into_iter().filter(|c| cites_only(&c.sources, &valid)).collect::<Vec<_>>();
    let summary = raw.summary.filter(|c| cites_only(&c.sources, &valid)).unwrap_or_else(|| {
        let meetings = context.sources.iter().filter(|s| s.kind == PrepSourceKind::Meeting).count();
        let emails = context.sources.len() - meetings;
        Claim { text: format!("Prepared from {meetings} earlier meetings and {emails} email threads."), sources: Vec::new() }
    });
    let notes = raw
        .attendees
        .into_iter()
        .filter(|n| cites_only(&n.sources, &valid))
        .map(|n| AttendeeNote {
            last_interaction_unix_ms: last_interaction_for(context, &n.who),
            who: n.who,
            note: n.note,
            sources: n.sources,
        })
        .collect();
    Ok(Brief {
        summary,
        attendees: notes,
        open_items: keep(raw.open_items),
        open_questions: keep(raw.open_questions),
        sources: brief_sources(context),
    })
}

/// The brief when there is no history at all: no model is called and nothing is cited.
pub fn deterministic_brief(attendees: &[String], context: &PrepContext) -> Brief {
    Brief {
        summary: Claim { text: "No earlier meetings or email with these people.".into(), sources: Vec::new() },
        attendees: attendees
            .iter()
            .map(|who| AttendeeNote {
                who: who.clone(),
                note: "Nothing on record.".into(),
                sources: Vec::new(),
                last_interaction_unix_ms: last_interaction_for(context, who),
            })
            .collect(),
        open_items: Vec::new(),
        open_questions: Vec::new(),
        sources: Vec::new(),
    }
}

pub fn generate(
    client: &dyn LlmClient,
    title: &str,
    attendees: &[String],
    context: &PrepContext,
) -> Result<(Brief, TokenUsage), LlmError> {
    let completion = client.complete(&build_prompt(title, attendees, context, ""))?;
    let brief = parse_brief(&completion.text, title, attendees, context)?;
    Ok((brief, completion.token_usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Completion;
    use memory::prep::{LastInteraction, PrepSource};

    fn source(id: &str, kind: PrepSourceKind, title: &str) -> PrepSource {
        PrepSource {
            id: id.into(),
            kind,
            title: title.into(),
            ts_unix_ms: 1_000,
            attendee: "Ben Lawrick".into(),
            text: format!("text of {id}"),
            meeting_id: (kind == PrepSourceKind::Meeting).then_some(7),
            thread_id: (kind == PrepSourceKind::Email).then_some(9),
        }
    }

    fn context() -> PrepContext {
        PrepContext {
            sources: vec![source("M1", PrepSourceKind::Meeting, "Acme kickoff"), source("E1", PrepSourceKind::Email, "Re: pricing")],
            last_interaction: vec![LastInteraction { attendee: "Ben Lawrick".into(), ts_unix_ms: Some(5_000) }],
        }
    }

    fn attendees() -> Vec<String> {
        vec!["Ben Lawrick".to_string()]
    }

    #[test]
    fn the_prompt_lists_every_source_with_its_id_and_asks_for_citations() {
        let prompt = build_prompt("Acme renewal", &attendees(), &context(), "");
        assert!(prompt.contains("Acme renewal") && prompt.contains("Ben Lawrick"));
        assert!(prompt.contains("[M1]") && prompt.contains("text of M1") && prompt.contains("[E1]"));
        assert!(prompt.to_lowercase().contains("cite"), "{prompt}");
        assert!(prompt.contains("\"open_questions\""), "names the JSON shape");
    }

    #[test]
    fn the_prep_prompt_carries_the_profile_before_the_sources_and_omits_it_when_blank() {
        let ctx = context();
        let with = build_prompt("Sync", &["Dana".into()], &ctx, "Role: CFO");
        let without = build_prompt("Sync", &["Dana".into()], &ctx, "  ");
        assert!(with.contains("About the user"));
        assert!(with.contains("Role: CFO"));
        assert!(with.find("About the user").unwrap() < with.find("Sources:").unwrap());
        assert!(!without.contains("About the user"));
    }

    #[test]
    fn a_valid_answer_becomes_a_brief_with_sources_and_last_interaction() {
        let answer = r#"Here you go: {"summary":{"text":"Follow up on pricing.","sources":["M1","E1"]},
            "attendees":[{"who":"Ben Lawrick","note":"Sent the deck.","sources":["E1"]}],
            "open_items":[{"text":"Ben sends the deck","sources":["M1"]}],
            "open_questions":[{"text":"Pricing?","sources":["E1"]}]}"#;
        let brief = parse_brief(answer, "Acme renewal", &attendees(), &context()).unwrap();

        assert_eq!(brief.summary.sources, ["M1", "E1"]);
        assert_eq!(brief.attendees[0].last_interaction_unix_ms, Some(5_000));
        assert_eq!(brief.open_items.len(), 1);
        assert_eq!(brief.sources.len(), 2);
        assert_eq!((brief.sources[0].id.as_str(), brief.sources[0].meeting_id), ("M1", Some(7)));
        assert_eq!(brief.sources[1].thread_id, Some(9));
    }

    #[test]
    fn a_claim_citing_an_unknown_or_no_source_is_dropped() {
        let answer = r#"{"summary":{"text":"Bad summary","sources":["M99"]},
            "attendees":[{"who":"Ben Lawrick","note":"Invented","sources":["Z1"]},
                         {"who":"Ben Lawrick","note":"Real","sources":["M1"]}],
            "open_items":[{"text":"No citation","sources":[]},{"text":"Mixed","sources":["M1","M42"]},{"text":"Good","sources":["E1"]}],
            "open_questions":[]}"#;
        let brief = parse_brief(answer, "T", &attendees(), &context()).unwrap();

        assert_eq!(brief.open_items.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Good"]);
        assert_eq!(brief.attendees.len(), 1);
        assert_eq!(brief.attendees[0].note, "Real");
        assert!(brief.summary.text.contains("earlier"), "falls back to a plain summary: {}", brief.summary.text);
        assert!(brief.summary.sources.is_empty());
    }

    #[test]
    fn an_answer_with_no_json_is_a_provider_error() {
        assert!(matches!(parse_brief("no braces here", "T", &attendees(), &context()), Err(LlmError::Provider(_))));
    }

    #[test]
    fn with_no_history_the_deterministic_brief_says_so_and_cites_nothing() {
        let empty = PrepContext { sources: vec![], last_interaction: vec![LastInteraction { attendee: "Stranger".into(), ts_unix_ms: None }] };
        let brief = deterministic_brief(&["Stranger".to_string()], &empty);
        assert_eq!(brief.summary.text, "No earlier meetings or email with these people.");
        assert!(brief.summary.sources.is_empty() && brief.open_items.is_empty() && brief.sources.is_empty());
        assert_eq!(brief.attendees.len(), 1);
    }

    struct Canned(String);
    impl LlmClient for Canned {
        fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
            Ok(Completion { text: self.0.clone(), token_usage: TokenUsage::default() })
        }
    }

    #[test]
    fn generate_prompts_the_client_and_returns_the_validated_brief() {
        let client = Canned(r#"{"summary":{"text":"S","sources":["M1"]},"attendees":[],"open_items":[],"open_questions":[]}"#.into());
        let (brief, _usage) = generate(&client, "T", &attendees(), &context()).unwrap();
        assert_eq!(brief.summary.text, "S");
    }
}
