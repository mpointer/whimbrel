//! Connected Topics: model-written topic names and the read surface the shells call.
//! A topic is named by the same governed chain as every other call (spend cap, Local-only
//! routing, fallback); any failure falls back to its top entities so a topic always gets a name.

use serde::{Deserialize, Serialize};

use crate::envelope::{ApiError, ApiResult};
use crate::routing::{build_chain_for, RoutingConfig};

pub use memory::topics::read::{
    LeftOff, OpenTarget, TopicDetail, TopicMember, TopicNameInput, TopicSummary,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamingReport {
    /// Topics given a name (model-written or fallback).
    pub named: usize,
    /// Of those, how many used the entity-list fallback.
    pub fallback: usize,
    /// Of `named`, how many names a model wrote (so a failing model reports 0).
    #[serde(default)]
    pub model_named: usize,
}

fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

fn build_prompt(t: &TopicNameInput, lang: &str) -> String {
    let list = |v: &[String]| {
        v.iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let prompt = format!(
        "{}Name this group of related emails, meetings and activity.\n\
         Key people and topics:\n{}\n\nRecent items:\n{}\n\n\
         Reply with only JSON: {{\"name\": \"<=6 words\", \"status\": \"<=14 words on where things stand\"}}",
        llm::profile::language_prefix(lang),
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
    let all: Vec<&str> = words.split_whitespace().collect();
    let mut cut = all.len() > max_words;
    let mut out = all.into_iter().take(max_words).collect::<Vec<_>>().join(" ");
    if out.chars().count() > max_chars {
        cut = true;
        out = out.chars().take(max_chars + 1).collect();
        // Back to the last whole word rather than mid-word.
        out = match out.rfind(' ') {
            Some(i) => out[..i].to_string(),
            None => out.chars().take(max_chars).collect(),
        };
    }
    if cut { trim_dangling(&out) } else { out.trim().to_string() }
}

/// A capped phrase must not end on a joining word or mark ("weekly numbers and",
/// "current state is"): drop those from the end, keeping at least one word.
fn trim_dangling(s: &str) -> String {
    const DANGLING: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "has", "have",
        "in", "into", "is", "it", "its", "of", "on", "or", "over", "so", "than", "that", "the",
        "their", "to", "via", "was", "were", "while", "with", "&", "+", "-", "–", "—", "·", "/",
    ];
    let mut words: Vec<&str> = s.split_whitespace().collect();
    loop {
        if let Some(last) = words.last_mut() {
            *last = last.trim_end_matches([',', ';', ':', '-', '–', '—', '/', '&', '·']);
        }
        match words.last() {
            Some(w) if words.len() > 1 && (w.is_empty() || DANGLING.contains(&w.to_lowercase().as_str())) => {
                words.pop();
            }
            _ => break,
        }
    }
    words.join(" ").trim().to_string()
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

/// A readable name from an email entity: the name in "Name <addr>", else the title-cased
/// local part with any "+tag" cut ("mike+test@x.com" is "Mike"). Machine senders give "".
fn name_from_email(e: &str) -> String {
    if let (Some(lt), Some(gt)) = (e.find('<'), e.rfind('>')) {
        if let Some(addr) = e.get(lt + 1..gt) {
            let name = e[..lt].trim().trim_matches(['"', '\'']).trim();
            if !name.is_empty() {
                return name.to_string();
            }
            return name_from_email(addr);
        }
    }
    let local = e.split('@').next().unwrap_or(e);
    let local = local.split('+').next().unwrap_or(local);
    let key: String = local.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    if ["noreply", "donotreply", "notifications", "notification", "mailerdaemon", "postmaster"].contains(&key.as_str()) {
        return String::new();
    }
    local
        .split(['.', '_', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            cs.next()
                .map(|f| f.to_uppercase().chain(cs.flat_map(char::to_lowercase)).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn fallback_name(t: &TopicNameInput) -> String {
    // Entity names come from email headers, so they get the same cleaning as model output.
    // An organisation goes first; an email address is shown as its local part.
    let mut items: Vec<(bool, String)> = t
        .top_entities
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let org = t.top_entity_kinds.get(i).is_some_and(|k| k == "org");
            let e = e.trim();
            let shown = if e.contains('@') {
                name_from_email(e)
            } else {
                e.to_string()
            };
            (org, sanitize(&shown, 18, 60))
        })
        .filter(|(_, e)| !e.is_empty())
        .collect();
    items.sort_by_key(|(org, _)| !org); // stable: orgs first, the rest keep their order
    let mut seen = std::collections::HashSet::new();
    let parts: Vec<String> = items
        .into_iter()
        .map(|(_, e)| e)
        .filter(|e| seen.insert(e.to_lowercase()))
        .take(3)
        .collect();
    let name = sanitize(&parts.join(" · "), 18, 60);
    if name.is_empty() {
        "Untitled topic".into()
    } else {
        name
    }
}

fn unix_now_ms() -> i64 {
    crate::util::now_unix_ms()
}

/// Names up to `limit` topics. `complete` returns the model's reply or an error string
/// (spend cap, provider failure); an error or an unparseable reply uses the fallback, stored
/// as a placeholder (provisional) so `run_naming_batch` can replace it once a model works.
/// This is the suggestions read path: it keeps no retry state, so a `complete` that always
/// fails ("no model") never delays a real model pass.
pub fn run_naming_batch_with_complete(
    store: &memory::Store,
    limit: usize,
    lang: &str,
    complete: impl Fn(&str) -> Result<String, String>,
) -> ApiResult<NamingReport> {
    let mut report = NamingReport::default();
    for t in store.topics_needing_names(limit).map_err(db)? {
        let parsed = complete(&build_prompt(&t, lang))
            .ok()
            .and_then(|r| parse_reply(&r));
        match parsed {
            Some((name, status)) => {
                store
                    .apply_topic_name(t.id, &name, status.as_deref())
                    .map_err(db)?;
                report.model_named += 1;
            }
            None => {
                report.fallback += 1;
                store
                    .apply_fallback_topic_name(t.id, &fallback_name(&t), unix_now_ms())
                    .map_err(db)?
            }
        }
        report.named += 1;
    }
    Ok(report)
}

/// Marks an error from `complete` as "no model was called" (none set up, spend cap): it
/// does not start the one-day wait before a topic is tried again.
pub const NO_MODEL: &str = "no model";

const RETRY_WAIT_MS: i64 = 86_400_000;

/// Names up to `limit` topics with a model: unnamed ones first, then placeholders whose last
/// failed attempt is over a day old. The first failure (error or unusable reply) ends the
/// batch, so a model that is down is asked once, not once per topic. An unnamed topic that
/// fails gets the placeholder name; one that already has a name keeps it.
pub fn run_model_naming_batch_with_complete(
    store: &memory::Store,
    limit: usize,
    lang: &str,
    now_ms: i64,
    complete: impl Fn(&str) -> Result<String, String>,
) -> ApiResult<NamingReport> {
    let mut report = NamingReport::default();
    for t in store
        .topics_needing_names_due(limit, now_ms - RETRY_WAIT_MS)
        .map_err(db)?
    {
        let reply = complete(&build_prompt(&t, lang));
        let called = !matches!(&reply, Err(e) if e.starts_with(NO_MODEL));
        if let Some((name, status)) = reply.ok().and_then(|r| parse_reply(&r)) {
            store
                .apply_topic_name(t.id, &name, status.as_deref())
                .map_err(db)?;
            report.named += 1;
            report.model_named += 1;
            continue;
        }
        if t.current_name.is_empty() {
            store
                .apply_fallback_topic_name(t.id, &fallback_name(&t), now_ms)
                .map_err(db)?;
            report.named += 1;
            report.fallback += 1;
        }
        if called {
            store.note_topic_name_attempt(t.id, now_ms).map_err(db)?;
        }
        store.set_topic_naming_last_failure(now_ms).map_err(db)?;
        break;
    }
    Ok(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamingStatus {
    /// Auto topics still showing a placeholder name.
    pub provisional_count: i64,
    pub last_naming_failure_unix_ms: Option<i64>,
}

pub fn naming_status(store: &memory::Store) -> ApiResult<NamingStatus> {
    Ok(NamingStatus {
        provisional_count: store.provisional_topic_count().map_err(db)?,
        last_naming_failure_unix_ms: store.topic_naming_last_failure().map_err(db)?,
    })
}

fn chain_for(routing: &RoutingConfig, ollama_model: Option<&str>) -> Vec<llm::ChainLink> {
    let model = ollama_model.unwrap_or("");
    // Local-only with no local model: nothing may be called, so every topic uses the fallback.
    if routing.sensitive && model.is_empty() {
        return vec![];
    }
    build_chain_for(llm::TaskKind::TopicName, model, routing)
}

pub fn run_naming_batch(
    store: &memory::Store,
    routing: &RoutingConfig,
    ollama_model: Option<&str>,
    now_ms: i64,
    limit: usize,
) -> ApiResult<NamingReport> {
    let cap = routing.spend_cap_config();
    let no_model = |e: String| format!("{NO_MODEL}: {e}");
    run_model_naming_batch_with_complete(store, limit, &routing.output_language, now_ms, |prompt| {
        llm::check_spend_cap(store, &cap, None, Some("topic_name"), now_ms)
            .map_err(|e| no_model(format!("{e:?}")))?;
        // Built per topic: spend from the previous one can put a provider over budget.
        let chain = crate::budget::apply_budgets(store, chain_for(routing, ollama_model), false, now_ms)
            .map_err(|e| no_model(format!("{e:?}")))?;
        if chain.is_empty() {
            return Err(no_model("no provider".into()));
        }
        let (completion, link) = llm::try_chain(&chain, |l| l.client.complete(prompt))
            .ok_or_else(|| "provider failure".to_string())?;
        crate::util::log_link_usage(store, link, "topic_name", completion.token_usage);
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
        let r = run_naming_batch_with_complete(&s, 5, "", |_| {
            Ok("Sure!\n```json\n{\"name\": \"Acme pricing\", \"status\": \"Waiting on legal sign-off\"}\n```".into())
        })
        .unwrap();
        assert_eq!(
            r,
            NamingReport { named: 1, fallback: 0, model_named: 1 }
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
            let r = run_naming_batch_with_complete(&s, 5, "", |_| reply.clone()).unwrap();
            assert_eq!(
                r,
                NamingReport { named: 1, fallback: 1, model_named: 0 }
            );
            assert_eq!(row(&c, t), ("Ann · Acme · Bob".into(), None, 0));
        }
    }

    #[test]
    fn naming_never_overwrites_user_name() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        s.rename_topic(t, "Mine").unwrap();
        let calls = RefCell::new(0);
        let r = run_naming_batch_with_complete(&s, 5, "", |_| {
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
        let r = run_naming_batch_with_complete(&s, 5, "", |_| Ok("} x {".into())).unwrap();
        assert_eq!(
            r,
            NamingReport { named: 1, fallback: 1, model_named: 0 }
        );
    }

    #[test]
    fn model_output_is_sanitized() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        let long = "x".repeat(500);
        let reply = format!(
            "{{\"name\": \"\\u001b[31m# Acme\u{200d} \u{202e}[click](http://evil) `p`|<b>\", \"status\": \"{long}\"}}"
        );
        run_naming_batch_with_complete(&s, 5, "", |_| Ok(reply.clone())).unwrap();
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
            run_naming_batch_with_complete(&s, 5, "", |_| Ok("{\"name\": \"#`|[]\"}".into())).unwrap();
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
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        let r = run_naming_batch(&s, &routing, None, 0, 5).unwrap();
        assert_eq!(
            r,
            NamingReport { named: 1, fallback: 1, model_named: 0 }
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
        run_naming_batch_with_complete(&s, 5, "", |_| Err("x".into())).unwrap();
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
        run_naming_batch_with_complete(&s, 5, "", |_| Err("x".into())).unwrap();
        assert_eq!(row(&c, t).0, "Untitled topic");
    }

    const DAY: i64 = 86_400_000;

    /// Another unnamed topic with two sources, ids offset so `ref_id`s do not collide.
    fn add_topic(c: &rusqlite::Connection, n: i64) -> i64 {
        c.execute(
            "INSERT INTO topic (created_at_unix_ms, updated_at_unix_ms) VALUES (0, 0)",
            [],
        )
        .unwrap();
        let t = c.last_insert_rowid();
        for i in 0..5 {
            let kind = if i % 2 == 0 { "email" } else { "meeting" };
            c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, ?2, 'x', 0)", rusqlite::params![kind, 100 * n + i]).unwrap();
            let sid = c.last_insert_rowid();
            c.execute(
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')",
                [t, sid],
            )
            .unwrap();
        }
        t
    }

    /// Grows a `seeded` topic to what a suggestion needs: 5 members from 2 kinds.
    fn make_suggestible(c: &rusqlite::Connection, t: i64) {
        for i in 0..3 {
            c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('meeting', ?1, ?1, 'x', 0)", [9000 + t * 10 + i]).unwrap();
            let sid = c.last_insert_rowid();
            c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')", [t, sid]).unwrap();
        }
    }

    /// (name_fallback_at_unix_ms, name_attempt_at_unix_ms)
    fn marks(c: &rusqlite::Connection, t: i64) -> (Option<i64>, Option<i64>) {
        c.query_row(
            "SELECT name_fallback_at_unix_ms, name_attempt_at_unix_ms FROM topic WHERE id = ?1",
            [t],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    const OK: &str = "{\"name\": \"Acme pricing\", \"status\": \"Waiting\"}";

    fn suggestions_pass(s: &memory::Store) {
        run_naming_batch_with_complete(s, 5, "", |_| Err("no model".into())).unwrap();
    }

    #[test]
    fn fallback_name_uses_local_part_and_org_first() {
        let t = TopicNameInput {
            id: 1,
            current_name: String::new(),
            top_entities: vec![
                "joe@futuremeanswered.com".into(),
                "FMA".into(),
                "Brad Smith".into(),
                "JOE@other.com".into(),
                "brad@futuremeanswered.com".into(),
            ],
            top_entity_kinds: vec!["person".into(), "org".into(), "person".into(), "person".into(), "person".into()],
            sample_titles: vec![],
        };
        assert_eq!(fallback_name(&t), "FMA · Joe · Brad Smith");
        let only_mail = TopicNameInput {
            top_entities: vec!["mary.jones@x.com".into(), "mary.jones@y.com".into()],
            top_entity_kinds: vec![],
            ..t
        };
        assert_eq!(fallback_name(&only_mail), "Mary Jones");
    }

    #[test]
    fn suggestions_path_marks_names_provisional() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        suggestions_pass(&s);
        let (name, _, dirty) = row(&c, t);
        assert_eq!((name.as_str(), dirty), ("Ann · Acme · Bob", 0));
        assert!(marks(&c, t).0.is_some());
        assert_eq!(naming_status(&s).unwrap().provisional_count, 1);
    }

    #[test]
    fn model_names_replace_provisional_names() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        suggestions_pass(&s);
        let r = run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| Ok(OK.into())).unwrap();
        assert_eq!(r, NamingReport { named: 1, fallback: 0, model_named: 1 });
        assert_eq!(row(&c, t).0, "Acme pricing");
        assert_eq!(marks(&c, t), (None, None));
        assert_eq!(naming_status(&s).unwrap().provisional_count, 0);
    }

    #[test]
    fn failed_model_attempt_waits_a_day() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        suggestions_pass(&s);
        let calls = RefCell::new(0);
        let run = |now: i64| {
            run_model_naming_batch_with_complete(&s, 5, "", now, |_| {
                *calls.borrow_mut() += 1;
                Err("provider failure".into())
            })
            .unwrap();
        };
        run(1000);
        assert_eq!((*calls.borrow(), marks(&c, t).1), (1, Some(1000)));
        run(1000 + DAY - 1);
        assert_eq!(*calls.borrow(), 1);
        run(1000 + DAY);
        assert_eq!((*calls.borrow(), marks(&c, t).1), (2, Some(1000 + DAY)));
        // The placeholder is still there, not blanked by the failures.
        assert_eq!(row(&c, t).0, "Ann · Acme · Bob");
    }

    #[test]
    fn always_err_path_never_stamps_the_retry_timer() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        suggestions_pass(&s);
        suggestions_pass(&s);
        assert_eq!(marks(&c, t).1, None);
        // A batch that could not call a model (none set up, spend cap) does not stamp either.
        let calls = RefCell::new(0);
        run_model_naming_batch_with_complete(&s, 5, "", 7, |_| {
            *calls.borrow_mut() += 1;
            Err(format!("{NO_MODEL}: spend cap"))
        })
        .unwrap();
        assert_eq!((*calls.borrow(), marks(&c, t).1), (1, None));
    }

    #[test]
    fn user_renamed_topics_are_never_renamed() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        suggestions_pass(&s);
        s.rename_topic(t, "Mine").unwrap();
        let calls = RefCell::new(0);
        let r = run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Ok(OK.into())
        })
        .unwrap();
        assert_eq!((r, *calls.borrow(), row(&c, t).0.as_str()), (NamingReport::default(), 0, "Mine"));
        assert_eq!(naming_status(&s).unwrap().provisional_count, 0);
    }

    #[test]
    fn batch_stops_after_first_model_failure() {
        let (_d, s, c, t1) = seeded(&["a", "b"]);
        make_suggestible(&c, t1);
        let t2 = add_topic(&c, 5);
        let calls = RefCell::new(0);
        let r = run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Err("provider failure".into())
        })
        .unwrap();
        assert_eq!(*calls.borrow(), 1);
        assert_eq!((r.fallback, r.model_named), (1, 0));
        let dirty = |t| row(&c, t).2;
        assert_eq!(dirty(t1) + dirty(t2), 1);
        assert_eq!(naming_status(&s).unwrap().last_naming_failure_unix_ms, Some(1000));
    }

    #[test]
    fn naming_respects_the_limit() {
        let (_d, s, c, t0) = seeded(&["a", "b"]);
        make_suggestible(&c, t0);
        add_topic(&c, 5);
        add_topic(&c, 6);
        let calls = RefCell::new(0);
        let r = run_model_naming_batch_with_complete(&s, 2, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Ok(OK.into())
        })
        .unwrap();
        assert_eq!((r.named, *calls.borrow()), (2, 2));
    }

    #[test]
    fn naming_status_reports_provisional() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        assert_eq!(
            naming_status(&s).unwrap(),
            NamingStatus { provisional_count: 0, last_naming_failure_unix_ms: None }
        );
        suggestions_pass(&s);
        run_model_naming_batch_with_complete(&s, 5, "", 5000, |_| Err("provider failure".into())).unwrap();
        assert_eq!(
            naming_status(&s).unwrap(),
            NamingStatus { provisional_count: 1, last_naming_failure_unix_ms: Some(5000) }
        );
    }

    #[test]
    fn fallback_cleans_plus_tags_names_and_noreply() {
        let t = TopicNameInput {
            id: 1,
            current_name: String::new(),
            top_entities: vec![
                "noreply@x.com".into(),
                "mike+test@x.com".into(),
                "no-reply@y.com".into(),
                "\"Ann Lee\" <ann@x.com>".into(),
                "notifications@github.com".into(),
                "mailer-daemon@x.com".into(),
                "Bob".into(),
            ],
            top_entity_kinds: vec![],
            sample_titles: vec![],
        };
        assert_eq!(fallback_name(&t), "Mike · Ann Lee · Bob");
        let only_noreply = TopicNameInput { top_entities: vec!["noreply@x.com".into()], ..t };
        assert_eq!(fallback_name(&only_noreply), "Untitled topic");
    }

    #[test]
    fn capped_names_end_on_a_whole_word() {
        assert_eq!(
            sanitize("Future Me, Answered: weekly numbers and growth targets", 6, 60),
            "Future Me, Answered: weekly numbers"
        );
        assert_eq!(
            sanitize("The project is moving and the current state is good overall", 9, 140),
            "The project is moving and the current state"
        );
        // Character cap: back to the last whole word, never mid-word.
        assert_eq!(sanitize("alpha beta gamma delta", 10, 13), "alpha beta");
        // Uncut text keeps its joining words.
        assert_eq!(sanitize("Bits and Pieces", 6, 60), "Bits and Pieces");
        // Never empties a one-word cut.
        assert_eq!(sanitize("and and and", 1, 60), "and");
        assert_eq!(sanitize("Überprüfung und Planung für", 3, 60), "Überprüfung und Planung");
    }

    #[test]
    fn fallback_survives_reversed_or_odd_brackets() {
        for e in ["a>b<c@x.com", "><", "<>", "x<", ">y", "é<ü>"] {
            let _ = name_from_email(e);
        }
    }

    #[test]
    fn dismissed_topics_are_never_model_named() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        s.dismiss_suggestion(t).unwrap();
        let calls = RefCell::new(0);
        run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Ok(OK.into())
        })
        .unwrap();
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn small_topics_are_not_model_named() {
        // Two members of one kind: named by the suggestions path's fallback, never by a model.
        let (_d, s, c, t) = seeded(&["a", "b"]);
        let calls = RefCell::new(0);
        run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Ok(OK.into())
        })
        .unwrap();
        assert_eq!(*calls.borrow(), 0);
        // Five members of one kind is still not a suggestion.
        for i in 0..3 {
            c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('episode', ?1, ?1, 'x', 0)", [800 + i]).unwrap();
            let sid = c.last_insert_rowid();
            c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')", [t, sid]).unwrap();
        }
        run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Ok(OK.into())
        })
        .unwrap();
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn provisional_count_matches_listed_suggestions() {
        let (_d, s, c, big) = seeded(&["a", "b"]);
        make_suggestible(&c, big);
        let dismissed = add_topic(&c, 5);
        let small = seeded_small(&c, 6);
        suggestions_pass(&s);
        s.dismiss_suggestion(dismissed).unwrap();
        let _ = small;
        let listed = s.topic_suggestions(50).unwrap();
        assert_eq!(listed.iter().map(|t| t.topic_id).collect::<Vec<_>>(), vec![big]);
        assert_eq!(naming_status(&s).unwrap().provisional_count, 1);
    }

    /// A 2-member, single-kind topic.
    fn seeded_small(c: &rusqlite::Connection, n: i64) -> i64 {
        c.execute("INSERT INTO topic (created_at_unix_ms, updated_at_unix_ms) VALUES (0, 0)", []).unwrap();
        let t = c.last_insert_rowid();
        for i in 0..2 {
            c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('email', ?1, ?1, 'x', 0)", [100 * n + i]).unwrap();
            let sid = c.last_insert_rowid();
            c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')", [t, sid]).unwrap();
        }
        t
    }

    #[test]
    fn unparseable_reply_stamps_the_timer_and_stops() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        let t2 = add_topic(&c, 5);
        let calls = RefCell::new(0);
        let r = run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Ok("not json".into())
        })
        .unwrap();
        assert_eq!((*calls.borrow(), r.model_named), (1, 0));
        let stamped = [t, t2].iter().filter(|x| marks(&c, **x).1 == Some(1000)).count();
        assert_eq!(stamped, 1);
    }

    #[test]
    fn no_model_stops_the_batch_after_one_attempt() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        let t2 = add_topic(&c, 5);
        let calls = RefCell::new(0);
        run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| {
            *calls.borrow_mut() += 1;
            Err(format!("{NO_MODEL}: none"))
        })
        .unwrap();
        assert_eq!(*calls.borrow(), 1);
        assert_eq!((marks(&c, t).1, marks(&c, t2).1), (None, None));
    }

    #[test]
    fn a_named_topic_whose_naming_fails_keeps_its_name_and_is_stamped() {
        let (_d, s, c, t) = seeded(&["a", "b"]);
        make_suggestible(&c, t);
        c.execute("UPDATE topic SET name = 'Old name', name_dirty = 1 WHERE id = ?1", [t]).unwrap();
        let r = run_model_naming_batch_with_complete(&s, 5, "", 1000, |_| Err("provider failure".into())).unwrap();
        assert_eq!(r, NamingReport::default());
        assert_eq!(row(&c, t).0, "Old name");
        assert_eq!(marks(&c, t).1, Some(1000));
        let calls = RefCell::new(0);
        run_model_naming_batch_with_complete(&s, 5, "", 2000, |_| {
            *calls.borrow_mut() += 1;
            Ok(OK.into())
        })
        .unwrap();
        assert_eq!(*calls.borrow(), 0, "waits a day");
    }

    fn name_input() -> TopicNameInput {
        TopicNameInput { id: 1, current_name: String::new(), top_entities: vec!["Acme".into()], top_entity_kinds: vec!["org".into()], sample_titles: vec!["Re: pricing".into()] }
    }

    #[test]
    fn topic_names_es_us_adds_language_line_first() {
        let p = build_prompt(&name_input(), "es-US");
        let at = p.find(&llm::profile::language_line("es-US")).expect("language line present");
        assert!(at < p.find("Name this group").unwrap());
        assert!(p.contains("Keep JSON field names"));
    }

    #[test]
    fn topic_names_empty_lang_has_no_language_line() {
        let p = build_prompt(&name_input(), "");
        assert!(!p.contains("Language:"));
        assert!(p.starts_with("Name this group"));
    }

    #[test]
    fn prompt_is_redacted() {
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let (_d, s, _c, _t) = seeded(&[&format!("key {secret} leaked"), "b"]);
        let seen = RefCell::new(String::new());
        run_naming_batch_with_complete(&s, 5, "", |p| {
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
