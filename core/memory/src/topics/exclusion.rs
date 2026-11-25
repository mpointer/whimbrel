//! What "material belonging to an excluded client" is, as row ids (X6 client controls, spec 7.2).
//!
//! [`Store::excluded_material`] lists every source filed under a client excluded from hosted
//! models (membership, following merges), mapped to the ids the retrieval paths return: meetings,
//! episodes, `items` rows (emails, calendar events, slides, and the captures an episode was built
//! over), and email threads. A source in two topics is excluded when either client is. Text that
//! merely names a client is matched by the caller (`llm::guard::ExclusionSet`): this module only
//! knows ids and the client's terms.
//!
//! [`Store::excluded_material_with`] is the one predicate every path uses (X6 fix round 1, B4):
//! the membership above, plus every meeting, email (and its thread), slide and calendar event
//! whose text names a client (all candidates, never a capped page, B6), every meeting and email
//! that carries a client contact's address (the attendee details as stored, B4), and every
//! meeting recorded from a calendar event filed under the client (same title, start within 30
//! minutes). It is re-derived from what is stored, not tagged at ingestion: every caller asks
//! this one function over the same stored fields, so a tag would only cache the same answer
//! (which the generation cache below already does). The result is cached on the store until
//! `exclusion_generation` moves: triggers bump it on any change to the settings, membership,
//! terms and contacts (M5). New and changed meetings, emails, events, captures, transcripts,
//! episodes, threads and routine runs do not move it: triggers note them in `exclusion_dirty`,
//! and the next read checks only those rows and adds what they name or link to. A full
//! recompute still happens every 30 minutes.
//!
//! Derived outputs carry provenance instead of being deleted (B3). Since fix round 4 (S2) an answer
//! or a routine run records the inputs it was built from ([`input_refs_stamp`]): it is clean for
//! hosted use while none of those inputs is in the excluded material read now. New material of a
//! client that was not among its inputs never makes it unclean; an input later found to be the
//! client's (filed, named, a contact's) does. A run or answer with no recorded inputs (made before
//! this, from unfiltered local context, over the cap, or from another device) is kept out of hosted
//! prompts and MCP while any client is excluded. Answers made from an excluded client's material on this computer
//! are marked ([`Store::mark_local_only_text`]), and the exchange that stores one carries
//! `local_only` (B2). Turning exclusion on (or any change to what is excluded) drops the MCP digest
//! cache, which is only a cache.

use std::collections::HashSet;

use rusqlite::{params, OptionalExtension};

use crate::Store;

/// The ids of everything filed under an excluded client. Empty when nothing is excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExcludedMaterial {
    /// `topic_source.id` of every excluded source.
    pub source_ids: HashSet<i64>,
    /// The excluded clients' topics: their seeds, seeds merged into them, and topics merged into those.
    pub topic_ids: HashSet<i64>,
    /// The excluded seeds' keys, merged ones included.
    pub topic_keys: HashSet<String>,
    pub meetings: HashSet<i64>,
    pub episodes: HashSet<i64>,
    /// `items.id`: emails, calendar events, slides, and the captures under an excluded episode.
    pub items: HashSet<i64>,
    pub threads: HashSet<i64>,
    /// Lowercase addresses of the people met only in excluded clients' material (never the user's
    /// own, which the topic index never records, and never someone also met in other topics).
    pub attendee_emails: HashSet<String>,
    /// The excluded clients' names and aliases ([`Store::excluded_terms`]).
    pub terms: Vec<String>,
    /// Excluded calendar events as (lowercase title, start).
    pub events: HashSet<(String, i64)>,
    /// Excluded calendar events' calendar ids (`items.external_id`).
    pub event_ids: HashSet<String>,
    /// Excluded events' series ids ([`event_series_id`]), tombstoned ones included.
    pub event_series: HashSet<String>,
    /// Routine runs clean for every client excluded now (built from filtered material while they
    /// were excluded). Every other run is the excluded clients' as far as a hosted prompt goes.
    pub clean_runs: HashSet<i64>,
    /// Fingerprints of the excluded seeds' keys (never the keys or names themselves).
    pub key_hashes: Vec<String>,
    /// The calendar events filed under an excluded client, with the fingerprint of that client's
    /// key: a tombstone belongs to the event's own client (fix round 3, R6).
    pub event_owners: std::collections::HashMap<i64, Vec<String>>,
    /// [`Self::version`] of the material as computed, kept so clean stamps can be compared
    /// without recomputing it (fix round 3, R4). Empty until computed.
    pub material_version: String,
}

impl ExcludedMaterial {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.source_ids.is_empty() && self.topic_keys.is_empty()
    }

    /// Whether `(kind, id)` is excluded: the one membership answer every retrieved item, topic
    /// source and recorded input goes through. `kind` is a topic-source or retrieval kind:
    /// meeting, episode, thread, routine_run; anything else (capture, email, calendar_event,
    /// snapshot) is an `items.id`.
    pub fn excludes(&self, kind: &str, id: i64) -> bool {
        match kind {
            "meeting" => self.meetings.contains(&id),
            "episode" => self.episodes.contains(&id),
            "thread" => self.threads.contains(&id),
            // Derived text: only a run whose recorded inputs are all outside the material may go.
            "routine_run" => !self.clean_runs.contains(&id),
            _ => self.items.contains(&id),
        }
    }

    /// Whether recorded input `r` ([`input_ref`]) is excluded now. An `event` is its calendar id
    /// (and its series); anything unreadable is excluded. A `text` input
    /// ([`input_text_ref`]) has no id to look up: it is excluded here, and only a caller
    /// that reads its words by name (`api::ask`) may count it clean.
    pub fn excludes_input_ref(&self, r: &str) -> bool {
        let Some((kind, id)) = r.split_once(':') else { return true };
        if kind == "text" {
            return true;
        }
        if kind == "event" {
            return id.is_empty() || self.event_ids.contains(id) || self.event_series.contains(&event_series_id(id));
        }
        id.parse::<i64>().map_or(true, |id| self.excludes(kind, id))
    }

    /// Whether `stamp` ([`input_refs_stamp`]) records inputs none of which is excluded now.
    pub fn stamp_is_clean(&self, stamp: &str) -> bool {
        stamp_input_refs(stamp).is_some_and(|refs| refs.iter().all(|r| !self.excludes_input_ref(r)))
    }

    /// A fingerprint of what is excluded: the clients' terms, keys, topics, sources and contact
    /// addresses. Any change to the excluded set, a client's membership or its terms changes it,
    /// so a cache built under one state is never served under another. Never names a client.
    pub fn version(&self) -> String {
        fn sorted<T: Ord + Clone>(v: impl IntoIterator<Item = T>) -> Vec<T> {
            let mut v: Vec<T> = v.into_iter().collect();
            v.sort();
            v
        }
        let mut terms: Vec<String> = self.terms.iter().map(|t| t.to_lowercase()).collect();
        terms.sort();
        let text = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            terms,
            sorted(self.topic_keys.iter().cloned()),
            sorted(self.topic_ids.iter().copied()),
            sorted(self.source_ids.iter().copied()),
            sorted(self.meetings.iter().copied()),
            sorted(self.episodes.iter().copied()),
            sorted(self.items.iter().copied()),
            sorted(self.threads.iter().copied()),
            sorted(self.attendee_emails.iter().cloned()),
        );
        format!("x{:016x}", super::fnv1a(text.into_bytes()))
    }
}

fn text_hash(text: &str) -> i64 {
    super::fnv1a(text.trim().bytes()) as i64
}

/// The recorded input for words with no stored row (`text:<fingerprint of the whole text>`), with
/// that fingerprint and the word fingerprints a stamp keeps for it ([`Store::mark_clean_text`]).
/// Nothing is written here.
pub fn input_text_ref(text: &str) -> (String, i64, Vec<u32>) {
    let hash = text_hash(text);
    (input_ref("text", hash), hash, crate::words::word_fingerprints(text))
}

/// The first word of a kept fingerprint blob written since the fingerprints are taken over every
/// reading of the text (`crate::words::readings`; review 4, M4). Rows written before (v0.9.80 and
/// earlier, fingerprints of the text as written) lack it.
const FINGERPRINTS_V2: u32 = 0xFFFF_FFFE;

/// A fingerprint blob to keep: [`FINGERPRINTS_V2`], then the fingerprints.
pub(crate) fn encode_fingerprints(fingerprints: &[u32]) -> Vec<u8> {
    std::iter::once(FINGERPRINTS_V2).chain(fingerprints.iter().copied()).flat_map(u32::to_le_bytes).collect()
}

/// The fingerprints a kept blob holds. A blob from before [`FINGERPRINTS_V2`] holds fingerprints of
/// the text as written only, so what it would say about an encoded name is unknown: it reads as
/// [`crate::words::UNDECIDABLE_FINGERPRINT`] alone, which names every term (unclean while any
/// client is excluded, clean while none is).
fn decode_fingerprints(b: &[u8]) -> Vec<u32> {
    let words: Vec<u32> = b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    match words.split_first() {
        Some((&FINGERPRINTS_V2, rest)) => rest.to_vec(),
        _ => vec![crate::words::UNDECIDABLE_FINGERPRINT],
    }
}

/// Drops the clean stamps (and the word fingerprints they own) of the answers `select_answers`
/// returns (a `SELECT answer FROM conversation_exchange ...` with `params`): run before those
/// exchanges are deleted (fix round 5, N1).
pub(crate) fn drop_conversation_stamps_tx(
    conn: &rusqlite::Connection,
    select_answers: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<()> {
    let answers: Vec<String> = conn.prepare(select_answers)?.query_map(params, |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for a in answers {
        let h = text_hash(&a);
        conn.execute("DELETE FROM clean_text WHERE hash = ?1", [h])?;
        conn.execute("DELETE FROM input_words WHERE stamp = ?1", [h])?;
        // Main's per-answer SHA-256 (Limited Use): no hash of a deleted answer stays. A missing row reads as "may hold Google data", so this fails closed.
        conn.execute("DELETE FROM answer_provenance WHERE text_hash = ?1", [crate::limited_use::answer_key(&a)])?;
    }
    Ok(())
}

/// Takes client `key` out of every stored excluded event range (`excluded_event_span`), deleting a range no
/// other client holds. A purged client leaves no trace there; another excluded client's ranges stay.
pub(crate) fn purge_event_spans_of_tx(conn: &rusqlite::Connection, key: &str) -> rusqlite::Result<()> {
    let h = key_hash(key);
    let rows: Vec<(String, i64, String)> = conn
        .prepare("SELECT series, start_unix_ms, keys FROM excluded_event_span")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (series, start, keys) in rows {
        if !keys.split(',').any(|k| k == h) {
            continue;
        }
        let rest: Vec<&str> = keys.split(',').filter(|k| *k != h && !k.is_empty()).collect();
        if rest.is_empty() {
            conn.execute("DELETE FROM excluded_event_span WHERE series = ?1 AND start_unix_ms = ?2", params![series, start])?;
        } else {
            conn.execute(
                "UPDATE excluded_event_span SET keys = ?3 WHERE series = ?1 AND start_unix_ms = ?2",
                params![series, start, rest.join(",")],
            )?;
        }
    }
    Ok(())
}

/// [`Store::purge_input_words_naming`] inside a caller's transaction.
pub(crate) fn purge_input_words_naming_tx(conn: &rusqlite::Connection, terms: &[String]) -> rusqlite::Result<usize> {
    // The spelling without invisible characters, as the exclusion set matches (review 6, M3).
    let terms: Vec<Vec<String>> =
        terms.iter().map(|t| crate::words::folded_words(&crate::words::without_invisibles(t))).filter(|w| !w.is_empty()).collect();
    let rows: Vec<(i64, i64, Vec<u8>)> = conn
        .prepare("SELECT stamp, hash, fingerprints FROM input_words")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut n = 0;
    let mut gone_texts: HashSet<String> = HashSet::new();
    for (stamp, hash, blob) in rows {
        if !terms.is_empty() && crate::words::fingerprints_name_any(&decode_fingerprints(&blob), &terms) {
            n += conn.execute("DELETE FROM input_words WHERE stamp = ?1 AND hash = ?2", [stamp, hash])?;
            gone_texts.insert(input_ref("text", hash));
        }
    }
    // The stamps that recorded those texts (the owner, and any answer that inherited them through
    // its history) hold a hash of the whole question or window: a trace too (fix round 6, N1a).
    // They go, with the fingerprints they own; their answers are unclean either way.
    //
    // Fix round 7, F1: so do stamps that cannot be checked by name at all, whoever they were for:
    // a `text:` input with no kept fingerprints (written while nothing was excluded; meeting Ask and
    // live drafts are in no conversation, so nothing else ever drops them), and stamps any of whose
    // recorded inputs are gone (an answer built from material just deleted, keyed by the hash of
    // an answer that likely names the client). Every such stamp is unclean already; deleting it
    // only makes that permanent (fail-closed).
    //
    // `local_only_text` marks are not touched (ruling, round 7, F2): they are whole-text hashes that
    // keep a surviving answer or prompt off hosted models (Ollama cloud prompt gating); dropping
    // one could re-open that path. Only word fingerprints and the stamps that recorded a purged
    // text count as a trace.
    let kept: HashSet<i64> = conn
        .prepare("SELECT DISTINCT hash FROM input_words")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let stamps: Vec<(i64, String)> = conn
        .prepare("SELECT hash, keys FROM clean_text")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let all_refs: Vec<String> = stamps.iter().filter_map(|(_, k)| stamp_input_refs(k)).flatten().collect();
    let missing = missing_input_refs_conn(conn, &all_refs)?;
    for (owner, keys) in stamps {
        let drop = match stamp_input_refs(&keys) {
            Some(refs) => refs.iter().any(|r| {
                gone_texts.contains(r)
                    || missing.contains(r)
                    || r.strip_prefix("text:").is_some_and(|h| h.parse::<i64>().map_or(true, |h| !kept.contains(&h)))
            }),
            // Not a recorded-inputs stamp (an older format): never clean, so it goes too.
            None => true,
        };
        if drop {
            n += conn.execute("DELETE FROM clean_text WHERE hash = ?1", [owner])?;
            n += conn.execute("DELETE FROM input_words WHERE stamp = ?1", [owner])?;
        }
    }
    Ok(n)
}

/// [`Store::missing_input_refs`] on a connection (inside a caller's transaction).
pub(crate) fn missing_input_refs_conn<'a>(conn: &rusqlite::Connection, refs: impl IntoIterator<Item = &'a String>) -> rusqlite::Result<HashSet<String>> {
        let mut by_table: std::collections::HashMap<&'static str, Vec<(i64, &'a String)>> = std::collections::HashMap::new();
        let mut missing = HashSet::new();
        for r in refs {
            let Some((kind, id)) = r.split_once(':') else {
                missing.insert(r.clone());
                continue;
            };
            let table = match kind {
                "event" | "text" => continue,
                "meeting" => "meeting",
                "episode" => "episodes",
                "thread" => "threads",
                "routine_run" => "routine_run",
                _ => "items",
            };
            match id.parse::<i64>() {
                Ok(id) => by_table.entry(table).or_default().push((id, r)),
                Err(_) => {
                    missing.insert(r.clone());
                }
            }
        }
        for (table, ids) in by_table {
            for chunk in ids.chunks(500) {
                let list = chunk.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>().join(",");
                let found: HashSet<i64> = conn
                    .prepare(&format!("SELECT id FROM {table} WHERE id IN ({list})"))?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                missing.extend(chunk.iter().filter(|(id, _)| !found.contains(id)).map(|(_, r)| (*r).clone()));
            }
        }
        Ok(missing)}

/// The columns, tables and triggers the exclusion paths need. Runs last in the topic schema.
pub(crate) fn ensure_exclusion_schema(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    super::schema::ensure_column(conn, "routine_run", "provenance", "TEXT")?;
    // The calendar event a meeting was recorded from (set by the shell at start; device-local).
    super::schema::ensure_column(conn, "meeting", "calendar_event_id", "TEXT")?;
    super::schema::ensure_column(conn, "conversation_exchange", "local_only", "INTEGER NOT NULL DEFAULT 0")?;
    // Round 4's `input_words` had no owning stamp: its rows are dropped (they only make the stamps
    // that used them unclean, which is fail-closed).
    let has_stamp: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('input_words') WHERE name = 'stamp'")?
        .exists([])?;
    if !has_stamp {
        conn.execute_batch("DROP TABLE IF EXISTS input_words;")?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS local_only_text (hash INTEGER PRIMARY KEY, at_unix_ms INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS clean_text (hash INTEGER PRIMARY KEY, keys TEXT NOT NULL);
         -- Words an answer was built from that have no stored row (a window's text, the question),
         -- as word fingerprints only, never the words: enough for a later stamp check by name
         -- (fix round 4, S2). Owned by the stamp (`clean_text.hash`) that recorded them: written
         -- with it, replaced with it, dropped with it (fix round 5, N1). An earlier draft kept the
         -- words; that table is dropped.
         DROP TABLE IF EXISTS input_text;
         CREATE TABLE IF NOT EXISTS input_words (
             stamp INTEGER NOT NULL, hash INTEGER NOT NULL, fingerprints BLOB NOT NULL,
             at_unix_ms INTEGER NOT NULL, PRIMARY KEY (stamp, hash));
         CREATE INDEX IF NOT EXISTS idx_input_words_hash ON input_words(hash);
         CREATE INDEX IF NOT EXISTS idx_input_words_at ON input_words(at_unix_ms);
         -- The episode lift looks episodes up from each excluded capture (fix round 3, R3).
         CREATE INDEX IF NOT EXISTS idx_episodes_device_start_item ON episodes(device_id, start_item_id);
         -- Excluded calendar events' instances, kept after the event row goes (fix round 2, N3).
         CREATE TABLE IF NOT EXISTS excluded_event_span (
             series TEXT NOT NULL, start_unix_ms INTEGER NOT NULL, keys TEXT NOT NULL,
             PRIMARY KEY (series, start_unix_ms));
         INSERT OR IGNORE INTO kv_meta (key, value) VALUES ('exclusion_generation', '0');
         -- Round 1 dropped routine runs; runs are now kept out by provenance, never deleted.
         DROP TRIGGER IF EXISTS client_control_excluded_insert;
         DROP TRIGGER IF EXISTS client_control_excluded_update;
         DELETE FROM kv_meta WHERE key = 'exclusion_rebuild_owed';
         CREATE TRIGGER IF NOT EXISTS client_control_excluded_insert_v2 AFTER INSERT ON client_control
         WHEN NEW.exclude_hosted != 0 BEGIN
             DELETE FROM mcp_daily_digest_cache;
         END;",
    )?;
    // Two kinds of write. A write to a table that says who is excluded (settings, terms,
    // membership, contacts, indexed topic text) bumps the generation: the next read computes the
    // material in full. A write to content (a capture, an email, a meeting and its text, an
    // episode, a routine run) is only noted in `exclusion_dirty`: the next read checks those rows
    // against the cached material and adds what they name or link to.
    let bump = "UPDATE kv_meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'exclusion_generation';";
    let mut sql = String::from(
        "CREATE TABLE IF NOT EXISTS exclusion_dirty (
             seq INTEGER PRIMARY KEY AUTOINCREMENT, tbl TEXT NOT NULL, row_id INTEGER NOT NULL,
             at_unix_s INTEGER NOT NULL, UNIQUE (tbl, row_id));
         CREATE INDEX IF NOT EXISTS idx_exclusion_dirty_at ON exclusion_dirty(at_unix_s);
         -- Episodes covering a capture are looked up from the capture.
         CREATE INDEX IF NOT EXISTS idx_episodes_device_end_item ON episodes(device_id, end_item_id);\n",
    );
    for (table, insert_bumps, columns, when) in RULES_TABLES {
        // An insert of a row nothing links to yet (a source, its text, an entity, a person) moves
        // nothing: what links it (a membership, a mention) is read when that is written.
        if insert_bumps {
            sql.push_str(&format!(
                "CREATE TRIGGER IF NOT EXISTS exclusion_gen2_{table}_insert AFTER INSERT ON {table} BEGIN {bump} END;\n"
            ));
        } else {
            sql.push_str(&format!("DROP TRIGGER IF EXISTS exclusion_gen2_{table}_insert;\n"));
        }
        sql.push_str(&format!(
            "CREATE TRIGGER IF NOT EXISTS exclusion_gen2_{table}_delete AFTER DELETE ON {table} BEGIN {bump} END;\n"
        ));
        // An update of a column the material is not read from (a timestamp, a queue mark)
        // moves nothing. The narrowed triggers have their own names.
        if columns.is_empty() {
            sql.push_str(&format!(
                "CREATE TRIGGER IF NOT EXISTS exclusion_gen2_{table}_update AFTER UPDATE ON {table} BEGIN {bump} END;\n"
            ));
        } else {
            let when = if when.is_empty() { String::new() } else { format!("WHEN {when} ") };
            sql.push_str(&format!(
                "CREATE TRIGGER IF NOT EXISTS exclusion_gen4_{table}_update AFTER UPDATE OF {columns} ON {table} {when}BEGIN {bump} END;\n\
                 DROP TRIGGER IF EXISTS exclusion_gen2_{table}_update;\n"
            ));
        }
    }
    // A person row that arrives after the speaker rows pointing at it changes those meetings'
    // text. Rare (a synced speaker before its person), so it takes the full computation.
    sql.push_str(&format!(
        "CREATE TRIGGER IF NOT EXISTS exclusion_gen5_person_insert AFTER INSERT ON person \
         WHEN EXISTS (SELECT 1 FROM meeting_speaker WHERE person_key = NEW.person_key) BEGIN {bump} END;\n"
    ));
    // The exclusion setting is read here so that nothing is noted while no client is excluded.
    let on = "EXISTS (SELECT 1 FROM client_control WHERE exclude_hosted != 0)";
    let note = |tag: &str, id: &str| {
        format!(
            "DELETE FROM exclusion_dirty WHERE tbl = '{tag}' AND row_id = {id}; \
             INSERT INTO exclusion_dirty (tbl, row_id, at_unix_s) VALUES ('{tag}', {id}, CAST(strftime('%s', 'now') AS INTEGER));"
        )
    };
    // A membership or a mention is content: it is applied to the cached material row by row.
    for (table, id) in [("topic_member", "NEW.rowid"), ("entity_mention", "NEW.rowid")] {
        sql.push_str(&format!(
            "CREATE TRIGGER IF NOT EXISTS exclusion_note_{table}_insert AFTER INSERT ON {table} WHEN {on} BEGIN {} END;\n",
            note(table, id)
        ));
    }
    for (table, tag, id, extra, delete_bumps, columns) in CONTENT_TABLES {
        sql.push_str(&format!(
            "CREATE TRIGGER IF NOT EXISTS exclusion_note_{table}_insert AFTER INSERT ON {table} WHEN {extra}{on} BEGIN {} END;\n\
             CREATE TRIGGER IF NOT EXISTS exclusion_note_{table}_update AFTER UPDATE OF {columns} ON {table} WHEN {extra}{on} BEGIN {} END;\n",
            note(tag, id),
            note(tag, id)
        ));
        // A deleted row stays in the cached material (harmless). Rows other rows point at by id
        // (an input of a run, a thread's emails) still move the generation when they go.
        // `items`' trigger is named gen6 since it stopped firing for added items (review M15):
        // `CREATE TRIGGER IF NOT EXISTS` never replaces an existing trigger, so the gen3 one is dropped.
        if let Some(when) = delete_bumps {
            let generation = if table == "items" { "gen6" } else { "gen3" };
            sql.push_str(&format!(
                "CREATE TRIGGER IF NOT EXISTS exclusion_{generation}_{table}_delete AFTER DELETE ON {table} WHEN {when} BEGIN {bump} END;\n"
            ));
        }
    }
    sql.push_str("DROP TRIGGER IF EXISTS exclusion_gen3_items_delete;\n");
    // The triggers of earlier rounds give way: round 2's (captures were left out, R8), the
    // generation triggers the content tables had, and the first draft of the notes.
    for table in RULES_TABLES.iter().map(|(t, ..)| t).chain(CONTENT_TABLES.iter().map(|(t, ..)| t)) {
        for ev in ["insert", "update", "delete"] {
            sql.push_str(&format!("DROP TRIGGER IF EXISTS exclusion_gen_{table}_{ev};\n"));
        }
    }
    for (table, ..) in CONTENT_TABLES {
        for ev in ["insert", "update", "delete"] {
            sql.push_str(&format!("DROP TRIGGER IF EXISTS exclusion_gen2_{table}_{ev};\n"));
            sql.push_str(&format!("DROP TRIGGER IF EXISTS exclusion_dirty_{table}_{ev};\n"));
        }
    }
    conn.execute_batch(&sql)
}

/// Every `items` kind and how exclusion reaches it (held to `crate::ITEM_KINDS` by
/// `core/api/src/mcp_item_coverage.rs`).
pub const EXCLUSION_ITEM_KINDS: [(&str, crate::mcp_items::Coverage); 6] = {
    use crate::mcp_items::Coverage::{Covered, NotApplicable};
    [
        ("capture", Covered("names over title, text and url (capture_items_naming)")),
        ("transcript", NotApplicable("its meeting is the unit")),
        ("email", Covered("names (name_hits_limited) and contacts over actors")),
        ("calendar_event", Covered("names (name_hits_limited) and contacts over actors")),
        ("snapshot", Covered("names lift its meeting, whose slides are then excluded")),
        ("mcp_item", Covered("names (name_hits_limited) and contacts over actors")),
    ]
};

/// Tables that say who is excluded, or that the topic paths match by name: `(table, whether an
/// insert moves the generation, the columns an update must touch to move it, and a condition on
/// OLD and NEW)`. A write that moves the generation makes the next read compute in full.
const RULES_TABLES: [(&str, bool, &str, &str); 10] = [
    ("client_control", true, "", ""),
    // An insert is noted instead (see below); it is applied to the cached material.
    ("topic_member", false, "topic_id, source_id, origin", ""),
    ("topic_seed", true, "", ""),
    // A client's name is its topic's name; merges link topics to it.
    ("topic", true, "name, merged_into", ""),
    // A person's name and address are read through the speakers of a meeting.
    ("person", false, "person_key, name, email", "OLD.person_key IS NOT NEW.person_key OR OLD.name IS NOT NEW.name OR OLD.email IS NOT NEW.email"),
    ("entity", false, "kind, key", ""),
    // Also what `client_only_emails` reads: a client contact's addresses. An insert is noted.
    ("entity_mention", false, "entity_id, source_id", ""),
    // The user's own account addresses are never a client contact (M3).
    ("connector_account", true, "", ""),
    // The topic paths match a source's title and indexed text by name (M2), and the topic
    // naming batch keys on the generation.
    ("topic_source", false, "kind, ref_id, title", ""),
    ("topic_source_text", false, "text", ""),
];

/// Content tables: `(table, dirty tag, id of the row to check, extra WHEN, WHEN of a delete that
/// still moves the generation, the columns whose update is noted)`. Every table holding text
/// about a meeting that the meeting scan reads (S1) notes the meeting (`meeting_text`); a
/// transcript segment is checked alone, as the full scan does. Only the columns the predicates
/// read are listed: an embedding or sync flag update notes nothing.
#[allow(clippy::type_complexity)]
const CONTENT_TABLES: [(&str, &str, &str, &str, Option<&str>, &str); 16] = [
    (
        "items",
        "items",
        "NEW.id",
        "",
        // Review M15: nothing points at an added item by id either.
        Some("OLD.kind NOT IN ('capture', 'mcp_item')"),
        "kind, source, thread_id, ts_unix_ms, actors, title, body_text, refs, url, external_id, device_id, source_table, source_id",
    ),
    ("capture_item", "capture_item", "NEW.id", "", None, "window_title, text"),
    (
        "meeting",
        "meeting",
        "NEW.id",
        "",
        Some("1"),
        "title, attendees, attendee_details, notes, summary_tldr, summary_decisions_json, summary_action_items_json, \
         summary_open_questions_json, start_unix_ms, end_unix_ms, calendar_event_id, continue_key",
    ),
    (
        "meeting_note_draft",
        "meeting_text",
        "NEW.meeting_id",
        "",
        None,
        "summary_edited, decisions_edited_json, open_questions_edited_json, personal_notes",
    ),
    ("meeting_action_item", "meeting_text", "NEW.meeting_id", "", None, "text"),
    ("meeting_followup", "meeting_text", "NEW.meeting_id", "", None, "text"),
    ("meeting_speaker", "meeting_text", "NEW.meeting_id", "", None, "meeting_id, person_key"),
    ("live_alert", "meeting_text", "NEW.meeting_id", "NEW.meeting_id IS NOT NULL AND ", None, "meeting_id, quote, restatement, asker, draft_json"),
    ("meeting_marker", "meeting_text", "NEW.meeting_id", "", None, "meeting_id, label"),
    // The owner and due date of an action item and a normalized copy of a transcript line (device-local).
    ("meeting_line_source", "meeting_text", "NEW.meeting_id", "", None, "meeting_id, line_text, owner, due, anchor"),
    ("live_transcript_segment", "meeting_text", "NEW.meeting_id", "", None, "meeting_id, text"),
    ("meeting_snapshot", "meeting_text", "NEW.meeting_id", "", None, "meeting_id, ocr_text"),
    ("transcript_segment", "transcript_segment", "NEW.id", "", None, "meeting_id, text"),
    ("episodes", "episodes", "NEW.id", "", Some("1"), "device_id, start_item_id, end_item_id"),
    ("threads", "threads", "NEW.id", "", Some("1"), "source, external_thread_id, account_email"),
    ("routine_run", "routine_run", "NEW.id", "", Some("1"), "output_summary, output_json, provenance"),
];

const GENERATION: &str = "exclusion_generation";

/// How long the material is trusted without a full recompute. The dirty rows older than twice
/// this are dropped, so a cache older than this never needs them.
const FULL_RECOMPUTE_MS: i64 = 30 * 60_000;

/// The excluded material cached on a [`Store`], with what it was computed from.
pub(crate) struct ExclusionCache {
    generation: String,
    material: ExcludedMaterial,
    /// When the material was last computed in full (unix ms).
    full_at_ms: i64,
    /// The newest `exclusion_dirty.seq` already applied. Each store keeps its own: the rows are
    /// shared by every connection, so a read never deletes them (they age out).
    cursor: i64,
}

#[cfg(test)]
thread_local! {
    /// Full recomputes on this thread, for tests.
    pub(crate) static FULL_RECOMPUTES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Ids newly added to the material by one incremental pass.
#[derive(Default)]
struct Newly {
    items: Vec<i64>,
    meetings: Vec<i64>,
    threads: Vec<i64>,
}

impl Newly {
    fn item(&mut self, m: &mut ExcludedMaterial, id: i64) {
        if m.items.insert(id) {
            self.items.push(id);
        }
    }
    fn meeting(&mut self, m: &mut ExcludedMaterial, id: i64) {
        if m.meetings.insert(id) {
            self.meetings.push(id);
        }
    }
    fn thread(&mut self, m: &mut ExcludedMaterial, id: i64) {
        if m.threads.insert(id) {
            self.threads.push(id);
        }
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty() && self.meetings.is_empty() && self.threads.is_empty()
    }
}

/// `SELECT` of the ids noted under `tbl` in the dirty queue after `since`. There is no upper
/// bound: an update moves its row to a higher seq, so a bound read earlier could skip a row that
/// was noted again meanwhile. A row seen twice is checked twice, which is harmless.
fn dirty_ids(tbl: &str, since: i64) -> String {
    format!("SELECT row_id FROM exclusion_dirty WHERE tbl = '{tbl}' AND seq > {since}")
}

/// `ids` as a comma list for an `IN (...)`, in chunks that keep a statement short.
fn id_chunks(ids: &[i64]) -> impl Iterator<Item = String> + '_ {
    ids.chunks(500).map(|c| c.iter().map(i64::to_string).collect::<Vec<_>>().join(","))
}

/// How long before an excluded event's start a meeting overlapping it counts as the event's.
pub const EVENT_LEAD_MS: i64 = 30 * 60_000;
/// How long after an excluded event's start (events store no end) a meeting overlapping it
/// counts as the event's. Generous on purpose: over-excluding is accepted.
pub const EVENT_SPAN_MS: i64 = 3 * 3_600_000;
/// A meeting with no end yet spans this long for the overlap test.
const MEETING_DEFAULT_MS: i64 = 3_600_000;

/// A calendar event's series id: its calendar id without a recurring instance's stamp (the
/// topic index's series key without the source), so every instance of a series shares it.
pub fn event_series_id(external_id: &str) -> String {
    super::index::calendar_series_key("", external_id)
}

fn key_hash(key: &str) -> String {
    format!("{:016x}", super::fnv1a(key.bytes()))
}

/// The provenance of a run built from unfiltered material while some client was excluded, or
/// whose inputs were not recorded: never clean while any client is excluded.
pub const UNFILTERED_PROVENANCE: &str = "unfiltered";

/// The most inputs a clean stamp records (fix round 4, S2). More than this and the answer or run
/// gets no clean stamp at all: the list is never cut short.
pub const MAX_INPUT_REFS: usize = 256;

const REFS_PREFIX: &str = "refs:";

/// One input an answer or run was built from: `kind:id`, where `kind` is a retrieval or
/// topic-source kind (`meeting`, `episode`, `thread`, `routine_run`, `capture`, `snapshot`,
/// `email`, `calendar_event`: [`ExcludedMaterial::excludes`]) with an `items`/row id, or `event`
/// with a calendar event id.
pub fn input_ref(kind: &str, id: impl std::fmt::Display) -> String {
    format!("{kind}:{id}")
}

/// The stamp recording `refs` (deduplicated), or `None` when there are more than
/// [`MAX_INPUT_REFS`]: such an answer or run is not clean for hosted use.
pub fn input_refs_stamp(refs: impl IntoIterator<Item = String>) -> Option<String> {
    let refs: std::collections::BTreeSet<String> = refs.into_iter().collect();
    if refs.len() > MAX_INPUT_REFS {
        return None;
    }
    serde_json::to_string(&refs).ok().map(|json| format!("{REFS_PREFIX}{json}"))
}

/// The inputs a stamp from [`input_refs_stamp`] records; `None` for any other stamp (older
/// formats, `unfiltered`, nothing), which is never clean while a client is excluded.
pub fn stamp_input_refs(stamp: &str) -> Option<Vec<String>> {
    serde_json::from_str(stamp.strip_prefix(REFS_PREFIX)?).ok()
}

/// Lowercase addresses in an attendee or actor string (`Name <a@b>`, `a@b`, a list).
fn addresses(raw: &str) -> Vec<String> {
    super::entities::parse_addresses(raw).into_iter().map(|a| a.email.to_lowercase()).collect()
}

/// Every string value under an `"email"` key in a JSON document (attendee details).
pub(crate) fn json_emails(raw: &str) -> Vec<String> {
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, x) in m {
                    match (k.as_str(), x) {
                        ("email", serde_json::Value::String(e)) => out.push(e.trim().to_lowercase()),
                        _ => walk(x, out),
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            serde_json::Value::String(s) => out.extend(addresses(s)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        walk(&v, &mut out);
    }
    out
}

impl Store {
    /// Everything filed under a client excluded from hosted models. Errors when the setting or
    /// the membership cannot be read; callers then treat everything as excluded (fail closed).
    pub fn excluded_material(&self) -> rusqlite::Result<ExcludedMaterial> {
        if self.exclusion_unreadable() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut out = ExcludedMaterial { terms: self.excluded_terms()?, ..Default::default() };
        let keys: Vec<String> = self
            .conn
            .prepare(
                "SELECT c.topic_key FROM client_control c JOIN topic_seed s ON s.key = c.topic_key \
                 WHERE c.exclude_hosted != 0 AND s.deleted = 0 ORDER BY c.topic_key",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if keys.is_empty() {
            return Ok(out);
        }
        for key in &keys {
            for k in crate::client_forget::merge_chain(&self.conn, key)? {
                out.topic_keys.insert(k);
            }
            out.topic_ids.extend(self.client_scope(key)?.topic_ids);
            for m in self.client_members(key)? {
                out.source_ids.insert(m.source_id);
                let Some(r) = m.ref_id else { continue };
                self.add_member(&mut out, &m.kind, r, &key_hash(key))?;
            }
        }
        out.attendee_emails = self.client_only_emails(&out.topic_ids)?;
        Ok(out)
    }

    /// Adds the material a source of `kind` (`ref_id` `r`) filed under a client stands for.
    /// `owner` is the fingerprint of the client's key.
    fn add_member(&self, out: &mut ExcludedMaterial, kind: &str, r: i64, owner: &str) -> rusqlite::Result<()> {
        match kind {
            "meeting" => {
                out.meetings.insert(r);
            }
            "episode" => {
                out.episodes.insert(r);
                for (item, _, _) in self.episode_captures(r)? {
                    out.items.insert(item);
                }
            }
            "snapshot" => {
                out.items.insert(r);
                if let Some((meeting, _)) = self.snapshot_meeting(r)? {
                    out.meetings.insert(meeting);
                }
            }
            "email" => {
                out.items.insert(r);
                out.threads.extend(self.thread_of_email(r)?);
            }
            "calendar_event" => {
                out.items.insert(r);
                out.event_owners.entry(r).or_default().push(owner.to_string());
            }
            _ => {
                out.items.insert(r);
            }
        }
        Ok(())
    }

    /// The email thread holding email item `item_id`, if this device has it.
    pub fn thread_of_email(&self, item_id: i64) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT t.id FROM items i JOIN threads t ON t.source = i.source \
                   AND t.external_thread_id = json_extract(i.refs, '$.gmail_thread_id') \
                   AND t.account_email = json_extract(i.refs, '$.account_email') \
                 WHERE i.id = ?1 AND i.kind = 'email'",
                [item_id],
                |r| r.get(0),
            )
            .optional()
    }

    /// Addresses of the people mentioned by sources in `topics` and by no source filed under any
    /// other topic: the client's own contacts, not colleagues who attend everything. Known limit:
    /// the user's own addresses are known only from connected accounts (`connector_account`, see
    /// `index::self_emails`), so with none connected the user's own address, met only in one
    /// client's material, is counted as that client's contact.
    pub(crate) fn client_only_emails(&self, topics: &HashSet<i64>) -> rusqlite::Result<HashSet<String>> {
        let mut inside: HashSet<(i64, String)> = HashSet::new();
        let mut outside: HashSet<i64> = HashSet::new();
        // Only other clients' (seed) topics count as outside: automatic topics are clustered
        // independently of filing, so a frequent correspondent is in one whatever client he is
        // (fix round 3, R5).
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT m.topic_id, e.id, substr(e.key, 7), \
                    EXISTS (SELECT 1 FROM topic_seed sd WHERE sd.topic_id = m.topic_id AND sd.deleted = 0) \
             FROM topic_member m \
             JOIN entity_mention em ON em.source_id = m.source_id \
             JOIN entity e ON e.id = em.entity_id \
             WHERE m.origin != 'removed' AND e.kind = 'person' AND e.key LIKE 'email:%'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?))
        })?;
        for row in rows {
            let (topic, entity, email, seed) = row?;
            if topics.contains(&topic) {
                inside.insert((entity, email.to_lowercase()));
            } else if seed {
                outside.insert(entity);
            }
        }
        // Never the user's own account addresses (fix round 4, M3, as amended): only the exact
        // addresses. A domain is never treated as the user's, so a client contact who shares the
        // user's domain stays the client's; colleagues are over-excluded (the safe default).
        let own: HashSet<String> = super::index::self_emails(&self.conn).into_iter().collect();
        let is_own = |email: &str| own.contains(email);
        Ok(inside.into_iter().filter(|(e, m)| !outside.contains(e) && !is_own(m)).map(|(_, m)| m).collect())
    }

    /// Every topic source as `(id, kind, ref_id, title and indexed text)`: what
    /// `api::exclusion::excluded_source_ids` runs the shared predicate over.
    pub fn topic_source_refs(&self) -> rusqlite::Result<Vec<(i64, String, Option<i64>, String)>> {
        self.conn
            .prepare(
                "SELECT s.id, s.kind, s.ref_id, s.title || ' ' || coalesce(t.text, '') FROM topic_source s \
                 LEFT JOIN topic_source_text t ON t.source_id = s.id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect()
    }

    /// The topics with any of `sources` as a member (not `removed`).
    pub fn topics_holding_sources(&self, sources: &HashSet<i64>) -> rusqlite::Result<HashSet<i64>> {
        let mut out = HashSet::new();
        let mut stmt = self.conn.prepare("SELECT topic_id, source_id FROM topic_member WHERE origin != 'removed'")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (topic, source) = row?;
            if sources.contains(&source) {
                out.insert(topic);
            }
        }
        Ok(out)
    }

    /// The exclusion generation: moves on every write to what says who is excluded (settings,
    /// terms, membership, contacts) or to a topic source's indexed text. New or changed meetings,
    /// emails, captures and the like do not move it: they are noted in `exclusion_dirty`, which
    /// `excluded_material_with` applies. A cache of anything those rows can change must also
    /// key on [`ExcludedMaterial::material_version`].
    pub fn exclusion_generation(&self) -> rusqlite::Result<String> {
        Ok(self
            .conn
            .query_row("SELECT value FROM kv_meta WHERE key = ?1", [GENERATION], |r| r.get(0))
            .optional()?
            .unwrap_or_default())
    }

    /// How many topic sources and indexed texts there are: a new one moves no generation (it
    /// matters once a membership links it), so a cache of what names a client in them keys on
    /// this too.
    pub fn topic_source_signal(&self) -> rusqlite::Result<String> {
        let (a, b): (i64, i64) = self.conn.query_row(
            "SELECT (SELECT count(*) FROM topic_source), (SELECT count(*) FROM topic_source_text)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(format!("{a}.{b}"))
    }

    /// Every text about meeting `meeting_id` that a user or a model wrote beyond the `meeting`
    /// row: the workspace draft, action items, follow-ups, the people named as its speakers, live
    /// alerts, marker labels, the lines, owners, due dates and anchors of a note's line sources, live
    /// transcript lines and slide text (fix round 4, S1). The same tables the meeting scan in
    /// `name_hits_limited` reads.
    pub fn meeting_authored_text(&self, meeting_id: i64) -> rusqlite::Result<String> {
        let mut out = self.get_meeting_note_draft(meeting_id)?.map(|d| d.text()).unwrap_or_default();
        for sql in [
            "SELECT text FROM meeting_action_item WHERE meeting_id = ?1",
            "SELECT text FROM meeting_followup WHERE meeting_id = ?1",
            "SELECT p.name || ' ' || coalesce(p.email, '') FROM meeting_speaker sp JOIN person p ON p.person_key = sp.person_key WHERE sp.meeting_id = ?1",
            "SELECT quote || ' ' || restatement || ' ' || coalesce(asker, '') || ' ' || coalesce(draft_json, '') FROM live_alert WHERE meeting_id = ?1",
            "SELECT label FROM meeting_marker WHERE meeting_id = ?1 AND label IS NOT NULL",
            "SELECT line_text || ' ' || coalesce(owner, '') || ' ' || coalesce(due, '') || ' ' || coalesce(anchor, '') FROM meeting_line_source WHERE meeting_id = ?1",
            "SELECT text FROM live_transcript_segment WHERE meeting_id = ?1",
            "SELECT ocr_text FROM meeting_snapshot WHERE meeting_id = ?1",
        ] {
            let mut stmt = self.conn.prepare(sql)?;
            for t in stmt.query_map([meeting_id], |r| r.get::<_, String>(0))? {
                out.push('\n');
                // A live-alert draft is JSON: matched with escapes decoded (fix round 6, J2).
                out.push_str(&crate::words::name_text(&t?));
            }
        }
        Ok(out)
    }

    /// [`Self::excluded_material`] plus everything that names an excluded client (`names`: the
    /// caller's whole-word matcher over [`ExcludedMaterial::terms`]) or carries a client contact's
    /// address, the meetings recorded from the client's calendar events, and the routine runs
    /// still clean. The one predicate every exclusion path reads (see the module doc). Cached:
    /// computed in full when the exclusion generation moves (a setting, a term, membership, a
    /// contact) or every 30 minutes; in between, only the rows written since are checked and
    /// whatever they name or link to is added (never removed: a row edited to no longer name a
    /// client stays excluded until the next full computation). Errors fail closed at the caller.
    pub fn excluded_material_with(&self, names: &dyn Fn(&str) -> bool) -> rusqlite::Result<ExcludedMaterial> {
        self.excluded_material_upto(names, None)
    }

    /// [`Self::excluded_material_with`], with the newest queue row to account for given (read
    /// now when `None`). A row noted after it is applied when it is seen, and again next time.
    fn excluded_material_upto(&self, names: &dyn Fn(&str) -> bool, upper: Option<i64>) -> rusqlite::Result<ExcludedMaterial> {
        // Checked every time: a setting that stops being readable moves no generation.
        if self.exclusion_unreadable() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let generation: String = self
            .conn
            .query_row("SELECT value FROM kv_meta WHERE key = ?1", [GENERATION], |r| r.get(0))
            .optional()?
            .unwrap_or_default();
        // Taken out, so a failure leaves no half-updated cache: the next call computes in full.
        let prior = self.exclusion_cache.borrow_mut().take();
        let now = crate::now_unix_ms();
        // Read before anything is computed: a row noted while this runs is applied next time.
        let upper: i64 = match upper {
            Some(u) => u,
            None => self.conn.query_row("SELECT coalesce(max(seq), 0) FROM exclusion_dirty", [], |r| r.get(0))?,
        };
        if let Some(mut cache) = prior {
            let age = now - cache.full_at_ms;
            if cache.generation == generation && (0..=FULL_RECOMPUTE_MS).contains(&age) {
                let mut full = false;
                if upper > cache.cursor && !cache.material.is_empty() {
                    full = self.apply_dirty(&mut cache.material, names, cache.cursor)?;
                    self.prune_dirty(now);
                }
                if !full {
                    cache.cursor = upper;
                    let out = cache.material.clone();
                    *self.exclusion_cache.borrow_mut() = Some(cache);
                    return Ok(out);
                }
            }
        }
        let m = self.compute_excluded_material(names)?;
        if !m.is_empty() {
            self.prune_dirty(now);
        }
        *self.exclusion_cache.borrow_mut() = Some(ExclusionCache {
            generation,
            material: m.clone(),
            full_at_ms: now,
            cursor: upper,
        });
        Ok(m)
    }

    /// Drops dirty rows older than an hour. No cache older than 30 minutes uses the queue, so
    /// nothing a store still needs goes. Best effort: a busy database is tried next time.
    fn prune_dirty(&self, now_ms: i64) {
        let cutoff = now_ms / 1000 - 2 * FULL_RECOMPUTE_MS / 1000;
        let old = self
            .conn
            .query_row("SELECT 1 FROM exclusion_dirty WHERE at_unix_s < ?1 LIMIT 1", [cutoff], |_| Ok(()))
            .optional();
        if matches!(old, Ok(Some(()))) {
            // Never waits for the write lock (a read must not): busy timeout 0 while it runs, then
            // whatever the shell set (as `record_spans_best_effort`).
            let before = self.busy_timeout_ms().unwrap_or(5000).max(0) as u64;
            let _ = self.conn.busy_timeout(std::time::Duration::ZERO);
            let _ = self.conn.execute("DELETE FROM exclusion_dirty WHERE at_unix_s < ?1", [cutoff]);
            let _ = self.conn.busy_timeout(std::time::Duration::from_millis(before));
        }
    }

    fn compute_excluded_material(&self, names: &dyn Fn(&str) -> bool) -> rusqlite::Result<ExcludedMaterial> {
        #[cfg(test)]
        FULL_RECOMPUTES.with(|c| c.set(c.get() + 1));
        let mut m = self.excluded_material()?;
        if m.is_empty() {
            return Ok(m);
        }
        let mut hashes: Vec<String> = m.topic_keys.iter().map(|k| key_hash(k)).collect();
        hashes.sort();
        m.key_hashes = hashes;
        // Every candidate, never a capped page: a client named in thousands of places is still
        // found in all of them.
        if !m.terms.is_empty() {
            let hits = self.name_hits_limited(&m.terms, names, i64::MAX)?;
            for h in hits.hits {
                match h.kind {
                    "meeting" => {
                        m.meetings.insert(h.ref_id);
                    }
                    "email" => {
                        m.items.insert(h.ref_id);
                        m.threads.extend(self.thread_of_email(h.ref_id)?);
                    }
                    "calendar_event" | "mcp_item" => {
                        m.items.insert(h.ref_id);
                    }
                    // `capture_item.id`: its `items` twin, the id every other path uses.
                    "capture" => {
                        m.items.extend(self.capture_items_of_legacy(h.ref_id)?);
                    }
                    _ => {}
                }
            }
            // Captures with no legacy twin, and URLs (not in the full-text index).
            m.items.extend(self.capture_items_naming(&m.terms, names)?);
            // Added items named only outside the full-text index: the adding client's name, the
            // source label, the link, a person's name.
            m.items.extend(self.mcp_items_naming_outside_text(names, None)?);
        }
        self.contact_hits(&mut m, None, &mut Newly::default())?;
        self.lift_events_and_meetings(&mut m)?;
        // Derived from what is excluded, so the item-level answer matches the meeting-, thread-
        // and episode-level one (fix round 2, N2, N4): every slide of an excluded meeting, every
        // email of an excluded thread (and the thread of every excluded email), every episode
        // built over an excluded capture.
        self.lift_derived(&mut m)?;
        m.material_version = m.version();
        self.settle_runs(&mut m, names, None)?;
        Ok(m)
    }

    /// Added items (`mcp_item`) whose adding client's name, source label, link or people name a client.
    /// `ids_sql`, when given, limits it to the ids that subquery selects (the rows noted dirty).
    pub(crate) fn mcp_items_naming_outside_text(&self, names: &dyn Fn(&str) -> bool, ids_sql: Option<&str>) -> rusqlite::Result<Vec<i64>> {
        Ok(self.mcp_items_naming_outside_text_at(names, ids_sql)?.into_iter().map(|(id, _)| id).collect())
    }

    /// [`Self::mcp_items_naming_outside_text`] with each item's time.
    pub(crate) fn mcp_items_naming_outside_text_at(&self, names: &dyn Fn(&str) -> bool, ids_sql: Option<&str>) -> rusqlite::Result<Vec<(i64, i64)>> {
        let filter = ids_sql.map(|q| format!(" AND id IN ({q})")).unwrap_or_default();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, ts_unix_ms, coalesce(json_extract(refs, '$.client'), '') || '\n' || coalesce(source, '') || '\n' || \
             coalesce(url, '') || '\n' || coalesce(actors, '') FROM items WHERE kind = 'mcp_item'{filter}"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, ts, text) = row?;
            // The source is `mcp:<token>:<label>`: the label's dashes read as word breaks.
            if names(&text) || names(&text.replace('-', " ")) {
                out.push((id, ts));
            }
        }
        Ok(out)
    }

    /// A client contact's address: meetings by their stored attendees and attendee details,
    /// emails and events by their actors. `only` limits it to the rows noted dirty in that range.
    fn contact_hits(&self, m: &mut ExcludedMaterial, only: Option<i64>, newly: &mut Newly) -> rusqlite::Result<()> {
        if m.attendee_emails.is_empty() {
            return Ok(());
        }
        let filter = |tbl: &str, col: &str, and: bool| {
            only.map(|r| format!(" {} {col} IN ({})", if and { "AND" } else { "WHERE" }, dirty_ids(tbl, r))).unwrap_or_default()
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, attendees, coalesce(attendee_details, '') FROM meeting{}",
            filter("meeting", "id", false)
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in rows {
            let (id, attendees, details) = row?;
            if json_emails(&attendees).iter().chain(json_emails(&details).iter()).any(|e| m.attendee_emails.contains(e)) {
                newly.meeting(m, id);
            }
        }
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, kind, coalesce(actors, '') FROM items WHERE kind IN ('email', 'calendar_event', 'mcp_item'){}",
            filter("items", "id", true)
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut emails = Vec::new();
        for row in rows {
            let (id, kind, actors) = row?;
            if json_emails(&actors).iter().any(|e| m.attendee_emails.contains(e)) {
                newly.item(m, id);
                if kind == "email" {
                    emails.push(id);
                }
            }
        }
        for id in emails {
            if let Some(t) = self.thread_of_email(id)? {
                newly.thread(m, t);
            }
        }
        Ok(())
    }

    /// Routine runs clean for the material excluded now: none of their recorded inputs is in it
    /// (fix round 4, S2). A run built from another run is clean only while that run is, so
    /// this settles to a fixed point. `only` limits it to the runs noted dirty in that range.
    /// A run whose own words name a client excluded now is never clean, whatever its inputs (fix
    /// round 5, N4: a brief's event titles, an input deleted before it could be lifted).
    /// `output_json` is matched decoded (fix round 6, J3); one that does not parse is not clean.
    fn settle_runs(&self, m: &mut ExcludedMaterial, names: &dyn Fn(&str) -> bool, only: Option<i64>) -> rusqlite::Result<()> {
        let filter = only.map(|r| format!(" AND id IN ({})", dirty_ids("routine_run", r))).unwrap_or_default();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, provenance, coalesce(output_summary, ''), coalesce(output_json, '{{}}') \
             FROM routine_run WHERE provenance LIKE 'refs:%'{filter}"
        ))?;
        let mut pending: Vec<(i64, Vec<String>)> = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?
            .filter_map(|row| match row {
                Ok((id, stamp, summary, json)) => {
                    let words = crate::words::json_name_text(&json)?;
                    if names(&summary) || names(&words) {
                        return None;
                    }
                    stamp_input_refs(&stamp).map(|refs| Ok((id, refs)))
                }
                Err(e) => Some(Err(e)),
            })
            .collect::<rusqlite::Result<_>>()?;
        // An input whose row is gone cannot be checked: such a run is never clean.
        let missing = self.missing_input_refs(pending.iter().flat_map(|(_, refs)| refs.iter()))?;
        pending.retain(|(_, refs)| refs.iter().all(|r| !missing.contains(r)));
        loop {
            let before = pending.len();
            pending.retain(|(id, refs)| {
                if refs.iter().all(|r| !m.excludes_input_ref(r)) {
                    m.clean_runs.insert(*id);
                    false
                } else {
                    true
                }
            });
            if pending.len() == before {
                break;
            }
        }
        Ok(())
    }

    /// The incremental pass of [`Self::excluded_material_with`]: checks the rows noted dirty in
    /// `range` against the cached material and adds whatever they name or link to, by the same
    /// predicates as the full computation, row by row. Nothing is removed (an edited row that no
    /// longer names a client stays excluded until the next full computation). The one thing that
    /// can shrink is `clean_runs`, which is the permitted side: a run stops being clean when an
    /// input of it is now excluded.
    /// Returns `true` when a row needs the full computation (a calendar event filed under a
    /// client, whose series and tombstone owners are worked out there).
    fn apply_dirty(&self, m: &mut ExcludedMaterial, names: &dyn Fn(&str) -> bool, range: i64) -> rusqlite::Result<bool> {
        let dirty = |tbl: &str| dirty_ids(tbl, range);
        let mut newly = Newly::default();
        let mut members_grew = false;
        // Memberships written since: a source filed under an excluded client's topics brings
        // its material, as in `excluded_material`.
        let members: Vec<(i64, i64, String, Option<i64>)> = self
            .conn
            .prepare(&format!(
                "SELECT m.topic_id, s.id, s.kind, s.ref_id FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
                 WHERE m.origin != 'removed' AND m.rowid IN ({})",
                dirty("topic_member")
            ))?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut email_sources: Vec<i64> = Vec::new();
        // Sources filed under another client's topic: a person met there too is no longer only
        // this client's contact.
        let mut outside_sources: Vec<i64> = Vec::new();
        let mut is_seed = self.conn.prepare("SELECT 1 FROM topic_seed WHERE topic_id = ?1 AND deleted = 0")?;
        for (topic, source, kind, r) in members {
            if !m.topic_ids.contains(&topic) {
                if is_seed.exists([topic])? {
                    outside_sources.push(source);
                }
                continue;
            }
            if kind == "calendar_event" {
                return Ok(true);
            }
            members_grew |= m.source_ids.insert(source);
            email_sources.push(source);
            if let Some(r) = r {
                let mut found = ExcludedMaterial::default();
                self.add_member(&mut found, &kind, r, "")?;
                for id in found.items {
                    newly.item(m, id);
                }
                for id in found.meetings {
                    newly.meeting(m, id);
                }
                for id in found.threads {
                    newly.thread(m, id);
                }
                for id in found.episodes {
                    members_grew |= m.episodes.insert(id);
                }
            }
        }
        // Mentions written since, of a person's address on a source filed under a client.
        let mentioned: Vec<i64> = self
            .conn
            .prepare(&format!(
                "SELECT em.source_id FROM entity_mention em JOIN entity e ON e.id = em.entity_id \
                 WHERE e.kind = 'person' AND e.key LIKE 'email:%' AND em.rowid IN ({})",
                dirty("entity_mention")
            ))?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut member_of = self.conn.prepare("SELECT topic_id FROM topic_member WHERE source_id = ?1 AND origin != 'removed'")?;
        let mut has_address = self.conn.prepare(
            "SELECT 1 FROM entity_mention em JOIN entity e ON e.id = em.entity_id \
             WHERE em.source_id = ?1 AND e.kind = 'person' AND e.key LIKE 'email:%' LIMIT 1",
        )?;
        let mut contacts_may_grow = false;
        let mut contacts_may_shrink = false;
        for source in mentioned {
            for t in member_of.query_map([source], |r| r.get::<_, i64>(0))? {
                let t = t?;
                contacts_may_grow |= m.topic_ids.contains(&t);
                contacts_may_shrink |= !m.topic_ids.contains(&t) && is_seed.exists([t])?;
            }
        }
        for source in email_sources {
            contacts_may_grow |= has_address.exists([source])?;
        }
        for source in outside_sources {
            contacts_may_shrink |= has_address.exists([source])?;
        }
        if contacts_may_grow || contacts_may_shrink {
            // The client's contacts as the full path works them out; any new address is searched
            // for in meetings and emails.
            let fresh = self.client_only_emails(&m.topic_ids)?;
            // A contact who stopped being the client's is the full computation's to drop.
            if m.attendee_emails.iter().any(|e| !fresh.contains(e)) {
                return Ok(true);
            }
            let added: Vec<String> = fresh.into_iter().filter(|e| !m.attendee_emails.contains(e)).collect();
            // A new contact brings every older email, event and meeting of theirs, and the event
            // lift that goes with it: the full computation's.
            if !added.is_empty() {
                return Ok(true);
            }
        }
        // Names, as `name_hits_limited` and `capture_items_naming` read them, without the
        // full-text prefilter: the whole-word matcher decides.
        if !m.terms.is_empty() {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT id, kind, thread_id, coalesce(title, ''), body_text, url FROM items \
                 WHERE kind IN ('email', 'calendar_event', 'snapshot', 'capture', 'mcp_item') AND id IN ({})",
                dirty("items")
            ))?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, Option<String>>(5)?))
            })?;
            for row in rows {
                let (id, kind, thread, title, body, url) = row?;
                match kind.as_str() {
                    "email" if names(&format!("{title}\n{body}")) => {
                        newly.item(m, id);
                        if let Some(t) = self.thread_of_email(id)? {
                            newly.thread(m, t);
                        }
                    }
                    "calendar_event" if names(&format!("{title}\n{body}")) => newly.item(m, id),
                    "mcp_item" if names(&format!("{title}\n{body}")) || names(&url.clone().unwrap_or_default()) => newly.item(m, id),
                    "snapshot" if names(&format!("{title}\n{body}")) => {
                        if let Some(meeting) = thread {
                            newly.meeting(m, meeting);
                        }
                    }
                    "capture" => {
                        let url = url.unwrap_or_default();
                        if names(&format!("{title} {body} {url}")) || names(&url) {
                            newly.item(m, id);
                        }
                    }
                    _ => {}
                }
            }
            // Added items named outside their text (the adding client's name, label, link, people).
            for id in self.mcp_items_naming_outside_text(names, Some(&dirty("items")))? {
                newly.item(m, id);
            }
            // Legacy capture rows: the `items` twin is the id every path uses.
            let mut stmt = self.conn.prepare(&format!(
                "SELECT id, window_title, text FROM capture_item WHERE id IN ({})",
                dirty("capture_item")
            ))?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (id, window, text) = row?;
                if names(&format!("{window}\n{text}")) {
                    for item in self.capture_items_of_legacy(id)? {
                        newly.item(m, item);
                    }
                }
            }
            let mut stmt = self.conn.prepare(&format!(
                "SELECT s.meeting_id, s.text FROM transcript_segment s JOIN meeting m ON m.id = s.meeting_id WHERE s.id IN ({})",
                dirty("transcript_segment")
            ))?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (meeting, text) = row?;
                if names(&text) {
                    newly.meeting(m, meeting);
                }
            }
            let only = format!("{} UNION {}", dirty("meeting"), dirty("meeting_text"));
            for (id, _) in self.meeting_text_hits(names, Some(&only))? {
                newly.meeting(m, id);
            }
        }
        self.contact_hits(m, Some(range), &mut newly)?;
        // Meetings recorded from the client's calendar events, and events of an excluded series:
        // those tables are small, so the whole lift runs when a meeting or an event changed.
        let events_changed: bool = self
            .conn
            .prepare(&format!("SELECT 1 FROM items WHERE kind = 'calendar_event' AND id IN ({}) LIMIT 1", dirty("items")))?
            .exists([])?
            || self.conn.prepare(&format!("SELECT 1 WHERE EXISTS ({} LIMIT 1)", dirty("meeting")))?.exists([])?;
        let mut lifted = false;
        if events_changed {
            let before: HashSet<i64> = m.meetings.clone();
            let sizes = (m.items.len(), m.event_ids.len(), m.event_series.len(), m.events.len());
            self.lift_events_and_meetings(m)?;
            newly.meetings.extend(m.meetings.difference(&before).copied());
            lifted = sizes != (m.items.len(), m.event_ids.len(), m.event_series.len(), m.events.len());
        }
        // Derived, as in `lift_derived`. A new email in an excluded thread.
        let emails: Vec<i64> = self
            .conn
            .prepare(&format!("SELECT id FROM items WHERE kind = 'email' AND id IN ({})", dirty("items")))?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in emails {
            if let Some(t) = self.thread_of_email(id)? {
                if m.threads.contains(&t) {
                    newly.item(m, id);
                }
            }
        }
        // A thread that is new, or became excluded: every email of it goes with it, and an
        // excluded email makes its thread excluded.
        let mut threads: Vec<i64> = self
            .conn
            .prepare(&format!("SELECT id FROM threads WHERE id IN ({})", dirty("threads")))?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        threads.extend(newly.threads.iter().copied());
        threads.sort_unstable();
        threads.dedup();
        for list in id_chunks(&threads) {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT i.id, t.id FROM threads t CROSS JOIN items i ON i.source = t.source \
                   AND json_extract(i.refs, '$.gmail_thread_id') = t.external_thread_id \
                   AND json_extract(i.refs, '$.account_email') = t.account_email \
                 WHERE t.id IN ({list}) AND i.kind = 'email'"
            ))?;
            let mut by_thread: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
            for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
                let (item, thread) = row?;
                by_thread.entry(thread).or_default().push(item);
            }
            for (thread, items) in by_thread {
                if m.threads.contains(&thread) || items.iter().any(|i| m.items.contains(i)) {
                    newly.thread(m, thread);
                    for item in items {
                        newly.item(m, item);
                    }
                }
            }
        }
        // Slides of an excluded meeting.
        let meetings = newly.meetings.clone();
        for list in id_chunks(&meetings) {
            let ids: Vec<i64> = self
                .conn
                .prepare(&format!("SELECT id FROM items WHERE kind = 'snapshot' AND thread_id IN ({list})"))?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for id in ids {
                newly.item(m, id);
            }
        }
        let slides: Vec<(i64, i64)> = self
            .conn
            .prepare(&format!(
                "SELECT id, thread_id FROM items WHERE kind = 'snapshot' AND thread_id IS NOT NULL AND id IN ({})",
                dirty("items")
            ))?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (item, meeting) in slides {
            if m.meetings.contains(&meeting) {
                newly.item(m, item);
            }
        }
        // Episodes over an excluded capture: looked up from each newly excluded capture, and
        // each new episode over the captures already excluded.
        let mut episodes: HashSet<i64> = HashSet::new();
        let fresh = newly.items.clone();
        for list in id_chunks(&fresh) {
            let caps: Vec<(i64, String)> = self
                .conn
                .prepare(&format!("SELECT id, device_id FROM items WHERE kind = 'capture' AND id IN ({list})"))?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut stmt = self.conn.prepare(
                "SELECT id FROM episodes WHERE device_id = ?1 AND end_item_id >= ?2 AND start_item_id <= ?2",
            )?;
            for (item, device) in caps {
                for id in stmt.query_map(params![device, item], |r| r.get::<_, i64>(0))? {
                    episodes.insert(id?);
                }
            }
        }
        let new_episodes: Vec<i64> = self
            .conn
            .prepare(&format!("SELECT id FROM episodes WHERE id IN ({})", dirty("episodes")))?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in new_episodes {
            let captures = self.episode_captures(id)?;
            if m.episodes.contains(&id) {
                // Already excluded, and perhaps grown since: all its captures go with it.
                for (item, _, _) in captures {
                    newly.item(m, item);
                }
            } else if captures.iter().any(|(item, _, _)| m.items.contains(item)) {
                episodes.insert(id);
            }
        }
        let mut grew = lifted || members_grew || !newly.is_empty();
        for id in episodes {
            if m.episodes.insert(id) {
                grew = true;
                for (item, _, _) in self.episode_captures(id)? {
                    newly.item(m, item);
                }
            }
        }
        grew |= !newly.is_empty();
        if grew {
            m.material_version = m.version();
        }
        // Runs: a changed run is judged again; when anything was added or a run changed, a run
        // with an input that is excluded (or no longer clean) stops being clean, and every run
        // built on it.
        let changed_runs: Vec<i64> = self
            .conn
            .prepare(&dirty("routine_run"))?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in &changed_runs {
            m.clean_runs.remove(id);
        }
        if !changed_runs.is_empty() {
            self.settle_runs(m, names, Some(range))?;
        }
        if (grew || !changed_runs.is_empty()) && !m.clean_runs.is_empty() {
            let mut stamps: Vec<(i64, Vec<String>)> = Vec::new();
            let mut stmt = self.conn.prepare("SELECT id, provenance FROM routine_run WHERE provenance LIKE 'refs:%'")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (id, stamp) = row?;
                if m.clean_runs.contains(&id) {
                    stamps.push((id, stamp_input_refs(&stamp).unwrap_or_default()));
                }
            }
            loop {
                let before = m.clean_runs.len();
                for (id, refs) in &stamps {
                    if m.clean_runs.contains(id) && refs.iter().any(|r| m.excludes_input_ref(r)) {
                        m.clean_runs.remove(id);
                    }
                }
                if m.clean_runs.len() == before {
                    break;
                }
            }
        }
        Ok(false)
    }

    /// The `items` twins of legacy capture row `capture_item_id`.
    fn capture_items_of_legacy(&self, capture_item_id: i64) -> rusqlite::Result<Vec<i64>> {
        self.conn
            .prepare("SELECT id FROM items WHERE kind = 'capture' AND source_table = 'capture_item' AND source_id = ?1")?
            .query_map([capture_item_id], |r| r.get(0))?
            .collect()
    }

    /// Capture `items` whose title, text or URL names a client: full-text candidates, and URL
    /// candidates by substring, both confirmed by `names` (whole words).
    fn capture_items_naming(&self, terms: &[String], names: &dyn Fn(&str) -> bool) -> rusqlite::Result<HashSet<i64>> {
        let mut out = HashSet::new();
        if let Some(q) = super::inventory::fts_query_for(terms) {
            // Full-text first (CROSS JOIN keeps that order): a plan that walks every capture and
            // probes the index per row is what made a recompute take seconds.
            let mut stmt = self.conn.prepare(
                "SELECT i.id, coalesce(i.title, '') || ' ' || coalesce(i.body_text, '') || ' ' || coalesce(i.url, '') \
                 FROM items_fts f CROSS JOIN items i ON i.id = f.rowid WHERE items_fts MATCH ?1 AND i.kind = 'capture'",
            )?;
            let rows = stmt.query_map([q], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (id, text) = row?;
                if names(&text) {
                    out.insert(id);
                }
            }
        }
        let mut stmt = self.conn.prepare("SELECT id, url FROM items WHERE kind = 'capture' AND url LIKE ?1")?;
        for term in terms {
            let Some(word) = term.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty()) else { continue };
            let rows = stmt.query_map([format!("%{word}%")], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (id, url) = row?;
                if names(&url) {
                    out.insert(id);
                }
            }
        }
        Ok(out)
    }

    /// The client's calendar events and the meetings recorded from them, failing closed (fix
    /// round 2, N3; over-excluding is accepted, under-excluding is not):
    /// - every instance of an excluded event's recurring series is excluded too;
    /// - every excluded instance's time range is kept as a tombstone, so an event deleted or
    ///   repointed later still excludes;
    /// - a meeting linked to any of those events by its calendar id (`calendar_event_id`, or the
    ///   Mac's `cal:<event id>` continue key) is excluded, whatever its title or start;
    /// - a meeting whose time span overlaps any excluded event's range (from
    ///   [`EVENT_LEAD_MS`] before its start to [`EVENT_SPAN_MS`] after) is excluded, whatever its
    ///   title (retitled, renamed, started late).
    fn lift_events_and_meetings(&self, m: &mut ExcludedMaterial) -> rusqlite::Result<()> {
        let mut calendar: Vec<(i64, String, i64, Option<String>)> = Vec::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT id, coalesce(title, ''), ts_unix_ms, external_id FROM items WHERE kind = 'calendar_event'",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            for row in rows {
                calendar.push(row?);
            }
        }
        let mut series: HashSet<String> = calendar
            .iter()
            .filter(|(id, ..)| m.items.contains(id))
            .filter_map(|(_, _, _, ext)| ext.as_deref().map(event_series_id))
            .collect();
        // The excluded instances, each with the client(s) it belongs to: its own filing, else a
        // series-mate's, else (named or a contact's) every client excluded now.
        let mut series_owners: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for (id, _, _, ext) in &calendar {
            if let (Some(owners), Some(x)) = (m.event_owners.get(id), ext) {
                series_owners.entry(event_series_id(x)).or_default().extend(owners.iter().cloned());
            }
        }
        let mut current: Vec<(String, i64, String)> = Vec::new();
        for (id, title, ts, ext) in &calendar {
            let in_series = ext.as_deref().is_some_and(|x| series.contains(&event_series_id(x)));
            if !(m.items.contains(id) || in_series) {
                continue;
            }
            m.items.insert(*id);
            m.events.insert((title.trim().to_lowercase(), *ts));
            if let Some(x) = ext {
                m.event_ids.insert(x.clone());
            }
            let series_id = ext.as_deref().map(event_series_id).unwrap_or_else(|| format!("item:{id}"));
            let mut owners: Vec<String> = m
                .event_owners
                .get(id)
                .cloned()
                .or_else(|| series_owners.get(&series_id).cloned())
                .unwrap_or_else(|| m.key_hashes.clone());
            owners.sort();
            owners.dedup();
            current.push((series_id, *ts, owners.join(",")));
        }
        // Tombstones: the stored ranges of events since deleted or repointed, while their own
        // client is excluded. Read only; the write below never blocks or fails this read (R7).
        let mut stored: std::collections::HashMap<(String, i64), String> = std::collections::HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT series, start_unix_ms, keys FROM excluded_event_span")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (s, start, k) = row?;
                stored.insert((s, start), k);
            }
        }
        let changed: Vec<&(String, i64, String)> =
            current.iter().filter(|(s, t, k)| stored.get(&(s.clone(), *t)) != Some(k)).collect();
        if !changed.is_empty() {
            self.record_spans_best_effort(&changed);
        }
        let live: HashSet<(String, i64)> = current.iter().map(|(s, t, _)| (s.clone(), *t)).collect();
        let mut spans: Vec<i64> = current.iter().map(|(_, t, _)| *t).collect();
        for ((s, start), k) in &stored {
            if live.contains(&(s.clone(), *start)) {
                continue;
            }
            if k.split(',').any(|h| m.key_hashes.iter().any(|x| x == h)) {
                spans.push(*start);
                series.insert(s.clone());
            }
        }
        m.event_series = series.clone();
        let mut stmt = self.conn.prepare(
            "SELECT id, start_unix_ms, coalesce(end_unix_ms, start_unix_ms + ?1), calendar_event_id, continue_key FROM meeting",
        )?;
        let rows = stmt.query_map([MEETING_DEFAULT_MS], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?))
        })?;
        for row in rows {
            let (id, start, end, event, cont) = row?;
            let linked = event
                .iter()
                .chain(cont.as_deref().and_then(|c| c.strip_prefix("cal:")).map(str::to_string).iter())
                .any(|x| series.contains(&event_series_id(x)));
            let overlaps = spans.iter().any(|ev| start < ev + EVENT_SPAN_MS && end > ev - EVENT_LEAD_MS);
            if linked || overlaps {
                m.meetings.insert(id);
            }
        }
        Ok(())
    }

    /// Records excluded event ranges with their owning clients, best effort: one transaction that
    /// never waits for the write lock (busy timeout 0 while it runs), and whose failure changes
    /// nothing for the caller, which already holds the ranges in memory (fix round 3, R7).
    fn record_spans_best_effort(&self, spans: &[&(String, i64, String)]) {
        // Put back whatever the shell set, not a fixed 5 s.
        let before = self.busy_timeout_ms().unwrap_or(5000).max(0) as u64;
        let _ = self.conn.busy_timeout(std::time::Duration::ZERO);
        let result = (|| -> rusqlite::Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            for (series, start, keys) in spans {
                tx.execute(
                    "INSERT INTO excluded_event_span (series, start_unix_ms, keys) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(series, start_unix_ms) DO UPDATE SET keys = excluded.keys",
                    params![series, start, keys],
                )?;
            }
            tx.commit()
        })();
        let _ = self.conn.busy_timeout(std::time::Duration::from_millis(before));
        if let Err(e) = result {
            eprintln!("[exclusion] event ranges not recorded this time (they stay in memory): {e}");
        }
    }

    /// Slides of excluded meetings, emails of excluded threads (and threads of excluded emails),
    /// episodes over excluded captures.
    fn lift_derived(&self, m: &mut ExcludedMaterial) -> rusqlite::Result<()> {
        let emails: Vec<i64> = {
            let mut stmt = self.conn.prepare("SELECT id FROM items WHERE kind = 'email'")?;
            let all: Vec<i64> = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            all.into_iter().filter(|id| m.items.contains(id)).collect()
        };
        for id in emails {
            m.threads.extend(self.thread_of_email(id)?);
        }
        {
            let mut stmt = self.conn.prepare(
                "SELECT i.id, t.id FROM items i JOIN threads t ON t.source = i.source \
                   AND t.external_thread_id = json_extract(i.refs, '$.gmail_thread_id') \
                   AND t.account_email = json_extract(i.refs, '$.account_email') \
                 WHERE i.kind = 'email'",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (item, thread) = row?;
                if m.threads.contains(&thread) {
                    m.items.insert(item);
                }
            }
        }
        {
            let mut stmt = self.conn.prepare("SELECT id, thread_id FROM items WHERE kind = 'snapshot' AND thread_id IS NOT NULL")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (item, meeting) = row?;
                if m.meetings.contains(&meeting) {
                    m.items.insert(item);
                }
            }
        }
        // Episodes over excluded captures, looked up from each capture (never a join of every
        // capture with every episode, R3), then every capture of each such episode (R2).
        let mut by_device: std::collections::HashMap<String, Vec<(i64, i64, i64)>> = std::collections::HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, device_id, start_item_id, end_item_id FROM episodes")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?;
            for row in rows {
                let (id, device, start, end) = row?;
                by_device.entry(device).or_default().push((start, end, id));
            }
        }
        // Per device: sorted by start, with the running maximum end, so the search for the
        // episodes covering an item stops as soon as no earlier one can reach it.
        let index: std::collections::HashMap<String, (Vec<(i64, i64, i64)>, Vec<i64>)> = by_device
            .into_iter()
            .map(|(d, mut v)| {
                v.sort_unstable();
                let mut max = i64::MIN;
                let reach = v.iter().map(|(_, e, _)| { max = max.max(*e); max }).collect();
                (d, (v, reach))
            })
            .collect();
        let candidates: Vec<i64> = m.items.iter().copied().collect();
        let mut lifted: HashSet<i64> = HashSet::new();
        let mut stmt = self.conn.prepare("SELECT device_id FROM items WHERE id = ?1 AND kind = 'capture'")?;
        for item in candidates {
            let Some(device) = stmt.query_row([item], |r| r.get::<_, String>(0)).optional()? else { continue };
            let Some((eps, reach)) = index.get(&device) else { continue };
            let mut k = eps.partition_point(|(start, _, _)| *start <= item);
            while k > 0 {
                k -= 1;
                if reach[k] < item {
                    break;
                }
                let (_, end, id) = eps[k];
                if end >= item && !m.episodes.contains(&id) {
                    lifted.insert(id);
                }
            }
        }
        for id in lifted {
            m.episodes.insert(id);
            for (item, _, _) in self.episode_captures(id)? {
                m.items.insert(item);
            }
        }
        Ok(())
    }

    /// The fingerprints of the seeds excluded now, for a run's provenance.
    pub fn excluded_key_hashes(&self) -> rusqlite::Result<Vec<String>> {
        let m = self.excluded_material()?;
        let mut v: Vec<String> = m.topic_keys.iter().map(|k| key_hash(k)).collect();
        v.sort();
        Ok(v)
    }

    /// Records how routine run `id` was built: its inputs ([`input_refs_stamp`]) or
    /// [`UNFILTERED_PROVENANCE`].
    /// Device-local: a run pulled from another device has none, so it stays out of hosted prompts
    /// while any client is excluded.
    pub fn set_routine_run_provenance(&self, id: i64, provenance: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE routine_run SET provenance = ?2 WHERE id = ?1", params![id, provenance])?;
        Ok(())
    }

    /// Marks `text` (an answer made on this computer from an excluded client's material) so it
    /// never goes to a hosted model: as conversation history, a title's input, or anything else
    /// that checks [`Self::is_local_only_text`]. Kept by fingerprint only. A safety mark, not a
    /// trace (ruling, X6 fix round 7, F2): a client delete never drops it, since that could put a
    /// surviving answer or prompt back on a hosted model (Ollama cloud prompt gating).
    pub fn mark_local_only_text(&self, text: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO local_only_text (hash, at_unix_ms) VALUES (?1, ?2)",
            params![text_hash(text), crate::now_unix_ms()],
        )?;
        Ok(())
    }

    /// Records the inputs `text` (an answer) was built from ([`input_refs_stamp`]). The answer is
    /// clean history for a hosted turn while none of them is in the excluded material read then
    /// (fix round 4, S2); an answer with no record is not, while any client is excluded. `words`
    /// are the word fingerprints of its `text:` inputs ([`input_text_ref`]), written with the
    /// stamp and owned by it: a stamp that replaces this one replaces them too (fix round 5, N1).
    pub fn mark_clean_text(&self, text: &str, stamp: &str, words: &[(i64, Vec<u32>)]) -> rusqlite::Result<()> {
        let owner = text_hash(text);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM input_words WHERE stamp = ?1", [owner])?;
        tx.execute("INSERT OR REPLACE INTO clean_text (hash, keys) VALUES (?1, ?2)", params![owner, stamp])?;
        let at = crate::now_unix_ms();
        for (hash, fingerprints) in words {
            let blob = encode_fingerprints(fingerprints);
            tx.execute(
                "INSERT OR REPLACE INTO input_words (stamp, hash, fingerprints, at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
                params![owner, hash, blob, at],
            )?;
        }
        tx.commit()
    }

    /// The word fingerprints kept under `hash` by any stamp, sorted. A current-version row is read
    /// before a legacy one (review 5, M1): the same text stamped again since the upgrade is read by
    /// its new fingerprints, so a legacy row does not keep it unclean until retention.
    pub fn input_words(&self, hash: i64) -> rusqlite::Result<Option<Vec<u32>>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT fingerprints FROM input_words WHERE hash = ?1 \
                 ORDER BY substr(fingerprints, 1, 4) = ?2 DESC, at_unix_ms DESC LIMIT 1",
                params![hash, FINGERPRINTS_V2.to_le_bytes().to_vec()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(blob.map(|b| decode_fingerprints(&b)))
    }

    /// Drops word fingerprints kept before `cutoff_unix_ms` (retention; fix round 5, N1). Stamps that
    /// used them become unclean.
    pub fn prune_input_words_before(&self, cutoff_unix_ms: i64) -> rusqlite::Result<usize> {
        // In batches, each its own commit, so a large backlog never holds the write lock for long,
        // with the capture sweep's pause between them: a lock released and retaken at once starves
        // another connection's busy handler (a recording start above all; #850 review, I2).
        let batch = crate::RETENTION_DELETE_BATCH;
        let mut total = 0;
        loop {
            let n = self.conn.execute(
                &format!(
                    "DELETE FROM input_words WHERE rowid IN \
                     (SELECT rowid FROM input_words WHERE at_unix_ms < ?1 LIMIT {batch})"
                ),
                [cutoff_unix_ms],
            )?;
            total += n;
            if n < batch {
                return Ok(total);
            }
            std::thread::sleep(crate::RETENTION_BATCH_PAUSE);
        }
    }

    /// Drops every kept word fingerprint set that names any of `terms` (a client deleted or
    /// forgotten: no trace of its name stays; fix round 5, N1), and every clean stamp that recorded
    /// one of those texts (fix round 6, N1a). Their answers become unclean, which is fail-closed.
    pub fn purge_input_words_naming(&self, terms: &[String]) -> rusqlite::Result<usize> {
        purge_input_words_naming_tx(&self.conn, terms)
    }

    /// The recorded inputs among `refs` whose row no longer exists (deleted, pruned, merged away):
    /// they cannot be checked, so a stamp recording one is not clean while a client is excluded
    /// (fix round 4). `event` and `text` inputs have no row here and are never reported.
    pub fn missing_input_refs<'a>(&self, refs: impl IntoIterator<Item = &'a String>) -> rusqlite::Result<HashSet<String>> {
        missing_input_refs_conn(&self.conn, refs)
    }


    /// The stamp [`Self::mark_clean_text`] recorded for `text`, if any.
    pub fn clean_text_stamp(&self, text: &str) -> rusqlite::Result<Option<String>> {
        self.conn.query_row("SELECT keys FROM clean_text WHERE hash = ?1", [text_hash(text)], |r| r.get(0)).optional()
    }

    /// Whether `text` was marked by [`Self::mark_local_only_text`].
    pub fn is_local_only_text(&self, text: &str) -> rusqlite::Result<bool> {
        self.conn
            .query_row("SELECT 1 FROM local_only_text WHERE hash = ?1", [text_hash(text)], |_| Ok(()))
            .optional()
            .map(|r| r.is_some())
    }

    /// Records the calendar event meeting `meeting_id` was recorded from: the durable link the
    /// exclusion predicate follows whatever the meeting's title or start later become.
    pub fn set_meeting_calendar_event(&self, meeting_id: i64, event_id: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE meeting SET calendar_event_id = ?2 WHERE id = ?1", params![meeting_id, event_id])?;
        Ok(())
    }

    /// The `(kind, ref_id)` of topic source `source_id`.
    pub fn source_ref(&self, source_id: i64) -> rusqlite::Result<Option<(String, i64)>> {
        self.conn
            .query_row(
                "SELECT kind, ref_id FROM topic_source WHERE id = ?1 AND ref_id IS NOT NULL",
                [source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
    }

    /// Takes `source_id` off the judge's queues without a verdict.
    pub fn drop_judge_queue_rows(&self, source_id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM topic_judge_todo WHERE source_id = ?1", [source_id])?;
        self.conn.execute("DELETE FROM topic_rejudge WHERE source_id = ?1", [source_id])?;
        Ok(())
    }

    /// Meetings of client `key` (and of seeds merged into it) whose summary a hosted provider wrote:
    /// what turning exclusion on can offer to summarise again on this computer. A meeting whose
    /// summary records no provider (written before providers were recorded) is counted too.
    pub fn hosted_summarised_meetings(&self, key: &str) -> rusqlite::Result<Vec<i64>> {
        let mut out = Vec::new();
        for m in self.client_members(key)? {
            let meeting = match (m.kind.as_str(), m.ref_id) {
                ("meeting", Some(r)) => r,
                ("snapshot", Some(r)) => match self.snapshot_meeting(r)? {
                    Some((meeting, _)) => meeting,
                    None => continue,
                },
                _ => continue,
            };
            let made_by: Option<(Option<String>, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT summary_provider, summary_model FROM meeting WHERE id = ?1 AND summary_tldr IS NOT NULL AND summary_tldr != ''",
                    [meeting],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            // An Ollama cloud model (a `cloud` tag) is hosted, like `llm::is_hosted` says.
            let cloud = |model: &Option<String>| {
                model.as_deref().unwrap_or_default().to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "cloud")
            };
            let hosted = match made_by {
                None => false,
                Some((None, _)) => true,
                Some((Some(p), model)) => !matches!(p.as_str(), "ollama" | "local") || cloud(&model),
            };
            if hosted && !out.contains(&meeting) {
                out.push(meeting);
            }
        }
        out.sort_unstable();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::FakeEmbedder;
    use crate::topics::client_controls::ClientControl;
    use std::sync::Arc;

    /// Review 4, M4: fingerprints kept by v0.9.80 and earlier (of the text as written, no version
    /// word) read as unknown: unclean while any client is excluded, clean while none is. New rows
    /// round-trip.
    #[test]
    fn fingerprints_kept_before_readings_read_as_unclean_while_a_client_is_excluded() {
        let (_d, s) = store();
        s.mark_clean_text("An answer.", "stamp", &[(42, vec![1, 2, 3])]).unwrap();
        assert_eq!(s.input_words(42).unwrap(), Some(vec![1, 2, 3]));
        let old: Vec<u8> = crate::words::word_fingerprints("lunch on friday").iter().flat_map(|f| f.to_le_bytes()).collect();
        s.conn.execute("INSERT OR REPLACE INTO input_words (stamp, hash, fingerprints, at_unix_ms) VALUES (7, 43, ?1, 1)", [old]).unwrap();
        let fps = s.input_words(43).unwrap().unwrap();
        assert_eq!(fps, vec![crate::words::UNDECIDABLE_FINGERPRINT]);
        assert!(crate::words::fingerprints_name_any(&fps, &[crate::words::folded_words("Acme")]), "unclean while a client is excluded");
        assert!(!crate::words::fingerprints_name_any(&fps, &[]), "clean while none is");
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("t.db"), Arc::new(FakeEmbedder));
        (dir, s)
    }

    #[test]
    fn a_purged_client_leaves_no_event_range_and_others_keep_theirs() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let b = s.create_seed_topic("Globex", &[], None).unwrap();
        let (ha, hb) = (key_hash(&a.key), key_hash(&b.key));
        for (series, start, keys) in [("only-a", 1, ha.clone()), ("shared", 2, format!("{ha},{hb}")), ("only-b", 3, hb.clone())] {
            s.conn
                .execute("INSERT INTO excluded_event_span (series, start_unix_ms, keys) VALUES (?1, ?2, ?3)", params![series, start, keys])
                .unwrap();
        }
        s.forget_seed_topic(&a.key).unwrap();
        let rows: Vec<(String, String)> = s
            .conn
            .prepare("SELECT series, keys FROM excluded_event_span ORDER BY series")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows, vec![("only-b".to_string(), hb.clone()), ("shared".to_string(), hb)]);
    }

    fn file(s: &Store, topic: i64, kind: &str, r: i64) -> i64 {
        s.conn
            .execute(
                "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, 1, 1)",
                params![kind, r],
            )
            .unwrap();
        let sid = s.conn.last_insert_rowid();
        s.conn
            .execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![topic, sid])
            .unwrap();
        sid
    }

    fn exclude(s: &Store, key: &str) {
        s.set_client_control(key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
    }

    #[test]
    fn nothing_excluded_is_empty() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        file(&s, a.topic_id, "meeting", 7);
        assert!(s.excluded_material().unwrap().is_empty());
    }

    /// Spec 13 Q4: a source shared by two clients is excluded when either is.
    #[test]
    fn shared_source_excluded_if_either_client_excluded() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &["Roadrunner".to_string()], None).unwrap();
        let g = s.create_seed_topic("Globex", &[], None).unwrap();
        let shared = file(&s, a.topic_id, "meeting", 7);
        s.conn
            .execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![g.topic_id, shared])
            .unwrap();
        file(&s, g.topic_id, "meeting", 8);
        exclude(&s, &a.key);
        let m = s.excluded_material().unwrap();
        assert!(m.meetings.contains(&7));
        assert!(!m.meetings.contains(&8));
        assert!(m.source_ids.contains(&shared));
        assert!(m.topic_keys.contains(&a.key) && !m.topic_keys.contains(&g.key));
        assert_eq!(m.terms, vec!["Acme".to_string(), "Roadrunner".to_string()]);
    }

    #[test]
    fn unreadable_setting_is_an_error() {
        let (_d, s) = store();
        s.conn.execute("DROP TABLE client_control", []).unwrap();
        assert!(s.excluded_material().is_err());
    }

    /// Any change to an excluded client's membership or terms drops the digest cache, and moves
    /// the version; other clients' changes do neither.
    #[test]
    fn an_excluded_clients_membership_or_terms_change_drops_the_digest_cache() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let g = s.create_seed_topic("Globex", &[], None).unwrap();
        exclude(&s, &a.key);
        let cached = |s: &Store| s.get_cached_daily_digest("d").unwrap().is_some();
        let v0 = s.excluded_material().unwrap().version();

        s.cache_daily_digest("d", "{}", 1).unwrap();
        file(&s, g.topic_id, "meeting", 1);
        assert!(cached(&s), "another client's filing keeps it");
        assert_eq!(s.excluded_material().unwrap().version(), v0);

        let sid = file(&s, a.topic_id, "meeting", 2);
        assert!(!cached(&s), "a new source of the excluded client drops it");
        let v1 = s.excluded_material().unwrap().version();
        assert_ne!(v1, v0);

        s.cache_daily_digest("d", "{}", 1).unwrap();
        s.conn.execute("DELETE FROM topic_member WHERE source_id = ?1", [sid]).unwrap();
        assert!(!cached(&s), "a source leaving it drops it");

        s.cache_daily_digest("d", "{}", 1).unwrap();
        s.update_seed_topic(&a.key, "Acme", &["Roadrunner".to_string()], None).unwrap();
        assert!(!cached(&s), "new terms drop it");
        assert_ne!(s.excluded_material().unwrap().version(), v1);

        s.cache_daily_digest("d", "{}", 1).unwrap();
        s.set_client_control(&a.key, &ClientControl { exclude_hosted: false, keep_months: None }).unwrap();
        assert!(!cached(&s), "turning exclusion off drops it too");
        assert!(!format!("{:?}", s.excluded_material().unwrap().version()).to_lowercase().contains("acme"));
    }

    #[test]
    fn turning_exclusion_on_drops_the_digest_cache() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        s.cache_daily_digest("2026-11-11", "{}", 1).unwrap();
        // A setting that leaves exclusion off drops nothing.
        s.set_client_control(&a.key, &ClientControl { exclude_hosted: false, keep_months: Some(3) }).unwrap();
        assert!(s.get_cached_daily_digest("2026-11-11").unwrap().is_some());
        exclude(&s, &a.key);
        assert!(s.get_cached_daily_digest("2026-11-11").unwrap().is_none());
        // Already on: saving again drops nothing new.
        s.cache_daily_digest("2026-11-12", "{}", 1).unwrap();
        exclude(&s, &a.key);
        assert!(s.get_cached_daily_digest("2026-11-12").unwrap().is_some());
    }

    /// B3 and fix round 4 (S2): runs are kept, never deleted. A run is clean while none of the
    /// inputs it recorded is excluded: new material of a client that was not among its inputs
    /// leaves it clean; an input later filed under an excluded client makes it unclean, and so
    /// does every run built from that run. Runs with no record, an older stamp, or made
    /// unfiltered are not clean.
    #[test]
    fn routine_runs_are_clean_while_none_of_their_inputs_is_excluded() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let g = s.create_seed_topic("Globex", &[], None).unwrap();
        s.seed_builtin_routines().unwrap();
        let routine = s.list_routines().unwrap()[0].id;
        let run = |stamp: Option<&str>| {
            let id = s
                .insert_routine_run(&crate::NewRoutineRun {
                    routine_id: routine,
                    ran_at_unix_ms: 1,
                    status: crate::RunStatus::Success,
                    output_summary: "x".into(),
                    output_json: "{}".into(),
                })
                .unwrap();
            if let Some(stamp) = stamp {
                s.set_routine_run_provenance(id, stamp).unwrap();
            }
            id
        };
        let meeting = |title: &str| {
            s.insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: title.into(),
                attendees: vec![],
                trigger: crate::MeetingTrigger::Calendar,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap()
        };
        let file = |topic: i64, kind: &str, id: i64| {
            s.conn
                .execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, 1, 1)", params![kind, id])
                .unwrap();
            s.conn
                .execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![topic, s.conn.last_insert_rowid()])
                .unwrap();
        };
        let call = meeting("Call");
        let stamp = input_refs_stamp([input_ref("meeting", call)]).unwrap();
        let older = run(Some("clean:open"));
        let built = run(Some(&stamp));
        let derived = run(Some(&input_refs_stamp([input_ref("routine_run", built)]).unwrap()));
        let unfiltered = run(Some(UNFILTERED_PROVENANCE));
        let none = run(None);
        let gone = run(Some(&input_refs_stamp([input_ref("meeting", 999_999)]).unwrap()));
        // Fix round 5, N4: a run whose own words name a client excluded later.
        let worded = s
            .insert_routine_run(&crate::NewRoutineRun {
                routine_id: routine,
                ran_at_unix_ms: 1,
                status: crate::RunStatus::Success,
                output_summary: "Today: Globex standup.".into(),
                output_json: "{}".into(),
            })
            .unwrap();
        s.set_routine_run_provenance(worded, &input_refs_stamp([input_ref("event", "evt-9")]).unwrap()).unwrap();
        exclude(&s, &a.key);
        let clean = |id: i64| s.excluded_material_with(&|_| false).unwrap().clean_runs.contains(&id);
        assert!(clean(built) && clean(derived), "none of their inputs is Acme's");
        assert!(!clean(older) && !clean(unfiltered) && !clean(none));
        assert!(!clean(gone), "an input that no longer exists cannot be checked");
        // New Acme material that was not among the inputs leaves the run clean.
        file(a.topic_id, "meeting", meeting("Acme pricing"));
        assert!(clean(built) && clean(derived));
        // An input filed under an excluded client later makes the run, and runs built on it, unclean.
        file(g.topic_id, "meeting", call);
        assert!(clean(built), "Globex is not excluded");
        assert!(clean(worded), "control: its inputs are clean and nothing excluded is named");
        exclude(&s, &g.key);
        // The matcher the material is computed with names Globex, as the real one does once it is
        // excluded (the material is cached per generation, so this computes it first).
        let m = s.excluded_material_with(&|t| t.contains("Globex")).unwrap();
        assert!(!m.clean_runs.contains(&worded), "its own words name Globex, now excluded");
        assert!(!clean(built) && !clean(derived));
        let alive: i64 = s.conn.query_row("SELECT COUNT(*) FROM routine_run", [], |r| r.get(0)).unwrap();
        assert_eq!(alive, 7, "nothing deleted");
    }

    /// Fix round 6, J3: a run's `output_json` is matched decoded: a name behind an escaped newline
    /// in an open loop makes it unclean once that client is excluded, and an `output_json` that
    /// does not parse is never clean. Control: a run naming nobody stays clean.
    #[test]
    fn a_runs_json_output_is_matched_decoded() {
        let (_d, s) = store();
        let g = s.create_seed_topic("Globex", &[], None).unwrap();
        s.seed_builtin_routines().unwrap();
        let routine = s.list_routines().unwrap()[0].id;
        let run = |json: &str| {
            let id = s
                .insert_routine_run(&crate::NewRoutineRun { routine_id: routine, ran_at_unix_ms: 1, status: crate::RunStatus::Success, output_summary: "A day.".into(), output_json: json.into() })
                .unwrap();
            s.set_routine_run_provenance(id, &input_refs_stamp([input_ref("event", "evt-1")]).unwrap()).unwrap();
            id
        };
        let named = run(r#"{"open_loops":["Ping\nGlobex"]}"#);
        let broken = run("not json");
        let plain = run(r#"{"open_loops":["Ping Carol"]}"#);
        exclude(&s, &g.key);
        let names = |t: &str| crate::words::folded_words(t).iter().any(|w| w == "globex");
        assert!(!names(r#"{"open_loops":["Ping\nGlobex"]}"#), "raw folding would miss it");
        let m = s.excluded_material_with(&names).unwrap();
        assert!(!m.clean_runs.contains(&named), "named behind an escape");
        assert!(!m.clean_runs.contains(&broken), "output_json that does not parse");
        assert!(m.clean_runs.contains(&plain), "control");
    }

    /// Fix round 6, N1e: kept word fingerprints go with their stamp: a stamp written again for the
    /// same answer replaces them; deleting the conversation drops the stamp and them; capture
    /// retention prunes them; forgetting a client (purge_seed_tx) drops every set naming it and the
    /// stamps that recorded those texts. Controls stay.
    #[test]
    fn kept_fingerprints_go_with_their_stamp() {
        let (_d, s) = store();
        let rows = |s: &Store| -> Vec<(i64, i64)> {
            s.conn.prepare("SELECT stamp, hash FROM input_words ORDER BY hash").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
        };
        let stamped = |answer: &str, question: &str| {
            let (r, h, f) = input_text_ref(question);
            s.mark_clean_text(answer, &input_refs_stamp([r]).unwrap(), &[(h, f)]).unwrap();
            h
        };
        // Replacement.
        let first = stamped("Friday.", "when is it?");
        let second = stamped("Friday.", "what day is it?");
        assert_eq!(rows(&s).iter().map(|r| r.1).collect::<Vec<_>>(), vec![second], "the replaced stamp's words went");
        assert_ne!(first, second);
        // Conversation delete.
        let conv = s.insert_conversation("q", 1).unwrap();
        s.append_conversation_exchange(conv, "what day is it?", "Friday.", "[]", None, 1).unwrap();
        s.delete_conversation(conv).unwrap();
        assert!(rows(&s).is_empty() && s.clean_text_stamp("Friday.").unwrap().is_none(), "the conversation's stamp and words went");
        // Retention.
        stamped("Monday.", "when does it start?");
        s.conn.execute("UPDATE input_words SET at_unix_ms = 1", []).unwrap();
        stamped("Tuesday.", "when does it end?");
        s.apply_retention(&crate::RetentionPolicy { capture_item_days: Some(30), meeting_days: None, conversation_days: None }, crate::now_unix_ms()).unwrap();
        assert_eq!(rows(&s).len(), 1, "the old set aged out; control: the new one stays");
        // Forget a client.
        let initech = s.create_seed_topic("Initech", &[], None).unwrap();
        stamped("Signed.", "did Initech sign?");
        stamped("Stalled.", "did Hooli sign?");
        let (inherited_ref, ..) = input_text_ref("did Initech sign?");
        s.mark_clean_text("Later.", &input_refs_stamp([inherited_ref]).unwrap(), &[]).unwrap();
        s.forget_seed_topic(&initech.key).unwrap();
        let initech_terms = [crate::words::folded_words("Initech")];
        let left: Vec<Vec<u32>> = s
            .conn
            .prepare("SELECT fingerprints FROM input_words")
            .unwrap()
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(|b| decode_fingerprints(&b.unwrap()))
            .collect();
        assert!(!left.iter().any(|f| crate::words::fingerprints_name_any(f, &initech_terms)), "a set naming the forgotten client stayed");
        assert!(s.clean_text_stamp("Signed.").unwrap().is_none() && s.clean_text_stamp("Later.").unwrap().is_none(), "the stamps that recorded it went");
        assert!(s.clean_text_stamp("Stalled.").unwrap().is_some(), "control: another client's stamp stays");
    }

    /// Fix round 4, S2: a stamp records at most MAX_INPUT_REFS inputs; more gets no stamp at all,
    /// never a shortened list. An event input is its calendar id.
    #[test]
    fn input_ref_stamps_are_capped_never_cut_short() {
        assert!(input_refs_stamp((0..MAX_INPUT_REFS as i64).map(|i| input_ref("capture", i))).is_some());
        assert!(input_refs_stamp((0..=MAX_INPUT_REFS as i64).map(|i| input_ref("capture", i))).is_none());
        let stamp = input_refs_stamp([input_ref("capture", 7), input_ref("capture", 7), input_ref("event", "abc@google.com")]).unwrap();
        assert_eq!(stamp_input_refs(&stamp).unwrap(), vec!["capture:7".to_string(), "event:abc@google.com".to_string()]);
        let mut m = ExcludedMaterial::default();
        assert!(m.stamp_is_clean(&stamp));
        assert!(!m.stamp_is_clean("clean:open") && !m.stamp_is_clean(UNFILTERED_PROVENANCE));
        assert!(m.excludes_input_ref("event:") && m.excludes_input_ref("capture:x") && m.excludes_input_ref("junk"));
        m.event_series.insert(event_series_id("abc@google.com"));
        assert!(!m.stamp_is_clean(&stamp));
    }

    /// B4: an address met only in the client's material excludes a new meeting by its stored
    /// attendee details (the production shape: display names in `attendees`, emails in
    /// `attendee_details`), and a meeting recorded from the client's calendar event is excluded.
    #[test]
    fn contact_addresses_and_calendar_events_reach_their_meetings() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let filed = s
            .insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Kickoff".into(),
                attendees: vec!["Wile E. Coyote".into()],
                trigger: crate::MeetingTrigger::Calendar,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap();
        let sid = file(&s, a.topic_id, "meeting", filed);
        s.conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
        let wile = s.conn.last_insert_rowid();
        s.conn.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![wile, sid]).unwrap();
        let new = s
            .insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 9_000_000,
                end_unix_ms: Some(9_100_000),
                title: "Weekly sync".into(),
                attendees: vec!["Wile E. Coyote".into()],
                trigger: crate::MeetingTrigger::Calendar,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap();
        s.set_meeting_attendee_details(
            new,
            &[crate::speaker_names::Attendee { name: "Wile E. Coyote".into(), email: Some("wile@coyote.example".into()) }],
        )
        .unwrap();
        // A calendar event filed under the client, and the meeting recorded from it.
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                 VALUES ('d', 'calendar_event', 'gcal', 50000000, '[]', 'Budget review', 'Budget review', 'evt-9')",
                [],
            )
            .unwrap();
        let event = s.conn.last_insert_rowid();
        file(&s, a.topic_id, "calendar_event", event);
        let recorded = s
            .insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 50_000_000 + 60_000,
                end_unix_ms: Some(50_000_000 + 3_600_000),
                title: "Budget review".into(),
                attendees: vec![],
                trigger: crate::MeetingTrigger::Calendar,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap();
        exclude(&s, &a.key);
        let m = s.excluded_material_with(&|_| false).unwrap();
        assert!(m.meetings.contains(&new), "by the attendee address in attendee_details");
        assert!(m.meetings.contains(&recorded), "recorded from the client's event");
        assert!(m.event_ids.contains("evt-9"));
        assert!(m.events.contains(&("budget review".to_string(), 50_000_000)));
    }

    /// B6: every candidate is read, never a capped page: two meetings naming the client in their
    /// transcripts are both lifted, where a page of one would miss one.
    #[test]
    fn name_lifts_read_every_candidate() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let mut ids = Vec::new();
        for i in 0..2 {
            let id = s
                .insert_meeting(&crate::NewMeeting {
                    device_id: "d".into(),
                    start_unix_ms: 1_000 + i,
                    end_unix_ms: Some(2_000 + i),
                    title: "Sync".into(),
                    attendees: vec![],
                    trigger: crate::MeetingTrigger::Manual,
                    audio_path: None,
                    status: crate::MeetingStatus::Completed,
                })
                .unwrap();
            s.insert_transcript_segments(&[crate::NewTranscriptSegment { meeting_id: id, start_cs: 0, speaker_label: "S".into(), text: "Acme called.".into() }])
                .unwrap();
            ids.push(id);
        }
        exclude(&s, &a.key);
        let names = |t: &str| t.contains("Acme");
        assert!(s.name_hits_limited(&["Acme".to_string()], &names, 1).unwrap().truncated, "a page of one is truncated");
        let m = s.excluded_material_with(&names).unwrap();
        assert!(ids.iter().all(|id| m.meetings.contains(id)), "both lifted");
    }

    /// Fix round 2, N3: the event-to-meeting link fails closed. A meeting whose time overlaps an
    /// excluded client's calendar event is excluded whatever its title (retitled, renamed, late
    /// start), so is a past instance of the client's recurring series, and a meeting of an
    /// event deleted later; the durable calendar id on the meeting links it too. Control: a
    /// meeting at another time is not excluded.
    #[test]
    fn meetings_overlapping_a_client_event_are_excluded_whatever_their_title() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        const H: i64 = 3_600_000;
        let t = 1_000 * H;
        let event = |external: &str, ts: i64, title: &str| -> i64 {
            s.conn
                .execute(
                    "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                     VALUES ('d', 'calendar_event', 'gcal', ?1, '[]', ?2, ?2, ?3)",
                    params![ts, title, external],
                )
                .unwrap();
            s.conn.last_insert_rowid()
        };
        let meeting = |title: &str, start: i64| -> i64 {
            s.insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + H),
                title: title.into(),
                attendees: vec![],
                trigger: crate::MeetingTrigger::Calendar,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap()
        };
        // This week's instance of the weekly series is filed (the series repointed to it).
        let this_week = event("wk_20261105T090000Z", t, "Weekly sync");
        event("wk_20261029T090000Z", t - 7 * 24 * H, "Weekly sync");
        file(&s, a.topic_id, "calendar_event", this_week);
        let board = event("board-1", t + 48 * H, "Board");
        file(&s, a.topic_id, "calendar_event", board);
        let retitled = meeting("Room audio only \u{b7} Weekly sync", t + 5 * 60_000);
        let renamed = meeting("Budget chat", t + 10 * 60_000);
        let late = meeting("Weekly sync", t + 50 * 60_000);
        let past = meeting("Weekly sync", t - 7 * 24 * H + 2 * 60_000);
        let deleted = meeting("Board", t + 48 * H + 3 * 60_000);
        let linked = meeting("Catch-up", t + 200 * H);
        s.set_meeting_calendar_event(linked, "wk_20261112T090000Z").unwrap();
        let other = meeting("Lunch", t + 5 * H);
        exclude(&s, &a.key);
        let m = s.excluded_material_with(&|_| false).unwrap();
        for (what, id) in [("retitled", retitled), ("renamed", renamed), ("late", late), ("past instance", past), ("linked by id", linked)] {
            assert!(m.meetings.contains(&id), "{what}");
        }
        assert!(m.meetings.contains(&deleted));
        assert!(!m.meetings.contains(&other), "control: another time");
        // The event is deleted later: its span stays.
        s.conn.execute("DELETE FROM items WHERE id = ?1", [board]).unwrap();
        let m = s.excluded_material_with(&|_| false).unwrap();
        assert!(m.meetings.contains(&deleted), "deleted event still excludes its meeting");
        assert!(!m.meetings.contains(&other));
    }

    /// Fix round 2, N6: the cache moves on writes to `topic_source` and `entity` too.
    #[test]
    fn generation_moves_on_topic_source_and_entity_writes() {
        let (_d, s) = store();
        let gen = |s: &Store| -> String { s.conn.query_row("SELECT value FROM kv_meta WHERE key = 'exclusion_generation'", [], |r| r.get(0)).unwrap() };
        s.conn.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('meeting', 1, 1, 1)", []).unwrap();
        let g0 = gen(&s);
        s.conn.execute("UPDATE topic_source SET ref_id = 2 WHERE ref_id = 1", []).unwrap();
        let g1 = gen(&s);
        assert_ne!(g0, g1, "topic_source update");
        s.conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:x@y.z', 'X', 1)", []).unwrap();
        assert_eq!(gen(&s), g1, "an entity nothing links to yet moves nothing");
        s.conn.execute("UPDATE entity SET key = 'email:w@y.z' WHERE key = 'email:x@y.z'", []).unwrap();
        assert_ne!(gen(&s), g1, "entity key update");
        let g2 = gen(&s);
        s.conn.execute("UPDATE entity SET display = 'Y', updated_at_unix_ms = 9", []).unwrap();
        s.conn.execute("UPDATE topic_source SET indexed_at_unix_ms = 9, assigned_at_unix_ms = NULL", []).unwrap();
        assert_eq!(gen(&s), g2, "columns the material is not read from");
    }

    fn capture(s: &Store, ts: i64, url: Option<&str>, text: &str) -> i64 {
        s.insert_capture_item(&crate::NewCaptureItem {
            device_id: "d".into(),
            ts_unix_ms: ts,
            app_bundle_id: "app".into(),
            window_title: "w".into(),
            url: url.map(str::to_string),
            text: text.into(),
            hash: ts,
            source: "ax".into(),
        })
        .unwrap();
        s.conn.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = ?1", [ts], |r| r.get(0)).unwrap()
    }

    fn episode(s: &Store, first: i64, last: i64) -> i64 {
        s.conn
            .execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', 'app', 'w', 1, 2, ?1, ?2, 2, 'summarized', 1, 1)",
                params![first, last],
            )
            .unwrap();
        s.conn.last_insert_rowid()
    }

    /// Fix round 3, R2: an episode lifted by a capture naming the client lifts every capture it
    /// was built over, as a filed episode does. Control: another episode's captures stay.
    #[test]
    fn a_lifted_episode_lifts_its_captures() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let named = capture(&s, 100, Some("https://acme.example/deck"), "Slide one");
        let sibling = capture(&s, 101, None, "Q3 pricing slide two");
        episode(&s, named, sibling);
        let other1 = capture(&s, 200, None, "Lunch");
        let other2 = capture(&s, 201, None, "Menu");
        episode(&s, other1, other2);
        exclude(&s, &a.key);
        let m = s.excluded_material_with(&|t| t.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| w == "acme")).unwrap();
        assert!(m.items.contains(&sibling), "the episode's sibling capture");
        assert!(!m.items.contains(&other1) && !m.items.contains(&other2));
    }

    /// Fix round 3, R5: a client contact whose mail also clustered into an automatic topic is
    /// still the client's contact (automatic topics are not "outside"), through index, cluster
    /// and a judge filing. Control: someone also in another client's (seed) material is not.
    #[test]
    fn a_client_contact_in_an_auto_topic_is_still_the_clients() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let g = s.create_seed_topic("Globex", &[], None).unwrap();
        for (i, (from, subject)) in [
            ("Wile <wile@coyote.example>", "Budget notes"),
            ("Wile <wile@coyote.example>", "Budget follow-up"),
            ("Wile <wile@coyote.example>", "Budget numbers"),
            ("Carol <carol@shared.example>", "Budget review"),
        ]
        .iter()
        .enumerate()
        {
            s.upsert_email_items(&[crate::NewEmailItem {
                device_id: "d".into(),
                external_id: format!("m{i}"),
                ts_unix_ms: 1_000 + i as i64,
                from: from.to_string(),
                to: "me@example.com".into(),
                subject: subject.to_string(),
                body_text: format!("{subject} body"),
                gmail_thread_id: format!("t{i}"),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();
        }
        while crate::topics::index::run_topic_index_job(&s, 200).unwrap() {}
        while crate::topics::cluster::run_topic_cluster_job(&s, 200).unwrap() {}
        let src = |ext: &str| -> i64 {
            s.conn
                .query_row(
                    "SELECT s.id FROM topic_source s JOIN items i ON i.id = s.ref_id WHERE s.kind = 'email' AND i.external_id = ?1",
                    [ext],
                    |r| r.get(0),
                )
                .unwrap()
        };
        // The judge files one of Wile's emails under Acme, Carol's under Acme and Globex.
        s.record_topic_verdict(src("m0"), Some(a.topic_id), None, "jev", 2).unwrap();
        s.record_topic_verdict(src("m3"), Some(a.topic_id), None, "jev", 2).unwrap();
        s.conn.execute("INSERT OR IGNORE INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![g.topic_id, src("m3")]).unwrap();
        exclude(&s, &a.key);
        let m = s.excluded_material_with(&|_| false).unwrap();
        assert!(m.attendee_emails.contains("wile@coyote.example"), "{:?}", m.attendee_emails);
        let item = |ext: &str| -> i64 { s.conn.query_row("SELECT id FROM items WHERE external_id = ?1", [ext], |r| r.get(0)).unwrap() };
        assert!(m.items.contains(&item("m2")), "Wile's unfiled mail is the client's");
        assert!(!m.attendee_emails.contains("carol@shared.example"), "control: also in another client's material");
    }

    /// Fix round 4, M3 (amended): only the user's exact account addresses are never a client
    /// contact. No domain is treated as the user's: a client contact on the user's own domain is
    /// still the client's, and so is a colleague (over-exclusion is the safe default).
    #[test]
    fn only_the_users_exact_addresses_are_never_client_contacts() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        s.conn
            .execute("INSERT INTO connector_account (connector, email, is_default, position, connected_at_unix_ms) VALUES ('gmail', 'me@shared.example', 1, 0, 0)", [])
            .unwrap();
        for (i, from) in ["Wile <Wile@Shared.example>", "Dana <dana@shared.example>", "Me <me@shared.example>"].iter().enumerate() {
            s.upsert_email_items(&[crate::NewEmailItem {
                device_id: "d".into(),
                external_id: format!("m{i}"),
                ts_unix_ms: 1_000 + i as i64,
                from: from.to_string(),
                to: "me@shared.example".into(),
                subject: format!("Budget {i}"),
                body_text: "Budget body".into(),
                gmail_thread_id: format!("t{i}"),
                account_email: "me@shared.example".into(),
                ..Default::default()
            }])
            .unwrap();
        }
        while crate::topics::index::run_topic_index_job(&s, 200).unwrap() {}
        let sources: Vec<i64> = s.conn.prepare("SELECT id FROM topic_source WHERE kind = 'email'").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        for src in sources {
            s.record_topic_verdict(src, Some(a.topic_id), None, "jev", 2).unwrap();
        }
        // The user's own address shows up as a person too (as a shell might record it).
        let me: i64 = match s.conn.query_row("SELECT id FROM entity WHERE key = 'email:me@shared.example'", [], |r| r.get(0)).optional().unwrap() {
            Some(id) => id,
            None => {
                s.conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:me@shared.example', 'Me', 1)", []).unwrap();
                s.conn.last_insert_rowid()
            }
        };
        let first: i64 = s.conn.query_row("SELECT source_id FROM topic_member WHERE topic_id = ?1 LIMIT 1", [a.topic_id], |r| r.get(0)).unwrap();
        s.conn.execute("INSERT OR IGNORE INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![me, first]).unwrap();
        exclude(&s, &a.key);
        let m = s.excluded_material_with(&|_| false).unwrap();
        assert!(m.attendee_emails.contains("wile@shared.example"), "a client contact on the user's domain stays: {:?}", m.attendee_emails);
        assert!(m.attendee_emails.contains("dana@shared.example"), "a colleague is over-excluded (safe default)");
        assert!(!m.attendee_emails.contains("me@shared.example"), "the user's own address");
    }

    /// Fix round 3, R6: a tombstone belongs to the client that owned the event: after that client
    /// is un-excluded, another excluded client does not keep its meetings excluded.
    #[test]
    fn tombstones_belong_to_the_events_client() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let g = s.create_seed_topic("Globex", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                 VALUES ('d', 'calendar_event', 'gcal', 50000000, '[]', 'Board', 'Board', 'ev-a')",
                [],
            )
            .unwrap();
        let ev = s.conn.last_insert_rowid();
        file(&s, a.topic_id, "calendar_event", ev);
        let m_id = s
            .insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 50_000_000 + 60_000,
                end_unix_ms: Some(50_000_000 + 3_600_000),
                title: "Board".into(),
                attendees: vec![],
                trigger: crate::MeetingTrigger::Calendar,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap();
        exclude(&s, &a.key);
        exclude(&s, &g.key);
        assert!(s.excluded_material_with(&|_| false).unwrap().meetings.contains(&m_id));
        s.conn.execute("DELETE FROM items WHERE id = ?1", [ev]).unwrap();
        assert!(s.excluded_material_with(&|_| false).unwrap().meetings.contains(&m_id), "tombstone");
        s.set_client_control(&a.key, &ClientControl { exclude_hosted: false, keep_months: None }).unwrap();
        assert!(!s.excluded_material_with(&|_| false).unwrap().meetings.contains(&m_id), "Globex does not keep Acme's event");
    }

    /// Fix round 3, R7: a read never waits on, or fails because of, the write lock.
    #[test]
    fn a_read_never_waits_on_the_write_lock() {
        let (d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                 VALUES ('d', 'calendar_event', 'gcal', 50000000, '[]', 'Board', 'Board', 'ev-a')",
                [],
            )
            .unwrap();
        let ev = s.conn.last_insert_rowid();
        file(&s, a.topic_id, "calendar_event", ev);
        exclude(&s, &a.key);
        let other = crate::test_store::raw(&d.path().join("t.db"));
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let start = std::time::Instant::now();
        let m = s.excluded_material_with(&|_| false);
        assert!(m.is_ok(), "{m:?}");
        assert!(start.elapsed() < std::time::Duration::from_millis(1_000), "{:?}", start.elapsed());
        assert!(m.unwrap().items.contains(&ev));
        other.execute_batch("ROLLBACK").unwrap();
    }

    fn generation(s: &Store) -> String {
        s.conn.query_row("SELECT value FROM kv_meta WHERE key = 'exclusion_generation'", [], |r| r.get(0)).unwrap()
    }

    fn dirty_rows(s: &Store) -> i64 {
        s.conn.query_row("SELECT count(*) FROM exclusion_dirty", [], |r| r.get(0)).unwrap()
    }

    fn full_recomputes() -> u64 {
        FULL_RECOMPUTES.with(|c| c.get())
    }

    fn acme(t: &str) -> bool {
        t.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| w == "acme")
    }

    /// An email in thread `thread`, with its thread row; returns the item id.
    fn mail(s: &Store, ext: &str, thread: &str, from: &str, subject: &str) -> i64 {
        s.upsert_email_items(&[crate::NewEmailItem {
            device_id: "d".into(),
            external_id: ext.into(),
            ts_unix_ms: 1_000,
            from: from.into(),
            to: "me@example.com".into(),
            subject: subject.into(),
            body_text: format!("{subject} body"),
            gmail_thread_id: thread.into(),
            account_email: "me@example.com".into(),
            ..Default::default()
        }])
        .unwrap();
        s.conn
            .execute(
                "INSERT OR IGNORE INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('gmail', ?1, 'me@example.com', ?2, 1, 1, 'a', 'open', 1, 1)",
                params![thread, subject],
            )
            .unwrap();
        s.conn.query_row("SELECT id FROM items WHERE kind = 'email' AND external_id = ?1", [ext], |r| r.get(0)).unwrap()
    }

    fn new_meeting(s: &Store, start: i64, title: &str, attendees: &[&str]) -> i64 {
        s.insert_meeting(&crate::NewMeeting {
            device_id: "d".into(),
            start_unix_ms: start,
            end_unix_ms: Some(start + 1_000),
            title: title.into(),
            attendees: attendees.iter().map(|a| a.to_string()).collect(),
            trigger: crate::MeetingTrigger::Calendar,
            audio_path: None,
            status: crate::MeetingStatus::Completed,
        })
        .unwrap()
    }

    /// Review M15: nothing points at an added item by id, so deleting one leaves it in the cached
    /// material (harmless) and moves no generation: the next read is not a full recompute.
    #[test]
    fn deleting_an_added_item_moves_no_generation() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        crate::mcp_items::tests::seed_tokens(&s);
        let w = crate::mcp_items::McpWriter { token_id: 1, client_name: "Claude".into() };
        let it = |id: &str| crate::mcp_items::NewMcpItem {
            label: "outlook-work".into(),
            external_id: id.into(),
            kind: "note".into(),
            ts: "2026-11-21T14:03:00Z".into(),
            title: Some("Acme renewal".into()),
            text: "notes".into(),
            url: None,
            people: vec![],
        };
        let now = 1_795_269_780_000;
        let clock = crate::mcp_items::WriteClock { now_unix_ms: now, day_start_unix_ms: now - now % 86_400_000 };
        s.upsert_mcp_items(&w, &[it("a"), it("b")], &Default::default(), clock).unwrap();
        let m = s.excluded_material_with(&acme).unwrap();
        let added = s.mcp_item_id(1, "outlook-work", "a").unwrap().unwrap();
        assert!(m.items.contains(&added));
        let g0 = generation(&s);
        let before = full_recomputes();
        let r = s.delete_mcp_items(1, "outlook-work", &["a".to_string()], now).unwrap();
        assert_eq!(r[0].1, crate::mcp_items::McpDeleteStatus::Deleted);
        assert_eq!(generation(&s), g0, "the tool's delete");
        s.delete_mcp_source(1, "outlook-work", false).unwrap();
        assert_eq!(generation(&s), g0, "a source delete");
        s.excluded_material_with(&acme).unwrap();
        assert_eq!(full_recomputes(), before, "the next read adds no full recompute");
    }

    /// Control for the test above: an email is pointed at by id (a thread), so its delete still
    /// moves the generation.
    #[test]
    fn deleting_an_email_still_moves_the_generation() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let e = mail(&s, "e1", "t1", "pat@example.org", "Acme renewal");
        s.excluded_material_with(&acme).unwrap();
        let g0 = generation(&s);
        s.conn.execute("DELETE FROM items WHERE id = ?1", [e]).unwrap();
        assert_ne!(generation(&s), g0);
    }

    /// Fix round 3, R8, and the dirty queue: a capture write (a late text update too) no longer
    /// moves the generation; it is noted for the next read, and noted again when it is updated.
    #[test]
    fn capture_writes_are_noted_not_a_generation_move() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let g0 = generation(&s);
        let c = capture(&s, 10, None, "x");
        assert_eq!(generation(&s), g0, "capture insert");
        let noted = |s: &Store| -> i64 {
            s.conn.query_row("SELECT max(seq) FROM exclusion_dirty WHERE tbl = 'items' AND row_id = ?1", [c], |r| r.get(0)).unwrap()
        };
        let first = noted(&s);
        s.conn.execute("UPDATE items SET body_text = 'later OCR' WHERE id = ?1", [c]).unwrap();
        assert_eq!(generation(&s), g0, "late text update");
        assert!(noted(&s) > first, "noted again, after the first mark");
        let rows: i64 = s.conn.query_row("SELECT count(*) FROM exclusion_dirty WHERE tbl = 'items' AND row_id = ?1", [c], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "one row per item");
    }

    /// New material naming the client is excluded on the next read without a full recompute.
    #[test]
    fn a_new_capture_or_email_naming_the_client_is_caught_without_a_full_recompute() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let before = s.excluded_material_with(&acme).unwrap();
        let fulls = full_recomputes();
        let cap = capture(&s, 10, None, "Acme pricing notes");
        let url_cap = capture(&s, 11, Some("https://acme.example/deck"), "slides");
        let other = capture(&s, 12, None, "Lunch menu");
        let named = mail(&s, "m1", "t1", "Pat <pat@x.com>", "Acme renewal");
        let unrelated = mail(&s, "m2", "t2", "Pat <pat@x.com>", "Lunch");
        let after = s.excluded_material_with(&acme).unwrap();
        assert_eq!(full_recomputes(), fulls, "no full recompute");
        assert!(after.items.contains(&cap) && after.items.contains(&url_cap) && after.items.contains(&named));
        assert!(!after.items.contains(&other) && !after.items.contains(&unrelated));
        let thread: i64 = s.conn.query_row("SELECT id FROM threads WHERE external_thread_id = 't1'", [], |r| r.get(0)).unwrap();
        assert!(after.threads.contains(&thread), "the thread of the email");
        assert_ne!(after.material_version, before.material_version, "the version moves with the material");
        let again = s.excluded_material_with(&acme).unwrap();
        assert_eq!(again, after);
        assert_eq!(full_recomputes(), fulls);
    }

    /// A moment marked during a meeting (redesign live screen) whose label names the client excludes the
    /// meeting, without a full recompute.
    #[test]
    fn a_marker_label_naming_the_client_excludes_its_meeting() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let m = new_meeting(&s, 1_000, "Weekly sync", &[]);
        assert!(!s.excluded_material_with(&acme).unwrap().meetings.contains(&m));
        let fulls = full_recomputes();
        s.add_meeting_marker(m, 500, Some("Acme pricing came up"), 2_000).unwrap();
        assert!(s.excluded_material_with(&acme).unwrap().meetings.contains(&m));
        assert_eq!(full_recomputes(), fulls, "no full recompute");
    }

    /// One `meeting_line_source` row with the text of each of its four columns given.
    fn line_source(line: &str, owner: Option<&str>, due: Option<&str>, spoken: Option<&str>) -> crate::line_source::NewLineSource {
        use crate::line_source::{anchor_of, base_fingerprint, LineKind, NewLineSource, SourceLocator};
        NewLineSource {
            kind: LineKind::ActionItem,
            line_text: line.into(),
            owner: owner.map(Into::into),
            due: due.map(Into::into),
            locator: spoken.map(|s| SourceLocator { offset_secs: 0, base: base_fingerprint("first"), anchor: anchor_of(s) }),
        }
    }

    /// The four text columns of a line source, one row each: only that column names the client.
    fn line_source_cases() -> [(&'static str, crate::line_source::NewLineSource); 4] {
        [
            ("line_text", line_source("Send Acme the deck", None, None, None)),
            ("owner", line_source("Send the deck", Some("Acme"), None, None)),
            ("due", line_source("Send the deck", Some("Ana"), Some("before the Acme call"), None)),
            ("anchor", line_source("Send the deck", None, None, Some("So, ACME wants the deck by Friday!"))),
        ]
    }

    /// A summary line's source (`meeting_line_source`) holds text the note never shows as such: an action
    /// item's owner and due date, and a normalized copy of a transcript line. Each of the four columns that
    /// names the client excludes its meeting, without a full recompute; a row that names nobody does not.
    #[test]
    fn a_line_source_naming_the_client_excludes_its_meeting() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let cases = line_source_cases();
        let ids: Vec<i64> = (0..cases.len()).map(|i| new_meeting(&s, 1_000 + i as i64, "Weekly sync", &[])).collect();
        let control = new_meeting(&s, 9_000, "Weekly sync", &[]);
        let before = s.excluded_material_with(&acme).unwrap();
        assert!(ids.iter().all(|m| !before.meetings.contains(m)));
        let fulls = full_recomputes();
        for (m, (_, row)) in ids.iter().zip(cases) {
            s.replace_meeting_line_sources(*m, &[row], 5).unwrap();
        }
        s.replace_meeting_line_sources(control, &[line_source("Send the deck", Some("Ana"), Some("Friday"), Some("so we send the deck"))], 5)
            .unwrap();
        let after = s.excluded_material_with(&acme).unwrap();
        for (m, (column, _)) in ids.iter().zip(line_source_cases()) {
            assert!(after.meetings.contains(m), "{column} names the client");
        }
        assert!(!after.meetings.contains(&control), "a row that names nobody");
        assert_eq!(full_recomputes(), fulls, "no full recompute");
    }

    /// An edit of a row that already exists is noted too: a re-summary rewrites it, a prune clears its anchor.
    #[test]
    fn a_line_source_edited_to_name_the_client_excludes_its_meeting() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let columns = ["line_text", "owner", "due", "anchor"];
        let ids: Vec<i64> = columns.iter().enumerate().map(|(i, _)| new_meeting(&s, 1_000 + i as i64, "Weekly sync", &[])).collect();
        for m in &ids {
            s.replace_meeting_line_sources(*m, &[line_source("Send the deck", Some("Ana"), Some("Friday"), Some("send the deck"))], 5).unwrap();
        }
        let before = s.excluded_material_with(&acme).unwrap();
        assert!(ids.iter().all(|m| !before.meetings.contains(m)));
        let fulls = full_recomputes();
        for (m, column) in ids.iter().zip(columns) {
            s.conn.execute(&format!("UPDATE meeting_line_source SET {column} = 'Acme' WHERE meeting_id = ?1"), [m]).unwrap();
        }
        let after = s.excluded_material_with(&acme).unwrap();
        for (m, column) in ids.iter().zip(columns) {
            assert!(after.meetings.contains(m), "{column} edited");
        }
        assert_eq!(full_recomputes(), fulls, "no full recompute");
    }

    /// The whole-meeting scan (client inventory, the full recompute) and the text checked before a summary or
    /// an MCP answer both read every column.
    #[test]
    fn a_line_source_is_read_by_the_full_scan_and_by_the_authored_text() {
        let (_d, s) = store();
        for (column, row) in line_source_cases() {
            let m = new_meeting(&s, 1_000, &format!("Sync {column}"), &[]);
            assert!(!s.meeting_text_hits(&acme, None).unwrap().iter().any(|(id, _)| *id == m));
            s.replace_meeting_line_sources(m, &[row], 5).unwrap();
            let hits = s.meeting_text_hits(&acme, None).unwrap();
            assert!(hits.iter().any(|(id, _)| *id == m), "{column}: the full meeting scan");
            let text = s.meeting_authored_text(m).unwrap();
            assert!(acme(&text), "{column}: the text a summary or an MCP answer is checked against: {text}");
        }
        let plain = new_meeting(&s, 2_000, "Lunch", &[]);
        s.replace_meeting_line_sources(plain, &[line_source("Order lunch", Some("Ana"), Some("noon"), Some("who is ordering lunch"))], 5).unwrap();
        assert!(!s.meeting_text_hits(&acme, None).unwrap().iter().any(|(id, _)| *id == plain));
        assert!(!acme(&s.meeting_authored_text(plain).unwrap()));
    }

    /// The matcher finds a client's name inside a stored anchor, which is normalized text (lowercased, every
    /// run of punctuation one space): names with an apostrophe, an ampersand, a hyphen, an accent, a sharp s.
    #[test]
    fn a_client_name_is_found_inside_a_normalized_anchor() {
        for (spoken, term) in [
            ("So, ACME wants the deck!", "Acme"),
            ("We spoke to O'Brien & Sons today.", "O'Brien & Sons"),
            ("We spoke to O'Brien & Sons today.", "O'Brien"),
            ("AT&T called back.", "AT&T"),
            ("Smith-Jones signed off.", "Smith-Jones"),
            ("Smith-Jones signed off.", "Smith Jones"),
            ("Ask José about it.", "Jose"),
            ("Ask JOSÉ about it.", "José"),
            ("Strauß wants a quote.", "Strauss"),
            ("Acme, Inc. wants a quote.", "Acme, Inc."),
            ("The 3M deal slipped.", "3M"),
        ] {
            let anchor = crate::line_source::anchor_of(spoken);
            assert!(crate::words::names_any(&anchor, &[term.to_string()]), "{term} in {anchor:?}");
            assert!(crate::words::names_any(spoken, &[term.to_string()]), "control: {term} in the spoken line");
        }
        // The one shape that does not survive: an accent written as a separate combining mark inside a
        // word. Normalizing makes the mark a separator ("jose phine"), so a term "Josephine" no longer
        // matches the anchor. The same line in `transcript_segment` carries the original characters and
        // is scanned by its own trigger, so the meeting is still found there.
        let decomposed = "Jose\u{301}phine called";
        assert!(crate::words::names_any(decomposed, &["Josephine".to_string()]), "the original line matches");
        assert!(!crate::words::names_any(&crate::line_source::anchor_of(decomposed), &["Josephine".to_string()]));
    }

    /// Every table with a `meeting_id` column and a text column either has triggers noting its writes
    /// (`CONTENT_TABLES`) or is named below with the reason the meeting scan does not read it. A table that is
    /// neither fails here, so a new one cannot be missed because no fixture happens to fill it (`meeting_marker`
    /// and `meeting_line_source` both went unlisted that way).
    #[test]
    fn every_table_with_meeting_text_is_registered_or_exempt() {
        // (table, why the exclusion scan does not read it)
        const EXEMPT: &[(&str, &str)] = &[
            ("forgotten_meeting", "a tombstone: device id and meeting id, no text"),
            ("meeting_part", "audio and transcript file paths and times of a recording's parts"),
            ("meeting_speaker_pending", "a synced voice row waiting for its meeting: ids, a person key and a status"),
            ("meeting_summary_fallback", "content-free by design: provider ids, a model id and a reason code"),
            ("profile_suggestion", "a profile fact, not meeting text; `purge_client_text` scrubs it by name"),
            ("pushed_meeting_row", "which server rows a meeting was pushed under: ids and a row type"),
            ("sync_meeting_map", "the sync pairing of meeting ids with a peer device id"),
        ];
        let (_d, s) = store();
        let tables: Vec<String> = s
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut unregistered = Vec::new();
        for t in tables {
            if t.contains("_fts") || t.contains("_vec") {
                continue;
            }
            let cols: Vec<(String, String)> = s
                .conn
                .prepare(&format!("SELECT name, upper(type) FROM pragma_table_info('{t}')"))
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let about_a_meeting = cols.iter().any(|(n, _)| n.contains("meeting_id"));
            let holds_text = cols.iter().any(|(_, ty)| ty.contains("TEXT"));
            if !about_a_meeting || !holds_text {
                continue;
            }
            let registered = CONTENT_TABLES.iter().any(|(c, ..)| *c == t);
            let exempt = EXEMPT.iter().any(|(c, _)| *c == t);
            if !registered && !exempt {
                unregistered.push(t);
            }
        }
        assert!(unregistered.is_empty(), "a table with text about a meeting is neither a content table nor exempt: {unregistered:?}");
        for (t, why) in EXEMPT {
            assert!(!why.is_empty());
            assert!(!CONTENT_TABLES.iter().any(|(c, ..)| c == t), "{t} is exempt and registered");
        }
    }

    /// Screen capture inserts that name nobody cost no recompute, and what was read is not read
    /// again: another connection's cache still sees the same rows.
    #[test]
    fn a_burst_of_unrelated_captures_never_recomputes() {
        let (d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let other = crate::test_store::open_with_embedder(&d.path().join("t.db"), Arc::new(FakeEmbedder));
        let base = s.excluded_material_with(&acme).unwrap();
        other.excluded_material_with(&acme).unwrap();
        let fulls = full_recomputes();
        for i in 0..100 {
            capture(&s, 100 + i, None, &format!("note {i}"));
        }
        assert!(dirty_rows(&s) >= 100);
        assert_eq!(s.excluded_material_with(&acme).unwrap(), base);
        // The rows were not consumed: the second connection reads them too.
        let named = capture(&s, 500, None, "Acme deck");
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&named));
        assert!(other.excluded_material_with(&acme).unwrap().items.contains(&named), "the other connection's cache");
        assert_eq!(full_recomputes(), fulls);
        // A cursor, not a rescan: nothing new, nothing applied.
        let cursor = s.exclusion_cache.borrow().as_ref().unwrap().cursor;
        s.excluded_material_with(&acme).unwrap();
        assert_eq!(s.exclusion_cache.borrow().as_ref().unwrap().cursor, cursor);
        // Old rows age out.
        s.conn.execute("UPDATE exclusion_dirty SET at_unix_s = at_unix_s - 7200", []).unwrap();
        capture(&s, 600, None, "more");
        s.excluded_material_with(&acme).unwrap();
        assert!(dirty_rows(&s) < 50, "{}", dirty_rows(&s));
    }

    /// A change to who is excluded (a name, a contact, a filing) is a full recompute.
    #[test]
    fn a_new_alias_or_a_changed_contact_recomputes_in_full() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let roadrunner = capture(&s, 10, None, "Roadrunner plans");
        let names = |t: &str| acme(t) || t.to_lowercase().contains("roadrunner");
        assert!(!s.excluded_material_with(&names).unwrap().items.contains(&roadrunner));
        let fulls = full_recomputes();
        s.update_seed_topic(&a.key, "Acme", &["Roadrunner".to_string()], None).unwrap();
        let m = s.excluded_material_with(&names).unwrap();
        assert_eq!(full_recomputes(), fulls + 1, "an alias");
        assert!(m.items.contains(&roadrunner));
        let sid = file(&s, a.topic_id, "meeting", 9);
        let m = s.excluded_material_with(&names).unwrap();
        assert_eq!(full_recomputes(), fulls + 1, "a filing is applied to the cached material");
        assert!(m.source_ids.contains(&sid) && m.meetings.contains(&9));
        s.conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
        s.excluded_material_with(&names).unwrap();
        assert_eq!(full_recomputes(), fulls + 1, "an entity nothing links to yet");
        s.conn.execute("UPDATE entity SET key = 'email:w2@coyote.example'", []).unwrap();
        s.excluded_material_with(&names).unwrap();
        assert_eq!(full_recomputes(), fulls + 2, "a changed contact key");
    }

    /// Nothing is removed in between: a row edited to no longer name the client stays excluded
    /// until a full recompute (the generation moving, or 30 minutes).
    #[test]
    fn an_edit_that_drops_the_name_keeps_the_row_excluded_until_a_full_recompute() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        s.excluded_material_with(&acme).unwrap();
        let fulls = full_recomputes();
        let cap = capture(&s, 10, None, "Acme pricing");
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&cap));
        s.conn.execute("UPDATE items SET body_text = 'lunch', title = 'w' WHERE id = ?1", [cap]).unwrap();
        s.conn.execute("UPDATE capture_item SET text = 'lunch', window_title = 'w' WHERE ts_unix_ms = 10", []).unwrap();
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&cap), "kept");
        assert_eq!(full_recomputes(), fulls);
        // A cache older than 30 minutes is computed in full again.
        s.exclusion_cache.borrow_mut().as_mut().unwrap().full_at_ms -= 31 * 60_000;
        assert!(!s.excluded_material_with(&acme).unwrap().items.contains(&cap), "dropped by the full recompute");
        assert_eq!(full_recomputes(), fulls + 1);
        // So does a moved generation.
        let cap2 = capture(&s, 11, None, "Acme again");
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&cap2));
        s.conn.execute("UPDATE items SET body_text = 'lunch', title = 'w' WHERE id = ?1", [cap2]).unwrap();
        s.conn.execute("UPDATE capture_item SET text = 'lunch', window_title = 'w' WHERE ts_unix_ms = 11", []).unwrap();
        s.update_seed_topic(&a.key, "Acme", &["Wile".to_string()], None).unwrap();
        assert!(!s.excluded_material_with(&acme).unwrap().items.contains(&cap2));
    }

    /// A meeting with a client contact among its attendees, and a new email in an excluded
    /// thread, are caught the way the full path catches them.
    #[test]
    fn a_new_meeting_with_a_contact_and_a_new_email_in_a_thread_are_caught() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let sid = file(&s, a.topic_id, "meeting", 5);
        s.conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
        let entity = s.conn.last_insert_rowid();
        s.conn.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![entity, sid]).unwrap();
        let first = mail(&s, "m1", "t1", "Pat <pat@x.com>", "Acme renewal");
        exclude(&s, &a.key);
        let before = s.excluded_material_with(&acme).unwrap();
        assert!(before.attendee_emails.contains("wile@coyote.example") && before.items.contains(&first));
        let fulls = full_recomputes();
        let meeting = new_meeting(&s, 9_000_000_000, "Weekly sync", &["wile@coyote.example"]);
        let control = new_meeting(&s, 8_000_000_000, "Weekly sync", &["pat@x.com"]);
        let reply = mail(&s, "m2", "t1", "Pat <pat@x.com>", "Re: lunch");
        let from_contact = mail(&s, "m3", "t3", "Wile <wile@coyote.example>", "Quick question");
        let m = s.excluded_material_with(&acme).unwrap();
        assert_eq!(full_recomputes(), fulls);
        assert!(m.meetings.contains(&meeting) && !m.meetings.contains(&control));
        assert!(m.items.contains(&reply), "same thread as an excluded email");
        assert!(m.items.contains(&from_contact), "from a client contact");
        let thread: i64 = s.conn.query_row("SELECT id FROM threads WHERE external_thread_id = 't3'", [], |r| r.get(0)).unwrap();
        assert!(m.threads.contains(&thread));
    }

    /// An item a local MCP client added (`mcp_item`); returns its `items` id.
    fn added(s: &Store, id: &str, title: &str, text: &str, email: Option<&str>) -> i64 {
        let item = crate::mcp_items::NewMcpItem {
            label: "outlook-work".into(),
            external_id: id.into(),
            kind: "email".into(),
            ts: "2026-11-21T14:03:00Z".into(),
            title: Some(title.into()),
            text: text.into(),
            url: None,
            people: email
                .map(|e| vec![crate::mcp_items::McpPerson { name: Some("Wile".into()), email: Some(e.into()), role: Some("from".into()) }])
                .unwrap_or_default(),
        };
        crate::mcp_items::tests::seed_tokens(s);
        let w = crate::mcp_items::McpWriter { token_id: 1, client_name: "Claude Code".into() };
        let clock = crate::mcp_items::WriteClock { now_unix_ms: 1_795_269_780_000, day_start_unix_ms: 1_795_219_200_000 };
        s.upsert_mcp_items(&w, &[item], &Default::default(), clock).unwrap();
        s.conn
            .query_row("SELECT id FROM items WHERE kind = 'mcp_item' AND external_id = ?1", [id], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn an_added_item_naming_an_excluded_client_is_excluded_in_full() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let in_title = added(&s, "a", "Acme renewal", "numbers", None);
        let in_text = added(&s, "b", "Renewal", "Notes for Acme's board", None);
        exclude(&s, &a.key);
        let m = s.excluded_material_with(&acme).unwrap();
        assert!(m.items.contains(&in_title) && m.items.contains(&in_text));
        assert!(m.excludes("mcp_item", in_title));
    }

    #[test]
    fn an_added_item_written_after_the_cache_is_noted_and_excluded() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        s.excluded_material_with(&acme).unwrap();
        let fulls = full_recomputes();
        let id = added(&s, "a", "Renewal", "Acme wants a discount", None);
        let noted: i64 = s.conn.query_row("SELECT COUNT(*) FROM exclusion_dirty WHERE tbl = 'items' AND row_id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(noted, 1);
        let m = s.excluded_material_with(&acme).unwrap();
        assert_eq!(full_recomputes(), fulls, "applied from the dirty queue");
        assert!(m.items.contains(&id));
    }

    #[test]
    fn an_added_item_carrying_a_client_contacts_address_is_excluded() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let sid = file(&s, a.topic_id, "meeting", 5);
        s.conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
        let entity = s.conn.last_insert_rowid();
        s.conn.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![entity, sid]).unwrap();
        let before = added(&s, "a", "Quick question", "About the invoice", Some("wile@coyote.example"));
        exclude(&s, &a.key);
        let m = s.excluded_material_with(&acme).unwrap();
        assert!(m.items.contains(&before), "full recompute");
        let after = added(&s, "b", "Another question", "Lunch?", Some("Wile@Coyote.example"));
        let m = s.excluded_material_with(&acme).unwrap();
        assert!(m.items.contains(&after), "incremental read");
    }

    #[test]
    fn an_added_item_naming_no_client_is_kept() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let id = added(&s, "a", "Lunch", "Tacos on Friday", Some("pat@x.com"));
        exclude(&s, &a.key);
        assert!(!s.excluded_material_with(&acme).unwrap().items.contains(&id));
        let later = added(&s, "b", "Dinner", "Pizza", None);
        assert!(!s.excluded_material_with(&acme).unwrap().items.contains(&later));
    }

    /// Nothing excluded: nothing is noted, and the answer is empty.
    #[test]
    fn nothing_is_noted_while_no_client_is_excluded() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        for i in 0..20 {
            capture(&s, i, None, "Acme");
        }
        mail(&s, "m1", "t1", "Pat <pat@x.com>", "Acme");
        new_meeting(&s, 1_000, "Acme sync", &[]);
        assert_eq!(dirty_rows(&s), 0);
        assert!(s.excluded_material_with(&acme).unwrap().is_empty());
        // Exclusion off again after a client was excluded: the setting is read at write time.
        exclude(&s, &a.key);
        capture(&s, 100, None, "x");
        assert!(dirty_rows(&s) > 0);
        s.set_client_control(&a.key, &ClientControl { exclude_hosted: false, keep_months: None }).unwrap();
        let rows = dirty_rows(&s);
        capture(&s, 101, None, "y");
        assert_eq!(dirty_rows(&s), rows);
        assert!(s.excluded_material_with(&acme).unwrap().is_empty());
    }

    /// A row noted again after the newest queue row was read is seen by that read already (an
    /// update moves its row to a higher seq, past a bound read earlier).
    #[test]
    fn a_row_noted_again_after_the_bound_was_read_is_seen() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        s.excluded_material_with(&acme).unwrap();
        let cap = capture(&s, 10, None, "plain");
        let stale: i64 = s.conn.query_row("SELECT max(seq) FROM exclusion_dirty", [], |r| r.get(0)).unwrap();
        s.conn.execute("UPDATE items SET body_text = 'Acme quote' WHERE id = ?1", [cap]).unwrap();
        assert!(s.excluded_material_upto(&acme, Some(stale)).unwrap().items.contains(&cap), "this read");
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&cap), "and the next");
    }

    /// One indexing pass for a new email: its source, text, person, mention and membership.
    fn index_email(s: &Store, ext: &str, thread: &str, from: &str, topic: i64) -> (i64, i64) {
        let item = mail(s, ext, thread, from, "Hello");
        s.conn
            .execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('email', ?1, 1, 'Hello', 1)", [item])
            .unwrap();
        let source = s.conn.last_insert_rowid();
        s.conn.execute("INSERT INTO topic_source_text (source_id, text) VALUES (?1, 'Hello body')", [source]).unwrap();
        let key = format!("email:{}", from.to_lowercase());
        s.conn
            .execute("INSERT OR IGNORE INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', ?1, 'P', 1)", [&key])
            .unwrap();
        let entity: i64 = s.conn.query_row("SELECT id FROM entity WHERE kind = 'person' AND key = ?1", [&key], |r| r.get(0)).unwrap();
        s.conn.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![entity, source]).unwrap();
        s.conn.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')", params![topic, source]).unwrap();
        (item, source)
    }

    /// A realistic indexing pass over an unrelated email costs no recompute, and the same pass
    /// for an email filed under the excluded client's topic excludes it without one.
    #[test]
    fn an_indexing_pass_is_applied_without_a_full_recompute() {
        let (d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        s.conn.execute("INSERT INTO topic (name, created_at_unix_ms, updated_at_unix_ms) VALUES ('Lunch', 1, 1)", []).unwrap();
        let other_topic = s.conn.last_insert_rowid();
        exclude(&s, &a.key);
        s.excluded_material_with(&acme).unwrap();
        let fulls = full_recomputes();
        let g0 = generation(&s);
        let (unrelated, _) = index_email(&s, "u1", "tu", "pat@x.com", other_topic);
        assert_eq!(generation(&s), g0, "the pass moves no generation");
        let m = s.excluded_material_with(&acme).unwrap();
        assert!(!m.items.contains(&unrelated));
        let (filed, source) = index_email(&s, "f1", "tf", "wile@coyote.example", a.topic_id);
        let m = s.excluded_material_with(&acme).unwrap();
        assert_eq!(full_recomputes(), fulls + 1, "a new contact address takes the full computation");
        assert!(m.items.contains(&filed) && m.source_ids.contains(&source), "filed under the client");
        assert!(m.attendee_emails.contains("wile@coyote.example"), "its address is a client contact now");
        let later = mail(&s, "f2", "tg", "Wile <wile@coyote.example>", "Quick question");
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&later), "mail from that contact");
        let (again, _) = index_email(&s, "f3", "th", "wile@coyote.example", a.topic_id);
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&again), "a known contact");
        assert_eq!(full_recomputes(), fulls + 1, "and nothing more");
        // The same state a fresh connection reaches in full.
        let fresh = crate::test_store::open_with_embedder(&d.path().join("t.db"), Arc::new(FakeEmbedder));
        let full = fresh.excluded_material_with(&acme).unwrap();
        let inc = s.excluded_material_with(&acme).unwrap();
        assert_eq!((inc.items, inc.source_ids, inc.attendee_emails), (full.items, full.source_ids, full.attendee_emails));
    }

    /// A calendar event filed under the client needs its series and owners: the full computation.
    #[test]
    fn a_calendar_event_filed_under_the_client_recomputes_in_full() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        s.excluded_material_with(&acme).unwrap();
        let fulls = full_recomputes();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                 VALUES ('d', 'calendar_event', 'gcal', 5000, '[]', 'Board', 'Board', 'ev-z')",
                [],
            )
            .unwrap();
        let ev = s.conn.last_insert_rowid();
        file(&s, a.topic_id, "calendar_event", ev);
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&ev));
        assert_eq!(full_recomputes(), fulls + 1);
    }

    /// Updates of columns the predicates do not read note nothing.
    #[test]
    fn an_update_of_an_unread_column_notes_nothing() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let cap = capture(&s, 10, None, "plain");
        let max = |s: &Store| -> i64 { s.conn.query_row("SELECT coalesce(max(seq), 0) FROM exclusion_dirty", [], |r| r.get(0)).unwrap() };
        let before = max(&s);
        s.conn.execute("UPDATE items SET hash = 7, flags = 'x', local_seq = 99 WHERE id = ?1", [cap]).unwrap();
        assert_eq!(max(&s), before);
        s.conn.execute("UPDATE items SET body_text = 'more' WHERE id = ?1", [cap]).unwrap();
        assert!(max(&s) > before);
        // A person written again with the same name and address moves nothing.
        s.conn.execute("INSERT INTO person (person_key, name, email, updated_at_unix_ms) VALUES ('p', 'Pat', 'p@x.com', 1)", []).unwrap();
        let g = generation(&s);
        s.conn.execute("UPDATE person SET name = 'Pat', email = 'p@x.com', updated_at_unix_ms = 5", []).unwrap();
        assert_eq!(generation(&s), g);
        s.conn.execute("UPDATE person SET name = 'Patricia'", []).unwrap();
        assert_ne!(generation(&s), g);
    }

    /// A synced speaker row can arrive before its person row: the person that arrives later is
    /// read through the meeting's text, so the meeting is checked again.
    #[test]
    fn a_person_arriving_after_its_speaker_row_is_applied() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let meeting = new_meeting(&s, 1_000, "Weekly sync", &[]);
        s.conn
            .execute("INSERT INTO meeting_speaker (meeting_id, speaker_id, person_key, updated_at_unix_ms) VALUES (?1, 1, 'p1', 1)", [meeting])
            .unwrap();
        assert!(!s.excluded_material_with(&acme).unwrap().meetings.contains(&meeting));
        s.conn.execute("INSERT INTO person (person_key, name, email, updated_at_unix_ms) VALUES ('p1', 'Acme Rep', 'rep@x.com', 1)", []).unwrap();
        assert!(s.excluded_material_with(&acme).unwrap().meetings.contains(&meeting));
        // A person key changed under a speaker is read again too.
        let other = new_meeting(&s, 9_000, "Planning", &[]);
        s.conn
            .execute("INSERT INTO meeting_speaker (meeting_id, speaker_id, person_key, updated_at_unix_ms) VALUES (?1, 1, 'p2', 1)", [other])
            .unwrap();
        s.conn.execute("INSERT INTO person (person_key, name, email, updated_at_unix_ms) VALUES ('p2', 'Pat', 'pat@x.com', 1)", []).unwrap();
        s.excluded_material_with(&acme).unwrap();
        s.conn.execute("UPDATE person SET person_key = 'p3', name = 'Acme Lead' WHERE person_key = 'p2'", []).unwrap();
        s.conn.execute("UPDATE meeting_speaker SET person_key = 'p3' WHERE meeting_id = ?1", [other]).unwrap();
        assert!(s.excluded_material_with(&acme).unwrap().meetings.contains(&other));
    }

    /// A contact found while applying rows can have older calendar events and recorded meetings:
    /// those need the event lift, which is the full computation's.
    #[test]
    fn a_new_contact_brings_its_events_and_their_meetings() {
        let (d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                 VALUES ('d', 'calendar_event', 'gcal', 50000000, '[\"wile@coyote.example\"]', 'Catch up', 'x', 'ev-w')",
                [],
            )
            .unwrap();
        let event = s.conn.last_insert_rowid();
        let recorded = new_meeting(&s, 50_000_500, "Sync", &[]);
        let far = new_meeting(&s, 90_000_000_000, "Elsewhere", &[]);
        exclude(&s, &a.key);
        s.excluded_material_with(&acme).unwrap();
        let (filed, _) = index_email(&s, "f1", "tf", "wile@coyote.example", a.topic_id);
        let m = s.excluded_material_with(&acme).unwrap();
        assert!(m.items.contains(&filed) && m.items.contains(&event), "the contact's event");
        assert!(m.meetings.contains(&recorded) && !m.meetings.contains(&far), "the meeting recorded over it");
        let fresh = crate::test_store::open_with_embedder(&d.path().join("t.db"), Arc::new(FakeEmbedder));
        let full = fresh.excluded_material_with(&acme).unwrap();
        assert_eq!((m.items, m.meetings, m.event_ids), (full.items, full.meetings, full.event_ids));
    }

    /// An excluded episode that gains captures brings them.
    #[test]
    fn an_excluded_episode_that_grows_brings_its_new_captures() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let named = capture(&s, 10, None, "Acme deck");
        let ep = episode(&s, named, named);
        let m = s.excluded_material_with(&acme).unwrap();
        assert!(m.episodes.contains(&ep));
        let later = capture(&s, 11, None, "slide two");
        assert!(!s.excluded_material_with(&acme).unwrap().items.contains(&later));
        s.conn.execute("UPDATE episodes SET end_item_id = ?1 WHERE id = ?2", params![later, ep]).unwrap();
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&later));
    }

    /// A write that carries its own conflict clause (`UPDATE OR IGNORE`) still re-notes the row:
    /// the trigger must not lean on its own `OR REPLACE`, which the outer statement overrides.
    #[test]
    fn a_row_noted_again_under_a_conflict_clause_is_applied() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let cap = capture(&s, 10, None, "plain");
        s.excluded_material_with(&acme).unwrap();
        s.conn.execute("UPDATE OR IGNORE items SET body_text = 'Acme quote' WHERE id = ?1", [cap]).unwrap();
        assert!(s.excluded_material_with(&acme).unwrap().items.contains(&cap));
    }

    /// Pruning the queue never makes a read wait for the write lock.
    #[test]
    fn pruning_the_queue_never_waits_on_the_write_lock() {
        let (d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        s.excluded_material_with(&acme).unwrap();
        capture(&s, 10, None, "x");
        s.conn.execute("UPDATE exclusion_dirty SET at_unix_s = at_unix_s - 7200", []).unwrap();
        let other = crate::test_store::raw(&d.path().join("t.db"));
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let start = std::time::Instant::now();
        let m = s.excluded_material_with(&acme);
        assert!(m.is_ok(), "{m:?}");
        assert!(start.elapsed() < std::time::Duration::from_millis(1_000), "{:?}", start.elapsed());
        other.execute_batch("ROLLBACK").unwrap();
        assert_eq!(s.busy_timeout_ms().unwrap(), 5_000, "the busy timeout is put back");
    }

    /// The incremental and the full paths agree: after inserts only, what a connection reaches by
    /// applying rows one read at a time is what a fresh connection computes in full.
    #[test]
    fn the_incremental_result_equals_a_fresh_full_recompute() {
        let (d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let sid = file(&s, a.topic_id, "meeting", 5);
        s.conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:wile@coyote.example', 'Wile', 1)", []).unwrap();
        let entity = s.conn.last_insert_rowid();
        s.conn.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![entity, sid]).unwrap();
        exclude(&s, &a.key);
        mail(&s, "m0", "t0", "Pat <pat@x.com>", "Acme kickoff");
        s.excluded_material_with(&acme).unwrap();
        let fulls = full_recomputes();
        let compare = |s: &Store| {
            let fresh = crate::test_store::open_with_embedder(&d.path().join("t.db"), Arc::new(FakeEmbedder));
            let (inc, full) = (s.excluded_material_with(&acme).unwrap(), fresh.excluded_material_with(&acme).unwrap());
            assert_eq!(inc.items, full.items, "items");
            assert_eq!(inc.meetings, full.meetings, "meetings");
            assert_eq!(inc.threads, full.threads, "threads");
            assert_eq!(inc.episodes, full.episodes, "episodes");
            assert_eq!(inc.events, full.events, "events");
            assert_eq!(inc.event_ids, full.event_ids, "event ids");
            assert_eq!(inc.event_series, full.event_series, "event series");
            assert_eq!(inc.clean_runs, full.clean_runs, "clean runs");
            assert_eq!(inc.material_version, full.material_version, "version");
        };
        let named = capture(&s, 10, None, "Acme deck");
        let sibling = capture(&s, 11, None, "slide two");
        compare(&s);
        episode(&s, named, sibling);
        compare(&s);
        let later = capture(&s, 12, None, "Acme again");
        let beside = capture(&s, 13, None, "unrelated");
        episode(&s, later, beside);
        compare(&s);
        mail(&s, "m1", "t0", "Pat <pat@x.com>", "Re: lunch");
        mail(&s, "m2", "t5", "Wile <wile@coyote.example>", "Hello");
        compare(&s);
        let meeting = new_meeting(&s, 50_000_000, "Acme sync", &[]);
        new_meeting(&s, 60_000_000, "Planning", &["wile@coyote.example"]);
        new_meeting(&s, 70_000_000, "Planning", &[]);
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, thread_id, ts_unix_ms, title, body_text) VALUES ('d', 'snapshot', 'ax', ?1, 1, 'slide', 'text')",
                [meeting],
            )
            .unwrap();
        compare(&s);
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
                 VALUES ('d', 'calendar_event', 'gcal', 90000000, '[]', 'Acme board', 'Board', 'ev-1')",
                [],
            )
            .unwrap();
        new_meeting(&s, 90_000_000, "Board", &[]);
        compare(&s);
        s.conn.execute("UPDATE meeting SET notes = 'Acme asked for a quote' WHERE title = 'Planning' AND start_unix_ms = 70000000", []).unwrap();
        compare(&s);
        // Runs: one clean, one built on it. When the first stops being clean, so does the second,
        // though nothing else changed.
        s.seed_builtin_routines().unwrap();
        let routine = s.list_routines().unwrap()[0].id;
        let run = |stamp: &str| {
            let id = s
                .insert_routine_run(&crate::NewRoutineRun {
                    routine_id: routine,
                    ran_at_unix_ms: 1,
                    status: crate::RunStatus::Success,
                    output_summary: "x".into(),
                    output_json: "{}".into(),
                })
                .unwrap();
            s.set_routine_run_provenance(id, stamp).unwrap();
            id
        };
        let built = run(&input_refs_stamp([input_ref("event", "evt-9")]).unwrap());
        let derived = run(&input_refs_stamp([input_ref("routine_run", built)]).unwrap());
        compare(&s);
        assert!(s.excluded_material_with(&acme).unwrap().clean_runs.is_superset(&[built, derived].into()));
        s.set_routine_run_provenance(built, UNFILTERED_PROVENANCE).unwrap();
        compare(&s);
        let m = s.excluded_material_with(&acme).unwrap();
        assert!(!m.clean_runs.contains(&built) && !m.clean_runs.contains(&derived));
        assert_eq!(full_recomputes(), fulls + 9, "only the fresh connections' computations");
    }

    /// Fix round 3, R3: a recompute over a large database is fast (the episode lift is driven by
    /// the excluded captures, not a join of every capture with every episode).
    #[test]
    fn recompute_is_fast_on_a_large_database() {
        recompute_bench(40_000, 1_000, 500);
    }

    /// The R3 benchmark at the re-review's size: `cargo test -p memory --release -- --ignored recompute_bench_200k`.
    #[test]
    #[ignore]
    fn recompute_bench_200k() {
        recompute_bench(200_000, 4_000, 1_000);
    }

    fn recompute_bench(items: i64, episodes: i64, max_ms: u128) {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let tx = s.conn.unchecked_transaction().unwrap();
        {
            let mut ins = tx
                .prepare("INSERT INTO items (device_id, kind, source, ts_unix_ms, title, body_text, url) VALUES ('d', 'capture', 'ax', ?1, 'w', ?2, NULL)")
                .unwrap();
            for i in 0..items {
                let text = if i % 5_000 == 0 { "Acme deck".to_string() } else { format!("note {i}") };
                ins.execute(params![i, text]).unwrap();
            }
            let first: i64 = tx.query_row("SELECT MIN(id) FROM items", [], |r| r.get(0)).unwrap();
            let per = items / episodes;
            let mut ep = tx
                .prepare("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','app','w',1,2,?1,?2,?3,'summarized',1,1)")
                .unwrap();
            for e in 0..episodes {
                ep.execute(params![first + e * per, first + e * per + per - 1, per]).unwrap();
            }
        }
        tx.commit().unwrap();
        exclude(&s, &a.key);
        let names = |t: &str| t.split(|c: char| !c.is_alphanumeric()).any(|w| w == "Acme");
        let start = std::time::Instant::now();
        let m = s.excluded_material_with(&names).unwrap();
        let took = start.elapsed();
        eprintln!("recompute over {items} items / {episodes} episodes: {took:?} ({} episodes lifted)", m.episodes.len());
        assert!(m.episodes.len() as i64 >= items / 5_000, "the named captures' episodes are lifted");
        assert!(took.as_millis() < max_ms, "{took:?}");
    }

    /// M5: the result is cached until something it reads changes.
    #[test]
    fn excluded_material_is_cached_until_the_generation_moves() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        exclude(&s, &a.key);
        let calls = std::cell::Cell::new(0);
        let names = |t: &str| {
            calls.set(calls.get() + 1);
            t.contains("Acme")
        };
        s.excluded_material_with(&names).unwrap();
        let first = calls.get();
        s.excluded_material_with(&names).unwrap();
        assert_eq!(calls.get(), first, "served from the cache");
        file(&s, a.topic_id, "meeting", 77);
        assert!(s.excluded_material_with(&names).unwrap().meetings.contains(&77), "a membership change is seen");
    }

    /// B2: a marked answer is stored as a local-only exchange.
    #[test]
    fn a_marked_answer_is_stored_local_only() {
        let (_d, s) = store();
        s.mark_local_only_text("They agreed the price.").unwrap();
        assert!(s.is_local_only_text("  They agreed the price.\n").unwrap());
        assert!(!s.is_local_only_text("Something else").unwrap());
        let conv = s.insert_conversation("t", 1).unwrap();
        s.append_conversation_exchange(conv, "q", "They agreed the price.", "[]", None, 2).unwrap();
        s.append_conversation_exchange(conv, "q", "Other.", "[]", None, 3).unwrap();
        let flags: Vec<i64> =
            s.conn.prepare("SELECT local_only FROM conversation_exchange ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(flags, vec![1, 0]);
    }

}
