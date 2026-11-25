//! Display records for what Ask drew on and what a search found, read from the live rows, and
//! the numbered sources an answer saves (screen 5, memory search and Ask).
//!
//! A [`SourceCard`] is built from the stored row every time it is asked for. Nothing here keeps a
//! copy of the words a card shows. A saved answer holds only `[{n, kind, id}]` for the sources it
//! cited ([`SavedSource`]) beside its own text, so a trim, a forget, retention or a client delete
//! that removes the row removes the card's text from every old answer too: the card comes back
//! with `found: false`, no title and no text. The columns this adds to `conversation_exchange`
//! (`sources_json`, `answer_provider`, `answered_locally`, `left_out`) hold ids, a provider id and
//! two small numbers, so a client's name or words cannot be in them. They are local, never synced,
//! and go with the row on a conversation delete or a retention sweep.
//!
//! Exclusion: a card is shown on this computer to the person whose memory it is and is never put
//! in a prompt. Neither `citations::resolve` nor the Mac's chips filter an excluded client's rows
//! for display today, so this does not either; what an excluded client changes is what a hosted
//! model is sent, and that stays in the Ask path.

use std::collections::{HashMap, HashSet};

use rusqlite::params_from_iter;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

use crate::Store;

/// The longest stored text a card carries, in characters. Longer text is cut and ends in an ellipsis.
pub const CARD_TEXT_CHARS: usize = 600;

/// The kinds a card or a saved source can name: the `ContextItemRef` tags, and `meeting` for a
/// search hit on a meeting row.
pub const CARD_KINDS: [&str; 8] = ["capture", "transcript", "snapshot", "thread", "episode", "routine_run", "meeting", "mcp_item"];

/// What a card shows for one row. Keys that do not apply are absent from the JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceCard {
    /// `capture`, `transcript`, `snapshot` (a slide), `thread`, `episode`, `routine_run` or `meeting`.
    pub kind: String,
    pub id: i64,
    /// `false` when the row is gone from this device: no title, no text.
    pub found: bool,
    /// The row's own time. A transcript line: its absolute time. A slide or a meeting: the
    /// meeting's start. A thread: its last activity. An episode: when it ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts_unix_ms: Option<i64>,
    /// Window title (or the app) for a capture, the meeting's title for a line, a slide or a
    /// meeting, the subject for a thread, the window title for an episode, the routine's name for a run.
    pub title: String,
    /// The stored text, cut to [`CARD_TEXT_CHARS`]. Never the model's wording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Captures and episodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Lines, slides and meetings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meeting_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meeting_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meeting_start_unix_ms: Option<i64>,
    /// A transcript line's speaker: the person's name when the meeting names that speaker, else the stored label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_label: Option<String>,
    /// Whether that name is a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_is_guess: Option<bool>,
    /// A transcript line's offset into its meeting, in centiseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_cs: Option<i64>,
    /// A thread's last sender.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// Search only: why a transcript line matched a person (`speaker` or `attendee`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// An added item (`mcp_item`): what it said it was.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub declared_kind: Option<String>,
    /// An added item: the client that added it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub added_by: Option<String>,
    /// An added item: its source label.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_label: Option<String>,
    /// An added item: its link (https only).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub url: Option<String>,
}

impl SourceCard {
    fn gone(kind: &str, id: i64) -> Self {
        SourceCard { kind: kind.to_string(), id, found: false, ..Default::default() }
    }
}

/// `text` cut to [`CARD_TEXT_CHARS`] characters at a character boundary, with an ellipsis when cut.
pub fn cut_text(text: &str) -> String {
    match text.char_indices().nth(CARD_TEXT_CHARS) {
        Some((end, _)) => format!("{}\u{2026}", text[..end].trim_end()),
        None => text.to_string(),
    }
}

/// One source an answer cited, as saved: its number in the prompt, its kind and its id. Nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSource {
    pub n: i64,
    pub kind: String,
    pub id: i64,
}

/// An exchange to save with its cited sources (see [`Store::append_conversation_exchange_sourced`]).
#[derive(Debug, Clone)]
pub struct NewSourcedExchange<'a> {
    pub conversation_id: i64,
    pub query: &'a str,
    pub answer: &'a str,
    /// The cited sources only; entries with an unknown kind or a number below 1 are dropped.
    pub sources: &'a [SavedSource],
    /// The model that answered, as the old append stores it.
    pub model: Option<&'a str>,
    /// The provider id that answered (`anthropic`, `ollama`, ...). Never a client's name.
    pub provider: Option<&'a str>,
    /// Whether the answer was made on this computer.
    pub answered_locally: bool,
    /// How many retrieved items exclusion left out of a hosted prompt.
    pub left_out: i64,
    pub now_unix_ms: i64,
}

/// A saved cited source with its card read from the live row now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SavedSourceCard {
    pub n: i64,
    pub card: SourceCard,
    /// The card could not be read just now (a database error): `card.found` is false, but the row may well
    /// exist. Never the same as a row that is gone.
    pub unreadable: bool,
}

/// One exchange of a conversation read with its sources.
#[derive(Debug, Clone, Serialize)]
pub struct SourcedExchange {
    pub id: i64,
    pub query: String,
    pub answer: String,
    pub created_at_unix_ms: i64,
    pub model: Option<String>,
    /// Saved before numbered sources existed (by the Mac's `SavedCitation` or Windows' `ContextEntry`).
    /// Its `citations_json` is returned untouched for the app to draw today's chips from; `sources` is empty.
    pub legacy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations_json: Option<String>,
    pub provider: Option<String>,
    pub answered_locally: Option<bool>,
    pub left_out: Option<i64>,
    /// The numbers the saved answer cited, in first-cited order: the `n` of each of `sources`. A UI draws a
    /// mark for `[n]` in `answer` only when `n` is in this list. Empty for a legacy exchange.
    pub cited: Vec<usize>,
    /// Cited sources in first-cited order, each with its card read from the live row now.
    pub sources: Vec<SavedSourceCard>,
}

/// A conversation read with its sources.
#[derive(Debug, Clone, Serialize)]
pub struct SourcedConversation {
    pub id: i64,
    pub title: String,
    pub starred: bool,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub exchanges: Vec<SourcedExchange>,
}

const CHUNK: usize = 400;

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

impl Store {
    /// Adds the local columns a sourced exchange saves. Present columns are left alone, so a second
    /// open writes nothing.
    pub(crate) fn ensure_conversation_exchange_ask_columns(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        let existing: Vec<String> = conn
            .prepare("PRAGMA table_info(conversation_exchange)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        for (name, decl) in [
            ("sources_json", "TEXT"),
            ("answer_provider", "TEXT"),
            ("answered_locally", "INTEGER"),
            ("left_out", "INTEGER"),
        ] {
            if !existing.iter().any(|c| c == name) {
                conn.execute(&format!("ALTER TABLE conversation_exchange ADD COLUMN {name} {decl}"), [])?;
            }
        }
        Ok(())
    }

    /// The cards for `(kind, id)` refs, in input order, each read from its live row. A row that is
    /// gone gives `found: false` with no title and no text. Reads a few rows per kind, never one
    /// query per ref.
    pub fn resolve_cards(&self, refs: &[(String, i64)]) -> rusqlite::Result<Vec<SourceCard>> {
        let mut by_kind: HashMap<&str, Vec<i64>> = HashMap::new();
        for (kind, id) in refs {
            by_kind.entry(kind.as_str()).or_default().push(*id);
        }
        let mut found: HashMap<(String, i64), SourceCard> = HashMap::new();
        for (kind, ids) in by_kind {
            let mut ids: Vec<i64> = ids;
            ids.sort_unstable();
            ids.dedup();
            for chunk in ids.chunks(CHUNK) {
                for card in self.cards_of_kind(kind, chunk)? {
                    found.insert((card.kind.clone(), card.id), card);
                }
            }
        }
        Ok(refs
            .iter()
            .map(|(kind, id)| found.get(&(kind.clone(), *id)).cloned().unwrap_or_else(|| SourceCard::gone(kind, *id)))
            .collect())
    }

    fn cards_of_kind(&self, kind: &str, ids: &[i64]) -> rusqlite::Result<Vec<SourceCard>> {
        let list = placeholders(ids.len());
        let vals = || params_from_iter(ids.iter().map(|i| Value::Integer(*i)));
        match kind {
            "mcp_item" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT id, ts_unix_ms, coalesce(source, ''), title, body_text, url, coalesce(refs, '{{}}'), device_id \
                     FROM items WHERE kind = 'mcp_item' AND id IN ({list})"
                ))?;
                let rows = stmt.query_map(vals(), |r| {
                    let source: String = r.get(2)?;
                    let refs: serde_json::Value = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
                    let text = |k: &str| refs[k].as_str().map(str::to_string);
                    Ok(SourceCard {
                        kind: "mcp_item".into(),
                        id: r.get(0)?,
                        found: true,
                        ts_unix_ms: Some(r.get(1)?),
                        title: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        text: Some(cut_text(&r.get::<_, String>(4)?)),
                        device_id: Some(r.get(7)?),
                        declared_kind: text("as"),
                        added_by: text("client"),
                        source_label: crate::mcp_items::label_of_source(&source).map(str::to_string),
                        url: r.get(5)?,
                        ..Default::default()
                    })
                })?;
                rows.collect()
            }
            "capture" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT id, device_id, ts_unix_ms, app_bundle_id, title, body_text FROM items WHERE kind = 'capture' AND id IN ({list})"
                ))?;
                let rows = stmt.query_map(vals(), |r| {
                    let (app, title): (String, String) = (r.get(3)?, r.get(4)?);
                    Ok(SourceCard {
                        kind: "capture".into(),
                        id: r.get(0)?,
                        found: true,
                        ts_unix_ms: Some(r.get(2)?),
                        title: if title.is_empty() { app.clone() } else { title },
                        text: Some(cut_text(&r.get::<_, String>(5)?)),
                        app_bundle_id: Some(app),
                        device_id: Some(r.get(1)?),
                        ..Default::default()
                    })
                })?;
                rows.collect()
            }
            "transcript" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT i.id, i.thread_id, m.title, m.start_unix_ms, m.device_id, i.ts_unix_ms, json_extract(i.actors, '$[0]'), i.body_text \
                     FROM items i JOIN meeting m ON m.id = i.thread_id WHERE i.kind = 'transcript' AND i.id IN ({list})"
                ))?;
                let mut cards: Vec<SourceCard> = stmt
                    .query_map(vals(), |r| {
                        let (start, ts): (i64, i64) = (r.get(3)?, r.get(5)?);
                        Ok(SourceCard {
                            kind: "transcript".into(),
                            id: r.get(0)?,
                            found: true,
                            ts_unix_ms: Some(ts),
                            title: r.get(2)?,
                            text: Some(cut_text(&r.get::<_, String>(7)?)),
                            device_id: Some(r.get(4)?),
                            meeting_id: Some(r.get(1)?),
                            meeting_title: Some(r.get(2)?),
                            meeting_start_unix_ms: Some(start),
                            speaker_label: Some(r.get::<_, Option<String>>(6)?.unwrap_or_default()),
                            start_cs: Some((ts - start) / 10),
                            ..Default::default()
                        })
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                self.name_speakers(&mut cards)?;
                Ok(cards)
            }
            "snapshot" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT i.id, i.thread_id, m.title, m.start_unix_ms, m.device_id, i.body_text \
                     FROM items i JOIN meeting m ON m.id = i.thread_id WHERE i.kind = 'snapshot' AND i.id IN ({list})"
                ))?;
                let rows = stmt.query_map(vals(), |r| {
                    Ok(SourceCard {
                        kind: "snapshot".into(),
                        id: r.get(0)?,
                        found: true,
                        ts_unix_ms: Some(r.get(3)?),
                        title: r.get(2)?,
                        text: Some(cut_text(&r.get::<_, String>(5)?)),
                        device_id: Some(r.get(4)?),
                        meeting_id: Some(r.get(1)?),
                        meeting_title: Some(r.get(2)?),
                        meeting_start_unix_ms: Some(r.get(3)?),
                        ..Default::default()
                    })
                })?;
                rows.collect()
            }
            "thread" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT id, subject, last_activity_ts_unix_ms, last_message_from, summary FROM threads WHERE id IN ({list})"
                ))?;
                let rows = stmt.query_map(vals(), |r| {
                    let subject: Option<String> = r.get(1)?;
                    let summary: Option<String> = r.get(4)?;
                    Ok(SourceCard {
                        kind: "thread".into(),
                        id: r.get(0)?,
                        found: true,
                        ts_unix_ms: Some(r.get(2)?),
                        title: subject.unwrap_or_default(),
                        // An email's words can hold a credential: redacted, as the thread's text in a prompt is.
                        text: summary.filter(|s| !s.trim().is_empty()).map(|s| cut_text(&crate::redact::redact_secrets(&s))),
                        sender: Some(crate::redact::redact_secrets(&r.get::<_, String>(3)?).into_owned()),
                        ..Default::default()
                    })
                })?;
                rows.collect()
            }
            "episode" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT id, device_id, app_bundle_id, window_title, end_ts_unix_ms, summary FROM episodes WHERE id IN ({list})"
                ))?;
                let rows = stmt.query_map(vals(), |r| {
                    let (app, title): (String, Option<String>) = (r.get(2)?, r.get(3)?);
                    Ok(SourceCard {
                        kind: "episode".into(),
                        id: r.get(0)?,
                        found: true,
                        ts_unix_ms: Some(r.get(4)?),
                        title: title.filter(|t| !t.is_empty()).unwrap_or_else(|| app.clone()),
                        text: r.get::<_, Option<String>>(5)?.filter(|s| !s.trim().is_empty()).map(|s| cut_text(&s)),
                        app_bundle_id: Some(app),
                        device_id: Some(r.get(1)?),
                        ..Default::default()
                    })
                })?;
                rows.collect()
            }
            "routine_run" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT rr.id, r.name, rr.ran_at_unix_ms, rr.output_summary, rr.origin_device_id \
                     FROM routine_run rr JOIN routine r ON r.id = rr.routine_id WHERE rr.id IN ({list})"
                ))?;
                let rows = stmt.query_map(vals(), |r| {
                    Ok(SourceCard {
                        kind: "routine_run".into(),
                        id: r.get(0)?,
                        found: true,
                        ts_unix_ms: Some(r.get(2)?),
                        title: r.get(1)?,
                        text: Some(cut_text(&r.get::<_, String>(3)?)),
                        device_id: r.get(4)?,
                        ..Default::default()
                    })
                })?;
                rows.collect()
            }
            "meeting" => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT id, device_id, start_unix_ms, title, attendees FROM meeting WHERE id IN ({list})"
                ))?;
                let rows = stmt.query_map(vals(), |r| {
                    let attendees: Vec<String> = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default();
                    Ok(SourceCard {
                        kind: "meeting".into(),
                        id: r.get(0)?,
                        found: true,
                        ts_unix_ms: Some(r.get(2)?),
                        title: r.get(3)?,
                        text: (!attendees.is_empty()).then(|| cut_text(&format!("With {}", attendees.join(", ")))),
                        device_id: Some(r.get(1)?),
                        meeting_id: Some(r.get(0)?),
                        meeting_title: Some(r.get(3)?),
                        meeting_start_unix_ms: Some(r.get(2)?),
                        ..Default::default()
                    })
                })?;
                rows.collect()
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Replaces "Speaker N" on transcript cards with the name the meeting gives that speaker. One read per meeting.
    fn name_speakers(&self, cards: &mut [SourceCard]) -> rusqlite::Result<()> {
        let mut speakers: HashMap<i64, Vec<crate::speaker_names::MeetingSpeaker>> = HashMap::new();
        for card in cards.iter_mut() {
            let (Some(meeting), Some(label)) = (card.meeting_id, card.speaker_label.clone()) else { continue };
            if !speakers.contains_key(&meeting) {
                speakers.insert(meeting, self.meeting_speakers(meeting)?);
            }
            let (name, guess) = crate::speaker_names::display_label(&label, &speakers[&meeting]);
            card.speaker_is_guess = (name != label).then_some(guess);
            card.speaker_label = Some(name);
        }
        Ok(())
    }

    /// [`Store::append_conversation_exchange`] for a new-page answer: the same stored-as-local-only
    /// marking and the same drop when the conversation is gone, and the cited sources as
    /// `[{n, kind, id}]` only, with the provider, whether it was answered on this computer and the
    /// left-out count. `citations_json` is stored empty, so no old chip is saved.
    pub fn append_conversation_exchange_sourced(&self, e: &NewSourcedExchange<'_>) -> rusqlite::Result<()> {
        let local_only = self.is_local_only_text(e.answer)?;
        let mut seen = HashSet::new();
        let kept: Vec<&SavedSource> =
            e.sources.iter().filter(|s| s.n >= 1 && CARD_KINDS.contains(&s.kind.as_str()) && seen.insert(s.n)).collect();
        let sources_json = serde_json::to_string(&kept).unwrap_or_else(|_| "[]".into());
        self.conn.execute(
            "INSERT INTO conversation_exchange (conversation_id, query, answer, citations_json, model, created_at_unix_ms, local_only, \
                                                sources_json, answer_provider, answered_locally, left_out) \
             SELECT ?1, ?2, ?3, '[]', ?4, ?5, ?6, ?7, ?8, ?9, ?10 WHERE EXISTS (SELECT 1 FROM conversation WHERE id = ?1)",
            rusqlite::params![
                e.conversation_id,
                e.query,
                e.answer,
                e.model,
                e.now_unix_ms,
                local_only,
                sources_json,
                e.provider,
                e.answered_locally,
                e.left_out.max(0)
            ],
        )?;
        self.conn.execute(
            "UPDATE conversation SET updated_at_unix_ms = ?1 WHERE id = ?2",
            rusqlite::params![e.now_unix_ms, e.conversation_id],
        )?;
        Ok(())
    }

    /// A conversation as the exports write it: its fields and every exchange, with what a new-page
    /// answer holds beyond the old columns. An exchange saved with numbered sources gets `sources` (the
    /// saved `{n, kind, id}` only, never their words), `provider`, `answered_locally` and `left_out`; one
    /// saved before them (`sources_json` NULL) is exactly the old shape, with none of those keys.
    pub fn get_conversation_export(&self, id: i64) -> rusqlite::Result<Option<serde_json::Value>> {
        let Some(c) = self.get_conversation(id)? else { return Ok(None) };
        let mut stmt = self.conn.prepare(
            "SELECT id, sources_json, answer_provider, answered_locally, left_out FROM conversation_exchange WHERE conversation_id = ?1",
        )?;
        let extra: HashMap<i64, (Option<String>, Option<String>, Option<bool>, Option<i64>)> = stmt
            .query_map(rusqlite::params![id], |r| Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?
            .collect::<rusqlite::Result<_>>()?;
        let exchanges: Vec<serde_json::Value> = c
            .exchanges
            .iter()
            .map(|x| {
                let mut v = serde_json::to_value(x).unwrap_or(serde_json::Value::Null);
                if let (Some(obj), Some((Some(sources), provider, answered_locally, left_out))) = (v.as_object_mut(), extra.get(&x.id)) {
                    let saved: Vec<SavedSource> = serde_json::from_str(sources).unwrap_or_default();
                    obj.insert("sources".into(), serde_json::to_value(saved).unwrap_or_default());
                    obj.insert("provider".into(), serde_json::json!(provider));
                    obj.insert("answered_locally".into(), serde_json::json!(answered_locally));
                    obj.insert("left_out".into(), serde_json::json!(left_out));
                }
                v
            })
            .collect();
        Ok(Some(serde_json::json!({
            "id": c.id, "title": c.title, "starred": c.starred,
            "created_at_unix_ms": c.created_at_unix_ms, "updated_at_unix_ms": c.updated_at_unix_ms,
            "exchanges": exchanges,
        })))
    }

    /// A conversation with every exchange's sources read from the live rows. An exchange saved
    /// before numbered sources (`sources_json` NULL) comes back `legacy` with its saved
    /// `citations_json` untouched.
    pub fn get_conversation_sourced(&self, id: i64) -> rusqlite::Result<Option<SourcedConversation>> {
        let Some(c) = self.get_conversation(id)? else { return Ok(None) };
        let mut stmt = self.conn.prepare(
            "SELECT id, sources_json, answer_provider, answered_locally, left_out FROM conversation_exchange WHERE conversation_id = ?1",
        )?;
        type Extra = (Option<String>, Option<String>, Option<bool>, Option<i64>);
        let extra: HashMap<i64, Extra> = stmt
            .query_map(rusqlite::params![id], |r| Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?
            .collect::<rusqlite::Result<_>>()?;
        let saved: Vec<Vec<SavedSource>> = c
            .exchanges
            .iter()
            .map(|x| {
                extra
                    .get(&x.id)
                    .and_then(|(s, ..)| s.as_deref())
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default()
            })
            .collect();
        let refs: Vec<(String, i64)> = saved.iter().flatten().map(|s| (s.kind.clone(), s.id)).collect();
        // A failed read of the cards is not a failed conversation: the text still shows, and each card says it
        // is unreadable (never "no longer on this device").
        let (cards, unreadable) = match self.resolve_cards(&refs) {
            Ok(cards) => (cards, false),
            Err(_) => (refs.iter().map(|(kind, id)| SourceCard { kind: kind.clone(), id: *id, ..Default::default() }).collect(), true),
        };
        let mut cards = cards.into_iter();
        let exchanges = c
            .exchanges
            .into_iter()
            .zip(saved)
            .map(|(x, saved)| {
                let (sources_json, provider, answered_locally, left_out) = extra.get(&x.id).cloned().unwrap_or((None, None, None, None));
                let legacy = sources_json.is_none();
                let cited = saved.iter().filter_map(|s| usize::try_from(s.n).ok().filter(|n| *n >= 1)).collect();
                let sources = saved.iter().map(|s| SavedSourceCard { n: s.n, card: cards.next().unwrap_or_default(), unreadable }).collect();
                SourcedExchange {
                    id: x.id,
                    query: x.query,
                    answer: x.answer,
                    created_at_unix_ms: x.created_at_unix_ms,
                    model: x.model,
                    legacy,
                    citations_json: legacy.then_some(x.citations_json),
                    provider,
                    answered_locally,
                    left_out,
                    cited,
                    sources,
                }
            })
            .collect();
        Ok(Some(SourcedConversation {
            id: c.id,
            title: c.title,
            starred: c.starred,
            created_at_unix_ms: c.created_at_unix_ms,
            updated_at_unix_ms: c.updated_at_unix_ms,
            exchanges,
        }))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{MeetingStatus, MeetingTrigger, NewCaptureItem, NewMeeting, NewTranscriptSegment};

    pub(crate) fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("c.db"), std::sync::Arc::new(crate::tests::FakeEmbedder));
        (dir, s)
    }

    pub(crate) fn capture(s: &Store, n: i64, ts: i64, app: &str, title: &str, text: &str) -> i64 {
        s.insert_capture_item(&NewCaptureItem {
            device_id: "mac-1".into(),
            ts_unix_ms: ts,
            app_bundle_id: app.into(),
            window_title: title.into(),
            url: None,
            text: text.into(),
            hash: n,
            source: "ax".into(),
        })
        .unwrap()
        .unwrap()
    }

    pub(crate) fn meeting(s: &Store, start: i64, title: &str, attendees: &[&str], lines: &[&str]) -> i64 {
        let id = s
            .insert_meeting(&NewMeeting {
                device_id: "mac-1".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 60_000),
                title: title.into(),
                attendees: attendees.iter().map(|a| a.to_string()).collect(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        let segs: Vec<NewTranscriptSegment> = lines
            .iter()
            .enumerate()
            .map(|(i, t)| NewTranscriptSegment { meeting_id: id, start_cs: i as i64 * 100, speaker_label: "Speaker 1".into(), text: t.to_string() })
            .collect();
        s.insert_transcript_segments(&segs).unwrap();
        id
    }

    fn transcript_ids(s: &Store, meeting: i64) -> Vec<i64> {
        s.get_transcript_segments_page(meeting, 0, 100).unwrap().0.iter().map(|x| x.id).collect()
    }

    #[test]
    fn an_added_item_card_carries_its_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::mcp_items::tests::open(&dir);
        crate::mcp_items::tests::add(&s, 3, &[crate::mcp_items::tests::item("outlook-work", "a")]);
        let id = crate::mcp_items::tests::row_id(&s, 3, "outlook-work", "a").unwrap();
        let card = s.resolve_cards(&[("mcp_item".to_string(), id)]).unwrap().remove(0);
        assert!(card.found);
        assert_eq!(card.kind, "mcp_item");
        assert_eq!(card.title, "Q4 forecast");
        assert_eq!(card.text.as_deref(), Some("Numbers for the quarter"));
        assert_eq!(card.ts_unix_ms, Some(crate::mcp_items::tests::T0));
        assert_eq!(card.declared_kind.as_deref(), Some("email"));
        assert_eq!(card.added_by.as_deref(), Some("Claude Code"));
        assert_eq!(card.source_label.as_deref(), Some("outlook-work"));
        assert_eq!(card.url.as_deref(), Some("https://outlook.office.com/mail/1"));
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["added_by"], "Claude Code");
        // A capture card has none of the four keys.
        let cap = capture(&s, 1, 1_000, "com.a", "W", "text");
        let cap = s.resolve_cards(&[("capture".to_string(), cap)]).unwrap().remove(0);
        let json = serde_json::to_value(&cap).unwrap();
        assert!(json.get("added_by").is_none() && json.get("source_label").is_none() && json.get("declared_kind").is_none() && json.get("url").is_none());
        // Not an added item (the capture's `items` row): gone.
        let cap_item: i64 = s.conn.query_row("SELECT id FROM items WHERE kind = 'capture'", [], |r| r.get(0)).unwrap();
        assert_ne!(cap_item, id);
        assert!(!s.resolve_cards(&[("mcp_item".to_string(), cap_item)]).unwrap()[0].found);
    }

    #[test]
    fn a_saved_source_of_kind_mcp_item_is_kept() {
        let (_d, s) = store();
        let conv = s.insert_conversation("q", 1).unwrap();
        s.append_conversation_exchange_sourced(&NewSourcedExchange {
            conversation_id: conv,
            query: "what?",
            answer: "It was added [1].",
            sources: &[SavedSource { n: 1, kind: "mcp_item".into(), id: 7 }],
            model: None,
            provider: Some("anthropic"),
            answered_locally: false,
            left_out: 0,
            now_unix_ms: 10,
        })
        .unwrap();
        let sources_json: String = s.conn.query_row("SELECT sources_json FROM conversation_exchange", [], |r| r.get(0)).unwrap();
        assert_eq!(sources_json, r#"[{"n":1,"kind":"mcp_item","id":7}]"#);
    }

    #[test]
    fn a_card_is_read_from_the_live_row_and_cut_to_length() {
        let (_d, s) = store();
        let long = "word ".repeat(400);
        let c = capture(&s, 1, 5_000, "com.slack", "launch plan", &long);
        let card = &s.resolve_cards(&[("capture".into(), c)]).unwrap()[0];
        assert!(card.found);
        assert_eq!((card.title.as_str(), card.app_bundle_id.as_deref(), card.device_id.as_deref()), ("launch plan", Some("com.slack"), Some("mac-1")));
        let text = card.text.as_deref().unwrap();
        assert!(text.chars().count() <= CARD_TEXT_CHARS + 1 && text.ends_with('\u{2026}'), "{}", text.len());
    }

    #[test]
    fn a_transcript_card_names_the_speaker_and_the_meeting() {
        let (_d, s) = store();
        let m = meeting(&s, 1_000_000, "Acme sync", &["Sam Okafor"], &["we ship friday", "send the deck"]);
        s.upsert_person("name:sam", "Sam Okafor", None).unwrap();
        s.set_meeting_speaker(m, 0, Some("name:sam"), Some(crate::speaker_names::NameStatus::Confirmed)).unwrap();
        let ids = transcript_ids(&s, m);
        let card = &s.resolve_cards(&[("transcript".into(), ids[1])]).unwrap()[0];
        assert_eq!(card.meeting_id, Some(m));
        assert_eq!(card.meeting_title.as_deref(), Some("Acme sync"));
        assert_eq!(card.speaker_label.as_deref(), Some("Sam Okafor"));
        assert_eq!(card.start_cs, Some(100));
        assert_eq!(card.meeting_start_unix_ms, Some(1_000_000));
        assert_eq!(card.text.as_deref(), Some("send the deck"));
    }

    #[test]
    fn a_saved_answer_holds_only_n_kind_and_id_and_a_gone_row_has_no_words() {
        let (_d, s) = store();
        let cap = capture(&s, 1, 5_000, "com.slack", "secret title", "zebracanary in the capture");
        let m = meeting(&s, 1_000_000, "Quiet meeting", &[], &["okapicanary line"]);
        let line = transcript_ids(&s, m)[0];
        let conv = s.insert_conversation("q", 1).unwrap();
        s.append_conversation_exchange_sourced(&NewSourcedExchange {
            conversation_id: conv,
            query: "what?",
            answer: "It was said [1] and [2].",
            sources: &[
                SavedSource { n: 1, kind: "capture".into(), id: cap },
                SavedSource { n: 2, kind: "transcript".into(), id: line },
                SavedSource { n: 0, kind: "capture".into(), id: cap },
                SavedSource { n: 3, kind: "nonsense".into(), id: 1 },
                SavedSource { n: 1, kind: "capture".into(), id: cap },
            ],
            model: Some("claude-x"),
            provider: Some("anthropic"),
            answered_locally: false,
            left_out: 2,
            now_unix_ms: 10,
        })
        .unwrap();

        // What the row holds: ids and small numbers, no title, no text.
        let (sources_json, provider, local, left_out, citations): (String, String, bool, i64, String) = s
            .conn
            .query_row(
                "SELECT sources_json, answer_provider, answered_locally, left_out, citations_json FROM conversation_exchange",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(sources_json, format!(r#"[{{"n":1,"kind":"capture","id":{cap}}},{{"n":2,"kind":"transcript","id":{line}}}]"#));
        assert_eq!((provider.as_str(), local, left_out, citations.as_str()), ("anthropic", false, 2, "[]"));
        for words in ["zebracanary", "secret title", "okapicanary", "Quiet meeting"] {
            assert!(!sources_json.contains(words));
        }
        // Nothing but the answer holds a word: no column besides query and answer carries one.
        let dump: Vec<String> = s
            .conn
            .prepare("SELECT sources_json || answer_provider || IFNULL(model, '') || citations_json FROM conversation_exchange")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(dump.iter().all(|d| !d.contains("canary")));

        // Read live: both cards found with their words.
        let c = s.get_conversation_sourced(conv).unwrap().unwrap();
        let x = &c.exchanges[0];
        assert!(!x.legacy && x.citations_json.is_none());
        assert_eq!((x.provider.as_deref(), x.answered_locally, x.left_out), (Some("anthropic"), Some(false), Some(2)));
        assert_eq!(x.sources.iter().map(|s| s.n).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(x.sources[0].card.text.as_deref(), Some("zebracanary in the capture"));
        assert_eq!(x.sources[1].card.text.as_deref(), Some("okapicanary line"));

        // The capture is deleted and the meeting forgotten: the cards come back with no words.
        s.conn.execute("DELETE FROM items WHERE id = ?1", [cap]).unwrap();
        assert!(s.delete_meeting(m).unwrap());
        let c = s.get_conversation_sourced(conv).unwrap().unwrap();
        for src in &c.exchanges[0].sources {
            assert!(!src.card.found, "{src:?}");
            assert!(src.card.text.is_none() && src.card.title.is_empty() && src.card.meeting_title.is_none(), "{src:?}");
        }
        let json = serde_json::to_string(&c).unwrap();
        for words in ["zebracanary", "secret title", "okapicanary", "Quiet meeting"] {
            assert!(!json.contains(words), "{words} is still in the read: {json}");
        }
        assert_eq!(c.exchanges[0].sources.len(), 2, "the numbers stay so the answer's marks still line up");
    }

    #[test]
    fn an_old_exchange_loads_through_the_old_call_unchanged_and_is_marked_legacy_by_the_new_one() {
        let (_d, s) = store();
        let conv = s.insert_conversation("q", 1).unwrap();
        let saved = r#"[{"kind":"capture","id":7,"title":"Old chip","text":"kept words"}]"#;
        s.append_conversation_exchange(conv, "old q", "old a", saved, Some("m"), 2).unwrap();
        s.append_conversation_exchange_sourced(&NewSourcedExchange {
            conversation_id: conv,
            query: "new q",
            answer: "new a",
            sources: &[],
            model: None,
            provider: None,
            answered_locally: true,
            left_out: 0,
            now_unix_ms: 3,
        })
        .unwrap();
        let old = s.get_conversation(conv).unwrap().unwrap();
        assert_eq!(old.exchanges[0].citations_json, saved);
        assert_eq!(old.exchanges[1].citations_json, "[]");
        let new = s.get_conversation_sourced(conv).unwrap().unwrap();
        assert!(new.exchanges[0].legacy);
        assert_eq!(new.exchanges[0].citations_json.as_deref(), Some(saved));
        assert!(new.exchanges[0].sources.is_empty() && new.exchanges[0].provider.is_none());
        assert!(!new.exchanges[1].legacy);
        assert_eq!(new.exchanges[1].answered_locally, Some(true));
        let json = serde_json::to_value(&new).unwrap();
        assert!(json["exchanges"][1].get("citations_json").is_none());
    }

    #[test]
    fn an_exchange_for_a_deleted_conversation_is_dropped_and_a_local_only_answer_is_marked() {
        let (_d, s) = store();
        let none = NewSourcedExchange {
            conversation_id: 99,
            query: "q",
            answer: "a",
            sources: &[],
            model: None,
            provider: None,
            answered_locally: false,
            left_out: 0,
            now_unix_ms: 1,
        };
        s.append_conversation_exchange_sourced(&none).unwrap();
        let n: i64 = s.conn.query_row("SELECT COUNT(*) FROM conversation_exchange", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        let conv = s.insert_conversation("q", 1).unwrap();
        s.mark_local_only_text("They agreed the price.").unwrap();
        s.append_conversation_exchange_sourced(&NewSourcedExchange { conversation_id: conv, answer: "They agreed the price.", ..none }).unwrap();
        let local: bool = s.conn.query_row("SELECT local_only FROM conversation_exchange", [], |r| r.get(0)).unwrap();
        assert!(local, "an answer made from an excluded client's material is stored as such");
    }

    #[test]
    fn thousands_of_cards_resolve_in_a_few_queries() {
        let (_d, s) = store();
        for i in 0..3_000 {
            capture(&s, i, 1_000 + i, "app", &format!("t{i}"), &format!("text {i}"));
        }
        let ids: Vec<i64> = s.conn.prepare("SELECT id FROM items WHERE kind = 'capture'").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        let refs: Vec<(String, i64)> = ids.iter().map(|i| ("capture".to_string(), *i)).chain([("capture".to_string(), 9_999_999)]).collect();
        let start = std::time::Instant::now();
        let cards = s.resolve_cards(&refs).unwrap();
        assert_eq!(cards.len(), 3_001);
        assert!(cards[..3_000].iter().all(|c| c.found) && !cards[3_000].found);
        // Chunked reads (a few per kind), not 3,000 lookups: well inside a second even unoptimised.
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
    }

    /// A failed read of the cards is not a failed conversation, and not a deleted row.
    #[test]
    fn a_saved_conversation_survives_a_failed_card_read_and_says_so() {
        let (_d, s) = store();
        let cap = capture(&s, 1, 5_000, "com.slack", "title", "text of the capture");
        let conv = s.insert_conversation("q", 1).unwrap();
        s.append_conversation_exchange_sourced(&NewSourcedExchange {
            conversation_id: conv,
            query: "q",
            answer: "see [2] then [1]",
            sources: &[SavedSource { n: 2, kind: "capture".into(), id: cap }, SavedSource { n: 1, kind: "capture".into(), id: cap + 1 }],
            model: Some("m"),
            provider: Some("ollama"),
            answered_locally: true,
            left_out: 0,
            now_unix_ms: 10,
        })
        .unwrap();
        let ok = s.get_conversation_sourced(conv).unwrap().unwrap();
        let x = &ok.exchanges[0];
        assert_eq!(x.cited, vec![2, 1], "the numbers a UI may draw a mark for, first-cited first");
        assert!(x.sources.iter().all(|c| !c.unreadable));
        assert!(x.sources[0].card.found && !x.sources[1].card.found, "the second row never existed: gone, and readable");
        // The cards cannot be read just now: the conversation still comes back, each source says so.
        s.conn.execute("DROP TABLE items", []).unwrap();
        let hurt = s.get_conversation_sourced(conv).unwrap().unwrap();
        let x = &hurt.exchanges[0];
        assert_eq!((x.answer.as_str(), x.cited.clone()), ("see [2] then [1]", vec![2, 1]));
        assert!(x.sources.iter().all(|c| c.unreadable && !c.card.found && c.card.text.is_none()), "{:?}", x.sources);
        assert_eq!(x.sources[0].n, 2);
    }

}
