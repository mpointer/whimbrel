//! Conversations and topics: which topic an Ask conversation joins on its own
//! (docs/ui-redesign/screens/conversations.md, part 5).

use std::collections::{HashMap, HashSet};

use crate::{ApiError, ApiResult};

/// The topic most of these cited sources belong to, if one clearly does. Each citation maps to
/// the topic item the topic index keeps (a meeting's transcript lines are one item), and
/// several citations of one item count once. A citation with no item (a routine run, a row
/// this device lacks) counts as an item of its own, so unplaced sources dilute a topic. A
/// topic wins only with more than half of all items and at least two.
pub fn majority_topic(store: &memory::Store, refs: &[llm::ContextItemRef]) -> Option<i64> {
    let cited: Vec<Cited> = refs.iter().copied().map(Cited::Ref).collect();
    majority_of(store, &cited, 0)
}

pub(crate) fn kind_and_id(r: &llm::ContextItemRef) -> (&'static str, i64) {
    use llm::ContextItemRef as R;
    match *r {
        R::Capture { id } => ("capture", id),
        R::Transcript { id } => ("transcript", id),
        R::RoutineRun { id } => ("routine_run", id),
        R::Episode { id } => ("episode", id),
        R::Thread { id } => ("thread", id),
        R::Snapshot { id } => ("snapshot", id),
        R::McpItem { id } => ("mcp_item", id),
    }
}

/// One saved citation: a context item, or a meeting. Find's numbered sources can cite a meeting
/// (`{kind: "meeting", id}`), which `llm::ContextItemRef` has no variant for; a meeting is its
/// own topic item, the one its transcript lines map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Cited {
    Ref(llm::ContextItemRef),
    Meeting(i64),
}

/// The topic item a citation counts as (see [`memory::Store::topic_item_for_citation`]).
fn topic_item(store: &memory::Store, c: &Cited) -> rusqlite::Result<Option<(String, i64)>> {
    match c {
        Cited::Ref(r) => {
            let (kind, id) = kind_and_id(r);
            store.topic_item_for_citation(kind, id)
        }
        Cited::Meeting(id) => Ok(Some(("meeting".to_string(), *id))),
    }
}

/// `unparsed` citations the stored JSON held that this build cannot read: each its own item.
fn majority_of(store: &memory::Store, refs: &[Cited], unparsed: usize) -> Option<i64> {
    let mut seen = HashSet::new();
    let mut items: HashSet<(String, i64)> = HashSet::new();
    let mut total = unparsed;
    for r in refs {
        // The same source cited in several answers is still one source.
        if !seen.insert(*r) {
            continue;
        }
        match topic_item(store, r) {
            Ok(Some(item)) => {
                items.insert(item);
            }
            _ => total += 1,
        }
    }
    total += items.len();
    let mut per_topic: HashMap<i64, usize> = HashMap::new();
    for (kind, ref_id) in &items {
        if let Ok(Some(topic)) = store.live_topic_of_item(kind, *ref_id) {
            *per_topic.entry(topic).or_default() += 1;
        }
    }
    let (topic, count) = per_topic.into_iter().max_by_key(|&(t, n)| (n, std::cmp::Reverse(t)))?;
    (count >= 2 && count * 2 > total).then_some(topic)
}

/// The citations of one stored `citations_json`. The apps save a list of entries holding the
/// citation under `item` (`{kind, id}`, as `llm::ContextItemRef` serializes); a bare `{kind, id}`
/// is read too, and so is a meeting (`{kind: "meeting", id}`, which Find's numbered sources save).
/// Returns the readable ones and how many entries were not.
fn parse_citations(json: &str) -> (Vec<Cited>, usize) {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(json) else {
        return (Vec::new(), 0);
    };
    let mut refs = Vec::new();
    let mut unparsed = 0;
    for e in entries {
        let inner = e.get("item").cloned().unwrap_or(e);
        if inner.get("kind").and_then(|k| k.as_str()) == Some("meeting") {
            match inner.get("id").and_then(|i| i.as_i64()) {
                Some(id) => refs.push(Cited::Meeting(id)),
                None => unparsed += 1,
            }
            continue;
        }
        match serde_json::from_value(inner) {
            Ok(r) => refs.push(Cited::Ref(r)),
            Err(_) => unparsed += 1,
        }
    }
    (refs, unparsed)
}

/// Each exchange's cited sources, oldest exchange first, with how many entries could not be read. An
/// exchange saved with numbered sources holds them as `[{n, kind, id}]` (`sources_json`, read through
/// [`memory::Store::get_conversation_export`], which reads no card); one saved before holds its
/// `citations_json`. Both go through [`parse_citations`]. `NotFound` when there is no such conversation.
pub(crate) fn saved_refs(store: &memory::Store, conversation_id: i64) -> ApiResult<Vec<(Vec<Cited>, usize)>> {
    let convo = store
        .get_conversation_export(conversation_id)
        .map_err(|_| ApiError::ProviderFailure)?
        .ok_or(ApiError::NotFound)?;
    let exchanges = convo.get("exchanges").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    Ok(exchanges
        .iter()
        .map(|ex| match ex.get("sources") {
            Some(sources) => parse_citations(&sources.to_string()),
            None => parse_citations(ex.get("citations_json").and_then(|c| c.as_str()).unwrap_or("[]")),
        })
        .collect())
}

/// The live topics the conversation's stored citations belong to, each once, in the order first
/// cited: what its topic chip offers. Each citation maps to its topic item as [`majority_topic`]
/// counts it, and the item to the live topic holding it; a citation with no item or no topic
/// adds nothing. `NotFound` when there is no such conversation.
pub fn cited_topics(store: &memory::Store, conversation_id: i64) -> ApiResult<Vec<memory::TopicRef>> {
    let mut seen_refs = HashSet::new();
    let mut seen_topics = HashSet::new();
    let mut out = Vec::new();
    for (refs, _) in saved_refs(store, conversation_id)? {
        for r in refs {
            if !seen_refs.insert(r) {
                continue;
            }
            let Ok(Some((item_kind, item_id))) = topic_item(store, &r) else { continue };
            let Ok(Some(topic)) = store.live_topic_of_item(&item_kind, item_id) else { continue };
            if !seen_topics.insert(topic) {
                continue;
            }
            if let Ok(Some(topic_ref)) = store.live_topic_ref(topic) {
                out.push(topic_ref);
            }
        }
    }
    Ok(out)
}

/// Joins the conversation to the topic most of its cited sources come from, unless it already
/// has a live topic or a link the user made: once joined it stays, and a topic the user set or
/// removed is theirs (even after that topic is gone). An automatic link whose topic is gone
/// counts as no link (spec call 3). The link is made with `by_user = false`. Returns the
/// conversation's topic after the call. `NotFound` when there is no such conversation.
pub fn auto_attach_topic(
    store: &memory::Store,
    conversation_id: i64,
) -> ApiResult<Option<memory::TopicRef>> {
    let db = |_| ApiError::ProviderFailure;
    let saved = saved_refs(store, conversation_id)?;
    let link = store.conversation_topic(conversation_id).map_err(db)?;
    if link.topic.is_none() && !link.set_by_user {
        let mut refs = Vec::new();
        let mut unparsed = 0;
        for (r, n) in saved {
            refs.extend(r);
            unparsed += n;
        }
        if let Some(topic) = majority_of(store, &refs, unparsed) {
            store
                .set_conversation_topic(conversation_id, Some(topic), false)
                .map_err(db)?;
        }
    }
    Ok(store.conversation_topic(conversation_id).map_err(db)?.topic)
}

/// Whether a saved Ask exchange touches an excluded client's material, so its generated title must never go to a hosted model.
/// Read from the stored rows and the scope, never from what a page says. Fails closed: true when the exclusion setting, the
/// scope's topic or a cited row cannot be read. False only when no client is excluded, or nothing in the exchange touches one.
/// It checks, in order: no active exclusion -> false; the setting closed, or the question or the answer names a client -> true;
/// a meeting or topic scope whose request origin carries a client, or a topic scope that is gone -> true; every cited
/// `(kind, id)` read in full now and run through the hosted-prompt filter (`cited_row_touches`).
pub fn exchange_touches_excluded_client(
    s: &memory::Store,
    scope: &memory::ConversationScope,
    query: &str,
    answer: &str,
    cited: &[(String, i64)],
) -> bool {
    let filter = crate::exclusion::context_filter(s);
    if !filter.is_active() {
        return false;
    }
    if filter.is_closed() || filter.names_client(query) || filter.names_client(answer) {
        return true;
    }
    let origin = match scope {
        memory::ConversationScope::Everything => None,
        memory::ConversationScope::Meeting(id) => Some(crate::exclusion::RequestOrigin::Meeting { meeting_id: *id }),
        memory::ConversationScope::Topic(id) => match s.live_topic_ref(*id) {
            Ok(Some(topic)) => Some(crate::exclusion::RequestOrigin::TopicPage { key: topic.key }),
            // A topic that is gone (deleted or merged) cannot be checked, so it counts.
            Ok(None) | Err(_) => return true,
        },
    };
    if origin.is_some_and(|origin| crate::exclusion::request_carries_client(s, "", origin)) {
        return true;
    }
    cited.iter().any(|(kind, id)| cited_row_touches(s, &filter, kind, *id))
}

/// Whether one cited row, read in full (not as a display card, which cuts its text and leaves out the url, sender, window title
/// and attendees), is an excluded client's. Uses the hosted-prompt filter itself (`ContextFilter::keeps`), so the two cannot
/// drift, and the meeting check `request_carries_client` uses (title and attendees). True for a row that is gone, a meeting that
/// cannot be read, a read error, or a kind with no full read.
fn cited_row_touches(s: &memory::Store, filter: &llm::ContextFilter, kind: &str, id: i64) -> bool {
    use llm::ContextItem as C;
    let meeting_touches = |meeting_id: i64| match s.get_meeting(meeting_id) {
        Ok(Some(m)) => filter.excludes_meeting(m.id, &m.title, &m.attendees),
        _ => true,
    };
    let item = match kind {
        "meeting" => return meeting_touches(id),
        "capture" => s.get_capture_item(id).map(|r| r.map(C::Capture)),
        "transcript" => s.get_transcript_segment(id).map(|r| r.map(C::Transcript)),
        "snapshot" => s.get_snapshot_hit(id).map(|r| r.map(C::Snapshot)),
        "routine_run" => s.get_routine_run_with_name(id).map(|r| r.map(C::RoutineRun)),
        "episode" => s.get_episode(id).map(|r| r.map(C::Episode)),
        "thread" => s.get_thread(id).map(|r| r.map(C::Thread)),
        "mcp_item" => s.added_item_hit(id).map(|r| r.map(C::McpItem)),
        _ => return true,
    };
    let Ok(Some(item)) = item else { return true };
    let meeting = match &item {
        C::Transcript(t) => Some(t.meeting_id),
        C::Snapshot(h) => Some(h.meeting_id),
        _ => None,
    };
    !filter.keeps(&item) || meeting.is_some_and(meeting_touches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::ContextItemRef as R;
    use rusqlite::{params, Connection};
    use tempfile::TempDir;

    /// A file-backed store plus a second connection for filing topic sources, which the
    /// indexer would do in the app.
    struct Env {
        _dir: TempDir,
        store: memory::Store,
        side: Connection,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let store = memory::Store::open(&path, dir.path()).unwrap();
        let side = Connection::open(&path).unwrap();
        side.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        Env { _dir: dir, store, side }
    }

    impl Env {
        fn topic(&self, name: &str) -> i64 {
            self.store.create_seed_topic(name, &[], None).unwrap().topic_id
        }
        /// An episode row, filed under `topic` when given. Returns the episode id.
        fn episode(&self, topic: Option<i64>) -> i64 {
            self.side
                .execute(
                    "INSERT INTO episodes (device_id, app_bundle_id, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES ('d', 'a', 1, 2, 1, 1, 1, 'done', 0, 0)",
                    [],
                )
                .unwrap();
            let id = self.side.last_insert_rowid();
            if let Some(t) = topic {
                self.file("episode", id, t);
            }
            id
        }
        fn file(&self, kind: &str, ref_id: i64, topic: i64) {
            self.side
                .execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, 1, 't', 0)",
                    params![kind, ref_id],
                )
                .unwrap();
            let sid = self.side.last_insert_rowid();
            self.side
                .execute(
                    "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')",
                    params![topic, sid],
                )
                .unwrap();
        }
        /// A transcript line (an `items` row) of `meeting`; the meeting filed under `topic` once.
        fn transcript(&self, meeting: i64) -> i64 {
            self.side
                .execute(
                    "INSERT INTO items (device_id, kind, thread_id, ts_unix_ms, body_text) VALUES ('d', 'transcript', ?1, 1, 'x')",
                    [meeting],
                )
                .unwrap();
            self.side.last_insert_rowid()
        }
        fn convo(&self, cited: &[Vec<R>]) -> i64 {
            let c = self.store.insert_conversation("c", 1).unwrap();
            for refs in cited {
                self.cite(c, refs);
            }
            c
        }
        fn cite(&self, c: i64, refs: &[R]) {
            let saved: Vec<_> = refs.iter().map(|r| serde_json::json!({"item": r, "label": "l", "text": "t", "open": null})).collect();
            self.store
                .append_conversation_exchange(c, "q", "a", &serde_json::to_string(&saved).unwrap(), None, 2)
                .unwrap();
        }
        /// An exchange saved the new way: `[{n, kind, id}]` in `sources_json`, `citations_json` left `[]`.
        fn cite_sourced(&self, c: i64, refs: &[R]) {
            let sources: Vec<memory::cards::SavedSource> = refs
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let (kind, id) = kind_and_id(r);
                    memory::cards::SavedSource { n: i as i64 + 1, kind: kind.into(), id }
                })
                .collect();
            self.cite_saved(c, &sources);
        }
        /// An exchange saved the new way with exactly these pointers, whatever their kind.
        fn cite_saved(&self, c: i64, sources: &[memory::cards::SavedSource]) {
            self.store
                .append_conversation_exchange_sourced(&memory::cards::NewSourcedExchange {
                    conversation_id: c,
                    query: "q",
                    answer: "a",
                    sources,
                    model: None,
                    provider: Some("ollama"),
                    answered_locally: false,
                    left_out: 0,
                    now_unix_ms: 2,
                })
                .unwrap();
        }
    }

    fn eps(e: &Env, topic: Option<i64>, n: usize) -> Vec<R> {
        (0..n).map(|_| R::Episode { id: e.episode(topic) }).collect()
    }

    #[test]
    fn two_moments_of_one_meeting_count_once() {
        let e = env();
        let t = e.topic("Acme");
        e.file("meeting", 7, t);
        let (a, b) = (e.transcript(7), e.transcript(7));
        let ep = e.episode(Some(t));
        let refs = [R::Transcript { id: a }, R::Transcript { id: b }, R::Episode { id: ep }];
        assert_eq!(majority_topic(&e.store, &refs), Some(t));
        assert_eq!(majority_topic(&e.store, &refs[..2]), None, "one item is not enough");
    }

    #[test]
    fn a_topic_needs_more_than_half_and_at_least_two() {
        let e = env();
        let (t, u) = (e.topic("Acme"), e.topic("Birch"));
        let mut tie = eps(&e, Some(t), 2);
        tie.extend(eps(&e, Some(u), 2));
        assert_eq!(majority_topic(&e.store, &tie), None);

        let mut win = eps(&e, Some(t), 3);
        win.extend(eps(&e, Some(u), 2));
        assert_eq!(majority_topic(&e.store, &win), Some(t));

        let mut diluted = eps(&e, Some(t), 2);
        diluted.extend([R::RoutineRun { id: 1 }, R::RoutineRun { id: 2 }, R::RoutineRun { id: 3 }]);
        assert_eq!(majority_topic(&e.store, &diluted), None);

        assert_eq!(majority_topic(&e.store, &[]), None);
    }

    #[test]
    fn cited_topics_are_each_live_topic_once_in_first_cited_order() {
        let e = env();
        let (t, u) = (e.topic("Acme"), e.topic("Birch"));
        e.file("meeting", 7, u);
        let moment = e.transcript(7);
        let first = eps(&e, Some(t), 1);
        let unfiled = eps(&e, None, 1);
        let again = eps(&e, Some(t), 1);
        let c = e.convo(&[
            vec![R::RoutineRun { id: 1 }, unfiled[0], R::Transcript { id: moment }],
            vec![first[0], again[0], R::Transcript { id: moment }],
        ]);
        let names: Vec<String> = cited_topics(&e.store, c).unwrap().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["Birch".to_string(), "Acme".to_string()]);
        let empty = e.convo(&[]);
        assert!(cited_topics(&e.store, empty).unwrap().is_empty());
        assert!(matches!(cited_topics(&e.store, 9999), Err(crate::ApiError::NotFound)));
    }

    #[test]
    fn auto_attach_never_overrides_the_users_choice() {
        let e = env();
        let t = e.topic("Acme");
        let c = e.convo(&[eps(&e, Some(t), 3)]);
        e.store.set_conversation_topic(c, None, true).unwrap();
        assert_eq!(auto_attach_topic(&e.store, c).unwrap(), None);
        let link = e.store.conversation_topic(c).unwrap();
        assert!(link.topic.is_none() && link.set_by_user);
    }

    #[test]
    fn auto_attach_keeps_a_topic_once_joined() {
        let e = env();
        let (t, u) = (e.topic("Acme"), e.topic("Birch"));
        let c = e.convo(&[eps(&e, Some(t), 2)]);
        assert_eq!(auto_attach_topic(&e.store, c).unwrap().unwrap().topic_id, t);
        assert!(!e.store.conversation_topic(c).unwrap().set_by_user);
        e.cite(c, &eps(&e, Some(u), 5));
        assert_eq!(auto_attach_topic(&e.store, c).unwrap().unwrap().topic_id, t);
    }

    #[test]
    fn auto_attach_replaces_an_automatic_link_whose_topic_is_gone_but_not_the_users() {
        let e = env();
        let (gone, birch) = (e.store.create_seed_topic("Gone", &[], None).unwrap(), e.topic("Birch"));
        // Joined automatically, then its topic is deleted: the next attach finds the majority again.
        let auto = e.convo(&[eps(&e, Some(gone.topic_id), 2)]);
        assert_eq!(auto_attach_topic(&e.store, auto).unwrap().unwrap().topic_id, gone.topic_id);
        // Set by the user to the same topic before it goes: their choice stays, even with no topic left.
        let mine = e.convo(&[eps(&e, Some(birch), 3)]);
        e.store.set_conversation_topic(mine, Some(gone.topic_id), true).unwrap();
        e.store.delete_seed_topic(&gone.key).unwrap();
        e.cite(auto, &eps(&e, Some(birch), 5));
        assert_eq!(auto_attach_topic(&e.store, auto).unwrap().unwrap().topic_id, birch);
        assert!(!e.store.conversation_topic(auto).unwrap().set_by_user);
        assert_eq!(auto_attach_topic(&e.store, mine).unwrap(), None);
        assert!(e.store.conversation_topic(mine).unwrap().set_by_user);
    }

    #[test]
    fn auto_attach_counts_across_exchanges_and_ignores_a_missing_conversation() {
        let e = env();
        let t = e.topic("Acme");
        let c = e.convo(&[eps(&e, Some(t), 1), eps(&e, Some(t), 1)]);
        assert_eq!(auto_attach_topic(&e.store, c).unwrap().unwrap().topic_id, t);
        assert!(matches!(auto_attach_topic(&e.store, 9999), Err(crate::ApiError::NotFound)));
        let none = e.convo(&[eps(&e, None, 3)]);
        assert_eq!(auto_attach_topic(&e.store, none).unwrap(), None);
        assert!(!e.store.conversation_has_topic_link(none).unwrap(), "no majority leaves no link");
    }

    #[test]
    fn a_citation_maps_to_the_item_the_topic_index_keys() {
        let e = env();
        let t = e.topic("Acme");
        e.file("meeting", 7, t);
        let tr = e.transcript(7);
        assert_eq!(e.store.topic_item_for_citation("transcript", tr).unwrap(), Some(("meeting".into(), 7)));
        assert_eq!(e.store.topic_item_for_citation("routine_run", 1).unwrap(), None);
        assert_eq!(e.store.topic_item_for_citation("transcript", 999).unwrap(), None);
        assert_eq!(e.store.live_topic_of_item("meeting", 7).unwrap(), Some(t));
        assert_eq!(e.store.live_topic_of_item("meeting", 8).unwrap(), None);
    }

    #[test]
    fn captures_snapshots_and_threads_map_to_their_indexed_items() {
        let e = env();
        let ins = |sql: &str| {
            e.side.execute(sql, []).unwrap();
            e.side.last_insert_rowid()
        };
        // Captures 1..=2 sit in the episode covering items 1..=2 of device d.
        let c1 = ins("INSERT INTO items (device_id, kind, ts_unix_ms, body_text) VALUES ('d', 'capture', 1, 'x')");
        let c2 = ins("INSERT INTO items (device_id, kind, ts_unix_ms, body_text) VALUES ('d', 'capture', 1, 'x')");
        let ep = ins(&format!("INSERT INTO episodes (device_id, app_bundle_id, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d', 'a', 1, 2, {c1}, {c2}, 2, 'done', 0, 0)"));
        let want = Some(("episode".to_string(), ep));
        assert_eq!(e.store.topic_item_for_citation("capture", c1).unwrap(), want);
        assert_eq!(e.store.topic_item_for_citation("capture", c2).unwrap(), want);
        let snap = ins("INSERT INTO items (device_id, kind, thread_id, ts_unix_ms, body_text) VALUES ('d', 'snapshot', 3, 1, 'x')");
        assert_eq!(e.store.topic_item_for_citation("snapshot", snap).unwrap(), Some(("snapshot".into(), snap)));
        let th = ins("INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('gmail', 'g1', 'me@x.com', 's', 1, 1, 'a@b.c', 'unresolved', 0, 0)");
        let mail = ins("INSERT INTO items (device_id, kind, ts_unix_ms, body_text, refs) VALUES ('d', 'email', 1, 'x', '{\"gmail_thread_id\":\"g1\",\"account_email\":\"me@x.com\"}')");
        assert_eq!(e.store.topic_item_for_citation("thread", th).unwrap(), None, "not indexed yet");
        let t = e.topic("Acme");
        e.file("email", mail, t);
        assert_eq!(e.store.topic_item_for_citation("thread", th).unwrap(), Some(("email".into(), mail)));
    }

    #[test]
    fn a_sourced_exchange_saves_no_citations_json_and_its_pointers_are_read() {
        let e = env();
        let t = e.topic("Acme");
        let c = e.store.insert_conversation("c", 1).unwrap();
        let refs = eps(&e, Some(t), 3);
        e.cite_sourced(c, &refs);
        assert_eq!(e.store.get_conversation(c).unwrap().unwrap().exchanges[0].citations_json, "[]");
        let saved = saved_refs(&e.store, c).unwrap();
        assert_eq!(saved, vec![(refs.iter().copied().map(Cited::Ref).collect::<Vec<_>>(), 0)]);
        assert_eq!(majority_topic(&e.store, &refs), Some(t));
    }

    #[test]
    fn auto_attach_reads_a_sourced_exchange_and_counts_it_with_a_legacy_one() {
        let e = env();
        let (t, u) = (e.topic("Acme"), e.topic("Birch"));
        let c = e.store.insert_conversation("c", 1).unwrap();
        e.cite_sourced(c, &eps(&e, Some(t), 2));
        assert_eq!(auto_attach_topic(&e.store, c).unwrap().unwrap().topic_id, t);
        // Mixed: one legacy exchange for Birch, one sourced for Birch, outvote a sourced Acme.
        let m = e.store.insert_conversation("m", 1).unwrap();
        e.cite(m, &eps(&e, Some(u), 2));
        e.cite_sourced(m, &eps(&e, Some(u), 1));
        e.cite_sourced(m, &eps(&e, Some(t), 2));
        assert_eq!(auto_attach_topic(&e.store, m).unwrap().unwrap().topic_id, u);
    }

    /// Find saves a cited meeting as `{n, kind: "meeting", id}`. That is the meeting's own topic item, so it
    /// counts toward the meeting's topic (once with that meeting's transcript lines), not as an unread source.
    #[test]
    fn a_saved_meeting_source_counts_as_its_meeting() {
        use memory::cards::SavedSource as S;
        let s = |n: i64, kind: &str, id: i64| S { n, kind: kind.into(), id };
        let e = env();
        let (t, u) = (e.topic("Acme"), e.topic("Birch"));
        e.file("meeting", 7, t);
        e.file("meeting", 8, t);
        let c = e.store.insert_conversation("c", 1).unwrap();
        e.cite_saved(c, &[s(1, "meeting", 7), s(2, "meeting", 8), s(3, "episode", e.episode(Some(u)))]);
        assert_eq!(auto_attach_topic(&e.store, c).unwrap().unwrap().topic_id, t);
        let names: Vec<String> = cited_topics(&e.store, c).unwrap().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["Acme".to_string(), "Birch".to_string()]);

        // A meeting and a line of it are one item: with the other Acme meeting that is two of three.
        let d = e.store.insert_conversation("d", 1).unwrap();
        let line = e.transcript(7);
        e.cite_saved(d, &[s(1, "meeting", 7), s(2, "transcript", line), s(3, "meeting", 8), s(4, "routine_run", 1)]);
        assert_eq!(auto_attach_topic(&e.store, d).unwrap().unwrap().topic_id, t);

        // A cited meeting no topic holds still counts as an item: two Acme meetings of four is not a majority.
        let x = e.store.insert_conversation("x", 1).unwrap();
        e.cite_saved(x, &[s(1, "meeting", 7), s(2, "meeting", 8), s(3, "meeting", 9), s(4, "meeting", 10)]);
        assert_eq!(auto_attach_topic(&e.store, x).unwrap(), None);
    }

    #[test]
    fn cited_topics_read_a_sourced_exchange_in_first_cited_order() {
        let e = env();
        let (t, u) = (e.topic("Acme"), e.topic("Birch"));
        let c = e.store.insert_conversation("c", 1).unwrap();
        e.cite_sourced(c, &eps(&e, Some(u), 1));
        e.cite(c, &eps(&e, Some(t), 1));
        let names: Vec<String> = cited_topics(&e.store, c).unwrap().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["Birch".to_string(), "Acme".to_string()]);
    }

    // ---- exchange_touches_excluded_client ----

    fn exclude(e: &Env, name: &str) -> memory::TopicRef {
        let t = e.store.create_seed_topic(name, &[], None).unwrap();
        e.store
            .set_client_control(&t.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        e.store.live_topic_ref(t.topic_id).unwrap().unwrap()
    }

    /// A completed meeting titled `title` with one transcript line `text`; returns (meeting id, the line's item id).
    fn meeting(e: &Env, title: &str, text: &str) -> (i64, i64) {
        let id = e
            .store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: title.into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        e.store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "Ben".into(),
                text: text.into(),
            }])
            .unwrap();
        let line = e
            .side
            .query_row("SELECT id FROM items WHERE kind = 'transcript' AND thread_id = ?1", [id], |r| r.get(0))
            .unwrap();
        (id, line)
    }

    fn touches(e: &Env, scope: memory::ConversationScope, q: &str, a: &str, cited: &[(&str, i64)]) -> bool {
        let cited: Vec<(String, i64)> = cited.iter().map(|(k, i)| (k.to_string(), *i)).collect();
        exchange_touches_excluded_client(&e.store, &scope, q, a, &cited)
    }

    #[test]
    fn no_exclusion_means_nothing_touches_an_excluded_client() {
        let e = env();
        let (m, line) = meeting(&e, "Acme sync", "Acme budget");
        let all = memory::ConversationScope::Everything;
        assert!(!touches(&e, all, "what about Acme?", "Acme said yes", &[("transcript", line), ("transcript", 99_999)]));
        assert!(!touches(&e, memory::ConversationScope::Meeting(m), "q", "a", &[]));
    }

    #[test]
    fn a_client_named_in_the_question_or_the_answer_touches() {
        let e = env();
        exclude(&e, "Acme");
        let all = memory::ConversationScope::Everything;
        assert!(touches(&e, all, "what did Acme decide?", "Nothing.", &[]));
        assert!(touches(&e, all, "what was decided?", "The ACME contract moved.", &[]));
    }

    #[test]
    fn a_clean_exchange_with_exclusions_on_does_not_touch() {
        let e = env();
        exclude(&e, "Acme");
        let birch = e.topic("Birch");
        let (m, line) = meeting(&e, "Birch review", "the roadmap");
        assert!(!touches(&e, memory::ConversationScope::Everything, "what is the roadmap?", "A plan.", &[("transcript", line)]));
        assert!(!touches(&e, memory::ConversationScope::Meeting(m), "q", "a", &[("transcript", line)]));
        assert!(!touches(&e, memory::ConversationScope::Topic(birch), "q", "a", &[]));
    }

    #[test]
    fn a_cited_row_of_an_excluded_meeting_touches() {
        let e = env();
        exclude(&e, "Acme");
        let (m, line) = meeting(&e, "Acme sync", "the budget");
        let all = memory::ConversationScope::Everything;
        assert!(touches(&e, all, "what was the budget?", "Ten.", &[("transcript", line)]));
        assert!(touches(&e, all, "q", "a", &[("meeting", m)]));
    }

    #[test]
    fn a_cited_row_whose_text_names_a_client_touches() {
        let e = env();
        exclude(&e, "Acme");
        let (_, line) = meeting(&e, "Weekly", "we owe Acme a reply");
        assert!(touches(&e, memory::ConversationScope::Everything, "q", "a", &[("transcript", line)]));
    }

    #[test]
    fn a_cited_pointer_that_no_longer_exists_fails_closed() {
        let e = env();
        exclude(&e, "Acme");
        let (_, line) = meeting(&e, "Birch review", "the roadmap");
        let all = memory::ConversationScope::Everything;
        assert!(!touches(&e, all, "q", "a", &[("transcript", line)]));
        assert!(touches(&e, all, "q", "a", &[("transcript", line), ("transcript", 99_999)]));
        assert!(touches(&e, all, "q", "a", &[("capture", 99_999)]));
    }

    #[test]
    fn the_scope_of_an_excluded_meeting_or_topic_touches() {
        let e = env();
        let acme = exclude(&e, "Acme");
        let (m, _) = meeting(&e, "Acme sync", "the budget");
        assert!(touches(&e, memory::ConversationScope::Meeting(m), "what was said?", "Not much.", &[]));
        assert!(touches(&e, memory::ConversationScope::Topic(acme.topic_id), "what is next?", "A call.", &[]));
    }

    #[test]
    fn a_topic_scope_that_is_gone_fails_closed() {
        let e = env();
        exclude(&e, "Acme");
        assert!(touches(&e, memory::ConversationScope::Topic(987_654), "q", "a", &[]));
    }

    #[test]
    fn a_cited_meeting_excluded_by_attendee_only_touches() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &["Wile Coyote".to_string()], None).unwrap();
        e.store
            .set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        let id = e
            .store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Weekly".into(),
                attendees: vec!["Wile Coyote".into()],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        e.store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "Ben".into(),
                text: "the budget".into(),
            }])
            .unwrap();
        let line: i64 = e
            .side
            .query_row("SELECT id FROM items WHERE kind = 'transcript' AND thread_id = ?1", [id], |r| r.get(0))
            .unwrap();
        let all = memory::ConversationScope::Everything;
        assert!(touches(&e, all, "q", "a", &[("meeting", id)]));
        assert!(touches(&e, all, "q", "a", &[("transcript", line)]));
        assert!(touches(&e, all, "q", "a", &[("meeting", 99_999)]), "a meeting that is gone");
    }

    #[test]
    fn a_client_only_past_the_cards_cut_or_in_the_url_touches() {
        let e = env();
        exclude(&e, "Acme");
        let long = format!("{} then we talked to Acme", "filler words ".repeat(400));
        let (_, line) = meeting(&e, "Weekly", &long);
        // The card cuts its text; the full row does not.
        let card = &e.store.resolve_cards(&[("transcript".to_string(), line)]).unwrap()[0];
        assert!(!card.text.as_deref().unwrap_or("").to_lowercase().contains("acme"), "the card cut the name off");
        assert!(touches(&e, memory::ConversationScope::Everything, "q", "a", &[("transcript", line)]));
        // A capture whose client is only in its url.
        e.side
            .execute(
                "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, url, body_text, hash) \
                 VALUES ('d', 'capture', 1, 'com.app', 'Home', 'https://acme.example/plan', 'nothing here', 'h1')",
                [],
            )
            .unwrap();
        let cap = e.side.last_insert_rowid();
        assert!(touches(&e, memory::ConversationScope::Everything, "q", "a", &[("capture", cap)]));
    }
}
