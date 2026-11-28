//! Find: one search over what this device remembers, with filters that really filter (screen 5,
//! memory search and Ask). Read-only; no schema change.
//!
//! # Filters ([`FindFilter`])
//!
//! | Filter | Captures | Meetings | Transcript lines | Slides | Email threads | Episodes | Runs |
//! |---|---|---|---|---|---|---|---|
//! | Dates `[start, end)` | capture time | overlaps the range when its end is known, else its start | the line's time (meeting start plus its offset) | meeting start | last activity | when it ended | when it ran |
//! | App | by app | hidden | hidden | hidden | hidden | hidden | hidden |
//! | Person | listed apart as *mentions* | attended by | spoken by, or in a meeting they attended | in a meeting they attended | from or to | hidden | hidden |
//!
//! [`FindFilter::allows`] is that table, and it is the one place the rule lives: both apps and Ask read it.
//!
//! The person rule is whole-string, trimmed and case-insensitive, folded in Rust (full Unicode
//! lowercase, never SQLite's ASCII-only `lower()`): a meeting is attended by "Sam Okafor" when one of
//! its attendees, trimmed, equals that name (the Windows rule, over every meeting rather than the
//! newest 30 or 50). Attendance does not depend on the date filter: a transcript line of an attended
//! meeting counts when the LINE's own time is inside the range, however early the meeting began.
//! A line is spoken by the person when the meeting names its speaker (`meeting_speaker`, `person`)
//! with a person row of that name (folded) or that person key. Mentions are captures whose text or
//! title holds the name as a phrase, not an OR of its words. Email: by the address on record for the
//! person (`person.email`, or a person key that is an address) EQUAL, ignoring case, to the parsed
//! address of ANY actor (sender or any recipient); with none on record, by a name of two or more
//! tokens as a run of whole name tokens inside the display name of any actor ("Ed Smith" matches
//! "Ed Smith" and, because a display name with exactly one comma also reads as First Last, "Smith, Ed";
//! never "Edward Smith" or "Fred Smith"; tokens are the runs of letters and digits, so "O'Brien" is
//! two). A name of ONE token ("Ed", "Will", "May") matches email by address only: with none on record it
//! matches no email, because a lone first name inside a display name is a guess. No address and no name
//! match is no email results, not a guess.
//!
//! # Cost
//!
//! What a person filter resolves to ([`ScopeCtx`]) is built once per [`Store::find`] call and once per
//! Ask, and the reads it needs are narrowed in SQL before any row is parsed (a `LIKE` on the address, the
//! longest name token or the name, plus every row holding a non-ASCII character, which `LIKE` cannot fold;
//! the Rust rule above still decides). The ids of the meetings and email items it selects live in a
//! per-connection TEMP table the queries join, never in a bound JSON list, and are removed when the
//! context drops.
//!
//! # Fail closed ([`FindFilter::validated`])
//!
//! A filter is never silently widened. [`FindFilter::from_json`] rejects an unknown key (a misspelt
//! or camelCase `startUnixMs` would otherwise parse to an empty filter and search everything), a
//! present but blank app or person name, and a range whose start is not before its end.
//! [`Store::find`] and every `scoped_*` read return an error for such a filter instead of searching.
//!
//! # Paging ([`Store::find`])
//!
//! A query's candidates are the best [`FIND_POOL`] rows of each kind for keyword search (on its words less the common ones,
//! [`keyword_words`]) and for meaning search (on the whole query) (meaning search reads up to 4,096 nearest neighbours of everything and filters
//! after; when that leaves a narrow filter short, the rows the filter selects are scored exactly
//! instead, up to [`EXACT_SEMANTIC_MAX_ROWS`] of them; past that the nearest-neighbour read stands). A meaning hit is kept only when it is
//! at least as like the query as [`meaning_floor_for`] says, so words that mean nothing near the library find nothing. Keyword and meaning hits are de-duplicated by
//! `(kind, id)`. Order `Time` sorts the candidates by time, newest first; order `Rank` scores each
//! candidate by its cosine to the query plus [`KEYWORD_WEIGHT`] times the share of the query's words it holds, and sorts by that
//! score. Ties break by the best keyword rank, then time, kind,
//! id, so the order is total and deterministic for a store that has not changed, and `offset` is a
//! stable cursor. `total` and the per-kind `counts` are of the candidates; `capped` says a list hit
//! the pool (counted from the rows the query read, before any de-duplication), so more may exist
//! beyond it. `offset` is clamped to [`FIND_MAX_OFFSET`] and `limit` to [`FIND_MAX_LIMIT`]. An empty
//! query with a filter lists by time and is not pool-limited: captures in the range (or, with a person, the meetings that person attended that
//! overlap the range, and the mentions), counted exactly. An empty query with no filter finds nothing.

use std::cell::{Cell, OnceCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params_from_iter, OptionalExtension};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

use crate::cards::SourceCard;
use crate::{embedding_to_json, fts5_all_words, sanitize_fts5_query, CaptureItem, Episode, MeetingSnapshotHit, MeetingTranscriptSegment, RoutineRunWithName, Store, Thread, ITEMS_VEC_MAX_K};

/// The error a find or scoped read returns, named so a crate without its own SQLite dependency can carry it.
pub type ReadError = rusqlite::Error;

/// How many of the best rows each kind and method contributes to a query's candidates.
pub const FIND_POOL: usize = 400;
/// The largest page [`Store::find`] returns.
pub const FIND_MAX_LIMIT: usize = 200;
/// The largest `offset` [`Store::find`] honours; a bigger one is this one (an empty page in practice).
pub const FIND_MAX_OFFSET: usize = 1_000_000;
/// Meaning search under a filter scores exactly, in Rust, when the filter selects at most this many rows.
/// Each row is one 384-float embedding read and compared on the caller's thread, so the cap is what keeps a
/// narrow-filter meaning search fast. In release on a laptop one leg takes about 34 ms at 1,000 rows, 56 ms at
/// 4,000, 83 ms at 8,000 and 170 ms at 20,000 (`measure_exact_meaning_search_by_rows_selected`, which prints them),
/// about 30 ms of each being the nearest-neighbour read the leg makes anyway.
pub const EXACT_SEMANTIC_MAX_ROWS: usize = 4_000;

/// The least cosine similarity a meaning-search hit needs to be a Find result when the query has two or more words that are not
/// common words. Below it the row is only the nearest of nothing in particular: without a floor any words at all
/// (`zebra quantum harmonica`) listed the whole library. Keyword hits are not affected, and only Find applies it; Ask's retrieval is
/// unchanged. Tuned on the labelled set in `find_eval` (`find_eval_floor_at_library_size` prints the numbers): odd two- and
/// three-word queries reach 0.33 against 3,000 chunks of this repository's docs ("xyzzy plover" 0.329, "zebra quantum harmonica"
/// 0.321, "asdf qwerty" 0.328), while real questions come to 0.38 and up against the row they are about. It used to be 0.28 when a
/// word of the query matched something by keyword, 0.4 when none did; with common words no longer keywords that split let one shared
/// word ("for") pull in every row a little like the question, and the one floor finds more (see the PR's table).
pub const MEANING_MIN_SIMILARITY: f32 = 0.36;
/// The floor when the query is a single word that is not a common word. One abstract word sits further from any whole sentence than
/// a question does: on the labelled set "transportation" comes to 0.399 against a shuttle line, "money" to 0.339, "weather" to
/// 0.336 and "fundraising" to 0.327. Odd single words come as close at library size ("quantum" 0.339, "zebra" 0.322, "xyzzy"
/// 0.392 against the docs), so no floor on this model separates them: this one keeps "quantum" (by 0.001) and "zebra" out and misses
/// "money", "weather", "fundraising" and the screen-reader lines for "accessibility" (0.117 to 0.273, under a parking line at 0.322).
/// 0.33 would bring back "money" and "weather" here and let "quantum" in at library size. A floor relative to the query's own scores
/// does not separate them either: at library size the best row of a nonsense word stands 5 to 8 standard deviations above the mean,
/// as the right row of a real one does (5 to 6).
pub const MEANING_SHORT_MIN_SIMILARITY: f32 = 0.34;
/// The floor when every word of the query is a common word ("the", "for", "Where is it"): it has no topic, so only a row that is
/// nearly the same words counts. Against the docs "the" comes to 0.68 and "for" to 0.60.
pub const MEANING_COMMON_ONLY_MIN_SIMILARITY: f32 = 0.7;

/// The meaning floor for `query`, by how many of its words are not common words ([`keyword_words`]).
pub fn meaning_floor_for(query: &str) -> f32 {
    let class = match keyword_words(query).len() {
        0 => 0,
        1 => 1,
        _ => 2,
    };
    #[cfg(test)]
    if let Some(floors) = MEANING_FLOORS_FOR_EVAL.with(|f| f.get()) {
        return floors[class];
    }
    [MEANING_COMMON_ONLY_MIN_SIMILARITY, MEANING_SHORT_MIN_SIMILARITY, MEANING_MIN_SIMILARITY][class]
}

#[cfg(test)]
thread_local! {
    /// Test builds only: the model evaluation in `find_eval` runs Find with floors fitted to another model (common words only, one
    /// word, two or more words). The floors above are tuned to the shipped model's cosines and mean nothing for another model.
    pub(crate) static MEANING_FLOORS_FOR_EVAL: std::cell::Cell<Option<[f32; 3]>> = const { std::cell::Cell::new(None) };
}

/// Common words of every language the app ships (English, Spanish, French, German and Portuguese), left out of Find's keyword
/// matching: as keywords they match nearly every row, so "accessibility for blind users" listed every line holding "for" and
/// "Where is the kite festival?" everything with "the" or "is". The meaning search still reads the whole query. Kept short on purpose:
/// a word that is a common word in one language and a real word in another stays a keyword ("car", "war", "die", "son", "sin",
/// "man", "pour", "mit", "con", "will", "may", "us", "it", "also").
const STOP_WORDS: &[&str] = &[
    // English
    "a", "an", "the", "and", "or", "but", "of", "to", "in", "on", "at", "by", "for", "from", "with", "about", "into", "as", "is",
    "are", "was", "were", "be", "been", "am", "do", "does", "did", "has", "have", "had", "i", "me", "my", "we", "our", "you", "your",
    "he", "she", "his", "her", "they", "them", "their", "its", "this", "that", "these", "those", "there", "what", "where", "when",
    "who", "whom", "which", "why", "how", "much", "many", "any", "some", "not", "no", "so", "if", "than", "then",
    // Spanish
    "el", "la", "los", "las", "un", "una", "unos", "unas", "de", "del", "al", "y", "o", "en", "por", "para", "que", "qué", "es",
    "está", "se", "lo", "como", "cómo", "donde", "dónde", "cuál", "cual", "cuando", "cuándo", "mi", "su", "sus", "nos", "le", "les",
    // French
    "une", "des", "du", "et", "ou", "où", "au", "aux", "par", "avec", "est", "sont", "qui", "quoi", "quel", "quelle", "comment", "ce",
    "cette", "il", "elle", "nous", "vous", "dans", "sur",
    // German
    "der", "das", "den", "dem", "ein", "eine", "einen", "einem", "einer", "und", "oder", "ist", "sind", "wo", "was", "wer", "wie",
    "wann", "warum", "welche", "zu", "zum", "zur", "im", "von", "für", "auf", "am", "ich", "wir", "sie", "es",
    // Portuguese
    "os", "as", "um", "uma", "do", "da", "dos", "das", "e", "em", "no", "na", "nas", "onde", "quando", "qual", "é", "são", "ao",
];

/// Words that are common in English but also a word in their own right ("IT", "Will", "May", "US"): left out of the keyword
/// match only when the query has another word to match on, so "is it going to rain" does not list every row holding "it" while
/// "IT" alone still searches.
const SOFT_STOP_WORDS: &[&str] = &["it", "will", "may", "also", "us", "can"];

/// The words of `query` that Find's keyword search matches on: split where the search index splits a word (at anything but a
/// letter or a digit, so "l'école" gives "école"), folded to lower case, without the common words of [`STOP_WORDS`] and single
/// letters, each once, in order. Empty for a query made only of common words, which then finds by meaning alone.
pub fn keyword_words(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in query.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let w = w.to_lowercase();
        let single_letter = w.chars().count() == 1 && !w.chars().all(|c| c.is_numeric());
        if single_letter || STOP_WORDS.contains(&w.as_str()) || out.contains(&w) {
            continue;
        }
        out.push(w);
    }
    if out.iter().any(|w| !SOFT_STOP_WORDS.contains(&w.as_str())) {
        out.retain(|w| !SOFT_STOP_WORDS.contains(&w.as_str()));
    }
    out
}

/// How much the share of the query's words a row holds counts against its cosine to the query, in Find's rank order
/// ([`Store::rank`]). Tuned on the labelled set in `find_eval`.
pub const KEYWORD_WEIGHT: f64 = 0.5;

/// A row count for a SQL `LIMIT`: never negative (SQLite reads -1 as unlimited) and never absurd.
fn sql_limit(n: usize) -> i64 {
    n.min(FIND_MAX_OFFSET + FIND_MAX_LIMIT) as i64
}

/// Why a filter was refused. A refused filter is an error, never a wider search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterError {
    /// The JSON does not parse as a filter: not JSON, a wrong type, or a key this filter does not have.
    Malformed(String),
    /// An app is present but blank.
    BlankApp,
    /// A person is present but its name is blank, or a person key came with no person.
    BlankPerson,
    /// The range is empty: start is not before end.
    EmptyRange,
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterError::Malformed(why) => write!(f, "the filter is not understood: {why}"),
            FilterError::BlankApp => f.write_str("the app in the filter is blank"),
            FilterError::BlankPerson => f.write_str("the person in the filter has no name"),
            FilterError::EmptyRange => f.write_str("the date range in the filter is empty"),
        }
    }
}

impl std::error::Error for FilterError {}

fn invalid(e: FilterError) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(e.to_string())
}

/// Lowercase, trimmed: the one folding every name comparison uses (full Unicode, done in Rust).
fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The runs of letters and digits of `s`, folded. "Jean-Luc O'Brien" is `jean luc o brien`.
fn name_tokens(s: &str) -> Vec<String> {
    fold(s).split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// An email actor as `Display Name <address>`, a bare address, or a bare name: `(address, display name)`.
fn parse_actor(actor: &str) -> (Option<String>, String) {
    let a = actor.trim();
    if let (Some(l), Some(r)) = (a.rfind('<'), a.rfind('>')) {
        if l < r {
            let addr = fold(&a[l + 1..r]);
            return ((!addr.is_empty()).then_some(addr), a[..l].trim().trim_matches('"').trim().to_string());
        }
    }
    if a.contains('@') && !a.chars().any(char::is_whitespace) {
        return (Some(fold(a)), String::new());
    }
    (None, a.to_string())
}

fn contains_run(hay: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Whether an email actor's display name is the person `tokens` name, for a person with no address on
/// record. A name of one token matches nothing (a lone first name is a guess: see the module notes). A
/// longer name matches as a run of whole tokens, and a display name with exactly one comma also reads as
/// First Last: "Smith, Ed" is "Ed Smith".
fn display_is_person(display: &str, tokens: &[String]) -> bool {
    if tokens.len() < 2 {
        return false;
    }
    if contains_run(&name_tokens(display), tokens) {
        return true;
    }
    match display.split_once(',') {
        Some((last, first)) if !first.contains(',') => {
            let mut swapped = name_tokens(first);
            swapped.extend(name_tokens(last));
            contains_run(&swapped, tokens)
        }
        _ => false,
    }
}

/// Characters `LIKE` can match the way the Rust rule does: ASCII letters and digits and a few marks that
/// JSON writes as themselves. A needle of anything else is not pre-filtered in SQL.
fn like_safe(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || " .-_'@+,()".contains(c))
}

/// `GLOB` pattern for a text holding any non-ASCII character: `LIKE` folds ASCII only, so a row with one
/// cannot be ruled out by it and always goes to the Rust rule.
const NON_ASCII_GLOB: &str = "*[^\u{1}-\u{7f}]*";

/// A SQL condition on `col` that holds for every row the Rust rule could accept for a needle that must
/// appear in the folded text (`needle` is folded here), with its binds. `None` when no safe pre-filter
/// exists (scan everything).
fn prefilter(col: &str, needle: &str, first_bind: usize) -> Option<(String, Vec<Value>)> {
    let needle = fold(needle);
    if !like_safe(&needle) {
        return None;
    }
    let sql = format!("({col} LIKE ?{first_bind} ESCAPE '\\' OR {col} GLOB ?{})", first_bind + 1);
    Some((sql, vec![Value::Text(format!("%{}%", like_escape(&needle))), Value::Text(NON_ASCII_GLOB.to_string())]))
}

/// A meeting that ran from `start` to `end` (when known) is in `[from, to)`: it overlaps the range
/// when its end is known, else its start decides.
fn meeting_in_range(start: i64, end: Option<i64>, from: Option<i64>, to: Option<i64>) -> bool {
    let end = end.filter(|e| *e > start);
    let before_to = to.is_none_or(|t| start < t);
    let after_from = from.is_none_or(|f| match end {
        Some(e) => e > f,
        None => start >= f,
    });
    before_to && after_from
}

/// A person, by the name the apps show and, when known, the key of their `person` row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonFilter {
    pub name: String,
    #[serde(default)]
    pub person_key: Option<String>,
}

/// Date, app and person bounds. Every field is optional. Unknown keys are refused when parsing, and a
/// present but blank app or person is refused by [`FindFilter::validated`]: see "Fail closed" above.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FindFilter {
    /// Inclusive.
    pub start_unix_ms: Option<i64>,
    /// Exclusive.
    pub end_unix_ms: Option<i64>,
    pub app_bundle_id: Option<String>,
    pub person: Option<PersonFilter>,
}

/// An added item as Ask and Find read it: its provenance, title, text and link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddedItemHit {
    pub id: i64,
    pub ts_unix_ms: i64,
    /// What the item said it was (`email`, `calendar_event`, `chat_message`, `document`, `note`).
    pub declared_kind: String,
    /// The client that added it, by its token's name when it wrote.
    pub client: String,
    /// The source label, without the `mcp:<token id>:` prefix.
    pub source_label: String,
    pub title: Option<String>,
    pub text: String,
    pub url: Option<String>,
}

const ADDED_HIT_SELECT: &str = "SELECT id, ts_unix_ms, coalesce(source, ''), title, body_text, url, coalesce(refs, '{}') FROM items WHERE kind = 'mcp_item'";

fn added_hit_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AddedItemHit> {
    let source: String = r.get(2)?;
    let refs: serde_json::Value = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
    Ok(AddedItemHit {
        id: r.get(0)?,
        ts_unix_ms: r.get(1)?,
        declared_kind: refs["as"].as_str().unwrap_or_default().to_string(),
        client: refs["client"].as_str().unwrap_or_default().to_string(),
        source_label: crate::mcp_items::label_of_source(&source).unwrap_or_default().to_string(),
        title: r.get(3)?,
        text: r.get(4)?,
        url: r.get(5)?,
    })
}

/// A kind of result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FindKind {
    Capture,
    Meeting,
    Transcript,
    Snapshot,
    Thread,
    Episode,
    RoutineRun,
    /// An item a local MCP client added (`items` kind `mcp_item`).
    McpItem,
}

impl FindKind {
    /// The tag cards and saved sources use.
    pub fn as_str(self) -> &'static str {
        match self {
            FindKind::Capture => "capture",
            FindKind::Meeting => "meeting",
            FindKind::Transcript => "transcript",
            FindKind::Snapshot => "snapshot",
            FindKind::Thread => "thread",
            FindKind::Episode => "episode",
            FindKind::RoutineRun => "routine_run",
            FindKind::McpItem => "mcp_item",
        }
    }
}

impl FindFilter {
    /// Parses a filter and refuses what could widen a search: see [`FindFilter::validated`] and
    /// [`FilterError`]. Blank or `null` input is no filter.
    pub fn from_json(raw: &str) -> Result<Self, FilterError> {
        if raw.trim().is_empty() {
            return Ok(FindFilter::default());
        }
        // An object (or null) only: serde would also read a JSON array as the struct's fields in order, and
        // `[]` as every field absent, which is no bound at all.
        let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| FilterError::Malformed(e.to_string()))?;
        match value {
            serde_json::Value::Null => Ok(FindFilter::default()),
            serde_json::Value::Object(_) => serde_json::from_value::<FindFilter>(value).map_err(|e| FilterError::Malformed(e.to_string()))?.validated(),
            _ => Err(FilterError::Malformed("a filter is a JSON object".into())),
        }
    }

    /// The filter normalized (strings trimmed, a blank person key dropped), or the reason it is
    /// refused: a present but blank app or person name, or a range whose start is not before its end.
    pub fn validated(self) -> Result<Self, FilterError> {
        if self.app_bundle_id.as_ref().is_some_and(|a| a.trim().is_empty()) {
            return Err(FilterError::BlankApp);
        }
        if self.person.as_ref().is_some_and(|p| p.name.trim().is_empty()) {
            return Err(FilterError::BlankPerson);
        }
        if let (Some(s), Some(e)) = (self.start_unix_ms, self.end_unix_ms) {
            if s >= e {
                return Err(FilterError::EmptyRange);
            }
        }
        Ok(self.normalized())
    }

    /// The filter with blank app and person dropped and the person's name trimmed. This quietly
    /// widens; the boundaries call [`FindFilter::validated`] first.
    pub fn normalized(mut self) -> Self {
        self.app_bundle_id = self.app_bundle_id.map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
        self.person = self.person.and_then(|p| {
            let name = p.name.trim().to_string();
            (!name.is_empty()).then_some(PersonFilter { name, person_key: p.person_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) })
        });
        self
    }

    /// No bound at all.
    pub fn is_empty(&self) -> bool {
        let n = self.clone().normalized();
        n.start_unix_ms.is_none() && n.end_unix_ms.is_none() && n.app_bundle_id.is_none() && n.person.is_none()
    }

    /// Whether `kind` can appear at all under this filter (the table in the module notes). Captures
    /// always can: under a person they are the mentions. An app hides every kind but captures; a
    /// person hides episodes and runs.
    pub fn allows(&self, kind: FindKind) -> bool {
        let f = self.clone().normalized();
        let (app, person) = (f.app_bundle_id.is_some(), f.person.is_some());
        match kind {
            FindKind::Capture => true,
            FindKind::Meeting | FindKind::Transcript | FindKind::Snapshot | FindKind::Thread => !app,
            FindKind::Episode => !app && !person,
            FindKind::RoutineRun => !app && !person,
            // No app, and no person in v1: an added item's people are not matched to a person yet.
            FindKind::McpItem => !app && !person,
        }
    }

    /// This filter with `[start, end)` as its dates when it has none. A date filter wins over a window
    /// (the question's own "today" or "yesterday"): when either side is set, the window is ignored.
    pub fn or_window(&self, start: i64, end: i64) -> Self {
        let mut f = self.clone();
        if f.start_unix_ms.is_none() && f.end_unix_ms.is_none() {
            f.start_unix_ms = Some(start);
            f.end_unix_ms = Some(end);
        }
        f
    }
}

/// How [`Store::find`] orders its page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindOrder {
    /// Newest first.
    Time,
    /// Best match first.
    Rank,
}

/// One page of results.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FindPage {
    pub results: Vec<SourceCard>,
    /// Captures that mention the filtered person, listed apart (empty without a person). Paged with
    /// the same `offset` and `limit`, newest first.
    pub mentions: Vec<SourceCard>,
    /// How many candidates there are, across pages.
    pub total: usize,
    pub mentions_total: usize,
    /// Candidates per kind (`capture`, `transcript`, `snapshot`, `thread`, `episode`, `routine_run`,
    /// `meeting`), and `mentions`.
    pub counts: BTreeMap<String, usize>,
    /// The search model is installed and was used for this query.
    pub semantic_used: bool,
    /// The search model could be loaded (always false for an empty query).
    pub semantic_available: bool,
    /// More results follow `offset + results.len()`.
    pub more: bool,
    /// A list hit [`FIND_POOL`]: more may exist beyond the candidates.
    pub capped: bool,
    /// Meaning search was used while the embedding sweep still has rows to embed (items a local Claude added, synced rows, rows
    /// written before the model was there): the same words can find more once it is done. The page searches again then and
    /// replaces what it showed, so a rerun of the same words never shows an older, shorter answer.
    pub meaning_catching_up: bool,
}

/// A candidate: its kind, id, time, whether a transcript line matched a person as its speaker, and
/// an auxiliary id (a transcript line or slide's meeting, a thread's best-matching message).
#[derive(Debug, Clone, Copy)]
struct Key {
    kind: FindKind,
    id: i64,
    ts: i64,
    speaker: bool,
    aux: i64,
}

/// Numbered bind values, placeholders starting at `?base`.
struct Binds {
    base: usize,
    vals: Vec<Value>,
}

impl Binds {
    fn new(base: usize) -> Self {
        Binds { base, vals: Vec::new() }
    }
    fn add(&mut self, v: Value) -> String {
        self.vals.push(v);
        format!("?{}", self.base + self.vals.len() - 1)
    }
}

/// What a person filter resolves to in this store. Built by [`Store::scope_ctx`] or inside
/// [`Store::find`], and dropped with them: it holds this connection's TEMP rows for its ids.
struct PersonCtx<'s> {
    store: &'s Store,
    /// This context's rows in the TEMP table.
    id: u64,
    /// The name's tokens.
    tokens: Vec<String>,
    /// The address on record, lowercase.
    email: Option<String>,
    /// Every `person_key` this name or key stands for, for the speaker rule.
    speaker_keys: Vec<String>,
    /// `(meeting id, start, end)` of every meeting they attended, newest first, whatever the filter's dates.
    attended: Vec<(i64, i64, Option<i64>)>,
    /// Ids of the email items sent by or to them, found on first use.
    mail: OnceCell<Vec<i64>>,
    /// The attended meetings' ids are in the TEMP table.
    attended_staged: Cell<bool>,
    /// The mail ids are in the TEMP table.
    mail_staged: Cell<bool>,
}

impl PersonCtx<'_> {
    /// `(meeting id, start)` of the attended meetings that are inside `f`'s dates (overlap, or start when
    /// no end is known), newest first.
    fn in_range(&self, f: &FindFilter) -> Vec<(i64, i64)> {
        self.attended.iter().filter(|(_, s, e)| meeting_in_range(*s, *e, f.start_unix_ms, f.end_unix_ms)).map(|(id, start, _)| (*id, *start)).collect()
    }
}

impl Drop for PersonCtx<'_> {
    fn drop(&mut self) {
        if self.attended_staged.get() || self.mail_staged.get() {
            // Best effort: the rows are this context's alone, and a connection that cannot run this is going away.
            let _ = self.store.conn.execute("DELETE FROM temp.find_scope_ids WHERE ctx = ?1", [self.id as i64]);
        }
    }
}

/// A filter checked once, with what its person resolves to, for a run of reads that share them: one
/// [`Store::find`] call is built inside, one Ask builds one here and passes it to every source and to both
/// of its contexts. Read-only data, bound to the connection that built it. See "Cost" in the module notes.
pub struct ScopeCtx<'s> {
    filter: FindFilter,
    person: Option<Rc<PersonCtx<'s>>>,
}

impl<'s> ScopeCtx<'s> {
    /// The validated, normalized filter.
    pub fn filter(&self) -> &FindFilter {
        &self.filter
    }

    /// No bound at all.
    pub fn is_empty(&self) -> bool {
        self.filter.is_empty()
    }

    /// This context with `[start, end)` as the dates when the filter has none (`FindFilter::or_window`); the
    /// person is shared, not resolved again.
    pub fn or_window(&self, start: i64, end: i64) -> ScopeCtx<'s> {
        ScopeCtx { filter: self.filter.or_window(start, end), person: self.person.clone() }
    }
}

/// Source of the TEMP rows' context ids, across connections of this process.
static NEXT_CTX: AtomicU64 = AtomicU64::new(1);

/// How many person contexts this connection has built (`find` builds one per call with a person; an Ask
/// builds one). For tests of that count.
pub fn person_context_builds(store: &Store) -> u64 {
    store.person_ctx_builds.get()
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// The FTS5 phrase for `name`: its words in order, nothing parsed as an operator. `None` when the
/// name has no word to match.
fn phrase_of(name: &str) -> Option<String> {
    let words: Vec<String> = name.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).map(|w| w.replace('"', "\"\"")).collect();
    (!words.is_empty()).then(|| format!("\"{}\"", words.join(" ")))
}

impl Store {
    /// `f` checked and normalized, and its person (if any) resolved once. Every `scoped_*` read takes it, so
    /// an Ask resolves the person one time however many sources and contexts it reads.
    pub fn scope_ctx(&self, f: &FindFilter) -> rusqlite::Result<ScopeCtx<'_>> {
        let filter = f.clone().validated().map_err(invalid)?;
        let person = match &filter.person {
            Some(p) => Some(Rc::new(self.person_ctx(p)?)),
            None => None,
        };
        Ok(ScopeCtx { filter, person })
    }

    /// The person's identity here: their address on record, the person rows they stand for, and the
    /// meetings they attended. Names are compared folded, in Rust, after a SQL pre-filter that can only
    /// leave rows in.
    fn person_ctx(&self, p: &PersonFilter) -> rusqlite::Result<PersonCtx<'_>> {
        self.person_ctx_builds.set(self.person_ctx_builds.get() + 1);
        let name_lc = fold(&p.name);
        let key = p.person_key.clone().filter(|k| !k.trim().is_empty());
        let mut speaker_keys: Vec<String> = key.iter().cloned().collect();
        let mut on_record: Option<String> = None;
        let mut on_record_by_key = false;
        {
            // The rows that can be this person: the key, or a name that holds theirs (when LIKE can say so).
            let mut sql = "SELECT person_key, name, email FROM person".to_string();
            let mut binds: Vec<Value> = Vec::new();
            if let Some((cond, pb)) = prefilter("name", &name_lc, 2) {
                sql.push_str(&format!(" WHERE person_key = ?1 OR {cond}"));
                binds.push(key.clone().map_or(Value::Null, Value::Text));
                binds.extend(pb);
            }
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(binds), |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)))?;
            for row in rows {
                let (pk, name, email) = row?;
                let by_key = key.as_deref() == Some(pk.as_str());
                if !by_key && name.as_deref().map(fold).as_deref() != Some(name_lc.as_str()) {
                    continue;
                }
                if !speaker_keys.contains(&pk) {
                    speaker_keys.push(pk);
                }
                if let Some(e) = email.filter(|e| !e.trim().is_empty()) {
                    if on_record.is_none() || (by_key && !on_record_by_key) {
                        on_record = Some(e);
                        on_record_by_key = by_key;
                    }
                }
            }
        }
        let email = on_record.or_else(|| key.clone().filter(|k| k.contains('@'))).map(|e| fold(&e)).filter(|e| !e.is_empty());
        Ok(PersonCtx {
            store: self,
            id: NEXT_CTX.fetch_add(1, Ordering::Relaxed),
            tokens: name_tokens(&p.name),
            email,
            speaker_keys,
            attended: self.attended_rows(&p.name)?,
            mail: OnceCell::new(),
            attended_staged: Cell::new(false),
            mail_staged: Cell::new(false),
        })
    }

    /// `(id, start, end)` of every meeting `name` attended, newest first: an attendee, trimmed, equals
    /// the name ignoring case. Only meetings whose attendees can hold the name are read (a SQL pre-filter).
    fn attended_rows(&self, name: &str) -> rusqlite::Result<Vec<(i64, i64, Option<i64>)>> {
        let want = fold(name);
        if want.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = "SELECT id, start_unix_ms, end_unix_ms, attendees FROM meeting".to_string();
        let mut binds: Vec<Value> = Vec::new();
        if let Some((cond, pb)) = prefilter("attendees", &want, 1) {
            sql.push_str(&format!(" WHERE {cond}"));
            binds = pb;
        }
        sql.push_str(" ORDER BY start_unix_ms DESC, id DESC");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(binds), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, String>(3)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, ts, end, attendees) = row?;
            let list: Vec<String> = serde_json::from_str(&attendees).unwrap_or_default();
            if list.iter().any(|a| fold(a) == want) {
                out.push((id, ts, end));
            }
        }
        Ok(out)
    }

    /// `(id, start)` of the meetings starting in `[start, end)` that `name` attended, newest first: an
    /// attendee, trimmed, equals the name ignoring case. One query over the meetings.
    pub fn meetings_attended_by(&self, name: &str, start: Option<i64>, end: Option<i64>) -> rusqlite::Result<Vec<(i64, i64)>> {
        Ok(self
            .attended_rows(name)?
            .into_iter()
            .filter(|(_, ts, _)| start.is_none_or(|s| *ts >= s) && end.is_none_or(|e| *ts < e))
            .map(|(id, ts, _)| (id, ts))
            .collect())
    }

    /// Ids of the email items whose sender or any recipient is the person: the address on record
    /// equal (ignoring case) to an actor's parsed address; with none on record, a name of two or more
    /// tokens as a run of whole tokens in an actor's display name ([`display_is_person`]). Read once per
    /// context, narrowed in SQL by the address or the longest name token.
    fn mail_ids<'a>(&self, p: &'a PersonCtx<'_>) -> rusqlite::Result<&'a Vec<i64>> {
        if let Some(ids) = p.mail.get() {
            return Ok(ids);
        }
        let mut ids = Vec::new();
        let needle: Option<&str> = match &p.email {
            Some(addr) => Some(addr.as_str()),
            None => p.tokens.iter().max_by_key(|t| t.chars().count()).map(String::as_str),
        };
        // A name of one token and no address matches no email: nothing to scan for.
        let nothing = p.email.is_none() && p.tokens.len() < 2;
        if !nothing {
            let mut sql = "SELECT id, actors FROM items WHERE kind = 'email' AND actors IS NOT NULL".to_string();
            let mut binds: Vec<Value> = Vec::new();
            if let Some((cond, pb)) = needle.and_then(|n| prefilter("actors", n, 1)) {
                sql.push_str(&format!(" AND {cond}"));
                binds = pb;
            }
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(binds), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (id, actors) = row?;
                let list: Vec<String> = serde_json::from_str(&actors).unwrap_or_default();
                let hit = list.iter().any(|a| {
                    let (addr, display) = parse_actor(a);
                    match &p.email {
                        Some(want) => addr.as_deref() == Some(want.as_str()),
                        None => display_is_person(&display, &p.tokens),
                    }
                });
                if hit {
                    ids.push(id);
                }
            }
        }
        Ok(p.mail.get_or_init(|| ids))
    }

    /// Puts `ids` in this connection's TEMP table under `(ctx, kind)`, for a query to join.
    fn stage_ids(&self, ctx: u64, kind: i64, ids: &[i64]) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS find_scope_ids (ctx INTEGER NOT NULL, kind INTEGER NOT NULL, id INTEGER NOT NULL, PRIMARY KEY (ctx, kind, id)) WITHOUT ROWID",
        )?;
        self.conn.execute_batch("SAVEPOINT find_scope_ids")?;
        let filled = (|| -> rusqlite::Result<()> {
            let mut stmt = self.conn.prepare_cached("INSERT OR IGNORE INTO temp.find_scope_ids (ctx, kind, id) VALUES (?1, ?2, ?3)")?;
            for id in ids {
                stmt.execute(rusqlite::params![ctx as i64, kind, id])?;
            }
            Ok(())
        })();
        match filled {
            Ok(()) => self.conn.execute_batch("RELEASE find_scope_ids"),
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK TO find_scope_ids; RELEASE find_scope_ids");
                Err(e)
            }
        }
    }

    // ---- conditions -----------------------------------------------------------------------------

    fn dates(b: &mut Binds, f: &FindFilter, col: &str) -> String {
        let mut sql = String::new();
        if let Some(s) = f.start_unix_ms {
            sql.push_str(&format!(" AND {col} >= {}", b.add(Value::Integer(s))));
        }
        if let Some(e) = f.end_unix_ms {
            sql.push_str(&format!(" AND {col} < {}", b.add(Value::Integer(e))));
        }
        sql
    }

    /// Conditions on `items i` for captures: app, dates, and (with a person) a phrase mention of the name.
    fn capture_conds(b: &mut Binds, f: &FindFilter, mention: Option<&str>) -> String {
        let mut sql = Self::dates(b, f, "i.ts_unix_ms");
        if let Some(app) = &f.app_bundle_id {
            sql.push_str(&format!(" AND i.app_bundle_id = {}", b.add(Value::Text(app.clone()))));
        }
        if let Some(phrase) = mention {
            sql.push_str(&format!(" AND i.id IN (SELECT rowid FROM items_fts WHERE items_fts MATCH {})", b.add(Value::Text(phrase.to_string()))));
        }
        sql
    }

    /// A subquery of the ids of the meetings the person attended, from the TEMP table (staged on first use).
    fn attended_ids(&self, p: &PersonCtx<'_>) -> rusqlite::Result<String> {
        if !p.attended_staged.get() {
            let ids: Vec<i64> = p.attended.iter().map(|(id, _, _)| *id).collect();
            self.stage_ids(p.id, 2, &ids)?;
            p.attended_staged.set(true);
        }
        Ok(format!("SELECT id FROM temp.find_scope_ids WHERE ctx = {} AND kind = 2", p.id))
    }

    // ---- one candidate list ---------------------------------------------------------------------

    /// The best `limit` candidates of `kind` for `query`, by keyword (`embedding` None) or meaning
    /// (`embedding` the query's JSON), best first, with the filter's bounds in the SQL before the limit.
    /// The flag says the read hit its limit before de-duplication, so more rows may exist.
    fn leg(
        &self,
        kind: FindKind,
        query: &str,
        embedding: Option<&str>,
        f: &FindFilter,
        person: Option<&PersonCtx<'_>>,
        limit: usize,
    ) -> rusqlite::Result<(Vec<Key>, bool)> {
        let mut b = Binds::new(4);
        let (select, from_extra, item_kind, conds) = match kind {
            FindKind::Capture => ("i.id, i.ts_unix_ms, 0, 0", "", "capture", Self::capture_conds(&mut b, f, None)),
            FindKind::McpItem => ("i.id, i.ts_unix_ms, 0, 0", "", "mcp_item", Self::dates(&mut b, f, "i.ts_unix_ms")),
            FindKind::Transcript => {
                let mut c = Self::dates(&mut b, f, "i.ts_unix_ms");
                let mut via = "0".to_string();
                if let Some(p) = person {
                    let attended = self.attended_ids(p)?;
                    let keys = b.add(Value::Text(serde_json::to_string(&p.speaker_keys).unwrap_or_else(|_| "[]".into())));
                    let spoken = format!(
                        "EXISTS (SELECT 1 FROM meeting_speaker ms WHERE ms.meeting_id = i.thread_id \
                         AND 'Speaker ' || (ms.speaker_id + 1) = json_extract(i.actors, '$[0]') \
                         AND ms.person_key IN (SELECT value FROM json_each({keys})))"
                    );
                    c.push_str(&format!(" AND (i.thread_id IN ({attended}) OR {spoken})"));
                    via = format!("CASE WHEN {spoken} THEN 1 ELSE 0 END");
                }
                return self.run_leg(kind, query, embedding, limit, &b, &c, &format!("i.id, i.ts_unix_ms, {via}, i.thread_id"), "JOIN meeting m ON m.id = i.thread_id", "transcript");
            }
            FindKind::Snapshot => {
                let mut c = Self::dates(&mut b, f, "m.start_unix_ms");
                if let Some(p) = person {
                    c.push_str(&format!(" AND i.thread_id IN ({})", self.attended_ids(p)?));
                }
                return self.run_leg(
                    kind,
                    query,
                    embedding,
                    limit,
                    &b,
                    &c,
                    "i.id, m.start_unix_ms, 0, i.thread_id",
                    "JOIN meeting m ON m.id = i.thread_id JOIN meeting_snapshot s ON s.id = i.source_id",
                    "snapshot",
                );
            }
            FindKind::Thread => {
                let mut c = Self::dates(&mut b, f, "t.last_activity_ts_unix_ms");
                if let Some(p) = person {
                    c.push_str(&self.actor_cond(p, "i")?);
                }
                return self.run_leg(
                    kind,
                    query,
                    embedding,
                    limit.saturating_mul(4),
                    &b,
                    &c,
                    "t.id, t.last_activity_ts_unix_ms, 0, i.id",
                    "JOIN threads t ON t.source = i.source AND t.external_thread_id = json_extract(i.refs, '$.gmail_thread_id') \
                     AND t.account_email = json_extract(i.refs, '$.account_email')",
                    "email",
                );
            }
            _ => return Ok((Vec::new(), false)),
        };
        self.run_leg(kind, query, embedding, limit, &b, &conds, select, from_extra, item_kind)
    }

    /// The sender or recipient condition for a person on `alias`'s email message: the message is one
    /// of the ids [`Store::mail_ids`] found.
    fn actor_cond(&self, p: &PersonCtx<'_>, alias: &str) -> rusqlite::Result<String> {
        let ids = self.mail_ids(p)?;
        if !p.mail_staged.get() {
            self.stage_ids(p.id, 1, ids)?;
            p.mail_staged.set(true);
        }
        Ok(format!(" AND {alias}.id IN (SELECT id FROM temp.find_scope_ids WHERE ctx = {} AND kind = 1)", p.id))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_leg(
        &self,
        kind: FindKind,
        query: &str,
        embedding: Option<&str>,
        limit: usize,
        b: &Binds,
        conds: &str,
        select: &str,
        from_extra: &str,
        item_kind: &str,
    ) -> rusqlite::Result<(Vec<Key>, bool)> {
        let mut vals: Vec<Value> = Vec::new();
        let mut total_items = 0;
        let sql = match embedding {
            None => {
                vals.push(Value::Text(sanitize_fts5_query(query)));
                vals.push(Value::Integer(sql_limit(limit)));
                vals.push(Value::Text(fts5_all_words(query)));
                format!(
                    "SELECT {select} FROM items_fts f JOIN items i ON i.id = f.rowid {from_extra} \
                     WHERE items_fts MATCH ?1 AND i.kind = '{item_kind}'{conds} \
                     ORDER BY f.rowid IN (SELECT rowid FROM items_fts WHERE items_fts MATCH ?3) DESC, rank LIMIT ?2"
                )
            }
            Some(emb) => {
                total_items = self.conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get::<_, i64>(0))?;
                vals.push(Value::Text(emb.to_string()));
                vals.push(Value::Integer(total_items.clamp(1, ITEMS_VEC_MAX_K)));
                vals.push(Value::Integer(sql_limit(limit)));
                format!(
                    "SELECT {select} FROM items_vec v JOIN items i ON i.id = v.rowid {from_extra} \
                     WHERE v.embedding MATCH ?1 AND k = ?2 AND i.kind = '{item_kind}'{conds} \
                     ORDER BY v.distance LIMIT ?3"
                )
            }
        };
        vals.extend(b.vals.iter().cloned());
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(vals), |r| {
            Ok(Key { kind, id: r.get(0)?, ts: r.get(1)?, speaker: r.get::<_, i64>(2)? != 0, aux: r.get(3)? })
        })?;
        let mut keys: Vec<Key> = rows.collect::<rusqlite::Result<_>>()?;
        // The pool is hit when the read came back full, counted before any row is dropped or merged.
        let capped = keys.len() >= limit;
        // The k nearest neighbours of everything, filtered after, can leave a narrow filter short while more
        // rows it selects exist. Then score the rows the filter selects, exactly, when there are not too many.
        if let (Some(emb), true) = (embedding, keys.len() < limit && total_items > ITEMS_VEC_MAX_K) {
            if let Some(exact) = self.exact_semantic(emb, b, conds, select, from_extra, item_kind, limit, kind, keys.len())? {
                keys = exact;
            }
        }
        if matches!(kind, FindKind::Transcript | FindKind::Snapshot) {
            keys = self.drop_copy_hits(keys, |k: &Key| k.aux)?;
        }
        if kind == FindKind::Thread {
            // Several messages of one thread can match: the best-ranked one stands for it.
            let mut seen = HashSet::new();
            keys.retain(|k| seen.insert(k.id));
            keys.truncate(limit / 4);
        }
        Ok((keys, capped))
    }

    /// The rows the filter selects, best by meaning first, scored here against the query's embedding.
    /// `None` when the filter selects no more than `have` rows or more than [`EXACT_SEMANTIC_MAX_ROWS`]
    /// (then the nearest-neighbour read stands). Bounded: one count, then one scan of at most that many rows.
    #[allow(clippy::too_many_arguments)]
    fn exact_semantic(
        &self,
        emb: &str,
        b: &Binds,
        conds: &str,
        select: &str,
        from_extra: &str,
        item_kind: &str,
        limit: usize,
        kind: FindKind,
        have: usize,
    ) -> rusqlite::Result<Option<Vec<Key>>> {
        // The conditions bind from ?4: the first three placeholders are unused here.
        let params = || -> Vec<Value> {
            if b.vals.is_empty() {
                return Vec::new();
            }
            let mut v = vec![Value::Null, Value::Null, Value::Null];
            v.extend(b.vals.iter().cloned());
            v
        };
        let selected: i64 =
            self.conn.query_row(&format!("SELECT COUNT(*) FROM items i {from_extra} WHERE i.kind = '{item_kind}'{conds}"), params_from_iter(params()), |r| r.get(0))?;
        if selected as usize <= have || selected as usize > EXACT_SEMANTIC_MAX_ROWS {
            return Ok(None);
        }
        let Ok(query_vec) = serde_json::from_str::<Vec<f32>>(emb) else { return Ok(None) };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {select}, v.embedding FROM items i JOIN items_vec v ON v.rowid = i.id {from_extra} WHERE i.kind = '{item_kind}'{conds}"
        ))?;
        let width = stmt.column_count();
        let rows = stmt.query_map(params_from_iter(params()), |r| {
            let key = Key { kind, id: r.get(0)?, ts: r.get(1)?, speaker: r.get::<_, i64>(2)? != 0, aux: r.get(3)? };
            let blob: Vec<u8> = r.get(width - 1)?;
            Ok((key, blob))
        })?;
        let mut scored: Vec<(f32, Key)> = Vec::new();
        for row in rows {
            let (key, blob) = row?;
            if blob.len() != query_vec.len() * 4 {
                continue;
            }
            let d: f32 = blob.chunks_exact(4).zip(&query_vec).map(|(c, q)| (f32::from_le_bytes([c[0], c[1], c[2], c[3]]) - q).powi(2)).sum();
            scored.push((d, key));
        }
        scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.id.cmp(&b.1.id)));
        Ok(Some(scored.into_iter().take(limit).map(|(_, k)| k).collect()))
    }

    /// Each of `ids`' cosine to `query`, for the items that have an embedding. One read per 400 ids.
    fn cosines(&self, ids: &[i64], query: &[f32]) -> rusqlite::Result<HashMap<i64, f32>> {
        let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
        let qn = norm(query);
        let mut out = HashMap::new();
        if qn == 0.0 {
            return Ok(out);
        }
        for chunk in ids.chunks(400) {
            let list = vec!["?"; chunk.len()].join(",");
            let mut stmt = self.conn.prepare(&format!("SELECT rowid, embedding FROM items_vec WHERE rowid IN ({list})"))?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().map(|id| Value::Integer(*id))), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
            for row in rows {
                let (id, blob) = row?;
                if blob.len() != query.len() * 4 {
                    continue;
                }
                let v: Vec<f32> = blob.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                let vn = norm(&v);
                if vn > 0.0 {
                    out.insert(id, v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>() / (vn * qn));
                }
            }
        }
        Ok(out)
    }

    /// The meaning hits at least `floor` like the query, in their order. Read after the leg,
    /// so the leg's own choices (the nearest-neighbour read, the exact scan of a narrow filter) are unchanged. One read of at most
    /// [`FIND_POOL`] vectors.
    fn meaning_floor(&self, keys: Vec<Key>, query: &[f32], floor: f32) -> rusqlite::Result<Vec<Key>> {
        let cos = self.cosines(&keys.iter().map(|k| k.id).collect::<Vec<_>>(), query)?;
        Ok(keys.into_iter().filter(|k| cos.get(&k.id).is_some_and(|c| *c >= floor)).collect())
    }

    /// How much of the query's words each of `ids` holds: the sum of the inverse document frequency of each word it holds, over
    /// that sum for every word, so a rare word counts for more than one in most rows ("festival" against "kite" in sixty kite
    /// logs). One count and one read per word, of at most 16 words.
    fn coverage(&self, words: &[String], ids: &[i64]) -> rusqlite::Result<HashMap<i64, f64>> {
        let mut got: HashMap<i64, f64> = HashMap::new();
        if words.is_empty() || ids.is_empty() {
            return Ok(got);
        }
        let n = self.conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get::<_, i64>(0))?.max(1) as f64;
        let ids = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
        let mut total = 0.0;
        for w in words.iter().take(16) {
            let m = sanitize_fts5_query(w);
            let df: i64 = self.conn.query_row("SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH ?1", [&m], |r| r.get(0))?;
            let idf = (1.0 + n / df.max(1) as f64).ln();
            total += idf;
            let mut stmt = self.conn.prepare_cached("SELECT rowid FROM items_fts WHERE items_fts MATCH ?1 AND rowid IN (SELECT value FROM json_each(?2))")?;
            for id in stmt.query_map(rusqlite::params![m, ids], |r| r.get::<_, i64>(0))? {
                *got.entry(id?).or_default() += idf;
            }
        }
        if total > 0.0 {
            got.values_mut().for_each(|v| *v /= total);
        }
        Ok(got)
    }

    /// Leaves out an episode that covers a single item when that item is a candidate too: an imported note is stored as a capture
    /// and as a one-item episode with the same title, and Find listed it twice. An episode over several items stays, since it says
    /// more than any one of them.
    fn drop_one_item_episodes(&self, all: &mut Vec<(Key, usize)>) -> rusqlite::Result<()> {
        let episodes: Vec<i64> = all.iter().filter(|(k, _)| k.kind == FindKind::Episode).map(|(k, _)| k.id).collect();
        if episodes.is_empty() {
            return Ok(());
        }
        let captures: HashSet<i64> = all.iter().filter(|(k, _)| k.kind == FindKind::Capture).map(|(k, _)| k.id).collect();
        if captures.is_empty() {
            return Ok(());
        }
        let ids = serde_json::to_string(&episodes).unwrap_or_else(|_| "[]".into());
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, start_item_id FROM episodes WHERE start_item_id = end_item_id AND id IN (SELECT value FROM json_each(?1))",
        )?;
        let mut covered: HashSet<i64> = HashSet::new();
        for row in stmt.query_map([ids], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (episode, item) = row?;
            if captures.contains(&item) {
                covered.insert(episode);
            }
        }
        all.retain(|(k, _)| !(k.kind == FindKind::Episode && covered.contains(&k.id)));
        Ok(())
    }

    /// The candidates of the keyword lists and the meaning lists (each best first), de-duplicated by `(kind, id)` and ordered. For
    /// rank order each candidate scores its cosine to the query plus [`KEYWORD_WEIGHT`] times the share of the query's words it
    /// holds ([`Store::coverage`]), so a row that holds a word or two of a question but means something else does not push out one
    /// the query is about, and a row that holds the words and the meaning leads. Ties break by the best keyword rank, then time, kind
    /// and id, so pages are stable. A candidate with no cosine (an episode, a run, a meeting, a row whose embedding is still to come)
    /// counts as just at the floor, so a keyword hit there is neither sunk nor lifted by meaning; `shares` holds the share of the
    /// words of those with no search index row.
    fn rank(
        &self,
        keyword: &[Vec<Key>],
        near: &[Vec<Key>],
        words: &[String],
        shares: &HashMap<(FindKind, i64), f64>,
        query: Option<(&[f32], f32)>,
        order: FindOrder,
    ) -> rusqlite::Result<Vec<Key>> {
        let mut all: HashMap<(FindKind, i64), (Key, usize)> = HashMap::new();
        for (list, from_keyword) in keyword.iter().map(|l| (l, true)).chain(near.iter().map(|l| (l, false))) {
            for (i, k) in list.iter().enumerate() {
                let e = all.entry((k.kind, k.id)).or_insert((*k, usize::MAX));
                e.0.speaker |= k.speaker;
                if from_keyword {
                    e.1 = e.1.min(i);
                }
            }
        }
        let mut all: Vec<(Key, usize)> = all.into_values().collect();
        self.drop_one_item_episodes(&mut all)?;
        let by_time = |a: &Key, b: &Key| b.ts.cmp(&a.ts).then(a.kind.cmp(&b.kind)).then(b.id.cmp(&a.id));
        if order == FindOrder::Time {
            all.sort_by(|a, b| by_time(&a.0, &b.0));
            return Ok(all.into_iter().map(|(k, _)| k).collect());
        }
        // The item that stands for a candidate: a thread's best message, nothing for an episode, a run or a meeting.
        let item = |k: &Key| match k.kind {
            FindKind::Thread => Some(k.aux),
            FindKind::Capture | FindKind::Transcript | FindKind::Snapshot | FindKind::McpItem => Some(k.id),
            _ => None,
        };
        let items: Vec<i64> = all.iter().filter_map(|(k, _)| item(k)).collect();
        let cos = match query {
            Some((q, _)) => self.cosines(&items, q)?,
            None => HashMap::new(),
        };
        let neutral = query.map_or(0.0, |(_, floor)| f64::from(floor));
        let matched: Vec<i64> = all.iter().filter(|(_, pos)| *pos != usize::MAX).filter_map(|(k, _)| item(k)).collect();
        let cover = self.coverage(words, &matched)?;
        let mut scored: Vec<(f64, usize, Key)> = all
            .into_iter()
            .map(|(k, pos)| {
                let share = match (pos, item(&k)) {
                    (usize::MAX, _) => 0.0,
                    (_, Some(id)) => cover.get(&id).copied().unwrap_or(0.0),
                    (_, None) => shares.get(&(k.kind, k.id)).copied().unwrap_or(0.0),
                };
                let meaning = item(&k).and_then(|id| cos.get(&id)).map_or(neutral, |c| f64::from(c.max(0.0)));
                (meaning + KEYWORD_WEIGHT * share, pos, k)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)).then(by_time(&a.2, &b.2)));
        Ok(scored.into_iter().map(|(_, _, k)| k).collect())
    }

    /// Episodes and automation runs have no search index: a query matches when any of its words is
    /// inside their title or text. Each comes with the share of the words it holds.
    fn text_leg(&self, kind: FindKind, query: &str, f: &FindFilter, limit: usize) -> rusqlite::Result<Vec<(Key, f64)>> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| w.chars().count() >= 2).collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut b = Binds::new(2);
        let (select, table, time_col, text_cols) = match kind {
            FindKind::Episode => ("e.id, e.end_ts_unix_ms", "episodes e", "e.end_ts_unix_ms", ["coalesce(e.window_title, '')", "coalesce(e.summary, '')"]),
            _ => (
                "rr.id, rr.ran_at_unix_ms",
                "routine_run rr JOIN routine r ON r.id = rr.routine_id",
                "rr.ran_at_unix_ms",
                ["r.name", "rr.output_summary"],
            ),
        };
        let mut conds = Self::dates(&mut b, f, time_col);
        if kind != FindKind::Episode {
            conds.push_str(&format!(" AND rr.status IN ({}, {})", b.add(Value::Text("success".into())), b.add(Value::Text("failed".into()))));
        }
        let ors: Vec<String> = words
            .iter()
            .map(|w| {
                let p = b.add(Value::Text(format!("%{}%", like_escape(w))));
                format!("(lower({}) LIKE {p} ESCAPE '\\' OR lower({}) LIKE {p} ESCAPE '\\')", text_cols[0], text_cols[1])
            })
            .collect();
        let sql = format!(
            "SELECT {select}, lower({}) || ' ' || lower({}) FROM {table} WHERE ({}){conds} ORDER BY {time_col} DESC, 1 DESC LIMIT ?1",
            text_cols[0],
            text_cols[1],
            ors.join(" OR ")
        );
        let mut vals = vec![Value::Integer(sql_limit(limit))];
        vals.extend(b.vals);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(vals), |r| {
            let text: String = r.get(2)?;
            let share = words.iter().filter(|w| text.contains(w.as_str())).count() as f64 / words.len() as f64;
            Ok((Key { kind, id: r.get(0)?, ts: r.get(1)?, speaker: false, aux: 0 }, share))
        })?;
        rows.collect()
    }

    /// Captures that hold the person's name as a phrase (and, with a query, match it), newest first.
    fn mention_keys(&self, query: &str, f: &FindFilter, name: &str, limit: usize) -> rusqlite::Result<Vec<Key>> {
        let Some(phrase) = phrase_of(name) else { return Ok(Vec::new()) };
        let mut b = Binds::new(2);
        let mut conds = Self::capture_conds(&mut b, f, Some(&phrase));
        if !query.trim().is_empty() {
            conds.push_str(&format!(
                " AND i.id IN (SELECT rowid FROM items_fts WHERE items_fts MATCH {})",
                b.add(Value::Text(sanitize_fts5_query(query)))
            ));
        }
        let mut vals = vec![Value::Integer(sql_limit(limit))];
        vals.extend(b.vals);
        let mut stmt =
            self.conn.prepare(&format!("SELECT i.id, i.ts_unix_ms FROM items i WHERE i.kind = 'capture'{conds} ORDER BY i.ts_unix_ms DESC, i.id DESC LIMIT ?1"))?;
        let rows = stmt.query_map(params_from_iter(vals), |r| Ok(Key { kind: FindKind::Capture, id: r.get(0)?, ts: r.get(1)?, speaker: false, aux: 0 }))?;
        rows.collect()
    }

    /// The embedding sweep has work queued or under way: some rows may have no embedding yet. One indexed read of the jobs table.
    fn embedding_sweep_pending(&self) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM jobs WHERE job_type = 'reindex_sweep' AND status IN ('pending', 'running'))",
            [],
            |r| r.get(0),
        )
    }

    // ---- find -----------------------------------------------------------------------------------

    /// One page of results for `query` under `filter`. See the module notes for the rules.
    pub fn find(&self, query: &str, filter: &FindFilter, order: FindOrder, offset: usize, limit: usize) -> rusqlite::Result<FindPage> {
        let filter = filter.clone().validated().map_err(invalid)?;
        let query = query.trim();
        let limit = limit.clamp(1, FIND_MAX_LIMIT);
        let offset = offset.min(FIND_MAX_OFFSET);
        if query.is_empty() && filter.is_empty() {
            return Ok(FindPage::default());
        }
        let person = match &filter.person {
            Some(p) => Some(self.person_ctx(p)?),
            None => None,
        };
        let in_range = person.as_ref().map(|p| p.in_range(&filter)).unwrap_or_default();
        let mut page = FindPage::default();
        let mut main: Vec<Key>;
        let mut mentions: Vec<Key> = Vec::new();

        if query.is_empty() {
            // Listing: by time, exact, not pool-limited.
            let want = offset + limit;
            main = Vec::new();
            if person.is_none() {
                main.extend(self.list_captures(&filter, want)?);
                page.counts.insert("capture".into(), self.count_captures(&filter)?);
            }
            if filter.allows(FindKind::McpItem) {
                let (keys, total) = self.list_mcp_items(&filter, want)?;
                main.extend(keys);
                if total > 0 {
                    page.counts.insert("mcp_item".into(), total);
                }
            }
            if person.is_some() && filter.allows(FindKind::Meeting) {
                main.extend(in_range.iter().take(want).map(|(id, ts)| Key { kind: FindKind::Meeting, id: *id, ts: *ts, speaker: false, aux: 0 }));
                page.counts.insert("meeting".into(), in_range.len());
            }
            main.sort_by(|a, b| b.ts.cmp(&a.ts).then(a.kind.cmp(&b.kind)).then(b.id.cmp(&a.id)));
        } else {
            let embedding = self.embed_query(query).ok().map(|e| embedding_to_json(&e));
            page.semantic_available = embedding.is_some();
            page.semantic_used = embedding.is_some();
            page.meaning_catching_up = embedding.is_some() && self.embedding_sweep_pending()?;
            // (kind, method) lists, each best first. The meaning lists wait for their floor until every word list is in.
            let mut lists: Vec<Vec<Key>> = Vec::new();
            let mut meaning: Vec<Vec<Key>> = Vec::new();
            let hidden_captures = person.is_some();
            // The keyword legs match the query's words less its common ones; a query of common words only finds by meaning.
            let words = keyword_words(query);
            let kw = words.join(" ");
            // The share of the words held by a candidate with no search index row (an episode, a run, a meeting).
            let mut shares: HashMap<(FindKind, i64), f64> = HashMap::new();
            for kind in [FindKind::Capture, FindKind::Transcript, FindKind::Snapshot, FindKind::Thread, FindKind::McpItem] {
                if !filter.allows(kind) || (kind == FindKind::Capture && hidden_captures) {
                    continue;
                }
                if !kw.is_empty() {
                    let (hits, capped) = self.leg(kind, &kw, None, &filter, person.as_ref(), FIND_POOL)?;
                    page.capped |= capped;
                    lists.push(hits);
                }
                if kind != FindKind::Thread {
                    if let Some(emb) = &embedding {
                        meaning.push(self.leg(kind, query, Some(emb), &filter, person.as_ref(), FIND_POOL)?.0);
                    }
                }
            }
            for kind in [FindKind::Episode, FindKind::RoutineRun] {
                if filter.allows(kind) {
                    let l = self.text_leg(kind, &kw, &filter, FIND_POOL)?;
                    page.capped |= l.len() >= FIND_POOL;
                    shares.extend(l.iter().map(|(k, share)| ((k.kind, k.id), *share)));
                    lists.push(l.into_iter().map(|(k, _)| k).collect());
                }
            }
            if person.is_some() && filter.allows(FindKind::Meeting) {
                // Meetings the person attended (in the range) whose title holds a word of the query.
                let titles = self.meeting_titles(&in_range.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
                let mut hits = Vec::new();
                for (id, ts) in &in_range {
                    let Some(t) = titles.get(id).map(|t| t.to_lowercase()) else { continue };
                    let held = words.iter().filter(|w| t.contains(w.as_str())).count();
                    if held > 0 {
                        shares.insert((FindKind::Meeting, *id), held as f64 / words.len() as f64);
                        hits.push(Key { kind: FindKind::Meeting, id: *id, ts: *ts, speaker: false, aux: 0 });
                    }
                }
                lists.push(hits);
            }
            // Odd words are always somewhat near something in a large library: a meaning hit must be close enough to count.
            let floor = meaning_floor_for(query);
            let query_vec = embedding.as_deref().and_then(|e| serde_json::from_str::<Vec<f32>>(e).ok());
            let mut near_lists = Vec::new();
            if let Some(q) = &query_vec {
                for near in meaning {
                    near_lists.push(self.meaning_floor(near, q, floor)?);
                }
            }
            main = self.rank(&lists, &near_lists, &words, &shares, query_vec.as_deref().map(|q| (q, floor)), order)?;
            for k in &main {
                *page.counts.entry(k.kind.as_str().to_string()).or_default() += 1;
            }
        }

        if let Some(p) = &filter.person {
            // Mentions match by keyword: a query of common words only has none.
            let kw = keyword_words(query).join(" ");
            mentions = if !query.is_empty() && kw.is_empty() {
                Vec::new()
            } else {
                self.mention_keys(&kw, &filter, &p.name, if query.is_empty() { offset + limit } else { FIND_POOL })?
            };
            page.mentions_total = if query.is_empty() { self.count_mentions(&filter, &p.name)? } else { mentions.len() };
            page.capped |= !query.is_empty() && mentions.len() >= FIND_POOL;
            page.counts.insert("mentions".into(), page.mentions_total);
        }
        page.total = if query.is_empty() { page.counts.values().sum::<usize>() - page.counts.get("mentions").copied().unwrap_or(0) } else { main.len() };
        page.more = offset.saturating_add(limit) < page.total.max(page.mentions_total);

        let slice = |keys: &[Key]| -> Vec<Key> { keys.iter().skip(offset).take(limit).copied().collect() };
        let (results, mentioned) = (slice(&main), slice(&mentions));
        page.results = self.cards_for_keys(&results, person.is_some())?;
        page.mentions = self.cards_for_keys(&mentioned, false)?;
        Ok(page)
    }

    fn cards_for_keys(&self, keys: &[Key], person: bool) -> rusqlite::Result<Vec<SourceCard>> {
        let refs: Vec<(String, i64)> = keys.iter().map(|k| (k.kind.as_str().to_string(), k.id)).collect();
        let mut cards = self.resolve_cards(&refs)?;
        for (card, key) in cards.iter_mut().zip(keys) {
            if person && key.kind == FindKind::Transcript {
                card.via = Some(if key.speaker { "speaker" } else { "attendee" }.to_string());
            }
        }
        Ok(cards)
    }

    fn meeting_titles(&self, ids: &[i64]) -> rusqlite::Result<HashMap<i64, String>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(400) {
            let list = vec!["?"; chunk.len()].join(",");
            let mut stmt = self.conn.prepare(&format!("SELECT id, title FROM meeting WHERE id IN ({list})"))?;
            for row in stmt.query_map(params_from_iter(chunk.iter().map(|i| Value::Integer(*i))), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (id, title) = row?;
                out.insert(id, title);
            }
        }
        Ok(out)
    }

    fn list_captures(&self, f: &FindFilter, limit: usize) -> rusqlite::Result<Vec<Key>> {
        let mut b = Binds::new(2);
        let conds = Self::capture_conds(&mut b, f, None);
        let mut vals = vec![Value::Integer(sql_limit(limit))];
        vals.extend(b.vals);
        let mut stmt =
            self.conn.prepare(&format!("SELECT i.id, i.ts_unix_ms FROM items i WHERE i.kind = 'capture'{conds} ORDER BY i.ts_unix_ms DESC, i.id DESC LIMIT ?1"))?;
        let rows = stmt.query_map(params_from_iter(vals), |r| Ok(Key { kind: FindKind::Capture, id: r.get(0)?, ts: r.get(1)?, speaker: false, aux: 0 }))?;
        rows.collect()
    }

    fn count_captures(&self, f: &FindFilter) -> rusqlite::Result<usize> {
        let mut b = Binds::new(1);
        let conds = Self::capture_conds(&mut b, f, None);
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM items i WHERE i.kind = 'capture'{conds}"), params_from_iter(b.vals), |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
    }

    fn count_mentions(&self, f: &FindFilter, name: &str) -> rusqlite::Result<usize> {
        let Some(phrase) = phrase_of(name) else { return Ok(0) };
        let mut b = Binds::new(1);
        let conds = Self::capture_conds(&mut b, f, Some(&phrase));
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM items i WHERE i.kind = 'capture'{conds}"), params_from_iter(b.vals), |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
    }

    // ---- bounded rows for Ask -------------------------------------------------------------------

    /// Captures for a retrieval bounded by `f`, best first, by keyword or meaning. With a person
    /// they are the captures that mention the name as a phrase.
    pub fn scoped_capture_hits(&self, query: &str, semantic: bool, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<CaptureItem>> {
        let f = &scope.filter;
        let mention = f.person.as_ref().and_then(|p| phrase_of(&p.name));
        if f.person.is_some() && mention.is_none() {
            return Ok(Vec::new());
        }
        let mut b = Binds::new(4);
        let conds = Self::capture_conds(&mut b, f, mention.as_deref());
        let embedding = if semantic {
            match self.embed_query(query) {
                Ok(e) => Some(embedding_to_json(&e)),
                Err(_) => return Ok(Vec::new()),
            }
        } else {
            None
        };
        let (keys, _) = self.run_leg(FindKind::Capture, query, embedding.as_deref(), limit, &b, &conds, "i.id, i.ts_unix_ms, 0, 0", "", "capture")?;
        keys.iter().filter_map(|k| self.get_capture_item(k.id).transpose()).collect()
    }

    /// Transcript lines (with their absolute times) for a retrieval bounded by `f`.
    pub fn scoped_transcript_hits(&self, query: &str, semantic: bool, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<(MeetingTranscriptSegment, i64)>> {
        let f = &scope.filter;
        if !f.allows(FindKind::Transcript) {
            return Ok(Vec::new());
        }
        let embedding = if semantic {
            match self.embed_query(query) {
                Ok(e) => Some(embedding_to_json(&e)),
                Err(_) => return Ok(Vec::new()),
            }
        } else {
            None
        };
        let (keys, _) = self.leg(FindKind::Transcript, query, embedding.as_deref(), f, scope.person.as_deref(), limit)?;
        keys.iter().filter_map(|k| self.get_transcript_segment(k.id).transpose().map(|r| r.map(|s| (s, k.ts)))).collect()
    }

    /// Slides (with the time Ask ranks them by) for a retrieval bounded by `f`.
    pub fn scoped_snapshot_hits(&self, query: &str, semantic: bool, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<(MeetingSnapshotHit, i64)>> {
        let f = &scope.filter;
        if !f.allows(FindKind::Snapshot) {
            return Ok(Vec::new());
        }
        let embedding = if semantic {
            match self.embed_query(query) {
                Ok(e) => Some(embedding_to_json(&e)),
                Err(_) => return Ok(Vec::new()),
            }
        } else {
            None
        };
        let (keys, _) = self.leg(FindKind::Snapshot, query, embedding.as_deref(), f, scope.person.as_deref(), limit)?;
        let mut out = Vec::new();
        for k in keys {
            let hit = self
                .conn
                .query_row(
                    "SELECT i.id, i.thread_id, m.title, i.body_text, s.file_path, i.ts_unix_ms FROM items i \
                     JOIN meeting m ON m.id = i.thread_id JOIN meeting_snapshot s ON s.id = i.source_id WHERE i.id = ?1 AND i.kind = 'snapshot'",
                    [k.id],
                    |r| {
                        Ok((MeetingSnapshotHit { id: r.get(0)?, meeting_id: r.get(1)?, meeting_title: r.get(2)?, text: r.get(3)?, file_path: r.get(4)? }, r.get::<_, i64>(5)?))
                    },
                )
                // A row that is gone is no hit; a read that fails is an error, never a thinner answer.
                .optional()?;
            out.extend(hit);
        }
        Ok(out)
    }

    /// Added items by keyword (`semantic` false) or by meaning, with no bound. Ask's unscoped retrieval.
    pub fn search_mcp_items(&self, query: &str, semantic: bool, limit: usize) -> rusqlite::Result<Vec<AddedItemHit>> {
        let scope = self.scope_ctx(&FindFilter::default())?;
        self.scoped_mcp_item_hits(query, semantic, &scope, limit)
    }

    /// Added items for a retrieval bounded by the scope's filter: by its dates; none under an app or a person.
    pub fn scoped_mcp_item_hits(&self, query: &str, semantic: bool, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<AddedItemHit>> {
        let f = &scope.filter;
        if !f.allows(FindKind::McpItem) || query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let embedding = if semantic {
            match self.embed_query(query) {
                Ok(e) => Some(embedding_to_json(&e)),
                Err(_) => return Ok(Vec::new()),
            }
        } else {
            None
        };
        let (keys, _) = self.leg(FindKind::McpItem, query, embedding.as_deref(), f, None, limit)?;
        let mut out = Vec::new();
        for k in keys {
            out.extend(self.added_item_hit(k.id)?);
        }
        Ok(out)
    }

    /// Added items whose own time is in `[start, end)`, newest first (a dated Ask's window).
    pub fn mcp_items_in_window(&self, start_unix_ms: i64, end_unix_ms: i64, limit: usize) -> rusqlite::Result<Vec<AddedItemHit>> {
        let mut stmt = self.conn.prepare(&format!(
            "{ADDED_HIT_SELECT} AND ts_unix_ms >= ?1 AND ts_unix_ms < ?2 ORDER BY ts_unix_ms DESC, id DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(rusqlite::params![start_unix_ms, end_unix_ms, sql_limit(limit)], added_hit_row)?;
        rows.collect()
    }

    /// One added item by id, as a prompt would carry it.
    pub fn added_item_hit(&self, id: i64) -> rusqlite::Result<Option<AddedItemHit>> {
        self.conn.query_row(&format!("{ADDED_HIT_SELECT} AND id = ?1"), [id], added_hit_row).optional()
    }

    /// Added items under `f`'s dates, newest first, and how many there are (Find's listing).
    fn list_mcp_items(&self, f: &FindFilter, limit: usize) -> rusqlite::Result<(Vec<Key>, usize)> {
        let mut b = Binds::new(2);
        let conds = Self::dates(&mut b, f, "i.ts_unix_ms");
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM items i WHERE i.kind = 'mcp_item'{conds}"),
            params_from_iter(std::iter::once(Value::Null).chain(b.vals.iter().cloned())),
            |r| r.get(0),
        )?;
        let mut vals = vec![Value::Integer(sql_limit(limit))];
        vals.extend(b.vals);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT i.id, i.ts_unix_ms FROM items i WHERE i.kind = 'mcp_item'{conds} ORDER BY i.ts_unix_ms DESC, i.id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params_from_iter(vals), |r| Ok(Key { kind: FindKind::McpItem, id: r.get(0)?, ts: r.get(1)?, speaker: false, aux: 0 }))?;
        Ok((rows.collect::<rusqlite::Result<_>>()?, total as usize))
    }

    /// Email threads (with the matching message) for a retrieval bounded by `f`.
    pub fn scoped_email_hits(&self, query: &str, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<crate::EmailThreadHit>> {
        let f = &scope.filter;
        if !f.allows(FindKind::Thread) {
            return Ok(Vec::new());
        }
        let (keys, _) = self.leg(FindKind::Thread, query, None, f, scope.person.as_deref(), limit)?;
        let mut out = Vec::new();
        for k in keys {
            let Some(thread) = self.get_thread(k.id)? else { continue };
            let msg: Option<(String, i64, String)> = self
                .conn
                .query_row(
                    "SELECT COALESCE(json_extract(actors, '$[0]'), ''), ts_unix_ms, COALESCE(body_text, '') FROM items WHERE id = ?1",
                    [k.aux],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if let Some((from, ts_unix_ms, body)) = msg {
                out.push(crate::EmailThreadHit { thread, from, ts_unix_ms, body });
            }
        }
        Ok(out)
    }

    /// Episodes inside `f`'s dates (by when they ended), newest first. Hidden under an app or a person.
    pub fn scoped_episodes(&self, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<Episode>> {
        let f = &scope.filter;
        if !f.allows(FindKind::Episode) {
            return Ok(Vec::new());
        }
        let mut b = Binds::new(2);
        let conds = Self::dates(&mut b, f, "end_ts_unix_ms");
        let mut vals = vec![Value::Integer(sql_limit(limit))];
        vals.extend(b.vals);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, item_count, summary, status \
             FROM episodes WHERE 1 = 1{conds} ORDER BY end_ts_unix_ms DESC, id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params_from_iter(vals), Self::row_to_episode)?;
        rows.collect()
    }

    /// Automation runs inside `f`'s dates, newest first, with the statuses Ask reads. Hidden under an app or a person.
    pub fn scoped_routine_runs(&self, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<RoutineRunWithName>> {
        let f = &scope.filter;
        if !f.allows(FindKind::RoutineRun) {
            return Ok(Vec::new());
        }
        self.routine_runs_in_window(f.start_unix_ms.unwrap_or(i64::MIN), f.end_unix_ms.unwrap_or(i64::MAX), limit)
    }

    /// Threads waiting on someone, inside `f`'s dates and (with a person) with an email from or to
    /// them, newest first. Hidden under an app.
    pub fn scoped_waiting_threads(&self, scope: &ScopeCtx<'_>, limit: usize) -> rusqlite::Result<Vec<Thread>> {
        let f = &scope.filter;
        if !f.allows(FindKind::Thread) {
            return Ok(Vec::new());
        }
        let mut b = Binds::new(2);
        let mut conds = Self::dates(&mut b, f, "t.last_activity_ts_unix_ms");
        if let Some(p) = scope.person.as_deref() {
            // The threads that hold one of the person's messages, found once from the staged ids (a thread has many
            // messages and the ids are few): not a search of every email item for each thread.
            let cond = self.actor_cond(p, "i")?;
            conds.push_str(&format!(
                " AND t.id IN (SELECT t2.id FROM items i JOIN threads t2 ON t2.source = i.source \
                 AND t2.external_thread_id = json_extract(i.refs, '$.gmail_thread_id') \
                 AND t2.account_email = json_extract(i.refs, '$.account_email') WHERE i.kind = 'email'{cond})"
            ));
        }
        let mut vals = vec![Value::Integer(sql_limit(limit))];
        vals.extend(b.vals);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT t.id, t.source, t.external_thread_id, t.account_email, t.subject, t.message_count, t.last_activity_ts_unix_ms, t.last_message_from, t.summary, t.status, t.summarize_status \
             FROM threads t WHERE t.status IN ('waiting_on_me', 'waiting_on_them'){conds} ORDER BY t.last_activity_ts_unix_ms DESC, t.id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params_from_iter(vals), Self::row_to_thread)?;
        rows.collect()
    }

    /// Every capture inside `f` (dates required), oldest first, for a question about a day. With a
    /// person: the captures that mention the name.
    pub fn scoped_timeline(&self, scope: &ScopeCtx<'_>) -> rusqlite::Result<Vec<CaptureItem>> {
        let f = &scope.filter;
        let mention = f.person.as_ref().and_then(|p| phrase_of(&p.name));
        if f.person.is_some() && mention.is_none() {
            return Ok(Vec::new());
        }
        let mut b = Binds::new(1);
        let conds = Self::capture_conds(&mut b, f, mention.as_deref());
        let mut stmt = self.conn.prepare(&format!(
            "SELECT i.id, i.device_id, i.ts_unix_ms, i.app_bundle_id, i.title, i.url, i.body_text, i.hash, i.source, NULL \
             FROM items i WHERE i.kind = 'capture'{conds} ORDER BY i.ts_unix_ms ASC"
        ))?;
        let rows = stmt.query_map(params_from_iter(b.vals), Self::row_to_item)?;
        rows.collect()
    }

    /// Every transcript line inside `f` (dates required), oldest first, for a question about a day.
    /// Hidden under an app; with a person only their lines and lines of meetings they attended.
    pub fn scoped_transcripts_in_window(&self, scope: &ScopeCtx<'_>) -> rusqlite::Result<Vec<(MeetingTranscriptSegment, i64)>> {
        let f = &scope.filter;
        if !f.allows(FindKind::Transcript) {
            return Ok(Vec::new());
        }
        let mut b = Binds::new(1);
        let mut conds = Self::dates(&mut b, f, "i.ts_unix_ms");
        if let Some(p) = scope.person.as_deref() {
            let attended = self.attended_ids(p)?;
            let keys = b.add(Value::Text(serde_json::to_string(&p.speaker_keys).unwrap_or_else(|_| "[]".into())));
            conds.push_str(&format!(
                " AND (i.thread_id IN ({attended}) OR EXISTS (SELECT 1 FROM meeting_speaker ms \
                 WHERE ms.meeting_id = i.thread_id AND 'Speaker ' || (ms.speaker_id + 1) = json_extract(i.actors, '$[0]') \
                 AND ms.person_key IN (SELECT value FROM json_each({keys}))))"
            ));
        }
        let mut stmt = self.conn.prepare(&format!(
            "SELECT i.id, i.thread_id, m.title, (i.ts_unix_ms - m.start_unix_ms) / 10, json_extract(i.actors, '$[0]'), i.body_text, i.ts_unix_ms \
             FROM items i JOIN meeting m ON m.id = i.thread_id WHERE i.kind = 'transcript'{conds} ORDER BY i.ts_unix_ms ASC"
        ))?;
        let rows = stmt.query_map(params_from_iter(b.vals), Self::row_to_transcript_segment_with_ts)?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::tests::{capture, meeting, store};
    use crate::NewMeetingSnapshot;

    const R0: i64 = 1_000_000;
    const R1: i64 = 2_000_000;

    fn email(s: &Store, ext: &str, thread: &str, from: &str, to: &str, ts: i64, body: &str) {
        s.conn
            .execute(
                "INSERT OR IGNORE INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, summarize_status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('gmail', ?1, 'me@x.com', ?2, 1, ?3, ?4, 'waiting_on_me', 'pending_summary', 1, 1)",
                rusqlite::params![thread, format!("Subject {thread}"), ts, from],
            )
            .unwrap();
        let actors = serde_json::to_string(&[from, to]).unwrap();
        let refs = serde_json::json!({"gmail_thread_id": thread, "account_email": "me@x.com"}).to_string();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) VALUES ('d', 'email', 'gmail', ?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![ts, actors, format!("Subject {thread}"), body, refs, ext],
            )
            .unwrap();
    }

    fn episode(s: &Store, app: &str, end: i64, text: &str) {
        s.conn
            .execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', ?1, 'Window', ?2, ?3, 1, 2, 2, ?4, 'summarized', 1, 1)",
                rusqlite::params![app, end - 1_000, end, text],
            )
            .unwrap();
    }

    fn run(s: &Store, ran_at: i64, text: &str) {
        s.conn
            .execute("INSERT OR IGNORE INTO routine (id, name, template, schedule_hhmm, created_at_unix_ms) VALUES (1, 'Focus', 'focus_time', '09:00', 1)", [])
            .unwrap();
        s.conn
            .execute(
                "INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json) VALUES (1, ?1, 'success', ?2, '{}')",
                rusqlite::params![ran_at, text],
            )
            .unwrap();
    }

    fn slide(s: &Store, meeting: i64, ts: i64, ocr: &str) {
        s.insert_meeting_snapshot(&NewMeetingSnapshot { meeting_id: meeting, captured_unix_ms: ts, file_path: format!("/s/{ts}.png"), width: 1, height: 1, ocr_text: ocr.into() })
            .unwrap()
            .unwrap();
    }

    /// Every kind, one inside `[R0, R1)` and one outside, all holding "pricing".
    struct World {
        _d: tempfile::TempDir,
        s: Store,
        m_in: i64,
        m_out: i64,
    }

    fn added_at(s: &Store, id: &str, ts: &str, title: &str, text: &str) -> i64 {
        let mut it = crate::mcp_items::tests::item("outlook-work", id);
        it.ts = ts.into();
        it.title = Some(title.into());
        it.text = text.into();
        crate::mcp_items::tests::add(s, 3, &[it]);
        crate::mcp_items::tests::row_id(s, 3, "outlook-work", id).unwrap()
    }

    fn added_store() -> (tempfile::TempDir, Store, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::mcp_items::tests::open(&dir);
        // 1970-01-01T00:25:00Z is 1_500_000 ms: inside [R0, R1). 00:50:00Z is 3_000_000: after.
        let a = added_at(&s, "in", "1970-01-01T00:25:00Z", "Quokka forecast", "quokka numbers for the quarter");
        let b = added_at(&s, "out", "1970-01-01T00:50:00Z", "Later", "quokka numbers again");
        s.reindex_batch(100).unwrap();
        (dir, s, a, b)
    }

    fn kinds_ids(p: &FindPage) -> Vec<(String, i64)> {
        p.results.iter().map(|c| (c.kind.clone(), c.id)).collect()
    }

    #[test]
    fn an_imported_note_is_listed_once_not_also_as_its_one_item_episode() {
        let (_d, s) = store();
        capture(&s, 1, 3_000_000, "import.markdown", "Kite festival checklist", "Kite festival checklist: tables, cash box, change.");
        let item: i64 = s.conn.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = 3000000", [], |r| r.get(0)).unwrap();
        let episode = |start: i64, end: i64, count: i64, ts: i64| {
            s.conn
                .execute(
                    "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES ('d', 'import.markdown', 'Kite festival checklist', ?1, ?1, ?2, ?3, ?4, 'Kite festival checklist', 'summarized', 1, 1)",
                    rusqlite::params![ts, start, end, count],
                )
                .unwrap();
            s.conn.last_insert_rowid()
        };
        let single = episode(item, item, 1, 3_000_000);
        let several = episode(item, item + 5, 3, 3_100_000);
        for order in [FindOrder::Rank, FindOrder::Time] {
            let got = kinds_ids(&s.find("kite festival checklist", &FindFilter::default(), order, 0, 20).unwrap());
            assert!(got.iter().any(|(k, _)| k == "capture"), "{order:?}: {got:?}");
            assert!(!got.contains(&("episode".into(), single)), "{order:?}: the one-item episode repeats its capture: {got:?}");
            assert!(got.contains(&("episode".into(), several)), "{order:?}: an episode over several items stays: {got:?}");
        }
    }

    #[test]
    fn find_lists_added_items_by_keyword_and_meaning() {
        let (_d, s, a, b) = added_store();
        let page = s.find("quokka", &FindFilter::default(), FindOrder::Rank, 0, 20).unwrap();
        let got = kinds_ids(&page);
        assert!(got.contains(&("mcp_item".into(), a)) && got.contains(&("mcp_item".into(), b)), "{got:?}");
        let kw = s.search_mcp_items("forecast", false, 10).unwrap();
        assert_eq!(kw.iter().map(|h| h.id).collect::<Vec<_>>(), vec![a]);
        let meaning = s.search_mcp_items("quokka numbers for the quarter", true, 10).unwrap();
        assert_eq!(meaning.first().map(|h| h.id), Some(a), "{meaning:?}");
        let hit = &kw[0];
        assert_eq!((hit.declared_kind.as_str(), hit.client.as_str(), hit.source_label.as_str()), ("email", "Claude Code", "outlook-work"));
        assert_eq!(hit.title.as_deref(), Some("Quokka forecast"));
        assert_eq!(hit.url.as_deref(), Some("https://outlook.office.com/mail/1"));
    }

    #[test]
    fn an_app_or_person_filter_hides_added_items() {
        let (_d, s, _a, _b) = added_store();
        let app = FindFilter { app_bundle_id: Some("com.slack".into()), ..Default::default() };
        let person = FindFilter { person: Some(PersonFilter { name: "Ana Ruiz".into(), person_key: None }), ..Default::default() };
        for f in [app, person] {
            assert!(!f.allows(FindKind::McpItem));
            let page = s.find("quokka", &f, FindOrder::Rank, 0, 20).unwrap();
            assert!(page.results.iter().all(|c| c.kind != "mcp_item"), "{:?}", kinds_ids(&page));
            let scope = s.scope_ctx(&f).unwrap();
            assert!(s.scoped_mcp_item_hits("quokka", false, &scope, 10).unwrap().is_empty());
        }
    }

    #[test]
    fn a_date_filter_keeps_added_items_in_range() {
        let (_d, s, a, _b) = added_store();
        let f = FindFilter { start_unix_ms: Some(R0), end_unix_ms: Some(R1), ..Default::default() };
        let page = s.find("quokka", &f, FindOrder::Rank, 0, 20).unwrap();
        assert_eq!(kinds_ids(&page), vec![("mcp_item".to_string(), a)]);
        let listing = s.find("", &f, FindOrder::Time, 0, 20).unwrap();
        assert_eq!(kinds_ids(&listing), vec![("mcp_item".to_string(), a)]);
        assert_eq!(listing.counts.get("mcp_item"), Some(&1));
        let scope = s.scope_ctx(&f).unwrap();
        assert_eq!(s.scoped_mcp_item_hits("quokka", false, &scope, 10).unwrap().iter().map(|h| h.id).collect::<Vec<_>>(), vec![a]);
        assert_eq!(s.scoped_mcp_item_hits("quokka numbers", true, &scope, 10).unwrap().iter().map(|h| h.id).collect::<Vec<_>>(), vec![a]);
    }

    #[test]
    fn window_reads_added_items_by_their_own_time() {
        let (_d, s, a, b) = added_store();
        assert_eq!(s.mcp_items_in_window(R0, R1, 10).unwrap().iter().map(|h| h.id).collect::<Vec<_>>(), vec![a]);
        assert_eq!(s.mcp_items_in_window(0, 10_000_000, 10).unwrap().iter().map(|h| h.id).collect::<Vec<_>>(), vec![b, a], "newest first");
        assert!(s.mcp_items_in_window(R1, R1 + 1, 10).unwrap().is_empty());
    }

    fn world() -> World {
        let (d, s) = store();
        capture(&s, 1, 1_500_000, "com.slack", "in slack", "pricing review alpha");
        capture(&s, 2, 500_000, "com.slack", "before", "pricing review before");
        capture(&s, 3, 2_500_000, "com.slack", "after", "pricing review after");
        capture(&s, 4, 1_600_000, "com.notes", "in notes", "pricing review notes");
        capture(&s, 5, 1_700_000, "com.slack", "sam", "lunch with Sam Okafor about pricing");
        capture(&s, 6, 1_710_000, "com.slack", "reversed", "Okafor and Sam disagree about pricing");
        let m_in = meeting(&s, 1_200_000, "Planning", &["Sam Okafor", "Ana"], &["pricing starts here", "pricing second line"]);
        let m_out = meeting(&s, 5_000_000, "Later", &["Sam Okafor"], &["pricing later"]);
        let other = meeting(&s, 1_300_000, "Standup", &["Ana"], &["pricing in the standup"]);
        let _ = other;
        slide(&s, m_in, 1_200_500, "pricing slide in");
        slide(&s, m_out, 5_000_500, "pricing slide out");
        s.upsert_person("name:sam", "Sam Okafor", Some("sam@x.com")).unwrap();
        email(&s, "e1", "t-in", "sam@x.com", "me@x.com", 1_700_000, "pricing mail in");
        email(&s, "e2", "t-out", "sam@x.com", "me@x.com", 9_000_000, "pricing mail out");
        email(&s, "e3", "t-other", "zed@x.com", "me@x.com", 1_750_000, "pricing mail other");
        episode(&s, "com.slack", 1_800_000, "pricing episode in");
        episode(&s, "com.slack", 9_800_000, "pricing episode out");
        episode(&s, "com.notes", 1_810_000, "pricing episode notes");
        run(&s, 1_900_000, "pricing run in");
        run(&s, 9_900_000, "pricing run out");
        World { _d: d, s, m_in, m_out }
    }

    fn range() -> FindFilter {
        FindFilter { start_unix_ms: Some(R0), end_unix_ms: Some(R1), ..Default::default() }
    }

    fn all_cards(p: &FindPage) -> Vec<&SourceCard> {
        p.results.iter().chain(p.mentions.iter()).collect()
    }

    #[test]
    fn a_date_filter_wins_over_a_window() {
        let dated = FindFilter { start_unix_ms: Some(10), ..Default::default() };
        assert_eq!(dated.or_window(1, 5), dated, "one side set: the window is ignored whole");
        let open = FindFilter { app_bundle_id: Some("a".into()), ..Default::default() };
        let w = open.or_window(1, 5);
        assert_eq!((w.start_unix_ms, w.end_unix_ms, w.app_bundle_id.as_deref()), (Some(1), Some(5), Some("a")));
    }

    #[test]
    fn the_filter_table_is_encoded_once() {
        let none = FindFilter::default();
        let app = FindFilter { app_bundle_id: Some("a".into()), ..Default::default() };
        let person = FindFilter { person: Some(PersonFilter { name: "Sam".into(), person_key: None }), ..Default::default() };
        let blank = FindFilter { app_bundle_id: Some("  ".into()), person: Some(PersonFilter { name: " ".into(), person_key: None }), ..Default::default() };
        for k in [FindKind::Capture, FindKind::Meeting, FindKind::Transcript, FindKind::Snapshot, FindKind::Thread, FindKind::Episode, FindKind::RoutineRun] {
            assert!(none.allows(k) && blank.allows(k), "{k:?}");
        }
        assert!(blank.is_empty());
        let shown: Vec<FindKind> = [FindKind::Capture, FindKind::Meeting, FindKind::Transcript, FindKind::Snapshot, FindKind::Thread, FindKind::Episode, FindKind::RoutineRun]
            .into_iter()
            .filter(|k| app.allows(*k))
            .collect();
        assert_eq!(shown, vec![FindKind::Capture], "an app hides every kind but captures");
        let shown: Vec<FindKind> = [FindKind::Capture, FindKind::Meeting, FindKind::Transcript, FindKind::Snapshot, FindKind::Thread, FindKind::Episode, FindKind::RoutineRun]
            .into_iter()
            .filter(|k| person.allows(*k))
            .collect();
        assert_eq!(shown, vec![FindKind::Capture, FindKind::Meeting, FindKind::Transcript, FindKind::Snapshot, FindKind::Thread]);
        let both = FindFilter { app_bundle_id: Some("a".into()), person: person.person.clone(), ..Default::default() };
        assert!(both.allows(FindKind::Capture) && !both.allows(FindKind::Episode) && !both.allows(FindKind::Transcript));
        assert!(!app.allows(FindKind::Episode));
    }

    #[test]
    fn a_date_filter_keeps_every_kind_inside_its_range() {
        let w = world();
        let page = w.s.find("pricing", &range(), FindOrder::Time, 0, 100).unwrap();
        let kinds: HashSet<&str> = all_cards(&page).iter().map(|c| c.kind.as_str()).collect();
        for k in ["capture", "transcript", "snapshot", "thread", "episode", "routine_run"] {
            assert!(kinds.contains(k), "{k} missing: {kinds:?}");
        }
        for c in all_cards(&page) {
            let ts = c.ts_unix_ms.unwrap();
            assert!((R0..R1).contains(&ts), "{} {} at {ts} is outside the range", c.kind, c.id);
            if let Some(start) = c.meeting_start_unix_ms {
                assert!(start < R1 && start >= R0 || c.kind == "transcript");
            }
        }
        // The same query unfiltered finds the outside ones too.
        let all = w.s.find("pricing", &FindFilter::default(), FindOrder::Time, 0, 200).unwrap();
        assert!(all.results.iter().any(|c| c.ts_unix_ms.unwrap() >= R1));
        // Time order is newest first.
        let times: Vec<i64> = page.results.iter().map(|c| c.ts_unix_ms.unwrap()).collect();
        assert!(times.windows(2).all(|p| p[0] >= p[1]), "{times:?}");
    }

    #[test]
    fn an_app_hides_every_kind_but_captures_of_that_app() {
        let w = world();
        let f = FindFilter { app_bundle_id: Some("com.slack".into()), ..range() };
        let page = w.s.find("pricing", &f, FindOrder::Time, 0, 100).unwrap();
        for c in &page.results {
            assert_eq!(c.kind, "capture");
            assert_eq!(c.app_bundle_id.as_deref(), Some("com.slack"));
            assert!((R0..R1).contains(&c.ts_unix_ms.unwrap()));
        }
        assert!(!page.results.is_empty());
        for hidden in ["transcript", "snapshot", "thread", "episode", "routine_run", "meeting"] {
            assert!(!page.counts.contains_key(hidden), "{hidden}");
        }
        // An app with no dates reaches every capture, not only the last 30 days.
        let f = FindFilter { app_bundle_id: Some("com.slack".into()), ..Default::default() };
        let page = w.s.find("review", &f, FindOrder::Time, 0, 100).unwrap();
        // Meaning search also lists the nearest captures, so these three are there among the app's captures, all of that app.
        let got: Vec<&str> = page.results.iter().map(|c| c.title.as_str()).collect();
        for want in ["before", "in slack", "after"] {
            assert!(got.contains(&want), "{want} missing: {got:?}");
        }
        assert!(page.results.iter().all(|c| c.kind == "capture" && c.app_bundle_id.as_deref() == Some("com.slack")));
    }

    #[test]
    fn a_person_finds_lines_slides_and_mail_by_the_table_and_mentions_by_phrase() {
        let w = world();
        // Ana's lines are not Sam's. One of Sam's meeting lines is also spoken by Sam in a meeting Sam did not attend.
        let spoken_in = meeting(&w.s, 1_400_000, "Hallway", &["Ana"], &["pricing from the hallway", "an unrelated remark"]);
        w.s.set_meeting_speaker(spoken_in, 0, Some("name:sam"), Some(crate::speaker_names::NameStatus::Confirmed)).unwrap();
        let f = FindFilter { person: Some(PersonFilter { name: "  sam okafor ".into(), person_key: Some("name:sam".into()) }), ..range() };
        let page = w.s.find("pricing", &f, FindOrder::Time, 0, 100).unwrap();
        let of = |kind: &str| -> Vec<&SourceCard> { page.results.iter().filter(|c| c.kind == kind).collect() };
        // Lines: both of the planning meeting (attended), the hallway line Sam spoke; not the standup, not the later meeting.
        let lines: Vec<(&str, Option<&str>)> = of("transcript").iter().map(|c| (c.text.as_deref().unwrap(), c.via.as_deref())).collect();
        assert!(lines.contains(&("pricing starts here", Some("attendee"))), "{lines:?}");
        assert!(lines.contains(&("pricing second line", Some("attendee"))), "{lines:?}");
        assert!(lines.contains(&("pricing from the hallway", Some("speaker"))), "{lines:?}");
        assert!(!lines.iter().any(|(t, _)| t.contains("standup") || t.contains("later")), "{lines:?}");
        // Slides: of the meeting Sam attended and inside the dates.
        assert_eq!(of("snapshot").len(), 1);
        assert_eq!(of("snapshot")[0].meeting_id, Some(w.m_in));
        // Email: by the address on record: Sam's mail in range, not Zed's, not the old one.
        assert_eq!(of("thread").iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), vec!["Subject t-in"]);
        // Episodes and runs are hidden; plain captures are listed apart as mentions.
        assert!(of("episode").is_empty() && of("routine_run").is_empty() && of("capture").is_empty());
        let mention_texts: Vec<&str> = page.mentions.iter().map(|c| c.text.as_deref().unwrap()).collect();
        assert_eq!(mention_texts, vec!["lunch with Sam Okafor about pricing"], "a phrase match, not an OR of the words: {mention_texts:?}");
        assert_eq!(page.mentions_total, 1);
        assert_eq!(page.counts.get("mentions"), Some(&1));
        let _ = w.m_out;
    }

    #[test]
    fn email_by_person_falls_back_to_the_sender_name_and_never_guesses() {
        let (_d, s) = store();
        email(&s, "e1", "t1", "Pat Quill <pat@elsewhere.example>", "me@x.com", 10, "pricing note");
        email(&s, "e2", "t2", "other@x.com", "me@x.com", 11, "pricing other");
        let f = |name: &str| FindFilter { person: Some(PersonFilter { name: name.into(), person_key: None }), ..Default::default() };
        // No address on record: the name in the sender matches.
        let hits = s.find("pricing", &f("Pat Quill"), FindOrder::Time, 0, 10).unwrap();
        assert_eq!(hits.results.iter().filter(|c| c.kind == "thread").count(), 1);
        // No address and no name match: nothing, not a guess.
        let none = s.find("pricing", &f("Nobody Here"), FindOrder::Time, 0, 10).unwrap();
        assert!(none.results.iter().all(|c| c.kind != "thread"), "{none:?}");
        // An address on record is used instead of the name.
        s.upsert_person("pat@elsewhere.example", "Pat Quill", Some("pat@elsewhere.example")).unwrap();
        let hits = s.find("pricing", &f("Pat Quill"), FindOrder::Time, 0, 10).unwrap();
        assert_eq!(hits.results.iter().filter(|c| c.kind == "thread").count(), 1);
    }

    #[test]
    fn meetings_attended_use_the_trimmed_case_insensitive_whole_string_rule() {
        let (_d, s) = store();
        let a = meeting(&s, 100, "A", &["  sam okafor "], &["x"]);
        let _b = meeting(&s, 200, "B", &["Sam Okafor Jr"], &["x"]);
        let c = meeting(&s, 300, "C", &["SAM OKAFOR", "Ana"], &["x"]);
        let got: Vec<i64> = s.meetings_attended_by("Sam Okafor", None, None).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(got, vec![c, a], "newest first, whole string only");
        assert_eq!(s.meetings_attended_by("Sam Okafor", Some(150), Some(1000)).unwrap().len(), 1);
        assert!(s.meetings_attended_by("  ", None, None).unwrap().is_empty());
        // With an empty query, a person lists the meetings they attended, by time.
        let f = FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: None }), ..Default::default() };
        let page = s.find("", &f, FindOrder::Time, 0, 10).unwrap();
        assert_eq!(page.results.iter().map(|c| (c.kind.as_str(), c.id)).collect::<Vec<_>>(), vec![("meeting", c), ("meeting", a)]);
    }

    #[test]
    fn an_empty_query_lists_by_time_and_nothing_without_a_filter() {
        let (_d, s) = store();
        for i in 0..25 {
            capture(&s, i, 1_000 + i * 10, if i % 2 == 0 { "app.a" } else { "app.b" }, &format!("t{i}"), &format!("body {i}"));
        }
        assert_eq!(s.find("", &FindFilter::default(), FindOrder::Time, 0, 10).unwrap(), FindPage::default());
        assert_eq!(s.find("   ", &FindFilter::default(), FindOrder::Rank, 0, 10).unwrap().results.len(), 0);
        let f = FindFilter { start_unix_ms: Some(1_000), end_unix_ms: Some(1_200), app_bundle_id: Some("app.a".into()), ..Default::default() };
        let p1 = s.find("", &f, FindOrder::Time, 0, 5).unwrap();
        let p2 = s.find("", &f, FindOrder::Time, 5, 5).unwrap();
        let p3 = s.find("", &f, FindOrder::Time, 10, 5).unwrap();
        // Captures i = 0..20, even: 10 of them, newest first, exact count, not pool limited.
        assert_eq!((p1.total, p1.counts["capture"], p1.results.len(), p2.results.len(), p3.results.len()), (10, 10, 5, 5, 0));
        assert!(p1.more && !p2.more);
        let ts: Vec<i64> = p1.results.iter().chain(&p2.results).map(|c| c.ts_unix_ms.unwrap()).collect();
        assert_eq!(ts, vec![1_180, 1_160, 1_140, 1_120, 1_100, 1_080, 1_060, 1_040, 1_020, 1_000]);
        assert!(!p1.semantic_used, "an empty query uses no model");
    }

    #[test]
    fn pages_are_stable_and_complete_in_both_orders() {
        let (_d, s) = store();
        for i in 0..30 {
            capture(&s, i, 1_000 + i, "app", &format!("t{i}"), &format!("paging word {i}"));
        }
        for order in [FindOrder::Time, FindOrder::Rank] {
            let mut seen = Vec::new();
            for page in 0..4 {
                let p = s.find("paging", &FindFilter::default(), order, page * 10, 10).unwrap();
                assert_eq!(p.total, 30, "{order:?}");
                seen.extend(p.results.iter().map(|c| c.id));
            }
            assert_eq!(seen.len(), 30, "{order:?}");
            assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 30, "no duplicates across pages: {order:?}");
            let again: Vec<i64> = (0..3).flat_map(|p| s.find("paging", &FindFilter::default(), order, p * 10, 10).unwrap().results.into_iter().map(|c| c.id)).collect();
            assert_eq!(again, seen[..30].to_vec(), "the same pages again: {order:?}");
        }
        let time: Vec<i64> = s.find("paging", &FindFilter::default(), FindOrder::Time, 0, 30).unwrap().results.iter().map(|c| c.ts_unix_ms.unwrap()).collect();
        assert!(time.windows(2).all(|p| p[0] >= p[1]));
    }

    /// Unit vectors by topic: money words point one way, animals another, everything else a third, so a query's meaning is near
    /// only the notes on its own topic (as with the real model, which normalises its vectors).
    struct TopicEmbedder;
    impl crate::Embedder for TopicEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, crate::EmbeddingError> {
            let t = text.to_lowercase();
            let mut v = vec![0.0f32; crate::EMBEDDING_OUTPUT_DIM];
            if t.contains("coffee") {
                // A little like money (cosine 0.29 with it): between the two floors.
                v[0] = 0.3;
                v[4] = 0.954;
                return Ok(v);
            }
            let axis = if ["budget", "forecast", "finance", "spend"].iter().any(|w| t.contains(w)) {
                0
            } else if ["zebra", "giraffe"].iter().any(|w| t.contains(w)) {
                1
            } else {
                2
            };
            v[axis] = 0.96;
            v[3] = 0.28;
            Ok(v)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn meaning_search_finds_nothing_for_words_unrelated_to_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("m.db"), std::sync::Arc::new(TopicEmbedder));
        for i in 0..20 {
            capture(&s, i + 1, 10 + i, "app", "t", &format!("weekly standup notes {i}"));
        }
        let money = capture(&s, 100, 50, "app", "t", "finance review of the quarter");
        let p = s.find("zebra quantum harmonica", &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap();
        assert!(p.semantic_used);
        assert!(p.results.is_empty(), "unrelated words found {} rows", p.results.len());
        assert_eq!(p.total, 0);
        // Under a filter the floor holds too.
        let app = FindFilter { app_bundle_id: Some("app".into()), ..FindFilter::default() };
        assert!(s.find("zebra quantum harmonica", &app, FindOrder::Time, 0, 50).unwrap().results.is_empty());
        // A question whose meaning is near a note still finds it with no word in common.
        let p = s.find("budget", &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap();
        assert_eq!(p.results.iter().map(|c| c.id).collect::<Vec<_>>(), vec![money]);
        // And a word that is in the notes still finds them by keyword.
        assert_eq!(s.find("standup", &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap().results.len(), 20);
    }

    #[test]
    fn a_shared_word_does_not_lower_the_meaning_floor() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("m.db"), std::sync::Arc::new(TopicEmbedder));
        let item_at = |ts: i64| -> i64 { s.conn.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = ?1", [ts], |r| r.get(0)).unwrap() };
        capture(&s, 1, 10, "app", "t", "finance review of the quarter");
        capture(&s, 2, 20, "app", "t", "coffee with the team");
        let (money, _coffee) = (item_at(10), item_at(20));
        let ids = |q: &str| {
            let mut v: Vec<i64> = s.find(q, &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap().results.iter().map(|c| c.id).collect();
            v.sort();
            v
        };
        // Only the close meaning counts, not the one that is a little like it (0.29).
        assert_eq!(ids("budget"), vec![money]);
        // A note holding the word is found by keyword, and the row a little like the query still is not.
        capture(&s, 3, 30, "app", "t", "the budget sheet");
        assert_eq!(ids("budget"), vec![money, item_at(30)]);
        // A query of common words only needs a row nearly the same words.
        assert_eq!(meaning_floor_for("Where is the"), MEANING_COMMON_ONLY_MIN_SIMILARITY);
        assert_eq!(meaning_floor_for("the budget"), MEANING_SHORT_MIN_SIMILARITY);
        assert_eq!(meaning_floor_for("budget for the trip"), MEANING_MIN_SIMILARITY);
    }

    /// No model: Find searches by keyword alone.
    struct NoModel;
    impl crate::Embedder for NoModel {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, crate::EmbeddingError> {
            Err(crate::EmbeddingError::ModelFilesMissing(std::path::PathBuf::new()))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn common_words_are_not_keywords_in_any_shipped_language() {
        assert_eq!(keyword_words("Where is the kite festival?"), ["kite", "festival"]);
        assert_eq!(keyword_words("accessibility for blind users"), ["accessibility", "blind", "users"]);
        assert_eq!(keyword_words("¿Dónde está la venta de pasteles?"), ["venta", "pasteles"]);
        assert_eq!(keyword_words("Où est le festival ? L'école d'été"), ["festival", "école", "été"]);
        assert_eq!(keyword_words("Wo ist der Parkplatz für die Gäste"), ["parkplatz", "die", "gäste"]);
        assert_eq!(keyword_words("Onde está o dinheiro da venda"), ["dinheiro", "venda"]);
        assert_eq!(keyword_words("Kite log 42, kite LOG 4"), ["kite", "log", "42", "4"]);
        for only_common in ["the", "for", "THE", "Where is it?", "what is the", "a e o", "de la", "der das", "est-ce que", ""] {
            if only_common == "Where is it?" {
                assert_eq!(keyword_words(only_common), ["it"], "\"it\" alone is a word in its own right (IT)");
                continue;
            }
            assert!(keyword_words(only_common).is_empty(), "{only_common:?}: {:?}", keyword_words(only_common));
        }
        // A common word in one language that is a real word in another stays a keyword.
        for w in ["car", "war", "die", "son", "sin", "man", "pour", "mit", "con", "will", "may", "us", "it", "also"] {
            assert_eq!(keyword_words(w), [w], "{w}");
        }
        // "it", "will", "may" and the like are left out beside another word, kept alone.
        assert_eq!(keyword_words("is it going to rain"), ["going", "rain"]);
        assert_eq!(keyword_words("what will Sam say"), ["sam", "say"]);
        assert_eq!(keyword_words("IT"), ["it"]);
    }

    #[test]
    fn a_common_word_matches_nothing_by_keyword() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("m.db"), std::sync::Arc::new(NoModel));
        for n in 1..=30 {
            capture(&s, n, 1_000 + n, "com.notes", &format!("Kite log {n}"), &format!("Kite log {n}. Flew for {n} minutes."));
        }
        let reader = capture(&s, 100, 5_000, "com.notes", "a11y", "Screen reader users could not reach the settings button");
        let blind = capture(&s, 101, 5_001, "com.notes", "a11y", "Blind people need every button named");
        let ids = |q: &str| {
            let p = s.find(q, &FindFilter::default(), FindOrder::Rank, 0, 100).unwrap();
            assert!(!p.semantic_used);
            let mut v: Vec<i64> = p.results.iter().map(|c| c.id).collect();
            v.sort();
            v
        };
        let mut both = vec![reader, blind];
        both.sort();
        // "for" is in every kite log; only the rows with "blind" or "users" match.
        assert_eq!(ids("accessibility for blind users"), both);
        // A query of common words only finds nothing by keyword.
        for q in ["for", "the", "Where is the", "for the"] {
            assert!(ids(q).is_empty(), "{q}");
        }
        // The other words of a question still find their rows.
        assert_eq!(ids("Where is the kite log?").len(), 30);
        // Thirty rows holding "it" do not answer a question about rain.
        for n in 1..=30 {
            capture(&s, 200 + n, 9_000 + n, "com.notes", "note", &format!("it was fine on day {n}"));
        }
        let rain = capture(&s, 300, 9_500, "com.notes", "note", "rain all afternoon");
        let _ = rain;
        assert_eq!(ids("is it going to rain").len(), 1);
    }

    #[test]
    fn a_weak_keyword_hit_does_not_push_out_a_strong_meaning_hit() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("m.db"), std::sync::Arc::new(TopicEmbedder));
        let item_at = |ts: i64| -> i64 { s.conn.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = ?1", [ts], |r| r.get(0)).unwrap() };
        // Oldest: the row the query is about, with no word of it.
        capture(&s, 1, 10, "app", "t", "finance forecast for the quarter");
        let finance = item_at(10);
        // Newer rows that share one word of the query and nothing of its meaning, in captures and in meeting lines.
        for i in 0..3 {
            capture(&s, 10 + i, 100 + i, "app", "t", &format!("weekly review notes {i}"));
            meeting(&s, 1_000 + i * 100_000, "Sync", &["Ana"], &["review of the slides"]);
        }
        let p = s.find("budget review", &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap();
        assert_eq!(p.results.len(), 7);
        assert_eq!((p.results[0].kind.as_str(), p.results[0].id), ("capture", finance), "the meaning hit leads");
        // A row that holds every word and the meaning comes before both.
        capture(&s, 50, 5, "app", "t", "budget review sheet");
        let sheet = item_at(5);
        let p = s.find("budget review", &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap();
        assert_eq!(p.results.iter().take(2).map(|c| c.id).collect::<Vec<_>>(), vec![sheet, finance]);
    }

    #[test]
    fn an_episode_holding_every_word_is_not_sunk_for_having_no_meaning() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("m.db"), std::sync::Arc::new(TopicEmbedder));
        // Newer rows that hold one word of the query and none of its meaning.
        for i in 0..3 {
            capture(&s, 10 + i, 100 + i, "app", "t", &format!("weekly planning notes {i}"));
        }
        // Older, no search index row and no embedding, holding every word.
        episode(&s, "app", 50, "budget planning session");
        let p = s.find("budget planning", &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap();
        assert_eq!(p.results.len(), 4);
        assert_eq!(p.results[0].kind, "episode", "{:?}", p.results.iter().map(|c| (&c.kind, c.id)).collect::<Vec<_>>());
    }

    #[test]
    fn keyword_and_meaning_hits_are_merged_without_duplicates_and_say_whether_meaning_ran() {
        let (_d, s) = store();
        let id = capture(&s, 1, 10, "app", "t", "the budget review");
        let p = s.find("budget", &FindFilter::default(), FindOrder::Rank, 0, 10).unwrap();
        assert!(p.semantic_available && p.semantic_used);
        assert_eq!(p.results.iter().filter(|c| c.id == id).count(), 1, "a row both methods found appears once");
    }

    /// A few thousand rows: the person rule reads the meetings once, every leg is one query, and
    /// the page's cards are a few batched reads.
    #[test]
    fn a_large_store_is_searched_without_per_row_queries() {
        let (_d, s) = store();
        for i in 0..3_000 {
            capture(&s, i, 1_000 + i, "app", &format!("t{i}"), &format!("budget capture number {i} Sam Okafor"));
        }
        for m in 0..300 {
            let attendees: &[&str] = if m % 3 == 0 { &["Sam Okafor"] } else { &["Ana"] };
            meeting(&s, 10_000 + m * 100, &format!("M{m}"), attendees, &["budget a", "budget b", "budget c"]);
        }
        let f = FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: None }), ..Default::default() };
        let start = std::time::Instant::now();
        let page = s.find("budget", &f, FindOrder::Time, 0, 50).unwrap();
        let rows = s.scoped_transcript_hits("budget", false, &s.scope_ctx(&f).unwrap(), 20).unwrap();
        let caps = s.scoped_capture_hits("budget", false, &s.scope_ctx(&f).unwrap(), 20).unwrap();
        let elapsed = start.elapsed();
        assert!(page.results.iter().all(|c| c.kind == "transcript") && page.total >= 300);
        assert_eq!((rows.len(), caps.len()), (20, 20));
        assert!(page.mentions_total >= 400 && page.capped, "mentions reach the pool: {}", page.mentions_total);
        // A ceiling for a per-row-query regression, not a benchmark: the shared Windows runner took 20.2 s in a
        // debug build with the suite in parallel, so 20 s failed on unrelated PRs.
        assert!(elapsed < std::time::Duration::from_secs(60), "{elapsed:?}");
        // The listing path counts exactly.
        let listed = s.find("", &FindFilter { start_unix_ms: Some(1_000), end_unix_ms: Some(2_000), ..Default::default() }, FindOrder::Time, 0, 200).unwrap();
        assert_eq!((listed.total, listed.results.len()), (1_000, 200));
    }


    // ---- review findings ---------------------------------------------------------------------------

    fn email_to(s: &Store, ext: &str, thread: &str, actors: &[&str], ts: i64, body: &str) {
        s.conn
            .execute(
                "INSERT OR IGNORE INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, summarize_status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('gmail', ?1, 'me@x.com', ?2, 1, ?3, ?4, 'waiting_on_me', 'pending_summary', 1, 1)",
                rusqlite::params![thread, format!("Subject {thread}"), ts, actors[0]],
            )
            .unwrap();
        let refs = serde_json::json!({"gmail_thread_id": thread, "account_email": "me@x.com"}).to_string();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) VALUES ('d', 'email', 'gmail', ?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![ts, serde_json::to_string(actors).unwrap(), format!("Subject {thread}"), body, refs, ext],
            )
            .unwrap();
    }

    fn who(name: &str, key: Option<&str>) -> Option<PersonFilter> {
        Some(PersonFilter { name: name.into(), person_key: key.map(Into::into) })
    }

    fn kind_of(tag: &str) -> FindKind {
        [FindKind::Capture, FindKind::Meeting, FindKind::Transcript, FindKind::Snapshot, FindKind::Thread, FindKind::Episode, FindKind::RoutineRun]
            .into_iter()
            .find(|k| k.as_str() == tag)
            .unwrap_or_else(|| panic!("unknown kind tag {tag}"))
    }

    /// Every filter combination the table speaks of.
    fn every_combination() -> Vec<(&'static str, FindFilter)> {
        let app = Some("com.slack".to_string());
        let person = who("Sam Okafor", Some("name:sam"));
        let dated = || FindFilter { start_unix_ms: Some(R0), end_unix_ms: Some(R1), ..Default::default() };
        vec![
            ("none", FindFilter::default()),
            ("date", dated()),
            ("app", FindFilter { app_bundle_id: app.clone(), ..Default::default() }),
            ("person", FindFilter { person: person.clone(), ..Default::default() }),
            ("app+person", FindFilter { app_bundle_id: app.clone(), person: person.clone(), ..Default::default() }),
            ("date+person", FindFilter { person: person.clone(), ..dated() }),
            ("date+app", FindFilter { app_bundle_id: app.clone(), ..dated() }),
            ("all three", FindFilter { app_bundle_id: app, person, ..dated() }),
        ]
    }

    #[test]
    fn no_kind_bypasses_the_table_for_find_in_any_filter_combination() {
        let w = world();
        for (name, f) in every_combination() {
            for query in ["pricing", ""] {
                if query.is_empty() && f.is_empty() {
                    continue;
                }
                let page = w.s.find(query, &f, FindOrder::Time, 0, 200).unwrap();
                for c in &page.results {
                    let k = kind_of(&c.kind);
                    assert!(f.allows(k), "{name} / {query:?}: a {} result is hidden by the table", c.kind);
                    assert!(!(k == FindKind::Capture && f.person.is_some()), "{name} / {query:?}: captures are mentions under a person");
                }
                for key in page.counts.keys().filter(|k| *k != "mentions") {
                    assert!(f.allows(kind_of(key)), "{name} / {query:?}: count for hidden kind {key}");
                }
                assert!(page.mentions.is_empty() || f.person.is_some(), "{name}");
                if f.person.is_some() {
                    assert!(page.mentions.iter().all(|c| c.kind == "capture"), "{name}");
                }
            }
        }
    }

    #[test]
    fn no_kind_bypasses_the_table_for_the_ask_retrieval_in_any_filter_combination() {
        let w = world();
        for (name, f) in every_combination() {
            let s = &w.s;
            let probes: Vec<(&str, FindKind, usize)> = vec![
                ("capture hits", FindKind::Capture, s.scoped_capture_hits("pricing", false, &s.scope_ctx(&f).unwrap(), 20).unwrap().len()),
                ("transcript hits", FindKind::Transcript, s.scoped_transcript_hits("pricing", false, &s.scope_ctx(&f).unwrap(), 20).unwrap().len()),
                ("snapshot hits", FindKind::Snapshot, s.scoped_snapshot_hits("pricing", false, &s.scope_ctx(&f).unwrap(), 20).unwrap().len()),
                ("email hits", FindKind::Thread, s.scoped_email_hits("pricing", &s.scope_ctx(&f).unwrap(), 20).unwrap().len()),
                ("episodes", FindKind::Episode, s.scoped_episodes(&s.scope_ctx(&f).unwrap(), 10).unwrap().len()),
                ("runs", FindKind::RoutineRun, s.scoped_routine_runs(&s.scope_ctx(&f).unwrap(), 10).unwrap().len()),
                ("waiting threads", FindKind::Thread, s.scoped_waiting_threads(&s.scope_ctx(&f).unwrap(), 10).unwrap().len()),
                ("timeline", FindKind::Capture, s.scoped_timeline(&s.scope_ctx(&f).unwrap()).unwrap().len()),
                ("window lines", FindKind::Transcript, s.scoped_transcripts_in_window(&s.scope_ctx(&f).unwrap()).unwrap().len()),
            ];
            for (what, kind, n) in probes {
                if f.allows(kind) {
                    assert!(n > 0, "{name}: {what} should find the world's rows");
                } else {
                    assert_eq!(n, 0, "{name}: {what} is a {kind:?} source and the table hides it");
                }
            }
        }
    }

    #[test]
    fn person_names_with_accents_are_matched_whatever_the_case() {
        let (_d, s) = store();
        s.upsert_person("name:emile", "Émile Zola", None).unwrap();
        let spoken = meeting(&s, 1_000, "Salon", &["Ana"], &["pricing from the salon", "other"]);
        s.set_meeting_speaker(spoken, 0, Some("name:emile"), Some(crate::speaker_names::NameStatus::Confirmed)).unwrap();
        meeting(&s, 2_000, "Cafe", &["ÉMILE ZOLA"], &["pricing at the cafe"]);
        email_to(&s, "e1", "t1", &["Émile Zola <ez@elsewhere.example>", "me@x.com"], 10, "pricing note");
        capture(&s, 1, 20, "app", "c", "met Émile Zola about pricing");
        let f = FindFilter { person: who("émile ZOLA", None), ..Default::default() };
        let page = s.find("pricing", &f, FindOrder::Time, 0, 50).unwrap();
        let lines: Vec<(&str, Option<&str>)> = page.results.iter().filter(|c| c.kind == "transcript").map(|c| (c.text.as_deref().unwrap(), c.via.as_deref())).collect();
        assert!(lines.contains(&("pricing from the salon", Some("speaker"))), "{lines:?}");
        assert!(lines.contains(&("pricing at the cafe", Some("attendee"))), "{lines:?}");
        assert_eq!(page.results.iter().filter(|c| c.kind == "thread").count(), 1, "the sender name, folded");
        assert_eq!(page.mentions.len(), 1);
    }

    #[test]
    fn names_with_apostrophes_and_hyphens_match_as_token_sequences() {
        let (_d, s) = store();
        email_to(&s, "e1", "t1", &["Jean-Luc O'Brien <jl@x.example>", "me@x.com"], 10, "pricing one");
        email_to(&s, "e2", "t2", &["Jean Luc <jean@x.example>", "me@x.com"], 11, "pricing two");
        email_to(&s, "e3", "t3", &["jean-luc o'brien-smith <js@x.example>", "me@x.com"], 12, "pricing three");
        let f = |name: &str| FindFilter { person: who(name, None), ..Default::default() };
        let hits = |name: &str| -> Vec<String> {
            let mut t: Vec<String> = s.find("pricing", &f(name), FindOrder::Time, 0, 10).unwrap().results.iter().filter(|c| c.kind == "thread").map(|c| c.title.clone()).collect();
            t.sort();
            t
        };
        assert_eq!(hits("Jean-Luc O'Brien"), vec!["Subject t1", "Subject t3"], "the full name as a run of whole tokens");
        assert_eq!(hits("O'Brien"), vec!["Subject t1", "Subject t3"]);
        assert!(hits("Brie").is_empty() && hits("Jean O'Brien").is_empty(), "a piece of a token, or tokens that do not run together, are no match");
    }

    fn thread_titles(s: &Store, name: &str) -> Vec<String> {
        let f = FindFilter { person: who(name, None), ..Default::default() };
        let mut got: Vec<String> = s.find("pricing", &f, FindOrder::Time, 0, 20).unwrap().results.iter().filter(|c| c.kind == "thread").map(|c| c.title.clone()).collect();
        got.sort();
        got
    }

    #[test]
    fn a_name_without_an_address_matches_whole_tokens_never_substrings() {
        let (_d, s) = store();
        email_to(&s, "e1", "ed", &["Ed <ed@x.example>", "me@x.com"], 10, "pricing a");
        email_to(&s, "e2", "edward", &["Edward Smith <edward@x.example>", "me@x.com"], 11, "pricing b");
        email_to(&s, "e3", "fred", &["Fred Smith <fred@x.example>", "me@x.com"], 12, "pricing c");
        email_to(&s, "e4", "reed", &["Reed Smith <reed@x.example>", "me@x.com"], 13, "pricing d");
        email_to(&s, "e5", "edsmith", &["Ed Smith <es@x.example>", "me@x.com"], 14, "pricing e");
        assert_eq!(thread_titles(&s, "Ed Smith"), vec!["Subject edsmith"], "Ed Smith, not Edward, Fred or Reed Smith");
    }

    /// The rule for a person with no address on record: two or more name tokens run together, "Last, First"
    /// reads as First Last, and one token matches no email at all.
    #[test]
    fn a_display_name_in_last_first_order_matches_and_a_single_token_matches_nothing() {
        let (_d, s) = store();
        email_to(&s, "e1", "plain", &["Ed Smith <es@x.example>", "me@x.com"], 10, "pricing a");
        email_to(&s, "e2", "comma", &["Smith, Ed <es@y.example>", "me@x.com"], 11, "pricing b");
        email_to(&s, "e3", "quoted", &["\"Smith, Ed\" <es@z.example>", "me@x.com"], 12, "pricing c");
        email_to(&s, "e4", "edward", &["Smith, Edward <ew@x.example>", "me@x.com"], 13, "pricing d");
        email_to(&s, "e5", "nocomma", &["Smith Ed <se@x.example>", "me@x.com"], 14, "pricing e");
        email_to(&s, "e6", "two", &["Smith, Ed, Jr <sj@x.example>", "me@x.com"], 15, "pricing f");
        email_to(&s, "e7", "lone", &["Ed <ed@x.example>", "me@x.com"], 16, "pricing g");
        assert_eq!(thread_titles(&s, "Ed Smith"), vec!["Subject comma", "Subject plain", "Subject quoted"]);
        // Tokens are runs of letters and digits, so the comma separates and a name in the other order is its own run.
        assert_eq!(thread_titles(&s, "Smith Ed"), vec!["Subject comma", "Subject nocomma", "Subject quoted", "Subject two"]);
        // A lone first name is never a match by display name, whatever the display names hold.
        for lone in ["Ed", "Will", "May", "Smith"] {
            assert!(thread_titles(&s, lone).is_empty(), "{lone}");
        }
        // With an address on record the one-token name matches by that address, and only that address.
        s.upsert_person("ed@x.example", "Ed", Some("ed@x.example")).unwrap();
        assert_eq!(thread_titles(&s, "Ed"), vec!["Subject lone"]);
    }

    #[test]
    fn an_address_matches_exactly_across_every_actor() {
        let (_d, s) = store();
        s.upsert_person("al@x.com", "Al Person", Some("al@x.com")).unwrap();
        email_to(&s, "e1", "sender", &["Al <AL@x.com>", "me@x.com"], 10, "pricing sender");
        email_to(&s, "e2", "mal", &["mal@x.com", "me@x.com"], 11, "pricing mal");
        email_to(&s, "e3", "cc", &["zed@x.com", "me@x.com", "al@x.com"], 12, "pricing cc");
        email_to(&s, "e4", "third", &["zed@x.com", "me@x.com", "other@x.com", "Al Person <al@x.com>"], 13, "pricing fourth actor");
        email_to(&s, "e5", "tail", &["zed@x.com", "me@x.com", "al@x.com.evil.example"], 14, "pricing tail");
        let f = FindFilter { person: who("Al Person", None), ..Default::default() };
        let mut got: Vec<String> = s.find("pricing", &f, FindOrder::Time, 0, 10).unwrap().results.iter().filter(|c| c.kind == "thread").map(|c| c.title.clone()).collect();
        got.sort();
        assert_eq!(got, vec!["Subject cc", "Subject sender", "Subject third"]);
        let waiting = s.scoped_waiting_threads(&s.scope_ctx(&f).unwrap(), 10).unwrap();
        assert_eq!(waiting.len(), 3, "{waiting:?}");
        assert_eq!(s.scoped_email_hits("pricing", &s.scope_ctx(&f).unwrap(), 10).unwrap().len(), 3);
    }

    /// A long meeting that began before the range holds lines inside it.
    fn long_meeting(s: &Store, start: i64, end: Option<i64>, attendees: &[&str], lines: &[(i64, &str)]) -> i64 {
        let id = s
            .insert_meeting(&crate::NewMeeting {
                device_id: "mac-1".into(),
                start_unix_ms: start,
                end_unix_ms: end,
                title: "Long one".into(),
                attendees: attendees.iter().map(|a| a.to_string()).collect(),
                trigger: crate::MeetingTrigger::Manual,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap();
        let segs: Vec<crate::NewTranscriptSegment> =
            lines.iter().map(|(cs, t)| crate::NewTranscriptSegment { meeting_id: id, start_cs: *cs, speaker_label: "Speaker 1".into(), text: t.to_string() }).collect();
        s.insert_transcript_segments(&segs).unwrap();
        id
    }

    #[test]
    fn a_person_filter_keeps_lines_inside_the_range_of_a_meeting_that_began_before_it() {
        let (_d, s) = store();
        // Starts at 900_000, runs to 1_900_000. cs 20_000 is 200 s in: 1_100_000, inside [R0, R1).
        let long = long_meeting(&s, 900_000, Some(1_900_000), &["Sam Okafor"], &[(0, "pricing early line"), (20_000, "pricing late line")]);
        // Ended before the range; no end known and began before the range; wholly inside; wholly after.
        let before = long_meeting(&s, 100_000, Some(200_000), &["Sam Okafor"], &[(0, "pricing before")]);
        let open = long_meeting(&s, 800_000, None, &["Sam Okafor"], &[(0, "pricing open")]);
        let inside = long_meeting(&s, 1_300_000, Some(1_400_000), &["Sam Okafor"], &[(0, "pricing inside")]);
        let after = long_meeting(&s, 2_500_000, Some(2_600_000), &["Sam Okafor"], &[(0, "pricing after")]);
        let f = FindFilter { person: who("Sam Okafor", None), ..range() };
        let texts = |lines: Vec<(MeetingTranscriptSegment, i64)>| -> Vec<String> {
            let mut t: Vec<String> = lines.into_iter().map(|(seg, _)| seg.text).collect();
            t.sort();
            t
        };
        let want = vec!["pricing inside".to_string(), "pricing late line".to_string()];
        assert_eq!(texts(s.scoped_transcript_hits("pricing", false, &s.scope_ctx(&f).unwrap(), 20).unwrap()), want);
        assert_eq!(texts(s.scoped_transcripts_in_window(&s.scope_ctx(&f).unwrap()).unwrap()), want);
        let page = s.find("pricing", &f, FindOrder::Time, 0, 50).unwrap();
        let mut found: Vec<&str> = page.results.iter().filter(|c| c.kind == "transcript").map(|c| c.text.as_deref().unwrap()).collect();
        found.sort();
        assert_eq!(found, vec!["pricing inside", "pricing late line"]);
        // The meetings themselves: one that overlaps the range counts when its end is known; with no end, its start decides.
        let listed = s.find("", &f, FindOrder::Time, 0, 50).unwrap();
        let ids: Vec<i64> = listed.results.iter().filter(|c| c.kind == "meeting").map(|c| c.id).collect();
        assert!(ids.contains(&long) && ids.contains(&inside), "{ids:?}");
        assert!(!ids.contains(&before) && !ids.contains(&open) && !ids.contains(&after), "{ids:?}");
        assert_eq!(listed.counts.get("meeting"), Some(&2));
    }

    /// Items whose meaning vectors are far from the query's, selected by a filter that picks 20 of 5,000.
    #[test]
    fn meaning_search_under_a_narrow_filter_scores_the_rows_the_filter_selects() {
        let (_d, s) = store();
        let q = "zebra quokka";
        let eq = s.embed_query(q).unwrap();
        let dist = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>();
        let mut texts: Vec<(f32, String)> = (0..5_000).map(|i| format!("item number {i} filler")).map(|t| (dist(&eq, &s.embed_passage(&t).unwrap()), t)).collect();
        texts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        // The 20 farthest from the query are the ones inside the range: the k-nearest read never reaches them.
        let (outside, inside) = texts.split_at(4_980);
        for (n, (_, t)) in outside.iter().enumerate() {
            capture(&s, n as i64, 10_000_000 + n as i64, "app", "out", t);
        }
        for (n, (_, t)) in inside.iter().enumerate() {
            capture(&s, 100_000 + n as i64, 1_000 + n as i64, "app", "in", t);
        }
        let f = FindFilter { start_unix_ms: Some(0), end_unix_ms: Some(5_000), ..Default::default() };
        let got = s.scoped_capture_hits(q, true, &s.scope_ctx(&f).unwrap(), 20).unwrap();
        assert_eq!(got.len(), 20, "all 20 rows the filter selects, nearest first");
        let got_d: Vec<f32> = got.iter().map(|c| dist(&eq, &s.embed_passage(&c.text).unwrap())).collect();
        assert!(got_d.windows(2).all(|w| w[0] <= w[1]), "nearest first: {got_d:?}");
        let page = s.find(q, &f, FindOrder::Rank, 0, 50).unwrap();
        assert!(page.results.len() >= 20, "{}", page.results.len());
    }

    #[test]
    fn huge_offsets_and_limits_neither_panic_nor_wrap() {
        let w = world();
        for q in ["pricing", ""] {
            for f in [FindFilter::default(), range()] {
                if q.is_empty() && f.is_empty() {
                    continue;
                }
                let page = w.s.find(q, &f, FindOrder::Time, usize::MAX, usize::MAX).unwrap();
                assert!(page.results.is_empty() && page.mentions.is_empty() && !page.more, "{q:?}");
                let page = w.s.find(q, &f, FindOrder::Time, usize::MAX / 2, 5).unwrap();
                assert!(page.results.is_empty());
                let page = w.s.find(q, &f, FindOrder::Time, 0, usize::MAX).unwrap();
                assert!(page.results.len() <= FIND_MAX_LIMIT && !page.results.is_empty());
                let page = w.s.find(q, &f, FindOrder::Time, 0, (i64::MAX as usize) + 5).unwrap();
                assert!(page.results.len() <= FIND_MAX_LIMIT && !page.results.is_empty());
            }
        }
        assert!(w.s.scoped_capture_hits("pricing", false, &w.s.scope_ctx(&range()).unwrap(), usize::MAX).unwrap().len() <= 100);
        assert!(w.s.scoped_episodes(&w.s.scope_ctx(&range()).unwrap(), usize::MAX).unwrap().len() <= 10);
    }

    #[test]
    fn capped_is_computed_from_the_raw_rows_not_what_survives_de_duplication() {
        let (_d, s) = store();
        // 1,700 messages in 50 threads: the leg reads its whole 1,600 row pool, then collapses to 50 threads.
        for i in 0..1_700 {
            email_to(&s, &format!("e{i}"), &format!("t{}", i % 50), &["zed@x.com", "me@x.com"], 1_000 + i, "pricing message");
        }
        let page = s.find("pricing", &FindFilter::default(), FindOrder::Time, 0, 100).unwrap();
        assert_eq!(page.results.iter().filter(|c| c.kind == "thread").count(), 50);
        assert!(page.capped, "the thread leg hit its pool before collapsing messages into threads");
        let (_d2, s2) = store();
        for i in 0..30 {
            email_to(&s2, &format!("e{i}"), &format!("t{}", i % 5), &["zed@x.com", "me@x.com"], 1_000 + i, "pricing message");
        }
        assert!(!s2.find("pricing", &FindFilter::default(), FindOrder::Time, 0, 100).unwrap().capped);
    }

    #[test]
    fn a_filter_that_could_widen_a_search_is_refused_not_read_as_empty() {
        for (raw, want) in [
            (r#"{"startUnixMs":1000}"#, "malformed"),
            (r#"{"start_unix_ms":1,"endUnixMs":2}"#, "malformed"),
            (r#"{"person":{"name":"Sam","personKey":"x"}}"#, "malformed"),
            (r#"{"person":{"person_key":"x"}}"#, "malformed"),
            (r#"{"start_unix_ms":"soon"}"#, "malformed"),
            ("{nope", "malformed"),
            ("[]", "malformed"),
            (r#"["com.slack"]"#, "malformed"),
            ("5", "malformed"),
            (r#"{"app_bundle_id":"  "}"#, "blank app"),
            (r#"{"app_bundle_id":""}"#, "blank app"),
            (r#"{"person":{"name":" "}}"#, "blank person"),
            (r#"{"start_unix_ms":5,"end_unix_ms":5}"#, "empty range"),
            (r#"{"start_unix_ms":9,"end_unix_ms":5}"#, "empty range"),
        ] {
            let err = FindFilter::from_json(raw).expect_err(raw);
            let got = match err {
                FilterError::Malformed(_) => "malformed",
                FilterError::BlankApp => "blank app",
                FilterError::BlankPerson => "blank person",
                FilterError::EmptyRange => "empty range",
            };
            assert_eq!(got, want, "{raw}");
        }
        // What is fine stays fine: nothing, null, an empty object, a half-open range, trimmed strings.
        for ok in ["", "  ", "null", "{}", r#"{"start_unix_ms":5}"#, r#"{"end_unix_ms":5}"#] {
            assert!(FindFilter::from_json(ok).is_ok(), "{ok}");
        }
        assert!(FindFilter::from_json("{}").unwrap().is_empty());
        let f = FindFilter::from_json(r#"{"app_bundle_id":" com.a ","person":{"name":" Sam ","person_key":"  "}}"#).unwrap();
        assert_eq!((f.app_bundle_id.as_deref(), f.person), (Some("com.a"), Some(PersonFilter { name: "Sam".into(), person_key: None })));
        // The store reads refuse them too: an error, never an unbounded search.
        let w = world();
        let blank_app = FindFilter { app_bundle_id: Some(" ".into()), person: who("Sam Okafor", None), ..Default::default() };
        let blank_person = FindFilter { person: who(" ", None), ..range() };
        let reversed = FindFilter { start_unix_ms: Some(R1), end_unix_ms: Some(R0), ..Default::default() };
        for f in [&blank_app, &blank_person, &reversed] {
            assert!(w.s.find("pricing", f, FindOrder::Time, 0, 10).is_err());
            // The one place a filter is checked for the scoped reads: building their context.
            assert!(w.s.scope_ctx(f).is_err());
        }
    }

    #[test]
    fn scoped_rows_honour_every_bound() {
        let w = world();
        let f = range();
        for c in w.s.scoped_capture_hits("pricing", false, &w.s.scope_ctx(&f).unwrap(), 20).unwrap() {
            assert!((R0..R1).contains(&c.ts_unix_ms));
        }
        assert!(w.s.scoped_transcript_hits("pricing", false, &w.s.scope_ctx(&f).unwrap(), 20).unwrap().iter().all(|(_, ts)| (R0..R1).contains(ts)));
        assert!(!w.s.scoped_transcript_hits("pricing", false, &w.s.scope_ctx(&f).unwrap(), 20).unwrap().is_empty());
        let slides = w.s.scoped_snapshot_hits("pricing", false, &w.s.scope_ctx(&f).unwrap(), 20).unwrap();
        assert_eq!(slides.len(), 1);
        let mail = w.s.scoped_email_hits("pricing", &w.s.scope_ctx(&f).unwrap(), 20).unwrap();
        assert!(mail.iter().all(|h| (R0..R1).contains(&h.thread.last_activity_ts_unix_ms)) && mail.len() == 2);
        assert!(w.s.scoped_episodes(&w.s.scope_ctx(&f).unwrap(), 10).unwrap().iter().all(|e| (R0..R1).contains(&e.end_ts_unix_ms)));
        assert_eq!(w.s.scoped_routine_runs(&w.s.scope_ctx(&f).unwrap(), 10).unwrap().len(), 1);
        // An app hides runs, slides, mail; a person hides episodes and runs.
        let app = FindFilter { app_bundle_id: Some("com.slack".into()), ..f.clone() };
        assert!(w.s.scoped_routine_runs(&w.s.scope_ctx(&app).unwrap(), 10).unwrap().is_empty() && w.s.scoped_email_hits("pricing", &w.s.scope_ctx(&app).unwrap(), 10).unwrap().is_empty());
        assert!(w.s.scoped_transcript_hits("pricing", false, &w.s.scope_ctx(&app).unwrap(), 10).unwrap().is_empty() && w.s.scoped_snapshot_hits("pricing", false, &w.s.scope_ctx(&app).unwrap(), 10).unwrap().is_empty());
        assert!(w.s.scoped_episodes(&w.s.scope_ctx(&app).unwrap(), 10).unwrap().is_empty());
        let person = FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: None }), ..f.clone() };
        assert!(w.s.scoped_episodes(&w.s.scope_ctx(&person).unwrap(), 10).unwrap().is_empty() && w.s.scoped_routine_runs(&w.s.scope_ctx(&person).unwrap(), 10).unwrap().is_empty());
        let waiting = w.s.scoped_waiting_threads(&w.s.scope_ctx(&person).unwrap(), 10).unwrap();
        assert_eq!(waiting.len(), 1, "Sam's thread inside the range, found by the address on record: {waiting:?}");
        assert!(w.s.scoped_waiting_threads(&w.s.scope_ctx(&app).unwrap(), 10).unwrap().is_empty());
        let day = w.s.scoped_timeline(&w.s.scope_ctx(&f).unwrap()).unwrap();
        assert!(day.iter().all(|c| (R0..R1).contains(&c.ts_unix_ms)) && day.windows(2).all(|p| p[0].ts_unix_ms <= p[1].ts_unix_ms));
        assert_eq!(w.s.scoped_timeline(&w.s.scope_ctx(&FindFilter { app_bundle_id: Some("com.notes".into()), ..f.clone() }).unwrap()).unwrap().len(), 1);
        assert!(w.s.scoped_transcripts_in_window(&w.s.scope_ctx(&f).unwrap()).unwrap().iter().all(|(_, ts)| (R0..R1).contains(ts)));
        assert!(w.s.scoped_transcripts_in_window(&w.s.scope_ctx(&app).unwrap()).unwrap().is_empty());
    }

    // ---- cost: one person context, a narrowed read, bounded work -----------------------------------

    fn person_filter(name: &str) -> FindFilter {
        FindFilter { person: who(name, None), ..Default::default() }
    }

    /// One person context per `find` call, one per Ask however many sources read it, none without a person.
    #[test]
    fn a_person_is_resolved_once_per_find_and_once_per_ask() {
        let w = world();
        let s = &w.s;
        let f = FindFilter { person: who("Sam Okafor", Some("name:sam")), ..range() };
        assert_eq!(person_context_builds(s), 0);
        s.find("pricing", &f, FindOrder::Time, 0, 10).unwrap();
        assert_eq!(person_context_builds(s), 1, "a page is one context");
        s.find("pricing", &f, FindOrder::Rank, 10, 10).unwrap();
        assert_eq!(person_context_builds(s), 2, "the next page is a new call");
        s.find("", &f, FindOrder::Time, 0, 10).unwrap();
        assert_eq!(person_context_builds(s), 3, "the empty-query listing too");
        s.find("pricing", &range(), FindOrder::Time, 0, 10).unwrap();
        s.find("pricing", &FindFilter { app_bundle_id: Some("com.slack".into()), ..Default::default() }, FindOrder::Time, 0, 10).unwrap();
        assert_eq!(person_context_builds(s), 3, "no person, no context");
        // One Ask: every source, both the keyword and the meaning read, and a window, share one.
        let before = person_context_builds(s);
        let ctx = s.scope_ctx(&f).unwrap();
        assert_eq!(person_context_builds(s), before + 1);
        for semantic in [false, true] {
            s.scoped_capture_hits("pricing", semantic, &ctx, 20).unwrap();
            s.scoped_transcript_hits("pricing", semantic, &ctx, 20).unwrap();
            s.scoped_snapshot_hits("pricing", semantic, &ctx, 20).unwrap();
        }
        s.scoped_email_hits("pricing", &ctx, 20).unwrap();
        s.scoped_waiting_threads(&ctx, 10).unwrap();
        s.scoped_episodes(&ctx, 10).unwrap();
        s.scoped_routine_runs(&ctx, 10).unwrap();
        let windowed = ctx.or_window(R0, R1);
        s.scoped_timeline(&windowed).unwrap();
        s.scoped_transcripts_in_window(&windowed).unwrap();
        s.scoped_waiting_threads(&windowed, 10).unwrap();
        assert_eq!(person_context_builds(s), before + 1, "every source and the window share the one context");
    }

    /// The ids a person context stages for its queries are its own, and gone when it drops.
    #[test]
    fn a_contexts_temp_rows_go_when_it_drops_and_two_contexts_do_not_share() {
        let w = world();
        let s = &w.s;
        let staged = || s.conn.query_row("SELECT COUNT(*) FROM temp.find_scope_ids", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
        let sam = FindFilter { person: who("Sam Okafor", Some("name:sam")), ..Default::default() };
        let zed = FindFilter { person: who("Zed", Some("zed@x.com")), ..Default::default() };
        {
            let a = s.scope_ctx(&sam).unwrap();
            let b = s.scope_ctx(&zed).unwrap();
            let mail = |c: &ScopeCtx<'_>| s.scoped_email_hits("pricing", c, 20).unwrap().len();
            // Sam: the address on record finds two threads; Zed: one. Interleaved, so a shared table would mix them.
            assert_eq!((mail(&a), mail(&b), mail(&a), mail(&b)), (2, 1, 2, 1));
            assert!(staged() > 0);
        }
        assert_eq!(staged(), 0, "dropped with the contexts");
        s.find("pricing", &sam, FindOrder::Time, 0, 10).unwrap();
        assert_eq!(staged(), 0, "and after a find");
    }

    /// The SQL pre-filter only ever leaves rows in: text it cannot fold (non-ASCII), other cases, odd forms.
    #[test]
    fn the_prefilter_never_drops_a_row_the_rule_accepts() {
        let (_d, s) = store();
        // Meetings: the name in other cases, padded, inside non-ASCII neighbours; a non-ASCII name.
        let a = meeting(&s, 100, "A", &["  SAM OKAFOR  ", "Zoë"], &["x"]);
        let b = meeting(&s, 200, "B", &["Zoë", "sam okafor"], &["x"]);
        let _c = meeting(&s, 300, "C", &["Sam Okafor Jr"], &["x"]);
        let d = meeting(&s, 400, "D", &["ÉMILIE DUPONT"], &["x"]);
        let got: Vec<i64> = s.meetings_attended_by("Sam Okafor", None, None).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(got, vec![b, a]);
        let got: Vec<i64> = s.meetings_attended_by("Émilie Dupont", None, None).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(got, vec![d], "a non-ASCII name is not pre-filtered and still matches whatever the case");
        // Email: an address in a row with non-ASCII neighbours, in other case, and a LIKE wildcard in the name.
        email_to(&s, "e1", "t1", &["Zoë <zoe@x.com>", "SAM@X.COM"], 10, "pricing one");
        email_to(&s, "e2", "t2", &["Sam <sam@xx.com>", "me@x.com"], 11, "pricing two");
        email_to(&s, "e3", "t3", &["Sam_O <samxo@x.com>", "me@x.com"], 12, "pricing three");
        s.upsert_person("sam@x.com", "Sam Okafor", Some("sam@x.com")).unwrap();
        assert_eq!(thread_titles(&s, "Sam Okafor"), vec!["Subject t1"]);
        // A name with an underscore is a literal underscore, not a wildcard.
        s.upsert_person("sam_o@x.com", "Sam_O", Some("sam_o@x.com")).unwrap();
        assert!(thread_titles(&s, "Sam_O").is_empty(), "no actor has the address sam_o@x.com");
    }

    /// Synthetic store: `emails` email items (a tenth from or to Pat Quill, everyone to `me@x.com`) and
    /// `meetings` meetings (one in forty attended by Pat Quill).
    fn big_store(emails: i64, meetings: i64) -> (tempfile::TempDir, Store) {
        let (d, s) = store();
        s.conn.execute_batch("BEGIN").unwrap();
        {
            let mut thread = s
                .conn
                .prepare(
                    "INSERT OR IGNORE INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, summarize_status, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES ('gmail', ?1, 'me@x.com', ?2, 1, ?3, 'a', 'waiting_on_me', 'pending_summary', 1, 1)",
                )
                .unwrap();
            let mut item = s
                .conn
                .prepare("INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) VALUES ('d', 'email', 'gmail', ?1, ?2, ?3, ?4, ?5, ?6)")
                .unwrap();
            for i in 0..emails {
                let tid = format!("t{}", i % 5_000);
                let from = if i % 10 == 0 { "Pat Quill <pat@q.example>".to_string() } else { format!("Person {} <p{}@y.example>", i % 700, i % 700) };
                let actors = serde_json::to_string(&[from.as_str(), "me@x.com"]).unwrap();
                thread.execute(rusqlite::params![tid, format!("Subject {tid}"), 1_000 + i]).unwrap();
                let refs = serde_json::json!({"gmail_thread_id": tid, "account_email": "me@x.com"}).to_string();
                item.execute(rusqlite::params![1_000 + i, actors, format!("Subject {tid}"), format!("pricing note {i}"), refs, format!("e{i}")]).unwrap();
            }
            let mut m = s
                .conn
                .prepare("INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, status) VALUES ('d', ?1, ?2, ?3, ?4, 'manual', 'completed')")
                .unwrap();
            for i in 0..meetings {
                let who = if i % 40 == 0 { "Pat Quill".to_string() } else { format!("Person {}", i % 300) };
                m.execute(rusqlite::params![10_000 + i * 100, 10_050 + i * 100, format!("M{i}"), serde_json::to_string(&[who.as_str(), "Ana Lopez"]).unwrap()]).unwrap();
            }
        }
        s.conn.execute_batch("COMMIT").unwrap();
        (d, s)
    }

    /// A timing guard, generous on purpose: it catches a return to scanning and parsing every row per call, not a
    /// slow machine. Measured on the build machine in release: see the report of this change.
    #[test]
    fn a_person_filter_over_50k_emails_and_5k_meetings_stays_fast() {
        let (_d, s) = big_store(50_000, 5_000);
        let limit = std::time::Duration::from_secs(30);
        for (label, name, key) in [("name only", "Pat Quill", None), ("address on record", "Pat Quill", Some("pat@q.example")), ("the owner, on every email", "Me", Some("me@x.com"))] {
            let f = FindFilter { person: who(name, key), ..Default::default() };
            let t = std::time::Instant::now();
            let page = s.find("pricing", &f, FindOrder::Time, 0, 50).unwrap();
            let one_page = t.elapsed();
            let t = std::time::Instant::now();
            let ctx = s.scope_ctx(&f).unwrap();
            let built = t.elapsed();
            let t = std::time::Instant::now();
            let mail = s.scoped_email_hits("pricing", &ctx, 20).unwrap();
            let waiting = s.scoped_waiting_threads(&ctx, 10).unwrap();
            let reads = t.elapsed();
            eprintln!("{label}: find {one_page:?}, context {built:?}, mail reads {reads:?}, threads {}", page.results.iter().filter(|c| c.kind == "thread").count());
            assert!(one_page < limit && built < limit && reads < limit, "{label}: {one_page:?} {built:?} {reads:?}");
            if name == "Pat Quill" {
                assert!(!mail.is_empty() && !waiting.is_empty(), "{label}");
                assert!(page.results.iter().any(|c| c.kind == "meeting" || c.kind == "thread" || c.kind == "transcript"), "{label}");
            }
        }
        // The same person's meetings: a 40th of 5,000.
        assert_eq!(s.meetings_attended_by("Pat Quill", None, None).unwrap().len(), 125);
        // Two names that are not Pat's read nothing slowly either: no address and one token is no scan at all.
        let t = std::time::Instant::now();
        s.find("pricing", &person_filter("Pat"), FindOrder::Time, 0, 50).unwrap();
        assert!(t.elapsed() < limit);
    }

    /// Meaning search under a filter scores at most [`EXACT_SEMANTIC_MAX_ROWS`] rows exactly; this reads and scores
    /// exactly that many (the worst case) and bounds the time loosely. The number it prints is the one the cap was
    /// chosen by.
    #[test]
    fn exact_meaning_search_stays_fast_at_the_cap() {
        let (_d, s) = store();
        // One app holds `EXACT_SEMANTIC_MAX_ROWS` captures, the rest of the store is another app, so the
        // total is past the nearest-neighbour cap and the narrow filter selects exactly the cap.
        let narrow = EXACT_SEMANTIC_MAX_ROWS as i64;
        let total = narrow + ITEMS_VEC_MAX_K + 100;
        // The rest of the store is word for word the query, so its embeddings are the nearest of everything and the
        // nearest-neighbour read is all of it: the narrow app's rows are only reached by scoring them exactly.
        for i in 0..total {
            let (app, text) = if i < narrow { ("app.narrow", format!("note number {i} about budgets")) } else { ("app.rest", "budget note".to_string()) };
            capture(&s, i, 1_000 + i, app, &format!("t{i}"), &text);
        }
        let f = FindFilter { app_bundle_id: Some("app.narrow".into()), ..Default::default() };
        let ctx = s.scope_ctx(&f).unwrap();
        let t = std::time::Instant::now();
        let got = s.scoped_capture_hits("budget note", true, &ctx, FIND_POOL).unwrap();
        let took = t.elapsed();
        eprintln!("exact meaning leg over {narrow} rows: {took:?} ({} hits)", got.len());
        assert_eq!(got.len(), FIND_POOL.min(narrow as usize), "the rows the filter selects were scored, not the nearest of everything");
        // Loose by design: an unoptimised test build on a shared CI runner (two jobs on one PC) has run past
        // 5 s, so debug builds get 30 s. The row count above is the real check; release keeps 5 s.
        let bound = if cfg!(debug_assertions) { 30 } else { 5 };
        assert!(took < std::time::Duration::from_secs(bound), "{took:?}");
    }


    /// How long one exact meaning leg takes by the rows the filter selects. Not run with the suite: run it in release
    /// (`cargo test -p memory --release --lib measure_exact_meaning_search -- --ignored --nocapture`) to choose
    /// [`EXACT_SEMANTIC_MAX_ROWS`]; the target is well under 100 ms a leg. It prints times and asserts none of them,
    /// so it is safe in the main-branch `--ignored` step. What it does assert is the row count, and that depends on
    /// the cap: at or under it the filter's rows are scored exactly, so the pool fills; over it the nearest-neighbour
    /// read stands by design, and here that read is crowded out by the other app's rows (the exact text of the
    /// query), so only a few of the filter's rows come back.
    #[test]
    #[ignore = "measurement: prints timings for choosing EXACT_SEMANTIC_MAX_ROWS; run with --ignored --nocapture"]
    fn measure_exact_meaning_search_by_rows_selected() {
        let (_d, s) = store();
        let most = 20_000i64;
        let total = most + ITEMS_VEC_MAX_K + 100;
        for i in 0..total {
            let (app, text) = if i < most { ("app.narrow", format!("note number {i} about budgets")) } else { ("app.rest", "budget note".to_string()) };
            capture(&s, i, 1_000 + i, app, &format!("t{i}"), &text);
        }
        for rows in [1_000i64, 2_000, 4_000, 8_000, 12_000, 20_000] {
            let f = FindFilter { app_bundle_id: Some("app.narrow".into()), end_unix_ms: Some(1_000 + rows), ..Default::default() };
            let ctx = s.scope_ctx(&f).unwrap();
            let mut best = std::time::Duration::MAX;
            for _ in 0..5 {
                let t = std::time::Instant::now();
                let got = s.scoped_capture_hits("budget note", true, &ctx, FIND_POOL).unwrap();
                best = best.min(t.elapsed());
                if rows as usize <= EXACT_SEMANTIC_MAX_ROWS {
                    assert_eq!(got.len(), FIND_POOL, "{rows} rows: scored exactly, so the pool fills");
                } else {
                    assert!(got.len() <= FIND_POOL, "{rows} rows: {}", got.len());
                }
                assert!(got.iter().all(|c| c.app_bundle_id == "app.narrow" && c.ts_unix_ms < 1_000 + rows), "{rows} rows: only the filter's rows");
            }
            eprintln!("exact meaning leg, {rows} rows selected: best of 5 {best:?} (cap {EXACT_SEMANTIC_MAX_ROWS}: {})", if rows as usize <= EXACT_SEMANTIC_MAX_ROWS { "scored exactly" } else { "nearest neighbours only" });
        }
    }


    // ---- the same search, before and after new items have their embeddings ------------------------------------------

    /// Puts each text on the axis of the first concept it names (rides, sales), so meaning reaches rows that share no word with the query.
    struct ConceptEmbedder;
    impl crate::Embedder for ConceptEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, crate::embedding::EmbeddingError> {
            let t = text.to_lowercase();
            let mut v = vec![0f32; crate::embedding::EMBEDDING_OUTPUT_DIM];
            let axis = if ["transportation", "shuttle", "bus"].iter().any(|w| t.contains(w)) { 0 } else if ["fundraising", "bake sale"].iter().any(|w| t.contains(w)) { 1 } else { 2 + t.len() % 300 };
            v[axis] = 1.0;
            Ok(v)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    /// The real-PC rerun: an item a local Claude added waits for the embedding sweep, so the same words found it by meaning only once the
    /// sweep ran. The page says when that can still happen, so the page searches again and replaces what it showed.
    #[test]
    fn a_page_says_when_meaning_results_are_still_catching_up() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("c.db"), std::sync::Arc::new(ConceptEmbedder));
        // The app's jobs loop: runs the sweep until it is caught up and marks it done.
        let run_jobs = || while s.run_jobs_batch(10, 0).unwrap() > 0 {};
        run_jobs();
        let caught_up = s.find("transportation", &FindFilter::default(), FindOrder::Rank, 0, 20).unwrap();
        assert!(caught_up.results.is_empty() && !caught_up.meaning_catching_up);

        let shuttle = added_at(&s, "shuttle", "1970-01-01T00:25:00Z", "Getting there", "The shuttle leaves every twenty minutes");
        let before = s.find("transportation", &FindFilter::default(), FindOrder::Rank, 0, 20).unwrap();
        assert!(before.results.is_empty(), "{:?}", kinds_ids(&before));
        assert!(before.meaning_catching_up, "the added item has no embedding yet");

        run_jobs();
        let after = s.find("transportation", &FindFilter::default(), FindOrder::Rank, 0, 20).unwrap();
        assert_eq!(kinds_ids(&after), vec![("mcp_item".to_string(), shuttle)]);
        assert!(!after.meaning_catching_up, "the sweep is done");
        // Caught up, the same words give the same page every time.
        for _ in 0..3 {
            assert_eq!(kinds_ids(&s.find("transportation", &FindFilter::default(), FindOrder::Rank, 0, 20).unwrap()), kinds_ids(&after));
        }
    }

    #[test]
    fn a_page_without_the_model_never_says_catching_up() {
        let (_d, s) = store();
        capture(&s, 1, 1_000, "com.notes", "Bus", "bus line 14");
        let p = s.find("bus", &FindFilter::default(), FindOrder::Rank, 0, 20).unwrap();
        assert!(p.semantic_used || !p.meaning_catching_up);
    }

    // ---- the meaning floor on a small made-up library, with the real model -----------------------------------------

    /// A made-up library like the one the real-PC check used: four meetings and three notes about a school fair and an app review.
    /// Returns the store and, by name, the item ids a query should reach.
    fn fair_library() -> (tempfile::TempDir, Store, HashMap<&'static str, Vec<i64>>) {
        let dir = tempfile::tempdir().unwrap();
        let models = std::env::var_os("WHIMBREL_EMBEDDING_MODELS_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::embedding::embedding_model_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("models")));
        let s = crate::test_store::open_with_embedder(&dir.path().join("fair.db"), std::sync::Arc::new(crate::embedding::LazyModelEmbedder::new(models)));
        assert!(s.embedding_model_loaded(), "the embedding model is not there");
        let segs = |m: i64| -> Vec<i64> { transcript_items(&s, m) };
        let kite = meeting(&s, 10_000_000, "Kite app accessibility review", &["Ana", "Sam"], &[
            "VoiceOver reads the toolbar buttons without names",
            "Screen reader support for the settings page ships next sprint",
            "We raised the text contrast on the dark theme",
        ]);
        let design = meeting(&s, 11_000_000, "Design sync", &["Sam", "Priya"], &[
            "Sam showed the new onboarding flow",
            "Larger tap targets on the checkout screen",
            "Captions for the tutorial videos are still missing",
        ]);
        let logistics = meeting(&s, 12_000_000, "Spring fair logistics", &["Ana", "Leo"], &[
            "The shuttle leaves the station every twenty minutes",
            "Bus line 14 stops at the north gate",
            "Each van holds twelve people, so we need three vans",
            "Kites go up on the windy field behind the school",
        ]);
        let volunteers = meeting(&s, 13_000_000, "Fair volunteers check-in", &["Ana", "Priya"], &[
            "Ana runs the plant sale table",
            "We need two more people at the bake sale",
            "Money from both sales goes to the library roof",
        ]);
        // A note's item, by its time (the helper returns the capture's own id).
        let note = |n: i64, ts: i64, title: &str, text: &str| -> i64 {
            capture(&s, n, ts, "com.notes", title, text);
            s.conn.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = ?1", [ts], |r| r.get(0)).unwrap()
        };
        let plant = note(1, 14_000_000, "Plant sale", "Plant sale. Seedlings 3 dollars, herbs 2 dollars. Every dollar goes to the garden club.");
        let bake = note(2, 15_000_000, "Bake sale prices", "Bake sale prices. Cupcakes 2 dollars, brownies 3, whole pies 12. Proceeds for the class trip.");
        let route = note(3, 16_000_000, "Getting to the fair", "Getting to the fair. Park at the church lot. Shuttle every twenty minutes, or take bus 14.");
        let (k, d, l, v) = (segs(kite), segs(design), segs(logistics), segs(volunteers));
        let mut want: HashMap<&'static str, Vec<i64>> = HashMap::new();
        want.insert("kite", k);
        want.insert("design", d);
        want.insert("logistics", l.clone());
        want.insert("volunteers", v.clone());
        want.insert("transport", vec![l[0], l[1], l[2], route]);
        want.insert("money", vec![v[0], v[1], v[2], plant, bake]);
        want.insert("bake", vec![bake, v[1]]);
        want.insert("van", vec![l[2], l[0]]);
        want.insert("field", vec![l[3]]);
        want.insert("bus", vec![l[1], route]);
        want.insert("reader", vec![want["kite"][1]]);
        (dir, s, want)
    }

    fn transcript_items(s: &Store, meeting: i64) -> Vec<i64> {
        let mut st = s.conn.prepare("SELECT id FROM items WHERE kind = 'transcript' AND thread_id = ?1 ORDER BY id").unwrap();
        st.query_map([meeting], |r| r.get(0)).unwrap().collect::<rusqlite::Result<Vec<i64>>>().unwrap()
    }

    /// Every item's cosine to `query`, best first.
    fn all_scores(s: &Store, query: &str) -> Vec<(f32, i64)> {
        let q = s.embed_query(query).unwrap();
        let mut st = s.conn.prepare("SELECT rowid, embedding FROM items_vec").unwrap();
        let mut out: Vec<(f32, i64)> = st
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .unwrap()
            .map(|r| {
                let (id, blob) = r.unwrap();
                let v: Vec<f32> = blob.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                let dot: f32 = v.iter().zip(&q).map(|(a, b)| a * b).sum();
                let n = |x: &[f32]| x.iter().map(|y| y * y).sum::<f32>().sqrt();
                (dot / (n(&v) * n(&q)), id)
            })
            .collect();
        out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        out
    }

    /// Find's meaning floor on a small made-up library, with the real model: the queries a real-PC check made, which should find a note
    /// or a meeting line about them, and nonsense, which should find nothing. Prints each query's best scores (`*` marks a row it should
    /// reach) so a change of model or floor can be judged here.
    ///
    /// Best cosine of each query (16 rows). The floor now goes by how many words of the query are not common words (0.34 for one,
    /// 0.36 for more); before it went by whether any word matched by keyword (0.28 if one did, else 0.34 for one word, 0.40 for more):
    ///
    /// | query | best | floor before | floor now |
    /// |---|---|---|---|
    /// | vehicle capacity | 0.468 | 0.40 found | 0.36 found |
    /// | windy field | 0.643 | 0.28 found | 0.36 found |
    /// | dessert pricing | 0.716 | 0.40 found | 0.36 found |
    /// | getting there by bus | 0.588 | 0.28 found | 0.36 found |
    /// | screen reader support | 0.694 | 0.28 found | 0.36 found |
    /// | transportation | 0.387 | 0.34 found | 0.34 found |
    /// | fundraising | 0.357 | 0.34 found | 0.34 found |
    /// | accessibility for blind users | 0.317 | 0.28 found (by "for") | 0.36 **missed** (printed only) |
    /// | zebra quantum harmonica | 0.174 | 0.40 none | 0.36 none |
    /// | xyzzy plover | 0.265 | 0.40 none | 0.36 none |
    ///
    /// Single words printed below: "travel" 0.357, "commute" 0.386 and "donations" 0.344 find their rows, "charity" 0.319 and
    /// "accessibility" 0.182 do not, and "plover" (0.345, a real word near nothing here) finds one.
    #[test]
    #[ignore] // needs real model files
    fn find_meaning_floor_on_a_small_made_up_library() {
        let (_d, s, want) = fair_library();
        let should = [
            ("vehicle capacity", "van"),
            ("windy field", "field"),
            ("dessert pricing", "bake"),
            ("getting there by bus", "bus"),
            ("screen reader support", "reader"),
            ("transportation", "transport"),
            ("fundraising", "money"),
        ];
        let nonsense = ["zebra quantum harmonica", "xyzzy plover"];
        // Printed only: more short words, real and odd, to judge the floor by.
        // "accessibility for blind users" (0.317) was found only because "for" matched as a keyword; with common words left out it is
        // under the floor for two or more words (0.36), which odd word pairs reach at library size ("xyzzy plover" 0.329).
        let printed = [("accessibility for blind users", "kite"), ("travel", "transport"), ("commute", "transport"), ("donations", "money"), ("charity", "money"), ("desserts", "bake"), ("accessibility", "kite"), ("plover", ""), ("harmonica", ""), ("xyzzy", ""), ("zebra", "")];
        let line = |q: &str, w: &str| {
            let good = want.get(w).cloned().unwrap_or_default();
            let page = s.find(q, &FindFilter::default(), FindOrder::Rank, 0, 50).unwrap();
            let top: Vec<String> = all_scores(&s, q).iter().take(6).map(|(c, id)| format!("{c:.3}{}", if good.contains(id) { "*" } else { "" })).collect();
            eprintln!("{q:32} found {:2} | {}", page.results.len(), top.join(" "));
            (page.results.iter().map(|c| c.id).collect::<Vec<_>>(), good)
        };
        let mut missed = Vec::new();
        for (q, w) in should {
            let (found, good) = line(q, w);
            if !found.iter().any(|id| good.contains(id)) {
                missed.push(q);
            }
        }
        let mut noisy = Vec::new();
        for q in nonsense {
            if !line(q, "").0.is_empty() {
                noisy.push(q);
            }
        }
        for (q, w) in printed {
            line(q, w);
        }
        assert!(missed.is_empty(), "should have found something about: {missed:?}");
        assert!(noisy.is_empty(), "nonsense found something: {noisy:?}");
    }
}

#[cfg(test)]
#[path = "find_eval.rs"]
mod find_eval;
