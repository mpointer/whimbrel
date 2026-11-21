//! Sources for the lines of a finished note, and the end times of transcript lines. Both are
//! device-local: neither table is ever enqueued for sync, and neither enters a sync payload.
//!
//! `meeting_line_source` ties a decision, an open question or an action item to the transcript
//! line that supports it, and holds an action item's owner and due date. Each row is keyed by the
//! exact text of its summary line. A line that is edited, or regenerated on another device, no
//! longer matches its row, so the source stops showing instead of pointing at the wrong place.
//!
//! All four text columns (`line_text`, `owner`, `due`, `anchor`) can name a client, so the table is a
//! client exclusion content table (`topics::exclusion::CONTENT_TABLES`), is read by the meeting scan
//! and `Store::meeting_authored_text`, and goes with its meeting on a delete, a retention sweep or a merge.
//!
//! The locator is not a row id. The summary can be written before the transcript rows exist (the
//! Windows pipeline summarizes first), and a re-transcribe replaces every row, so a row id would
//! be missing or wrong. It is not a wall clock time either: the clock a transcript's tags count
//! from is not the clock a row's start is stored against (a calendar meeting's row starts at the
//! event, the tags at the recording), and a time zone or DST change shifts a wall clock by whole
//! hours. The locator is the line's offset, in seconds, from the first line of the transcript the
//! model saw (read from the `[HH:mm:ss]` tags, midnight wrap included), a short fingerprint of
//! that first line, and the line's normalized spoken text. At read time the line is found again
//! among the meeting's current rows by the same offset from the first current row. When the first
//! row has changed (a re-transcribe, a deleted first line) the offsets no longer share a base,
//! and only a long enough text that is unique in the whole meeting still resolves. A source is
//! returned only when exactly one row fits. A source that fits none, or two equally, is left out.
//!
//! The table holds no transcript words the person removed: [`Store::prune_unresolved_line_sources`]
//! runs after a trim and after a part's rows are replaced, and removes the locators that no longer
//! fit a current row (the whole row, unless it carries an action item's owner or due).
//!
//! `transcript_segment_end` holds how long each new transcript line ran (a duration, so a part
//! offset or a split needs no fix-up). It is removed with its line by a delete trigger.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::{MeetingTranscriptSegment, Store};

/// Longest anchor text kept, in characters.
const MAX_ANCHOR_CHARS: usize = 200;
/// How far a stored offset may sit from a row's own offset, in seconds, and still count as the
/// same moment (a tag has one second resolution).
const OFFSET_TOLERANCE_SECS: i64 = 5;
/// The longest a transcript line may run, in centiseconds (a day): a supplied end beyond it is
/// not an end.
pub(crate) const MAX_LINE_DURATION_CS: i64 = 24 * 3600 * 100;
/// How many leading words of the first line make its fingerprint. Few enough that a line the
/// speaker-labelling step split or joined keeps its start.
const BASE_WORDS: usize = 3;
/// The shortest text that may match by containment, in characters.
const MIN_CONTAINED_CHARS: usize = 15;
/// An anchor at least this long that matches exactly one row of the whole meeting resolves even
/// when its offset is out of tolerance.
const MIN_UNIQUE_ANCHOR_CHARS: usize = 30;

/// Which part of the note a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Decision,
    OpenQuestion,
    ActionItem,
}

impl LineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LineKind::Decision => "decision",
            LineKind::OpenQuestion => "open_question",
            LineKind::ActionItem => "action_item",
        }
    }
}

/// Where in the transcript a line came from, as the summarizer's transcript showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocator {
    /// Seconds from the first line of the transcript the model saw to this line, from the
    /// `[HH:mm:ss]` tags (a midnight wrap counts forward).
    pub offset_secs: i64,
    /// [`base_fingerprint`] of that first line, so a read can tell when the first line changed.
    pub base: i64,
    /// The line's spoken text, from the transcript (never the model's quote), as
    /// [`normalize_text`] writes it.
    pub anchor: String,
}

/// One row to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLineSource {
    pub kind: LineKind,
    /// The exact summary line (an action item: "Owner: task (due)").
    pub line_text: String,
    pub owner: Option<String>,
    pub due: Option<String>,
    pub locator: Option<SourceLocator>,
}

/// Lowercase, every run of non letters and digits one space, trimmed. Used to compare spoken
/// text with a quote or a stored anchor.
pub fn normalize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut gap = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            gap = false;
        } else if !gap {
            out.push(' ');
            gap = true;
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// A stable fingerprint (FNV-1a) of a first line's leading words, kept instead of the words.
pub fn base_fingerprint(spoken: &str) -> i64 {
    let n = normalize_text(spoken);
    let head: Vec<&str> = n.split(' ').take(BASE_WORDS).collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in head.join(" ").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h as i64
}

/// [`normalize_text`] cut to the anchor length, on a character boundary.
pub fn anchor_of(spoken: &str) -> String {
    let n = normalize_text(spoken);
    match n.char_indices().nth(MAX_ANCHOR_CHARS) {
        Some((i, _)) => n[..i].trim_end().to_string(),
        None => n,
    }
}

/// The transcript line a source points at, as the meeting holds it now. `segment_id` is the id
/// the transcript list calls return (`items.id`), so a view can scroll to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineSource {
    pub segment_id: i64,
    pub start_cs: i64,
    pub speaker_label: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourcedLine {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<LineSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourcedActionItem {
    /// The `meeting_action_item` row, `None` before the items are seeded from the summary.
    pub item_id: Option<i64>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<LineSource>,
}

/// The note's current lines with the sources that still hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteSources {
    pub decisions: Vec<SourcedLine>,
    pub open_questions: Vec<SourcedLine>,
    pub action_items: Vec<SourcedActionItem>,
}

/// One transcript line with its end time. `end_cs` is `None` for a line stored without one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptLine {
    pub id: i64,
    pub meeting_id: i64,
    pub meeting_title: String,
    pub start_cs: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_cs: Option<i64>,
    pub speaker_label: String,
    pub text: String,
}

pub(crate) fn ensure_line_source_tables(conn: &Connection) -> rusqlite::Result<()> {
    // An earlier shape of this device-local table located a line by wall clock (`clock_secs`). It
    // shipped in no release, so a database that still has it (a development one) loses those rows
    // instead of keeping a locator that can point at the wrong line; the next summary writes new ones.
    let old_shape: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('meeting_line_source') WHERE name = 'clock_secs'")?
        .exists([])?;
    if old_shape {
        conn.execute_batch("DROP TABLE meeting_line_source;")?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meeting_line_source (
            id INTEGER PRIMARY KEY,
            meeting_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            line_text TEXT NOT NULL,
            owner TEXT,
            due TEXT,
            offset_secs INTEGER,
            base_fp INTEGER,
            anchor TEXT,
            created_unix_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_meeting_line_source_meeting ON meeting_line_source(meeting_id);
        CREATE TABLE IF NOT EXISTS transcript_segment_end (
            segment_id INTEGER PRIMARY KEY,
            dur_cs INTEGER NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS transcript_segment_end_ad AFTER DELETE ON transcript_segment BEGIN
            DELETE FROM transcript_segment_end WHERE segment_id = old.id;
        END;",
    )
}

/// Whether a stored anchor and a row's normalized text are the same words: equal, or one inside
/// the other when the shorter has at least [`MIN_CONTAINED_CHARS`] characters (a line the
/// speaker-labelling step split or joined).
fn same_words(anchor: &str, row: &str) -> bool {
    if anchor.is_empty() || row.is_empty() {
        return false;
    }
    if anchor == row {
        return true;
    }
    let (short, long) = if anchor.len() <= row.len() { (anchor, row) } else { (row, anchor) };
    short.chars().count() >= MIN_CONTAINED_CHARS && long.contains(short)
}

struct Row {
    item_id: i64,
    start_cs: i64,
    /// Whole seconds from the first row (with words) of the meeting to this one.
    offset_secs: i64,
    speaker: String,
    text: String,
    norm: String,
}

#[cfg(test)]
thread_local! {
    /// How many times the meeting's rows were loaded and normalized, for the tests that check a
    /// batch of references does it once.
    static ROW_LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A meeting's current transcript rows, loaded and normalized once, with what a locator is
/// compared against. Build it with [`Store::line_source_index`] and reuse it for every reference
/// of the call.
pub struct LineSourceIndex {
    /// Rows that have words, in time order.
    rows: Vec<Row>,
    /// Whether the meeting has any transcript row at all.
    has_rows: bool,
    /// [`base_fingerprint`] of the first row with words.
    base: i64,
    /// Indexes into `rows` by their whole second offset.
    by_second: BTreeMap<i64, Vec<usize>>,
}

impl LineSourceIndex {
    fn build(raw: Vec<(i64, i64, String, String)>) -> Self {
        let has_rows = !raw.is_empty();
        let mut rows: Vec<Row> = raw
            .into_iter()
            .map(|(item_id, start_cs, speaker, text)| Row { item_id, start_cs, offset_secs: 0, speaker, norm: normalize_text(&text), text })
            .filter(|r| !r.norm.is_empty())
            .collect();
        let (base_cs, base) = rows.first().map_or((0, 0), |r| (r.start_cs, base_fingerprint(&r.norm)));
        let mut by_second: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
        for (i, r) in rows.iter_mut().enumerate() {
            r.offset_secs = r.start_cs.saturating_sub(base_cs).max(0) / 100;
            by_second.entry(r.offset_secs).or_default().push(i);
        }
        LineSourceIndex { rows, has_rows, base, by_second }
    }

    /// The one row a locator fits, or `None`.
    fn resolve(&self, loc: &SourceLocator) -> Option<&Row> {
        // Offsets share a base only while the first line is the one the locator counted from.
        if loc.base == self.base {
            let window = self
                .by_second
                .range(loc.offset_secs.saturating_sub(OFFSET_TOLERANCE_SECS)..=loc.offset_secs.saturating_add(OFFSET_TOLERANCE_SECS));
            let mut best: Option<(i64, &Row)> = None;
            let mut tied = false;
            for i in window.flat_map(|(_, v)| v.iter()) {
                let r = &self.rows[*i];
                if !same_words(&loc.anchor, &r.norm) {
                    continue;
                }
                let d = r.offset_secs.abs_diff(loc.offset_secs) as i64;
                match best {
                    Some((bd, _)) if d > bd => {}
                    Some((bd, _)) if d == bd => tied = true,
                    _ => {
                        best = Some((d, r));
                        tied = false;
                    }
                }
            }
            if let Some((_, r)) = best {
                // Two rows equally near: the line is ambiguous, so it has no source.
                return (!tied).then_some(r);
            }
        }
        if loc.anchor.chars().count() < MIN_UNIQUE_ANCHOR_CHARS {
            return None;
        }
        let mut fits = self.rows.iter().filter(|r| same_words(&loc.anchor, &r.norm));
        let first = fits.next()?;
        fits.next().is_none().then_some(first)
    }

    /// Whether `locator` fits a row. `None` when the meeting has no rows yet, so nothing can be
    /// said (the summary can be written before the rows are stored).
    pub fn resolves(&self, locator: &SourceLocator) -> Option<bool> {
        self.has_rows.then(|| self.resolve(locator).is_some())
    }
}

/// One stored row, as held on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoredLineSource {
    pub kind: String,
    pub line_text: String,
    pub owner: Option<String>,
    pub due: Option<String>,
    /// The transcript words the row locates its line by, `None` for a row with no locator.
    pub anchor: Option<String>,
    pub offset_secs: Option<i64>,
}

fn load_index(conn: &Connection, meeting_id: i64) -> rusqlite::Result<LineSourceIndex> {
    #[cfg(test)]
    ROW_LOADS.with(|c| c.set(c.get() + 1));
    let mut stmt = conn.prepare(
        "SELECT i.id, ts.start_cs, ts.speaker_label, ts.text FROM transcript_segment ts \
         JOIN items i ON i.kind = 'transcript' AND i.source_table = 'transcript_segment' AND i.source_id = ts.id \
         WHERE ts.meeting_id = ?1 ORDER BY ts.start_cs, ts.id",
    )?;
    let raw = stmt
        .query_map(params![meeting_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(LineSourceIndex::build(raw))
}

impl Store {
    /// Replaces the meeting's line sources with `rows`, as one set. Returns how many were stored;
    /// an error, with nothing changed, for a meeting that does not exist.
    pub fn replace_meeting_line_sources(
        &self,
        meeting_id: i64,
        rows: &[NewLineSource],
        now_unix_ms: i64,
    ) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let exists: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM meeting WHERE id = ?1)", params![meeting_id], |r| r.get(0))?;
        if !exists {
            return Err(rusqlite::Error::ToSqlConversionFailure("no such meeting".into()));
        }
        tx.execute("DELETE FROM meeting_line_source WHERE meeting_id = ?1", params![meeting_id])?;
        for row in rows {
            tx.execute(
                "INSERT INTO meeting_line_source (meeting_id, kind, line_text, owner, due, offset_secs, base_fp, anchor, created_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    meeting_id,
                    row.kind.as_str(),
                    row.line_text,
                    row.owner,
                    row.due,
                    row.locator.as_ref().map(|l| l.offset_secs),
                    row.locator.as_ref().map(|l| l.base),
                    row.locator.as_ref().map(|l| l.anchor.clone()),
                    now_unix_ms,
                ],
            )?;
        }
        tx.commit()?;
        Ok(rows.len())
    }

    /// Drops every line source of the meeting (a summary that was cleared).
    pub(crate) fn delete_line_sources_tx(tx: &Connection, meeting_id: i64) -> rusqlite::Result<()> {
        tx.execute("DELETE FROM meeting_line_source WHERE meeting_id = ?1", params![meeting_id])?;
        Ok(())
    }

    /// A duplicate copy's sources go to the kept meeting when it has none of its own, else with
    /// the copy. Same recording, same clock.
    pub(crate) fn fold_line_sources_tx(tx: &Connection, drop: i64, keep: i64) -> rusqlite::Result<()> {
        let kept: i64 =
            tx.query_row("SELECT COUNT(*) FROM meeting_line_source WHERE meeting_id = ?1", params![keep], |r| r.get(0))?;
        if kept == 0 {
            tx.execute("UPDATE meeting_line_source SET meeting_id = ?1 WHERE meeting_id = ?2", params![keep, drop])?;
        } else {
            Self::delete_line_sources_tx(tx, drop)?;
        }
        Ok(())
    }

    /// The meeting's current transcript rows, loaded and normalized once, for resolving any number
    /// of locators (ids are the ones a transcript list shows).
    pub fn line_source_index(&self, meeting_id: i64) -> rusqlite::Result<LineSourceIndex> {
        load_index(&self.conn, meeting_id)
    }

    /// Deletes the meeting's line source rows whose locator no longer fits a current transcript
    /// row, so the table keeps no transcript words the person removed (a trim, a re-transcribe, a
    /// deleted line). A row with no locator holds none and stays; one that has an owner or due (an
    /// action item) loses only its locator and stays. Returns how many locators went.
    pub fn prune_unresolved_line_sources(&self, meeting_id: i64) -> rusqlite::Result<usize> {
        Self::prune_unresolved_line_sources_tx(&self.conn, meeting_id)
    }

    /// [`Store::prune_unresolved_line_sources`] on the caller's connection or transaction, so it
    /// sees the rows the caller has just written or deleted.
    pub(crate) fn prune_unresolved_line_sources_tx(conn: &Connection, meeting_id: i64) -> rusqlite::Result<usize> {
        let mut stmt = conn.prepare(
            "SELECT id, offset_secs, base_fp, anchor, owner IS NOT NULL OR due IS NOT NULL FROM meeting_line_source \
             WHERE meeting_id = ?1 AND anchor IS NOT NULL",
        )?;
        let located = stmt
            .query_map(params![meeting_id], |r| {
                let offset: Option<i64> = r.get(1)?;
                let base: Option<i64> = r.get(2)?;
                let anchor: String = r.get(3)?;
                Ok((r.get::<_, i64>(0)?, r.get::<_, bool>(4)?, SourceLocator { offset_secs: offset.unwrap_or(0), base: base.unwrap_or(0), anchor }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if located.is_empty() {
            return Ok(0);
        }
        let index = load_index(conn, meeting_id)?;
        let mut gone = 0;
        for (id, has_details, locator) in located {
            if index.resolve(&locator).is_some() {
                continue;
            }
            gone += 1;
            if has_details {
                // An action item's owner and due are the person's to keep; only the transcript words go.
                conn.execute("UPDATE meeting_line_source SET offset_secs = NULL, base_fp = NULL, anchor = NULL WHERE id = ?1", params![id])?;
            } else {
                conn.execute("DELETE FROM meeting_line_source WHERE id = ?1", params![id])?;
            }
        }
        Ok(gone)
    }

    /// Every row held for the meeting, shown in the note or not, for "your data".
    pub fn meeting_line_source_rows(&self, meeting_id: i64) -> rusqlite::Result<Vec<StoredLineSource>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, line_text, owner, due, anchor, offset_secs FROM meeting_line_source WHERE meeting_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![meeting_id], |r| {
            Ok(StoredLineSource { kind: r.get(0)?, line_text: r.get(1)?, owner: r.get(2)?, due: r.get(3)?, anchor: r.get(4)?, offset_secs: r.get(5)? })
        })?;
        rows.collect()
    }

    /// The note's current lines (the edited ones when the person edited them, else the generated
    /// ones; action items from the item rows once they are seeded, else the generated strings),
    /// each with its source when one still holds: its row's `line_text` equals the line exactly
    /// and the transcript line is found among the meeting's current rows. Action items carry
    /// their owner and due when stored, tied to the item's text the same way. `None` for a
    /// meeting that does not exist.
    pub fn meeting_note_sources(&self, meeting_id: i64) -> rusqlite::Result<Option<NoteSources>> {
        let Some(meeting) = self.get_meeting(meeting_id)? else { return Ok(None) };
        let draft = self.get_meeting_note_draft(meeting_id)?;
        let decisions = draft.as_ref().and_then(|d| d.decisions_edited.clone()).unwrap_or(meeting.summary_decisions.clone());
        let questions = draft.as_ref().and_then(|d| d.open_questions_edited.clone()).unwrap_or(meeting.summary_open_questions.clone());
        let items = self.list_meeting_action_items(meeting_id)?;

        let mut stmt = self.conn.prepare(
            "SELECT kind, line_text, owner, due, offset_secs, base_fp, anchor FROM meeting_line_source WHERE meeting_id = ?1 ORDER BY id",
        )?;
        struct Stored {
            owner: Option<String>,
            due: Option<String>,
            locator: Option<SourceLocator>,
        }
        let mut stored: HashMap<(String, String), Stored> = HashMap::new();
        let mut any_locator = false;
        for row in stmt.query_map(params![meeting_id], |r| {
            let offset: Option<i64> = r.get(4)?;
            let base: Option<i64> = r.get(5)?;
            let anchor: Option<String> = r.get(6)?;
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                Stored {
                    owner: r.get(2)?,
                    due: r.get(3)?,
                    locator: anchor.map(|anchor| SourceLocator { offset_secs: offset.unwrap_or(0), base: base.unwrap_or(0), anchor }),
                },
            ))
        })? {
            let (key, value) = row?;
            any_locator |= value.locator.is_some();
            // The first row of two identical lines wins.
            stored.entry(key).or_insert(value);
        }
        // Loaded and normalized once, whatever the number of lines.
        let index = if any_locator { Some(self.line_source_index(meeting_id)?) } else { None };
        let source_of = |s: &Stored| -> Option<LineSource> {
            let row = index.as_ref()?.resolve(s.locator.as_ref()?)?;
            Some(LineSource { segment_id: row.item_id, start_cs: row.start_cs, speaker_label: row.speaker.clone(), text: row.text.clone() })
        };
        let lines = |kind: LineKind, texts: Vec<String>| -> Vec<SourcedLine> {
            texts
                .into_iter()
                .map(|text| {
                    let source = stored.get(&(kind.as_str().to_string(), text.clone())).and_then(&source_of);
                    SourcedLine { text, source }
                })
                .collect()
        };
        let action = |item_id: Option<i64>, text: String| -> SourcedActionItem {
            match stored.get(&(LineKind::ActionItem.as_str().to_string(), text.clone())) {
                Some(s) => SourcedActionItem { item_id, owner: s.owner.clone(), due: s.due.clone(), source: source_of(s), text },
                None => SourcedActionItem { item_id, text, owner: None, due: None, source: None },
            }
        };
        let action_items = if items.is_empty() {
            meeting.summary_action_items.iter().cloned().map(|t| action(None, t)).collect()
        } else {
            // A person's own item never takes a generated line's owner, due or source.
            items
                .into_iter()
                .map(|i| {
                    if i.source == "generated" {
                        action(Some(i.id), i.text)
                    } else {
                        SourcedActionItem { item_id: Some(i.id), text: i.text, owner: None, due: None, source: None }
                    }
                })
                .collect()
        };
        Ok(Some(NoteSources {
            decisions: lines(LineKind::Decision, decisions),
            open_questions: lines(LineKind::OpenQuestion, questions),
            action_items,
        }))
    }

    /// Whether `locator` fits one of the meeting's current transcript rows. `None` when the
    /// meeting has no rows yet, so nothing can be said (the summary can be written before the
    /// rows are stored). Loads the rows each call: to check several, use [`Store::line_source_index`].
    pub fn line_locator_resolves(&self, meeting_id: i64, locator: &SourceLocator) -> rusqlite::Result<Option<bool>> {
        if self.get_meeting(meeting_id)?.is_none() {
            return Ok(None);
        }
        Ok(self.line_source_index(meeting_id)?.resolves(locator))
    }

    /// How long each of the meeting's lines ran, by the id a transcript list shows. Lines stored
    /// without an end time are absent.
    pub fn transcript_end_cs_by_item(&self, meeting_id: i64) -> rusqlite::Result<HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, ts.start_cs + e.dur_cs FROM transcript_segment ts \
             JOIN transcript_segment_end e ON e.segment_id = ts.id \
             JOIN items i ON i.kind = 'transcript' AND i.source_table = 'transcript_segment' AND i.source_id = ts.id \
             WHERE ts.meeting_id = ?1",
        )?;
        let rows = stmt.query_map(params![meeting_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        rows.collect()
    }

    /// [`Store::get_transcript_segments_page`] with each line's end time (`None` when it was
    /// stored without one). Same ids, order and total.
    pub fn get_transcript_lines_page(
        &self,
        meeting_id: i64,
        offset: usize,
        limit: usize,
    ) -> rusqlite::Result<(Vec<TranscriptLine>, usize)> {
        let (segments, total) = self.get_transcript_segments_page(meeting_id, offset, limit)?;
        let ends = self.transcript_end_cs_by_item(meeting_id)?;
        Ok((segments.into_iter().map(|s| line_of(s, &ends)).collect(), total))
    }

    /// Records how long a just inserted line ran. Inside the insert transaction.
    pub(crate) fn set_segment_duration_tx(tx: &Connection, segment_id: i64, dur_cs: i64) -> rusqlite::Result<()> {
        tx.execute(
            "INSERT OR REPLACE INTO transcript_segment_end (segment_id, dur_cs) VALUES (?1, ?2)",
            params![segment_id, dur_cs.max(0)],
        )?;
        Ok(())
    }
}

fn line_of(s: MeetingTranscriptSegment, ends: &HashMap<i64, i64>) -> TranscriptLine {
    TranscriptLine {
        end_cs: ends.get(&s.id).copied(),
        id: s.id,
        meeting_id: s.meeting_id,
        meeting_title: s.meeting_title,
        start_cs: s.start_cs,
        speaker_label: s.speaker_label,
        text: s.text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting, NewTranscriptSegment};

    const START: i64 = 1_700_000_000_000;

    fn meeting(store: &Store, start: i64) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 600_000),
                title: "Standup".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn seg(m: i64, start_cs: i64, speaker: &str, text: &str) -> NewTranscriptSegment {
        NewTranscriptSegment { meeting_id: m, start_cs, speaker_label: speaker.into(), text: text.into() }
    }

    /// The locator for a line `secs` after the first line of a transcript that opened with `first`.
    fn locator(first: &str, secs: i64, spoken: &str) -> SourceLocator {
        SourceLocator { offset_secs: secs, base: base_fingerprint(first), anchor: anchor_of(spoken) }
    }

    fn row(kind: LineKind, line: &str, loc: Option<SourceLocator>) -> NewLineSource {
        NewLineSource { kind, line_text: line.into(), owner: None, due: None, locator: loc }
    }

    /// Every anchor the table holds for the meeting.
    fn anchors(s: &Store, m: i64) -> Vec<String> {
        s.conn
            .prepare("SELECT anchor FROM meeting_line_source WHERE meeting_id = ?1 AND anchor IS NOT NULL ORDER BY id")
            .unwrap()
            .query_map([m], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn summarized(s: &Store, m: i64) {
        s.update_meeting_summary(
            m,
            "tldr",
            &["Ship on Friday".to_string(), "Hire two".to_string()],
            &["Ana: send the deck (Monday)".to_string()],
            &["Who owns pricing?".to_string()],
            "p",
            "mod",
        )
        .unwrap();
    }

    fn outbox(store: &Store) -> i64 {
        store.conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0)).unwrap()
    }

    fn count(store: &Store, table: &str) -> i64 {
        store.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn normalizing_ignores_case_punctuation_and_spacing() {
        assert_eq!(normalize_text("  We'll SHIP, on  Friday!! "), "we ll ship on friday");
        assert_eq!(normalize_text("...").len(), 0);
        assert_eq!(anchor_of(&"word ".repeat(100)).chars().count() <= MAX_ANCHOR_CHARS, true);
    }

    #[test]
    fn a_source_returns_the_segments_own_text_and_a_list_id() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        s.insert_transcript_segments(&[
            seg(m, 0, "Speaker 1", "Welcome everyone to the standup."),
            seg(m, 6_000, "Speaker 2", "We will ship on Friday, no later than noon."),
        ])
        .unwrap();
        summarized(&s, m);
        s.replace_meeting_line_sources(
            m,
            &[row(LineKind::Decision, "Ship on Friday", Some(locator("Welcome everyone to the standup.", 60, "we will ship on friday no later than noon")))],
            5,
        )
        .unwrap();
        let notes = s.meeting_note_sources(m).unwrap().unwrap();
        let src = notes.decisions[0].source.clone().unwrap();
        assert_eq!(src.text, "We will ship on Friday, no later than noon.", "the row's own text, not the model's");
        assert_eq!((src.start_cs, src.speaker_label.as_str()), (6_000, "Speaker 2"));
        let (page, _) = s.get_transcript_segments_page(m, 0, 10).unwrap();
        assert_eq!(src.segment_id, page[1].id, "the id the transcript list shows");
        assert!(notes.decisions[1].source.is_none(), "a line without a row has no source");
        assert_eq!(notes.open_questions[0].text, "Who owns pricing?");
    }

    #[test]
    fn a_source_waits_for_rows_that_do_not_exist_yet() {
        // The Windows pipeline summarizes before the transcript rows are stored.
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        summarized(&s, m);
        s.replace_meeting_line_sources(
            m,
            &[row(LineKind::Decision, "Ship on Friday", Some(locator("Welcome everyone to the standup.", 60, "We will ship on Friday, no later than noon.")))],
            5,
        )
        .unwrap();
        assert!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.is_none());
        // The speaker labels come later, with the rows; the words are what hold the source.
        s.insert_transcript_segments(&[
            seg(m, 0, "Speaker 1", "Welcome everyone to the standup."),
            seg(m, 6_000, "Speaker 2", "We will ship on Friday, no later than noon."),
        ])
        .unwrap();
        assert!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.is_some());
    }

    #[test]
    fn a_changed_line_or_a_retranscribed_line_stops_resolving() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        s.set_own_device_id("d");
        s.begin_meeting_part(m, START, None).unwrap();
        s.complete_meeting_part(m, 1, START + 600_000, None).unwrap();
        s.insert_transcript_segments(&[seg(m, 6_000, "Speaker 2", "We will ship on Friday, no later than noon.")]).unwrap();
        summarized(&s, m);
        let said = "We will ship on Friday, no later than noon.";
        let loc = locator(said, 0, said);
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "Ship on Friday", Some(loc))], 5).unwrap();
        assert!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.is_some());
        // The person edits the line: the row no longer matches it.
        s.save_meeting_note_overrides(m, None, Some(&["Ship on Friday at noon".to_string()]), None, 9).unwrap();
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert_eq!(n.decisions[0].text, "Ship on Friday at noon");
        assert!(n.decisions[0].source.is_none());
        s.reset_meeting_note_draft(m, 10).unwrap();
        assert!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.is_some());
        // A re-transcribe that hears something else there: the source goes rather than move.
        s.replace_part_transcript_segments(m, 1, &[seg(m, 6_000, "Speaker 1", "Completely different words about lunch plans.")]).unwrap();
        assert!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.is_none());
    }

    #[test]
    fn two_lines_that_fit_equally_give_no_source_and_the_closer_one_wins_otherwise() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        summarized(&s, m);
        s.insert_transcript_segments(&[
            seg(m, 1_000, "Speaker 1", "Yes, that works for me."),
            seg(m, 90_000, "Speaker 2", "Yes, that works for me."),
        ])
        .unwrap();
        let near_second = row(LineKind::Decision, "Ship on Friday", Some(locator("Yes, that works for me.", 890, "Yes, that works for me.")));
        s.replace_meeting_line_sources(m, &[near_second], 1).unwrap();
        let src = s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.clone().unwrap();
        assert_eq!(src.start_cs, 90_000, "the offset picks between identical lines");
        // A tag that fits neither line, with the words on two rows: no source.
        let lost = row(LineKind::Decision, "Ship on Friday", Some(locator("Yes, that works for me.", 490, "Yes, that works for me.")));
        s.replace_meeting_line_sources(m, &[lost], 1).unwrap();
        assert!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.is_none());
    }

    #[test]
    fn action_items_carry_owner_due_and_source_by_their_text() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        s.insert_transcript_segments(&[seg(m, 500, "Speaker 1", "Ana, can you send the deck by Monday?")]).unwrap();
        summarized(&s, m);
        let line = "Ana: send the deck (Monday)";
        let mut r = row(LineKind::ActionItem, line, Some(locator("Ana, can you send the deck by Monday?", 0, "Ana, can you send the deck by Monday?")));
        r.owner = Some("Ana".into());
        r.due = Some("Monday".into());
        s.replace_meeting_line_sources(m, &[r], 1).unwrap();
        // Before the items are seeded, the generated strings are listed without an item id.
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert_eq!((n.action_items[0].item_id, n.action_items[0].owner.as_deref()), (None, Some("Ana")));
        assert!(n.action_items[0].source.is_some());
        s.seed_meeting_action_items_from_generated(m, &[line.to_string()], 2).unwrap();
        let id = s.list_meeting_action_items(m).unwrap()[0].id;
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert_eq!(n.action_items[0].item_id, Some(id));
        assert_eq!(n.action_items[0].due.as_deref(), Some("Monday"));
        // The person edits the item's text: owner, due and source stop attaching.
        s.update_meeting_action_item_text(id, "Ana: send the deck today", 3).unwrap();
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert_eq!((n.action_items[0].owner.clone(), n.action_items[0].due.clone(), n.action_items[0].source.is_some()), (None, None, false));
        // A person's own item with the same words never takes a generated line's details.
        let own = s.insert_meeting_action_item(m, line, 9, 4).unwrap();
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        let own_row = n.action_items.iter().find(|i| i.item_id == Some(own)).unwrap();
        assert!(own_row.owner.is_none() && own_row.source.is_none());
    }

    #[test]
    fn rows_are_replaced_as_a_set_and_need_a_meeting() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        let other = meeting(&s, START + 9_000_000);
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "a", None), row(LineKind::Decision, "b", None)], 1).unwrap();
        s.replace_meeting_line_sources(other, &[row(LineKind::Decision, "c", None)], 1).unwrap();
        assert_eq!(count(&s, "meeting_line_source"), 3);
        assert_eq!(s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "d", None)], 2).unwrap(), 1);
        assert_eq!(count(&s, "meeting_line_source"), 2, "the other meeting's row stays");
        assert!(s.replace_meeting_line_sources(m + 99, &[row(LineKind::Decision, "e", None)], 2).is_err());
        assert_eq!(count(&s, "meeting_line_source"), 2);
        assert!(s.meeting_note_sources(m + 99).unwrap().is_none());
    }

    #[test]
    fn deleting_forgetting_and_expiring_a_meeting_removes_its_rows_only() {
        let s = Store::open_in_memory().unwrap();
        let (a, b, c, keep) = (meeting(&s, 1_000), meeting(&s, 2_000), meeting(&s, 3_000), meeting(&s, 90_000_000));
        for m in [a, b, c, keep] {
            s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "x", None)], 1).unwrap();
        }
        assert!(s.delete_meeting(a).unwrap());
        s.forget_meeting(b).unwrap();
        assert_eq!(count(&s, "meeting_line_source"), 2);
        let (n, _) = s.delete_meetings_before(50_000_000).unwrap();
        assert_eq!(n, 1);
        let left: Vec<i64> = s
            .conn
            .prepare("SELECT meeting_id FROM meeting_line_source")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, vec![keep]);
    }

    #[test]
    fn a_split_and_a_merge_clear_the_rows_of_the_summaries_they_clear() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let m = meeting(&s, 1_000);
        s.begin_meeting_part(m, 1_000, None).unwrap();
        s.complete_meeting_part(m, 1, 61_000, None).unwrap();
        s.begin_meeting_part(m, 101_000, None).unwrap();
        s.complete_meeting_part(m, 2, 161_000, None).unwrap();
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "x", None)], 1).unwrap();
        let split = s.split_meeting(m, 2).unwrap();
        assert_eq!(count(&s, "meeting_line_source"), 0);
        // A merge clears the kept meeting's summary, and takes the other's rows with it.
        let into = meeting(&s, 500_000_000);
        let from = meeting(&s, 500_000_000 + 600_000 + 60_000);
        s.replace_meeting_line_sources(into, &[row(LineKind::Decision, "x", None)], 1).unwrap();
        s.replace_meeting_line_sources(from, &[row(LineKind::Decision, "y", None)], 1).unwrap();
        s.merge_into_previous(into, from).unwrap();
        assert_eq!(count(&s, "meeting_line_source"), 0);
        let _ = split;
    }

    #[test]
    fn a_duplicate_copy_hands_its_rows_to_the_kept_meeting_unless_it_has_its_own() {
        let s = Store::open_in_memory().unwrap();
        let (keep, drop) = (meeting(&s, 1_000), meeting(&s, 1_000));
        s.replace_meeting_line_sources(drop, &[row(LineKind::Decision, "from the copy", None)], 1).unwrap();
        assert!(s.fold_duplicate_meeting(keep, drop).unwrap());
        let lines: Vec<(i64, String)> = s
            .conn
            .prepare("SELECT meeting_id, line_text FROM meeting_line_source")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(lines, vec![(keep, "from the copy".to_string())]);
        let (keep2, drop2) = (meeting(&s, 7_000), meeting(&s, 7_000));
        s.replace_meeting_line_sources(keep2, &[row(LineKind::Decision, "mine", None)], 1).unwrap();
        s.replace_meeting_line_sources(drop2, &[row(LineKind::Decision, "theirs", None)], 1).unwrap();
        assert!(s.fold_duplicate_meeting(keep2, drop2).unwrap());
        assert_eq!(count(&s, "meeting_line_source"), 2, "the first fold's row and keep2's own");
    }

    #[test]
    fn a_trim_leaves_no_source_pointing_past_the_cut() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let m = meeting(&s, START);
        s.insert_transcript_segments(&[
            seg(m, 1_000, "Speaker 1", "We will ship on Friday, no later than noon."),
            seg(m, 3_000, "Speaker 2", "Hiring two engineers next quarter is the plan."),
        ])
        .unwrap();
        summarized(&s, m);
        s.replace_meeting_line_sources(
            m,
            &[
                row(LineKind::Decision, "Ship on Friday", Some(locator("We will ship on Friday, no later than noon.", 0, "We will ship on Friday, no later than noon."))),
                row(LineKind::Decision, "Hire two", Some(locator("We will ship on Friday, no later than noon.", 20, "Hiring two engineers next quarter is the plan."))),
            ],
            1,
        )
        .unwrap();
        assert!(s.trim_meeting(m, 20_000).unwrap().is_some());
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert!(n.decisions.iter().find(|d| d.text == "Ship on Friday").unwrap().source.is_some());
        assert!(n.decisions.iter().find(|d| d.text == "Hire two").unwrap().source.is_none());
        // The words of the removed line are gone from the table, not only hidden.
        assert_eq!(anchors(&s, m), vec!["we will ship on friday no later than noon".to_string()]);
    }

    #[test]
    fn none_of_it_enters_the_sync_outbox() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        let before = outbox(&s);
        s.replace_meeting_line_sources(m, &[row(LineKind::ActionItem, "Ana: x", Some(locator("hello there everyone", 0, "hello there everyone")))], 1).unwrap();
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "y", None)], 2).unwrap();
        assert_eq!(outbox(&s), before, "line sources are device-local");
        let queued: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sync_outbox WHERE payload_json LIKE '%meeting_line_source%' OR payload_json LIKE '%transcript_segment_end%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(queued, 0);
    }

    #[test]
    fn end_times_are_stored_with_new_lines_and_shift_with_a_part_offset() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let m = meeting(&s, 1_000);
        s.begin_meeting_part(m, 1_000, None).unwrap();
        s.complete_meeting_part(m, 1, 61_000, None).unwrap();
        s.begin_meeting_part(m, 101_000, None).unwrap(); // part 2 starts 100 s in
        s.complete_meeting_part(m, 2, 161_000, None).unwrap();
        s.replace_part_transcript_segments_with_ends(m, 1, &[seg(m, 0, "A", "first line here"), seg(m, 500, "B", "second line here")], &[Some(480), None])
            .unwrap();
        s.replace_part_transcript_segments_with_ends(m, 2, &[seg(m, 100, "A", "late line here")], &[Some(350)]).unwrap();
        let (lines, total) = s.get_transcript_lines_page(m, 0, 10).unwrap();
        assert_eq!(total, 3);
        let got: Vec<(i64, Option<i64>)> = lines.iter().map(|l| (l.start_cs, l.end_cs)).collect();
        // Part 2 begins 10_000 cs in: its line starts at 10_100 and runs until 10_350.
        assert_eq!(got, vec![(0, Some(480)), (500, None), (10_100, Some(10_350))]);
        // The same ids and order as the plain page.
        let (plain, _) = s.get_transcript_segments_page(m, 0, 10).unwrap();
        assert_eq!(plain.iter().map(|p| p.id).collect::<Vec<_>>(), lines.iter().map(|l| l.id).collect::<Vec<_>>());
        // Transcribing part 1 again replaces its lines and their ends; part 2's stay.
        s.replace_part_transcript_segments_with_ends(m, 1, &[seg(m, 0, "A", "redone")], &[Some(100)]).unwrap();
        let (lines, _) = s.get_transcript_lines_page(m, 0, 10).unwrap();
        assert_eq!(lines.iter().map(|l| (l.start_cs, l.end_cs)).collect::<Vec<_>>(), vec![(0, Some(100)), (10_100, Some(10_350))]);
        assert_eq!(count(&s, "transcript_segment_end"), 2, "a replaced line leaves no end behind");
        // Lines stored the old way have none.
        s.insert_transcript_segments(&[seg(m, 20_000, "A", "plain insert")]).unwrap();
        assert_eq!(s.get_transcript_lines_page(m, 0, 10).unwrap().0.last().unwrap().end_cs, None);
        assert_eq!(outbox_like(&s), 0);
    }

    fn outbox_like(s: &Store) -> i64 {
        s.conn.query_row("SELECT COUNT(*) FROM sync_outbox WHERE payload_json LIKE '%dur_cs%'", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn end_times_follow_their_line_through_delete_split_trim_and_merge() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        // Split: the line keeps its id and its duration, and starts earlier on the new clock.
        let m = meeting(&s, 8_000_000);
        s.begin_meeting_part(m, 8_000_000, None).unwrap();
        s.complete_meeting_part(m, 1, 8_060_000, None).unwrap();
        s.begin_meeting_part(m, 8_100_000, None).unwrap();
        s.complete_meeting_part(m, 2, 8_160_000, None).unwrap();
        s.replace_part_transcript_segments_with_ends(m, 1, &[seg(m, 0, "A", "early words here")], &[Some(200)]).unwrap();
        s.replace_part_transcript_segments_with_ends(m, 2, &[seg(m, 50, "A", "late words here")], &[Some(300)]).unwrap();
        let split = s.split_meeting(m, 2).unwrap();
        let (new_lines, _) = s.get_transcript_lines_page(split.new_meeting_id, 0, 10).unwrap();
        assert_eq!((new_lines[0].start_cs, new_lines[0].end_cs), (50, Some(300)));
        assert_eq!(s.get_transcript_lines_page(m, 0, 10).unwrap().0[0].end_cs, Some(200));
        // Delete: the new meeting's ends go with its lines.
        assert!(s.delete_meeting(split.new_meeting_id).unwrap());
        assert_eq!(count(&s, "transcript_segment_end"), 1);
        // Forget and the retention sweep remove their meetings' ends too.
        let f = meeting(&s, 9_000);
        s.insert_transcript_segments_with_ends(&[seg(f, 0, "A", "forget me please")], &[Some(10)]).unwrap();
        s.forget_meeting(f).unwrap();
        let old = meeting(&s, 2_000);
        s.insert_transcript_segments_with_ends(&[seg(old, 0, "A", "expire me please")], &[Some(10)]).unwrap();
        s.delete_meetings_before(5_000).unwrap();
        assert_eq!(count(&s, "transcript_segment_end"), 1);
        // Trim: a line past the cut goes with its end.
        let t = meeting(&s, 10_000_000);
        s.insert_transcript_segments_with_ends(&[seg(t, 1_000, "A", "kept line stays"), seg(t, 3_000, "A", "cut line goes")], &[Some(1_050), Some(3_060)]).unwrap();
        assert_eq!(count(&s, "transcript_segment_end"), 3);
        assert!(s.trim_meeting(t, 20_000).unwrap().is_some());
        assert_eq!(count(&s, "transcript_segment_end"), 2);
        // Merge: the absorbed meeting's lines are deleted (every part is transcribed again),
        // and their ends go with them.
        let into = meeting(&s, 500_000_000);
        let from = meeting(&s, 500_000_000 + 660_000);
        s.insert_transcript_segments_with_ends(&[seg(from, 10, "A", "merged line here")], &[Some(25)]).unwrap();
        let before = count(&s, "transcript_segment_end");
        s.merge_into_previous(into, from).unwrap();
        assert_eq!(count(&s, "transcript_segment_end"), before - 1);
    }

    #[test]
    fn opening_an_old_database_creates_both_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.db");
        let m = {
            let s = Store::open(&path, dir.path()).unwrap();
            let m = meeting(&s, START);
            s.conn.execute_batch("DROP TABLE meeting_line_source; DROP TABLE transcript_segment_end; DROP TRIGGER transcript_segment_end_ad;").unwrap();
            m
        };
        let s = Store::open(&path, dir.path()).unwrap();
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "back", None)], 1).unwrap();
        s.insert_transcript_segments_with_ends(&[seg(m, 0, "A", "again please")], &[Some(5)]).unwrap();
        assert_eq!(s.get_transcript_lines_page(m, 0, 5).unwrap().0[0].end_cs, Some(5));
    }

    /// A part-1 meeting with the recording starting at `START` and a scheduled start `skew_ms`
    /// earlier (a calendar meeting), as the Windows app stores it.
    fn calendar_meeting(s: &Store, skew_ms: i64) -> i64 {
        s.set_own_device_id("d");
        let m = meeting(s, START - skew_ms);
        s.begin_meeting_part(m, START, None).unwrap();
        s.complete_meeting_part(m, 1, START + 600_000, None).unwrap();
        m
    }

    #[test]
    fn a_meeting_row_that_starts_before_the_recording_cannot_pull_a_source_to_a_repeated_line() {
        // The row starts 90 s before the recording; the same short line is said again 90 s after the
        // first time. A wall clock read from the row's start would pick the second one.
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START - 90_000);
        s.insert_transcript_segments(&[
            seg(m, 0, "Speaker 1", "Okay, let us start the standup."),
            seg(m, 3_000, "Speaker 2", "Yes, that works for me."),
            seg(m, 12_000, "Speaker 2", "Yes, that works for me."),
        ])
        .unwrap();
        summarized(&s, m);
        let first = "Okay, let us start the standup.";
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "Ship on Friday", Some(locator(first, 30, "Yes, that works for me.")))], 1).unwrap();
        let src = s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.clone().unwrap();
        assert_eq!(src.start_cs, 3_000, "the true line, not the repeat 90 s later");
    }

    #[test]
    fn a_changed_first_line_moves_the_base_so_only_a_unique_long_line_still_resolves() {
        let s = Store::open_in_memory().unwrap();
        let m = calendar_meeting(&s, 0);
        s.insert_transcript_segments(&[
            seg(m, 0, "A", "Okay, let us start the standup."),
            seg(m, 6_000, "A", "Yes, that works for me."),
            seg(m, 9_000, "A", "We will ship the migration on Friday morning."),
        ])
        .unwrap();
        summarized(&s, m);
        let first = "Okay, let us start the standup.";
        s.replace_meeting_line_sources(
            m,
            &[
                row(LineKind::Decision, "Ship on Friday", Some(locator(first, 60, "Yes, that works for me."))),
                row(LineKind::Decision, "Hire two", Some(locator(first, 90, "We will ship the migration on Friday morning."))),
            ],
            1,
        )
        .unwrap();
        // Transcribed again: a different opening, the short line gone, and a different line where it
        // used to be 60 s in with the same words.
        s.replace_part_transcript_segments(
            m,
            1,
            &[
                seg(m, 0, "A", "Good morning, everyone, thanks for joining."),
                seg(m, 6_000, "A", "Yes, that works for me."),
                seg(m, 11_000, "A", "We will ship the migration on Friday morning."),
            ],
        )
        .unwrap();
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert!(n.decisions[0].source.is_none(), "the short line is not moved onto a repeat when the base changed");
        assert_eq!(n.decisions[1].source.clone().unwrap().start_cs, 11_000, "a long unique line is still found");
        // The short line's words did not stay behind.
        assert_eq!(anchors(&s, m), vec!["we will ship the migration on friday morning".to_string()]);
    }

    #[test]
    fn a_retranscribe_prunes_rows_that_no_longer_fit_and_keeps_the_rest() {
        let s = Store::open_in_memory().unwrap();
        let m = calendar_meeting(&s, 0);
        let first = "We will ship on Friday, no later than noon.";
        s.insert_transcript_segments(&[seg(m, 0, "A", first), seg(m, 2_000, "A", "Hiring two engineers next quarter is the plan.")]).unwrap();
        summarized(&s, m);
        s.replace_meeting_line_sources(
            m,
            &[
                row(LineKind::Decision, "Ship on Friday", Some(locator(first, 0, first))),
                row(LineKind::Decision, "Hire two", Some(locator(first, 20, "Hiring two engineers next quarter is the plan."))),
                row(LineKind::ActionItem, "Ana: send the deck (Monday)", None),
            ],
            1,
        )
        .unwrap();
        s.replace_part_transcript_segments(m, 1, &[seg(m, 0, "B", first), seg(m, 2_000, "B", "Something else about lunch was said here.")]).unwrap();
        assert_eq!(anchors(&s, m), vec!["we will ship on friday no later than noon".to_string()]);
        assert_eq!(count(&s, "meeting_line_source"), 2, "the row with no locator holds no words and stays");
        // The ends-aware replace prunes too.
        s.replace_part_transcript_segments_with_ends(m, 1, &[seg(m, 0, "B", "Nothing of the old note is here any more.")], &[Some(50)]).unwrap();
        assert!(anchors(&s, m).is_empty());
    }

    #[test]
    fn a_summary_written_before_its_rows_keeps_the_sources_the_new_rows_fit() {
        // The Windows order: the sources are stored while the meeting has no rows, then the rows are written.
        let s = Store::open_in_memory().unwrap();
        let m = calendar_meeting(&s, 0);
        summarized(&s, m);
        let first = "Okay, let us start the standup.";
        s.replace_meeting_line_sources(
            m,
            &[
                row(LineKind::Decision, "Ship on Friday", Some(locator(first, 6, "We will ship on Friday, no later than noon."))),
                row(LineKind::Decision, "Hire two", Some(locator(first, 8, "A line the final rows do not hold at all."))),
            ],
            1,
        )
        .unwrap();
        s.replace_part_transcript_segments(m, 1, &[seg(m, 0, "A", first), seg(m, 600, "B", "We will ship on Friday, no later than noon.")]).unwrap();
        assert_eq!(anchors(&s, m), vec!["we will ship on friday no later than noon".to_string()]);
        assert!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.is_some());
    }

    #[test]
    fn a_continued_meeting_counts_offsets_across_the_gap_between_parts() {
        // Part 2 starts 100 s after part 1; its lines carry start_cs counted from part 1, so the
        // offset from the first line is the difference of start_cs, gap included.
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let m = meeting(&s, START);
        s.begin_meeting_part(m, START, None).unwrap();
        s.complete_meeting_part(m, 1, START + 60_000, None).unwrap();
        s.begin_meeting_part(m, START + 100_000, None).unwrap();
        s.complete_meeting_part(m, 2, START + 160_000, None).unwrap();
        let first = "Part one opens with this line.";
        s.replace_part_transcript_segments(m, 1, &[seg(m, 0, "A", first), seg(m, 4_000, "A", "Yes, that works for me.")]).unwrap();
        s.replace_part_transcript_segments(m, 2, &[seg(m, 500, "A", "Yes, that works for me.")]).unwrap();
        summarized(&s, m);
        // The repeat in part 2 is 105 s after the first line (10_500 cs), the one in part 1 is 40 s after.
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "Ship on Friday", Some(locator(first, 105, "Yes, that works for me.")))], 1).unwrap();
        assert_eq!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.clone().unwrap().start_cs, 10_500);
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "Ship on Friday", Some(locator(first, 40, "Yes, that works for me.")))], 1).unwrap();
        assert_eq!(s.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.clone().unwrap().start_cs, 4_000);
    }

    #[test]
    fn a_trim_that_cuts_every_line_removes_every_anchor() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let m = meeting(&s, START);
        s.insert_transcript_segments(&[seg(m, 5_000, "A", "Only line here, said late in the meeting.")]).unwrap();
        summarized(&s, m);
        let said = "Only line here, said late in the meeting.";
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "Ship on Friday", Some(locator(said, 0, said)))], 1).unwrap();
        assert!(s.trim_meeting(m, 10_000).unwrap().is_some());
        assert!(anchors(&s, m).is_empty());
    }

    #[test]
    fn a_database_with_the_clock_based_table_is_rebuilt_without_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.db");
        let m = {
            let s = Store::open(&path, dir.path()).unwrap();
            let m = meeting(&s, START);
            s.conn.execute_batch("DROP TABLE meeting_line_source;").unwrap();
            s.conn
                .execute_batch(
                    "CREATE TABLE meeting_line_source (id INTEGER PRIMARY KEY, meeting_id INTEGER NOT NULL, kind TEXT NOT NULL, \
                     line_text TEXT NOT NULL, owner TEXT, due TEXT, clock_secs INTEGER, anchor TEXT, created_unix_ms INTEGER NOT NULL);
                     INSERT INTO meeting_line_source (meeting_id, kind, line_text, clock_secs, anchor, created_unix_ms) \
                     VALUES (1, 'decision', 'old', 36000, 'old words', 1);",
                )
                .unwrap();
            m
        };
        let s = Store::open(&path, dir.path()).unwrap();
        assert_eq!(count(&s, "meeting_line_source"), 0, "the old local-only rows are dropped");
        s.replace_meeting_line_sources(m, &[row(LineKind::Decision, "new", Some(locator("hello there", 0, "hello there")))], 1).unwrap();
        assert_eq!(anchors(&s, m), vec!["hello there".to_string()]);
        // Opening it again keeps the new shape and its rows.
        drop(s);
        let s = Store::open(&path, dir.path()).unwrap();
        assert_eq!(count(&s, "meeting_line_source"), 1);
    }

    #[test]
    fn every_stored_row_can_be_listed_for_the_export() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        s.replace_meeting_line_sources(
            m,
            &[row(LineKind::Decision, "Gone from the note", Some(locator("hello there", 12, "hello there friend"))), row(LineKind::ActionItem, "Ana: x", None)],
            1,
        )
        .unwrap();
        let rows = s.meeting_line_source_rows(m).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].line_text.as_str(), rows[0].anchor.as_deref(), rows[0].offset_secs), ("Gone from the note", Some("hello there friend"), Some(12)));
        assert_eq!(rows[1].anchor, None);
    }

    #[test]
    fn many_lines_load_the_rows_once_and_a_big_meeting_resolves_fast() {
        // The note's lines: one load of the meeting's rows, whatever their number.
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        let lines: Vec<NewTranscriptSegment> = (0..300).map(|i| seg(m, i * 100, "A", &format!("Line number {i} has some words in it."))).collect();
        s.insert_transcript_segments(&lines).unwrap();
        let decisions: Vec<String> = (0..60).map(|i| format!("Decision {i}")).collect();
        s.update_meeting_summary(m, "t", &decisions, &[], &[], "p", "mod").unwrap();
        let rows: Vec<NewLineSource> =
            (0..60).map(|i| row(LineKind::Decision, &format!("Decision {i}"), Some(locator("Line number 0 has some words in it.", i * 5, &format!("Line number {} has some words in it.", i * 5))))).collect();
        s.replace_meeting_line_sources(m, &rows, 1).unwrap();
        let before = ROW_LOADS.with(|c| c.get());
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert_eq!(ROW_LOADS.with(|c| c.get()) - before, 1);
        assert!(n.decisions.iter().all(|d| d.source.is_some()));

        // A synthetic 30,000 line meeting and 60 references: resolved through the index, not by
        // scanning and normalizing every row for each.
        let raw: Vec<(i64, i64, String, String)> =
            (0..30_000).map(|i| (i, i * 100, "A".to_string(), format!("Spoken line number {i} about the plan."))).collect();
        let index = LineSourceIndex::build(raw);
        let base = base_fingerprint("Spoken line number 0 about the plan.");
        let started = std::time::Instant::now();
        for k in 0..60 {
            let at = 400 + k * 450;
            let loc = SourceLocator { offset_secs: at, base, anchor: anchor_of(&format!("Spoken line number {at} about the plan.")) };
            assert_eq!(index.resolve(&loc).map(|r| r.start_cs), Some(at * 100));
        }
        // Lost lines take the whole-meeting path, which stops at the second hit.
        for k in 0..60 {
            let loc = SourceLocator { offset_secs: 9_000_000, base: base ^ 1, anchor: anchor_of(&format!("Spoken line number {} about the plan.", 100 + k)) };
            assert!(index.resolve(&loc).is_some());
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[test]
    fn an_end_that_is_not_after_the_start_or_overflows_is_dropped_and_the_line_kept() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let m = meeting(&s, 1_000);
        s.begin_meeting_part(m, 1_000, None).unwrap();
        s.complete_meeting_part(m, 1, 61_000, None).unwrap();
        s.begin_meeting_part(m, 101_000, None).unwrap();
        s.complete_meeting_part(m, 2, 161_000, None).unwrap();
        // Part 2 starts 10_000 cs in: an end near i64::MAX would overflow once shifted.
        s.replace_part_transcript_segments_with_ends(
            m,
            2,
            &[seg(m, 0, "A", "one"), seg(m, 100, "A", "two"), seg(m, 200, "A", "three"), seg(m, 300, "A", "four")],
            &[Some(i64::MAX), Some(100), Some(250), Some(400)],
        )
        .unwrap();
        let (lines, total) = s.get_transcript_lines_page(m, 0, 10).unwrap();
        assert_eq!(total, 4, "every line is kept");
        let ends: Vec<Option<i64>> = lines.iter().map(|l| l.end_cs).collect();
        // overflow: none; end == start: none; a real end: kept; another real end: kept.
        assert_eq!(ends, vec![None, None, Some(10_250), Some(10_400)]);
        // A plain insert with absurd ends: an end before the start, equal to it, a day and a bit past it,
        // and one whose subtraction would overflow.
        s.insert_transcript_segments_with_ends(
            &[seg(m, 20_000, "A", "five"), seg(m, 20_100, "A", "six"), seg(m, 20_200, "A", "seven"), seg(m, -5, "A", "eight")],
            &[Some(19_000), Some(20_100), Some(20_200 + 24 * 3600 * 100 + 1), Some(i64::MAX)],
        )
        .unwrap();
        let (lines, _) = s.get_transcript_lines_page(m, 0, 20).unwrap();
        let by_text = |t: &str| lines.iter().find(|l| l.text == t).unwrap().end_cs;
        assert_eq!((by_text("five"), by_text("six"), by_text("seven"), by_text("eight")), (None, None, None, None));
        // A day exactly is the longest accepted.
        s.insert_transcript_segments_with_ends(&[seg(m, 30_000, "A", "nine")], &[Some(30_000 + 24 * 3600 * 100)]).unwrap();
        let (lines, _) = s.get_transcript_lines_page(m, 0, 20).unwrap();
        assert_eq!(lines.iter().find(|l| l.text == "nine").unwrap().end_cs, Some(30_000 + 24 * 3600 * 100));
    }

    #[test]
    fn a_prune_keeps_an_action_items_owner_and_due_but_not_its_transcript_words() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, START);
        s.set_own_device_id("d");
        s.begin_meeting_part(m, START, None).unwrap();
        s.complete_meeting_part(m, 1, START + 60_000, None).unwrap();
        let first = "Ana, can you send the deck by Monday?";
        s.replace_part_transcript_segments(m, 1, &[seg(m, 0, "A", first)]).unwrap();
        summarized(&s, m);
        let mut r = row(LineKind::ActionItem, "Ana: send the deck (Monday)", Some(locator(first, 0, first)));
        r.owner = Some("Ana".into());
        r.due = Some("Monday".into());
        s.replace_meeting_line_sources(m, &[r], 1).unwrap();
        s.replace_part_transcript_segments(m, 1, &[seg(m, 0, "A", "Nothing about decks is said in this pass.")]).unwrap();
        assert!(anchors(&s, m).is_empty(), "the words are gone");
        let held = s.meeting_line_source_rows(m).unwrap();
        assert_eq!((held.len(), held[0].owner.as_deref(), held[0].due.as_deref(), held[0].anchor.clone()), (1, Some("Ana"), Some("Monday"), None));
        let n = s.meeting_note_sources(m).unwrap().unwrap();
        assert_eq!((n.action_items[0].owner.as_deref(), n.action_items[0].due.as_deref(), n.action_items[0].source.is_none()), (Some("Ana"), Some("Monday"), true));
    }
}
