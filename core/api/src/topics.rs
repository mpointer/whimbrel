//! Connected Topics: model-written topic names and the read surface the shells call.
//! A topic is named by the same governed chain as every other call (spend cap, Local-only
//! routing, fallback); any failure falls back to its top entities so a topic always gets a name.

use serde::{Deserialize, Serialize};

use crate::envelope::{ApiError, ApiResult};
use crate::routing::{build_chain, RoutingConfig};
use crate::util::log_usage_for;

pub use memory::topics::read::{
    LeftOff, OpenTarget, TopicDetail, TopicMember, TopicNameInput, TopicSummary,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamingReport {
    /// Topics given a name (model-written or fallback).
    pub named: usize,
    /// Of those, how many used the entity-list fallback.
    pub fallback: usize,
}

fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

fn build_prompt(t: &TopicNameInput) -> String {
    let list = |v: &[String]| {
        v.iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let prompt = format!(
        "Name this group of related emails, meetings and activity.\n\
         Key people and topics:\n{}\n\nRecent items:\n{}\n\n\
         Reply with only JSON: {{\"name\": \"<=6 words\", \"status\": \"<=14 words on where things stand\"}}",
        list(&t.top_entities),
        list(&t.sample_titles),
    );
    memory::redact::redact_secrets(&prompt).into_owned()
}

/// Model output is steered by attacker-controlled subjects and ends up in the UI and in
/// Markdown files: drop control/format/bidi chars, defang Markdown, collapse whitespace,
/// then cap words and characters.
fn sanitize(s: &str, max_words: usize, max_chars: usize) -> String {
    let format_char = |c: char| matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}');
    let cleaned: String = s
        .chars()
        .filter(|&c| c.is_whitespace() || !(c.is_control() || format_char(c)))
        .map(|c| {
            if c.is_whitespace() || "[]()`|<>".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words.trim_start_matches(['#', ' ']);
    let words = words
        .split_whitespace()
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ");
    words
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

/// `{"name", "status"}` out of a reply that may wrap the JSON in prose or a code fence.
fn parse_reply(reply: &str) -> Option<(String, Option<String>)> {
    let json = reply.get(reply.find('{')?..=reply.rfind('}')?)?;
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let name = sanitize(v.get("name")?.as_str()?, 6, 60);
    let status = v
        .get("status")
        .and_then(|s| s.as_str())
        .map(|s| sanitize(s, 14, 140));
    (!name.is_empty()).then_some((name, status.filter(|s| !s.is_empty())))
}

fn fallback_name(t: &TopicNameInput) -> String {
    // Entity names come from email headers, so they get the same cleaning as model output.
    let parts: Vec<String> = t
        .top_entities
        .iter()
        .take(3)
        .map(|e| sanitize(e, 18, 60))
        .filter(|e| !e.is_empty())
        .collect();
    let name = sanitize(&parts.join(" · "), 18, 60);
    if name.is_empty() {
        "Untitled topic".into()
    } else {
        name
    }
}

/// Names up to `limit` topics. `complete` returns the model's reply or an error string
/// (spend cap, provider failure); an error or an unparseable reply uses the fallback.
pub fn run_naming_batch_with_complete(
    store: &memory::Store,
    limit: usize,
    complete: impl Fn(&str) -> Result<String, String>,
) -> ApiResult<NamingReport> {
    let mut report = NamingReport::default();
    for t in store.topics_needing_names(limit).map_err(db)? {
        let parsed = complete(&build_prompt(&t))
            .ok()
            .and_then(|r| parse_reply(&r));
        let (name, status) = match parsed {
            Some(p) => p,
            None => {
                report.fallback += 1;
                (fallback_name(&t), None)
            }
        };
        store
            .apply_topic_name(t.id, &name, status.as_deref())
            .map_err(db)?;
        report.named += 1;
    }
    Ok(report)
}

fn chain_for(routing: &RoutingConfig, ollama_model: Option<&str>) -> Vec<llm::ChainLink> {
    let model = ollama_model.unwrap_or("");
    // Local-only with no local model: nothing may be called, so every topic uses the fallback.
    if routing.sensitive && model.is_empty() {
        return vec![];
    }
    build_chain(model, routing)
}

pub fn run_naming_batch(
    store: &memory::Store,
    routing: &RoutingConfig,
    ollama_model: Option<&str>,
    now_ms: i64,
    limit: usize,
) -> ApiResult<NamingReport> {
    let cap = routing.spend_cap_config();
    let chain = chain_for(routing, ollama_model);
    run_naming_batch_with_complete(store, limit, |prompt| {
        llm::check_spend_cap(store, &cap, None, Some("topic_name"), now_ms)
            .map_err(|e| format!("{e:?}"))?;
        let (completion, link) = llm::try_chain(&chain, |l| l.client.complete(prompt))
            .ok_or_else(|| "provider failure".to_string())?;
        log_usage_for(
            store,
            link.provider,
            &link.model,
            "topic_name",
            completion.token_usage,
        );
        Ok(completion.text)
    })
}

pub fn list(
    store: &memory::Store,
    active_only: bool,
    limit: usize,
) -> ApiResult<Vec<TopicSummary>> {
    store.list_topics(active_only, limit).map_err(db)
}

pub fn detail(store: &memory::Store, id: i64) -> ApiResult<Option<TopicDetail>> {
    store.topic_detail(id).map_err(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A file-backed store plus a second connection for seeding topic rows.
    fn seeded(titles: &[&str]) -> (tempfile::TempDir, memory::Store, rusqlite::Connection, i64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = memory::Store::open(&path, dir.path()).unwrap();
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "INSERT INTO topic (created_at_unix_ms, updated_at_unix_ms) VALUES (0, 0)",
            [],
        )
        .unwrap();
        let t = c.last_insert_rowid();
        for (i, title) in titles.iter().enumerate() {
            c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('episode', ?1, ?1, ?2, 0)", rusqlite::params![i as i64 + 1, title]).unwrap();
            let sid = c.last_insert_rowid();
            c.execute(
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')",
                [t, sid],
            )
            .unwrap();
            for (k, w) in [("Ann", 3.0), ("Acme", 2.0), ("Bob", 1.0), ("Cy", 0.5)] {
                c.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', ?1, ?1, 0) ON CONFLICT(kind, key) DO NOTHING", [k]).unwrap();
                c.execute("INSERT INTO entity_mention (entity_id, source_id, weight) SELECT id, ?2, ?3 FROM entity WHERE key = ?1", rusqlite::params![k, sid, w]).unwrap();
            }
        }
        (dir, store, c, t)
    }

    fn row(c: &rusqlite::Connection, t: i64) -> (String, Option<String>, i64) {
        c.query_row(
            "SELECT name, status_line, name_dirty FROM topic WHERE id = ?1",
            [t],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn naming_uses_model_json() {
        let (_d, s, c, t) = seeded(&["Pricing call", "Quote follow-up"]);
        let r = run_naming_batch_with_complete(&s, 5, |_| {
            Ok("Sure!\n```json\n{\"name\": \"Acme pricing\", \"status\": \"Waiting on legal sign-off\"}\n```".into())
        })
        .unwrap();
        assert_eq!(
            r,
            NamingReport {
                named: 1,
                fallback: 0
            }
        );
        assert_eq!(
            row(&c, t),
            (
                "Acme pricing".into(),
                Some("Waiting on legal sign-off".into()),
                0
            )
        );
    }

    #[test]
    fn naming_falls_back_on_garbage_and_on_spend_cap() {
        for reply in [
            Ok("not json".to_string()),
            Ok("{\"name\": \"\"}".into()),
            Err("spend cap".to_string()),
        ] {
            let (_d, s, c, t) = seeded(&["a", "b"]);
            let r = run_naming_batch_with_complete(&s, 5, |_| reply.clone()).unwrap();
            assert_eq!(
                r,
                NamingReport {
                    named: 1,
                    fallback: 1
                }
            );
            assert_eq!(row(&c, t), ("Ann · Acme · Bob".into(), None, 0));
        }
    }

    #[test]
    fn naming_never_overwrites_user_name() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        s.rename_topic(t, "Mine").unwrap();
        let calls = RefCell::new(0);
        let r = run_naming_batch_with_complete(&s, 5, |_| {
            *calls.borrow_mut() += 1;
            Ok("{\"name\": \"Auto\"}".into())
        })
        .unwrap();
        assert_eq!((r, *calls.borrow()), (NamingReport::default(), 0));
        assert_eq!(row(&c, t).0, "Mine");
        // Even a direct apply is a no-op.
        s.apply_topic_name(t, "Auto", None).unwrap();
        assert_eq!(row(&c, t).0, "Mine");
    }

    #[test]
    fn reversed_braces_do_not_panic() {
        let (_d, s, _c, _t) = seeded(&["a", "b"]);
        let r = run_naming_batch_with_complete(&s, 5, |_| Ok("} x {".into())).unwrap();
        assert_eq!(
            r,
            NamingReport {
                named: 1,
                fallback: 1
            }
        );
    }

    #[test]
    fn model_output_is_sanitized() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        let long = "x".repeat(500);
        let reply = format!(
            "{{\"name\": \"\\u001b[31m# Acme\u{200d} \u{202e}[click](http://evil) `p`|<b>\", \"status\": \"{long}\"}}"
        );
        run_naming_batch_with_complete(&s, 5, |_| Ok(reply.clone())).unwrap();
        let (name, status, _) = row(&c, t);
        for bad in [
            '\u{1b}', '\u{200d}', '\u{202e}', '[', ']', '(', ')', '`', '|', '<', '>',
        ] {
            assert!(!name.contains(bad), "{name:?} has {bad:?}");
        }
        assert!(
            !name.starts_with('#') && name.len() <= 60 && name.contains("Acme"),
            "{name:?}"
        );
        assert_eq!(status.unwrap().chars().count(), 140);
        // Only junk left: fallback.
        let (_d, s, c, t) = seeded(&["a", "b"]);
        let r =
            run_naming_batch_with_complete(&s, 5, |_| Ok("{\"name\": \"#`|[]\"}".into())).unwrap();
        assert_eq!(r.fallback, 1);
        assert_eq!(row(&c, t).0, "Ann · Acme · Bob");
    }

    #[test]
    fn sensitive_without_local_model_makes_no_model_call() {
        let routing = RoutingConfig {
            sensitive: true,
            ..Default::default()
        };
        assert!(chain_for(&routing, None).is_empty());
        assert!(chain_for(&routing, Some("")).is_empty());
        assert_eq!(chain_for(&routing, Some("llama3")).len(), 1);
        // And the batch still names via the fallback.
        let (_d, s, _c, _t) = seeded(&["a", "b"]);
        let r = run_naming_batch(&s, &routing, None, 0, 5).unwrap();
        assert_eq!(
            r,
            NamingReport {
                named: 1,
                fallback: 1
            }
        );
    }

    #[test]
    fn fallback_name_is_sanitized() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        let hostile = format!("\u{202e}[x](http://evil) {}", "w".repeat(300));
        c.execute(
            "UPDATE entity SET display = ?1 WHERE key = 'Ann'",
            [&hostile],
        )
        .unwrap();
        run_naming_batch_with_complete(&s, 5, |_| Err("x".into())).unwrap();
        let name = row(&c, t).0;
        assert!(
            name.chars().count() <= 60 && name.starts_with("x http://evil"),
            "{name:?}"
        );
        for bad in ['\u{202e}', '[', ']', '(', ')'] {
            assert!(!name.contains(bad), "{name:?}");
        }
        let (_d, s, c, t) = seeded(&["a", "b"]);
        c.execute("UPDATE entity SET display = '#`|[]'", [])
            .unwrap();
        run_naming_batch_with_complete(&s, 5, |_| Err("x".into())).unwrap();
        assert_eq!(row(&c, t).0, "Untitled topic");
    }

    #[test]
    fn prompt_is_redacted() {
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let (_d, s, _c, _t) = seeded(&[&format!("key {secret} leaked"), "b"]);
        let seen = RefCell::new(String::new());
        run_naming_batch_with_complete(&s, 5, |p| {
            seen.borrow_mut().push_str(p);
            Err("x".into())
        })
        .unwrap();
        let seen = seen.into_inner();
        assert!(
            seen.contains("leaked"),
            "prompt should still carry the title: {seen}"
        );
        assert!(!seen.contains("abcdefghijklmnop"), "{seen}");
    }

    #[test]
    fn list_and_detail_wrap_the_store() {
        let (_d, s, _c, t) = seeded(&["a", "b"]);
        // Seeded timestamps are ~0, so only the unfiltered list shows it.
        assert!(list(&s, true, 10).unwrap().is_empty());
        let all = list(&s, false, 10).unwrap();
        assert_eq!((all.len(), all[0].id, all[0].member_count), (1, t, 2));
        let d = detail(&s, t).unwrap().unwrap();
        assert_eq!(d.members.len(), 2);
        assert!(detail(&s, 999).unwrap().is_none());
        let json = serde_json::to_value(&d).unwrap();
        assert!(
            json["summary"]["member_count"].is_number()
                && json["card"]["open_action_items"].is_array()
        );
    }
}
