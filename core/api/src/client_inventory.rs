//! "Everything about this client", read-only (X6 client controls, spec 4.1-4.2): what is
//! linked to a client (a seed topic), what merely names it, and the places Whimbrel cannot
//! reach. Nothing is deleted here; the delete consumes [`inventory`].
//!
//! Linked material is what the seed owns: its meetings, emails, calendar events, screen
//! activity episodes and steps. Possible matches are other rows whose text names the client
//! or one of its aliases as whole words (the matcher the hosted-model gate uses, so "Ann"
//! never finds "Annual"). Possible matches are only ever offered, never deleted unless the
//! user ticks them.
//!
//! No client name or alias is logged or printed here: items carry row ids and dates.
//! `also_client` (another client that shares a source) and `label` (the item's own title, which
//! for a possible match names the client by construction) are display only, for the user's own
//! screen, and are redacted from `Debug`. Neither ever goes into a receipt, a log line or a
//! hosted-model request.

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::envelope::{ApiError, ApiResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Meeting,
    Email,
    Thread,
    CalendarEvent,
    Capture,
    Episode,
    Conversation,
    Step,
    VaultNote,
    ExportFolder,
    /// An item a local MCP client added (`items` kind `mcp_item`). Never linked: listed under Possible
    /// matches by name or by a client contact's address.
    McpItem,
}

/// Every `items` kind and whether the client review lists it (held to `memory::ITEM_KINDS` by
/// `mcp_item_coverage.rs`).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const INVENTORY_ITEM_KINDS: [(&str, memory::mcp_items::Coverage); 6] = {
    use memory::mcp_items::Coverage::{Covered, NotApplicable};
    [
        ("capture", Covered("possible matches by name")),
        ("transcript", NotApplicable("its meeting is the unit")),
        ("email", Covered("possible matches by name")),
        ("calendar_event", Covered("possible matches by name")),
        ("snapshot", NotApplicable("listed as its meeting")),
        ("mcp_item", Covered("possible matches by name or contact")),
    ]
};

/// One piece of a client's material. `id` is the row id (the step key for a step). A capture's id is its
/// `capture_item.id`, or `i:<items.id>` for a capture this device holds only in `items` (pulled from another
/// device after the cutover).
#[derive(Clone, PartialEq, Serialize)]
pub struct ClientItem {
    pub kind: ItemKind,
    pub id: String,
    /// Another client that also holds this item. Display only; never persisted.
    pub also_client: Option<String>,
    pub date_unix_ms: i64,
    /// A calendar event marked removed (gone from the user's calendars): still stored, still deleted.
    /// The review screen can show these as "removed events".
    pub removed: bool,
    /// A short name for the row on the user's own screen (a meeting's title, an email's subject, a step's
    /// text; one line, at most [`LABEL_CHARS`] characters). Client text: display only, never persisted, never
    /// in a receipt, a log line or a hosted-model request. `None` when the item has none: the screen shows its
    /// kind and date.
    pub label: Option<String>,
    /// Why it matched: the line of the item's text that names the client, trimmed around the name (at most
    /// [`EXCERPT_CHARS`] characters, with "…"), found with the same whole-word matcher the match used.
    /// Client text with the same rules as `label`: display only, never persisted, never in a receipt, a log
    /// line or a hosted-model request, and redacted from `Debug`. `None` when the item has no text that
    /// names the client (a title-only or unlinked-by-name row).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// A screen capture's app (its bundle id), so the screen can group captures from one app. Not client text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

/// The longest [`ClientItem::label`], in characters.
pub const LABEL_CHARS: usize = 80;

/// The longest [`ClientItem::excerpt`] not counting its "…" marks, in characters.
pub const EXCERPT_CHARS: usize = 120;

/// Possible matches that get an excerpt (the newest ones); the rest show no excerpt. Bounds the reads a
/// review screen's inventory makes when a common name matches thousands of rows.
pub const POSSIBLE_EXCERPT_MAX: usize = 200;

/// Linked items that get an excerpt: linked rows are the client's by filing, not by name, so most have none;
/// the cap bounds the extra reads on a client with thousands of rows. Screen captures of a linked episode
/// are skipped (they are the episode's text).
const LINKED_EXCERPT_MAX: usize = 200;

/// The line of `text` holding the match at `span` (byte range), cut to about [`EXCERPT_CHARS`] characters
/// around it, whitespace collapsed, with "…" where text was cut.
pub(crate) fn excerpt_around(text: &str, span: (usize, usize)) -> String {
    let line_start = text[..span.0].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[span.1..].find('\n').map_or(text.len(), |i| span.1 + i);
    let line = &text[line_start..line_end];
    let (ms, me) = (span.0 - line_start, span.1 - line_start);
    let before: Vec<char> = line[..ms].chars().collect();
    let hit: Vec<char> = line[ms..me].chars().take(EXCERPT_CHARS).collect();
    let after: Vec<char> = line[me..].chars().collect();
    let room = EXCERPT_CHARS.saturating_sub(hit.len());
    // A little more after the name than before it, and unused room on one side goes to the other.
    let mut take_before = before.len().min(room * 2 / 5);
    let take_after = after.len().min(room - take_before);
    take_before = before.len().min(room - take_after);
    let mut head: String = before[before.len() - take_before..].iter().collect();
    let mut tail: String = after[..take_after].iter().collect();
    let cut_head = take_before < before.len();
    let cut_tail = take_after < after.len();
    // Do not start or end on half a word.
    if cut_head && !before[before.len() - take_before - 1].is_whitespace() {
        head = match head.find(char::is_whitespace) {
            Some(i) => head[i..].to_string(),
            None => String::new(),
        };
    }
    if cut_tail && !after[take_after].is_whitespace() {
        tail = match tail.rfind(char::is_whitespace) {
            Some(i) => tail[..i].to_string(),
            None => String::new(),
        };
    }
    let body: String = hit.iter().collect();
    let joined = format!("{head}{body}{tail}");
    let one = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}{one}{}", if cut_head { "…" } else { "" }, if cut_tail || line[ms..me].chars().count() > EXCERPT_CHARS { "…" } else { "" })
}

/// The excerpt of the first of item `id`'s texts that names the client (`terms`), or `None`. Display only: a
/// read failure is no excerpt. One row read per item.
pub(crate) fn item_excerpt(store: &memory::Store, kind: ItemKind, id: &str, terms: &[String]) -> Option<String> {
    let k = match kind {
        ItemKind::Meeting => "meeting",
        ItemKind::Email => "email",
        ItemKind::CalendarEvent => "calendar_event",
        ItemKind::Capture => "capture",
        ItemKind::Conversation => "conversation",
        ItemKind::Step => "step",
        ItemKind::McpItem => "mcp_item",
        _ => return None,
    };
    store
        .client_item_find(k, id, &|t| memory::words::first_name_span(t, terms).map(|span| excerpt_around(t, span)))
        .ok()
        .flatten()
}

/// `raw`'s first non-empty line, its whitespace collapsed, cut to [`LABEL_CHARS`] characters (with "…").
pub(crate) fn short_label(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let one = words.join(" ");
    if one.chars().count() <= LABEL_CHARS {
        return Some(one);
    }
    let cut: String = one.chars().take(LABEL_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// Item `id`'s label, or `None` (no text, or the row could not be read: the label is display only, so a read
/// failure never fails the inventory).
pub(crate) fn item_label(store: &memory::Store, kind: ItemKind, id: &str) -> Option<String> {
    let k = match kind {
        ItemKind::Meeting => "meeting",
        ItemKind::Email => "email",
        ItemKind::CalendarEvent => "calendar_event",
        ItemKind::Capture => "capture",
        ItemKind::Episode => "episode",
        ItemKind::Conversation => "conversation",
        ItemKind::Step => "step",
        ItemKind::McpItem => "mcp_item",
        ItemKind::Thread | ItemKind::VaultNote | ItemKind::ExportFolder => return None,
    };
    store.client_item_label(k, id).ok().flatten().as_deref().and_then(short_label)
}

/// Never prints `also_client`: it is a client's name.
impl fmt::Debug for ClientItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientItem")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .field("shared", &self.also_client.is_some())
            .field("date_unix_ms", &self.date_unix_ms)
            .field("removed", &self.removed)
            .field("labelled", &self.label.is_some())
            .field("excerpted", &self.excerpt.is_some())
            .finish()
    }
}

/// Places Whimbrel cannot clear for the user.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftForYou {
    MediaFolder(PathBuf),
    VaultFolder(PathBuf),
    /// Folders earlier exports of this client went to (recorded in `client_export`).
    ExportFolders(Vec<PathBuf>),
    MailboxOriginals,
    BackupsAndCloud,
    RelayCopiesBeforeThisRelease,
    /// A delete found these meeting files outside the folders it may delete in, or could not remove them.
    FilesNotRemoved(Vec<PathBuf>),
    /// Derived copies other devices keep until they are cleared there: routine results, profile suggestions,
    /// people's names and meeting briefs that named the client have no deletion record that reaches them.
    CopiesOnOtherDevices,
    /// This many items of the client were removed earlier by keep-for-N-months on this computer only:
    /// the user's other devices keep their copies of them, and this delete cannot reach those.
    ExpiredHereOnly(usize),
    /// The Obsidian vault integration is off on this computer, so the vault was not touched: notes in it
    /// that name the client are the user's to delete (Mike: vault notes are deleted only while vault sync is on).
    VaultSyncOff,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClientInventory {
    /// Owned by the client (its seed topic).
    pub linked: Vec<ClientItem>,
    /// Name matches that are not linked. Offered to the user, never deleted unasked.
    pub possible: Vec<ClientItem>,
    /// The search for possible matches stopped at its limit: more may exist.
    pub possible_truncated: bool,
    /// Linked items per kind, in kind order; kinds with none are left out.
    pub counts: Vec<(ItemKind, usize)>,
    pub left_for_you: Vec<LeftForYou>,
}

/// Folders only the shell knows about.
#[derive(Debug, Clone, Default)]
pub struct KnownFolders {
    /// Where meeting media was moved to (the media folder setting).
    pub media: Option<PathBuf>,
    /// The Obsidian vault folder the topic export writes to, while vault sync is on.
    pub vault: Option<PathBuf>,
    /// Vault sync is off here: the vault is not touched, and "left for you" says so.
    pub vault_off: bool,
}

fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

/// [`inventory_in`] with no folders known.
pub fn inventory(store: &memory::Store, key: &str) -> ApiResult<ClientInventory> {
    inventory_in(store, key, &KnownFolders::default())
}

/// What is linked to client `key` and what only names it. Errors when the client is not a
/// live seed.
pub fn inventory_in(
    store: &memory::Store,
    key: &str,
    folders: &KnownFolders,
) -> ApiResult<ClientInventory> {
    build(store, key, folders, None, true)
}

/// [`inventory_in`] without the display text (labels, excerpts, apps): the same items, for a check or a
/// delete that only needs ids. Cheaper, and safe to run inside the job pause.
pub fn inventory_lean(
    store: &memory::Store,
    key: &str,
    folders: &KnownFolders,
) -> ApiResult<ClientInventory> {
    build(store, key, folders, None, false)
}

/// [`inventory_in`] with the candidate limit per full-text table overridden (tests).
fn build(
    store: &memory::Store,
    key: &str,
    folders: &KnownFolders,
    limit: Option<i64>,
    display: bool,
) -> ApiResult<ClientInventory> {
    let _seed = store
        .list_seed_topics()
        .map_err(db)?
        .into_iter()
        .find(|s| s.key == key)
        .ok_or(ApiError::NotFound)?;
    let mut linked: Vec<ClientItem> = Vec::new();
    let mut have: HashSet<(ItemKind, String)> = HashSet::new();
    let mut add = |linked: &mut Vec<ClientItem>, kind, id: String, other, date, removed| {
        if have.insert((kind, id.clone())) {
            linked.push(ClientItem { kind, id, also_client: other, date_unix_ms: date, removed, label: None, excerpt: None, app: None });
        }
    };
    for m in store.client_members(key).map_err(db)? {
        let Some(r) = m.ref_id else { continue };
        // The source kind that holds the row, for finding who else holds it.
        let mut other = store.other_client_for(&m.kind, r, key).map_err(db)?;
        let (kind, id, date) = match m.kind.as_str() {
            "meeting" => (ItemKind::Meeting, r, m.ts_unix_ms),
            "email" => (ItemKind::Email, r, m.ts_unix_ms),
            "calendar_event" => (ItemKind::CalendarEvent, r, m.ts_unix_ms),
            "episode" => (ItemKind::Episode, r, m.ts_unix_ms),
            // A slide is its meeting: dated and shared as the meeting is.
            "snapshot" => match store.snapshot_meeting(r).map_err(db)? {
                Some((meeting, start)) => {
                    if other.is_none() {
                        other = store.other_client_for("meeting", meeting, key).map_err(db)?;
                    }
                    (ItemKind::Meeting, meeting, start)
                }
                None => continue,
            },
            _ => continue,
        };
        add(&mut linked, kind, id.to_string(), other, date, m.removed);
        // An episode's text is its captures': they are the client's too, listed so the delete covers
        // exactly what the review showed.
        if kind == ItemKind::Episode {
            for (item, twin, ts) in store.episode_captures(r).map_err(db)? {
                let id = twin.map_or_else(|| format!("i:{item}"), |c| c.to_string());
                add(&mut linked, ItemKind::Capture, id, None, ts, false);
            }
        }
    }
    for (step, asked) in store.client_step_refs(key).map_err(db)? {
        add(&mut linked, ItemKind::Step, step, None, asked, false);
    }
    linked.sort_by(|a, b| b.date_unix_ms.cmp(&a.date_unix_ms).then(a.id.cmp(&b.id)));

    // The client's name and aliases, and those of every seed merged into it.
    let terms = store.seed_chain_terms(key).map_err(db)?;
    let mut possible: Vec<ClientItem> = Vec::new();
    let set = llm::guard::ExclusionSet::new(terms.clone());
    let found = if set.is_empty() {
        memory::topics::inventory::NameHits { hits: Vec::new(), truncated: false }
    } else {
        let confirm = |t: &str| set.names_in(t);
        match limit {
            Some(l) => store.name_hits_limited(&terms, &confirm, l),
            None => store.name_hits(&terms, &confirm),
        }
        .map_err(db)?
    };
    for h in found.hits {
        let kind = match h.kind {
            "meeting" => ItemKind::Meeting,
            "email" => ItemKind::Email,
            "calendar_event" => ItemKind::CalendarEvent,
            "capture" => ItemKind::Capture,
            "conversation" => ItemKind::Conversation,
            "mcp_item" => ItemKind::McpItem,
            _ => continue,
        };
        let id = h.ref_id.to_string();
        if have.contains(&(kind, id.clone())) {
            continue;
        }
        let other = match kind {
            ItemKind::Meeting | ItemKind::Email | ItemKind::CalendarEvent => {
                store.other_client_for(h.kind, h.ref_id, key).map_err(db)?
            }
            _ => None,
        };
        possible.push(ClientItem { kind, id, also_client: other, date_unix_ms: h.ts_unix_ms, removed: false, label: None, excerpt: None, app: None });
    }
    // Added items carrying a client contact's address (they name no one by text, or not only).
    for (item, ts) in store.mcp_items_with_client_contacts(key).map_err(db)? {
        let id = item.to_string();
        if possible.iter().any(|p| p.kind == ItemKind::McpItem && p.id == id) {
            continue;
        }
        possible.push(ClientItem { kind: ItemKind::McpItem, id, also_client: None, date_unix_ms: ts, removed: false, label: None, excerpt: None, app: None });
    }

    if display {
        // Display only: a short name for each row on the review screen.
        for item in linked.iter_mut().chain(possible.iter_mut()) {
            item.label = item_label(store, item.kind, &item.id);
        }
        // Why each listed row matched: the newest possible matches and a capped number of linked rows, one
        // bounded read each.
        for item in possible.iter_mut().take(POSSIBLE_EXCERPT_MAX) {
            item.excerpt = item_excerpt(store, item.kind, &item.id, &terms);
        }
        for item in linked.iter_mut().filter(|i| i.kind != ItemKind::Capture).take(LINKED_EXCERPT_MAX) {
            item.excerpt = item_excerpt(store, item.kind, &item.id, &terms);
        }
        // The app of a possible capture, for grouping (linked captures are not listed one by one).
        for item in possible.iter_mut().filter(|i| i.kind == ItemKind::Capture) {
            item.app = store.client_capture_app(&item.id).ok().flatten();
        }
    }

    let mut counts: Vec<(ItemKind, usize)> = Vec::new();
    for item in &linked {
        match counts.iter_mut().find(|(k, _)| *k == item.kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((item.kind, 1)),
        }
    }
    counts.sort_by_key(|(k, _)| *k);

    let mut left_for_you = Vec::new();
    if let Some(p) = &folders.media {
        left_for_you.push(LeftForYou::MediaFolder(p.clone()));
    }
    if folders.vault_off {
        left_for_you.push(LeftForYou::VaultSyncOff);
    } else if let Some(p) = &folders.vault {
        left_for_you.push(LeftForYou::VaultFolder(p.clone()));
    }
    // Folders earlier exports of this client (and of seeds merged into it) were written to.
    let exports = store.client_exports(key).map_err(db)?;
    left_for_you.push(LeftForYou::ExportFolders(exports.into_iter().map(PathBuf::from).collect()));
    left_for_you.push(LeftForYou::MailboxOriginals);
    left_for_you.push(LeftForYou::BackupsAndCloud);
    left_for_you.push(LeftForYou::RelayCopiesBeforeThisRelease);

    Ok(ClientInventory { linked, possible, possible_truncated: found.truncated, counts, left_for_you })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    struct Env {
        _dir: tempfile::TempDir,
        store: memory::Store,
        c: Connection,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = crate::test_store::open(&path, dir.path());
        let c = crate::test_store::raw(&path);
        Env { _dir: dir, store, c }
    }

    fn email(e: &Env, ext: &str, subject: &str, body: &str, ts: i64) -> i64 {
        e.c.execute(
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
             VALUES ('d','email','gmail',?1,'[]',?2,?3,'{}',?4)",
            params![ts, subject, body, ext],
        )
        .unwrap();
        e.c.last_insert_rowid()
    }

    fn meeting(e: &Env, title: &str, ts: i64) -> i64 {
        e.c.execute(
            "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) \
             VALUES ('d', ?1, ?2, '[]', 'manual', 'completed')",
            params![ts, title],
        )
        .unwrap();
        e.c.last_insert_rowid()
    }

    fn file(e: &Env, topic: i64, kind: &str, r: i64, ts: i64, origin: &str) {
        let sid: i64 = match e
            .c
            .query_row(
                "SELECT id FROM topic_source WHERE kind = ?1 AND ref_id = ?2",
                params![kind, r],
                |x| x.get(0),
            ) {
            Ok(id) => id,
            Err(_) => {
                e.c.execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, ?3, 1)",
                    params![kind, r, ts],
                )
                .unwrap();
                e.c.last_insert_rowid()
            }
        };
        e.c.execute(
            "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, ?3)",
            params![topic, sid, origin],
        )
        .unwrap();
    }

    fn ids(items: &[ClientItem], kind: ItemKind) -> Vec<String> {
        let mut v: Vec<String> = items.iter().filter(|i| i.kind == kind).map(|i| i.id.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn an_added_item_naming_the_client_is_a_possible_match() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let id = crate::test_store::add_mcp_item(&e.store, "a", "Renewal notes", "Budget line\nAcme wants the discount", None);
        crate::test_store::add_mcp_item(&e.store, "b", "Lunch", "Tacos", None);
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(ids(&inv.possible, ItemKind::McpItem), vec![id.to_string()]);
        assert!(inv.linked.iter().all(|i| i.kind != ItemKind::McpItem));
        let row = inv.possible.iter().find(|i| i.kind == ItemKind::McpItem).unwrap();
        assert_eq!(row.label.as_deref(), Some("Renewal notes"));
        assert_eq!(row.excerpt.as_deref(), Some("Acme wants the discount"));
        assert_eq!(serde_json::to_value(row.kind).unwrap(), "mcp_item");
    }

    #[test]
    fn an_added_item_from_a_client_contact_is_a_possible_match() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let m = meeting(&e, "Weekly", 100);
        file(&e, acme.topic_id, "meeting", m, 100, "seed");
        let sid: i64 = e.c.query_row("SELECT id FROM topic_source WHERE kind = 'meeting' AND ref_id = ?1", [m], |r| r.get(0)).unwrap();
        e.c.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'email:ana@contoso.com', 'Ana', 1)", []).unwrap();
        let ent = e.c.last_insert_rowid();
        e.c.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (?1, ?2)", params![ent, sid]).unwrap();
        let from_contact = crate::test_store::add_mcp_item(&e.store, "a", "Quick question", "About the invoice", Some("ana@contoso.com"));
        let both = crate::test_store::add_mcp_item(&e.store, "b", "Acme invoice", "Paid", Some("ana@contoso.com"));
        crate::test_store::add_mcp_item(&e.store, "c", "Other", "Nothing", Some("pat@x.com"));
        let inv = inventory(&e.store, &acme.key).unwrap();
        let mut want = vec![from_contact.to_string(), both.to_string()];
        want.sort();
        assert_eq!(ids(&inv.possible, ItemKind::McpItem), want, "a name match and a contact match are listed once each");
    }

    #[test]
    fn linked_lists_every_member_kind() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let m = meeting(&e, "Weekly", 100);
        let em = email(&e, "g1", "Hello", "plain", 200);
        let cal = email(&e, "g2", "Invite", "plain", 300);
        e.c.execute("UPDATE items SET kind = 'calendar_event' WHERE id = ?1", [cal]).unwrap();
        e.c.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, thread_id, body_text) VALUES ('d','snapshot',400,?1,'slide')",
            [m],
        )
        .unwrap();
        let snap = e.c.last_insert_rowid();
        file(&e, acme.topic_id, "meeting", m, 100, "seed");
        file(&e, acme.topic_id, "email", em, 200, "jev");
        file(&e, acme.topic_id, "calendar_event", cal, 300, "auto");
        file(&e, acme.topic_id, "snapshot", snap, 400, "auto");
        file(&e, acme.topic_id, "episode", 9, 500, "auto");
        file(&e, acme.topic_id, "email", 99, 600, "removed");
        e.c.execute(
            "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, asked_at_unix_ms, updated_at_unix_ms) \
             VALUES ('s1', ?1, 'k', 'meeting', 't', 'x', 'mine', 700, 700)",
            [&acme.key],
        )
        .unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(ids(&inv.linked, ItemKind::Meeting), vec![m.to_string()], "snapshot folds into its meeting");
        assert_eq!(ids(&inv.linked, ItemKind::Email), vec![em.to_string()]);
        assert_eq!(ids(&inv.linked, ItemKind::CalendarEvent), vec![cal.to_string()]);
        assert_eq!(ids(&inv.linked, ItemKind::Episode), vec!["9".to_string()]);
        assert_eq!(ids(&inv.linked, ItemKind::Step), vec!["s1".to_string()]);
        assert_eq!(inv.linked.len(), 5);
        assert_eq!(inv.linked[0].kind, ItemKind::Step, "newest first");
        assert_eq!(
            inv.counts,
            vec![
                (ItemKind::Meeting, 1),
                (ItemKind::Email, 1),
                (ItemKind::CalendarEvent, 1),
                (ItemKind::Episode, 1),
                (ItemKind::Step, 1)
            ]
        );
        assert!(inventory(&e.store, "nope").is_err());
    }

    #[test]
    fn rows_carry_a_short_label_for_the_screen_and_debug_hides_it() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let m = meeting(&e, "Weekly pricing sync", 100);
        let long = format!("Re: {}\nsecond line", "budget ".repeat(30));
        let em = email(&e, "g1", &long, "plain", 200);
        file(&e, acme.topic_id, "meeting", m, 100, "seed");
        file(&e, acme.topic_id, "email", em, 200, "seed");
        e.c.execute(
            "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, asked_at_unix_ms, updated_at_unix_ms) \
             VALUES ('s1', ?1, 'k', 'meeting', 't', '  Send the\n  revised quote  ', 'mine', 700, 700)",
            [&acme.key],
        )
        .unwrap();
        let loose = email(&e, "g2", "Lunch with Acme", "body", 300);
        let untitled = email(&e, "g3", "", "Acme again", 400);
        let conv = e.store.insert_conversation("New conversation", 1).unwrap();
        e.store.append_conversation_exchange(conv, "what did Acme say?", "ok", "[]", None, 500).unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        let label = |items: &[ClientItem], kind: ItemKind, id: &str| {
            items.iter().find(|i| i.kind == kind && i.id == id).unwrap().label.clone()
        };
        assert_eq!(label(&inv.linked, ItemKind::Meeting, &m.to_string()).as_deref(), Some("Weekly pricing sync"));
        let cut = label(&inv.linked, ItemKind::Email, &em.to_string()).unwrap();
        assert_eq!(cut.chars().count(), LABEL_CHARS, "{cut}");
        assert!(cut.starts_with("Re: budget budget") && cut.ends_with('…') && !cut.contains('\n'), "{cut}");
        assert_eq!(label(&inv.linked, ItemKind::Step, "s1").as_deref(), Some("Send the"), "first line, trimmed");
        assert_eq!(label(&inv.possible, ItemKind::Email, &loose.to_string()).as_deref(), Some("Lunch with Acme"));
        assert_eq!(label(&inv.possible, ItemKind::Email, &untitled.to_string()), None, "no subject: kind and date instead");
        assert_eq!(label(&inv.possible, ItemKind::Conversation, &conv.to_string()).as_deref(), Some("what did Acme say?"), "placeholder title: first question");
        let printed = format!("{inv:?}");
        assert!(!printed.contains("Weekly pricing") && !printed.contains("Lunch with"), "Debug never prints a label: {printed}");
    }

    #[test]
    fn possible_rows_carry_the_line_that_names_the_client() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let filler = "word ".repeat(60);
        let body = format!("first line\n{filler}so we met ACME on Tuesday and agreed a price {filler}\nlast line");
        let long = email(&e, "g1", "Hi", &body, 100);
        let short = email(&e, "g2", "Hi", "lunch with Acme tomorrow", 200);
        let substring = email(&e, "g3", "Annual", "the Acmeville plan", 300);
        let title_only = email(&e, "g4", "Acme invoice", "", 400);
        let inv = inventory(&e.store, &acme.key).unwrap();
        let ex = |id: i64| inv.possible.iter().find(|i| i.id == id.to_string()).map(|i| i.excerpt.clone());
        let long_ex = ex(long).unwrap().unwrap();
        assert!(long_ex.contains("ACME on Tuesday") && long_ex.starts_with('…') && long_ex.ends_with('…'), "{long_ex}");
        assert!(!long_ex.contains('\n') && !long_ex.contains("last line") && !long_ex.contains("first line"), "{long_ex}");
        assert!(long_ex.chars().count() <= EXCERPT_CHARS + 2, "{}", long_ex.chars().count());
        assert_eq!(ex(short).unwrap().as_deref(), Some("lunch with Acme tomorrow"), "short text: whole line, no marks");
        assert!(ex(substring).is_none(), "Acmeville is not Acme: not even listed");
        assert_eq!(ex(title_only).unwrap().as_deref(), Some("Acme invoice"), "a title names the client too");
        let printed = format!("{inv:?}");
        assert!(!printed.contains("lunch with") && !printed.contains("Tuesday"), "Debug never prints an excerpt: {printed}");
    }

    #[test]
    fn excerpts_are_capped_and_lean_inventories_carry_no_display_text() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        for n in 0..(POSSIBLE_EXCERPT_MAX + 5) {
            email(&e, &format!("g{n}"), "Hi", "lunch with Acme", 1000 + n as i64);
        }
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(inv.possible.len(), POSSIBLE_EXCERPT_MAX + 5);
        assert_eq!(inv.possible.iter().filter(|i| i.excerpt.is_some()).count(), POSSIBLE_EXCERPT_MAX);
        assert!(inv.possible[0].excerpt.is_some() && inv.possible.last().unwrap().excerpt.is_none(), "newest first");
        let lean = inventory_lean(&e.store, &acme.key, &KnownFolders::default()).unwrap();
        assert_eq!(lean.possible.len(), inv.possible.len());
        assert!(lean.possible.iter().all(|i| i.excerpt.is_none() && i.label.is_none() && i.app.is_none()));
    }

    #[test]
    fn a_meeting_excerpt_comes_from_the_transcript_segment_that_names_the_client() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let m = meeting(&e, "Weekly", 100);
        let segs: Vec<memory::NewTranscriptSegment> = ["hello all", "so ACME wants a quote by Friday", "bye"]
            .iter()
            .enumerate()
            .map(|(i, t)| memory::NewTranscriptSegment { meeting_id: m, start_cs: i as i64 * 100, speaker_label: "A".into(), text: (*t).into() })
            .collect();
        e.store.insert_transcript_segments(&segs).unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(inv.possible[0].excerpt.as_deref(), Some("so ACME wants a quote by Friday"));
    }

    /// A meeting whose only mention of the client is in a note line's source (the owner or due date of an
    /// action item, or the normalized copy of the spoken line) is a possible match, counted in the inventory.
    #[test]
    fn a_meeting_whose_line_source_names_the_client_is_a_possible_match() {
        use memory::line_source::{anchor_of, base_fingerprint, LineKind, NewLineSource, SourceLocator};
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let row = |owner: Option<&str>, due: Option<&str>, spoken: Option<&str>| NewLineSource {
            kind: LineKind::ActionItem,
            line_text: "Send the deck".into(),
            owner: owner.map(Into::into),
            due: due.map(Into::into),
            locator: spoken.map(|s| SourceLocator { offset_secs: 0, base: base_fingerprint("x"), anchor: anchor_of(s) }),
        };
        let by_owner = meeting(&e, "Weekly", 100);
        let by_due = meeting(&e, "Weekly", 200);
        let by_anchor = meeting(&e, "Weekly", 300);
        let quiet = meeting(&e, "Weekly", 400);
        e.store.replace_meeting_line_sources(by_owner, &[row(Some("Acme"), None, None)], 1).unwrap();
        e.store.replace_meeting_line_sources(by_due, &[row(Some("Ana"), Some("after the Acme call"), None)], 1).unwrap();
        e.store.replace_meeting_line_sources(by_anchor, &[row(None, None, Some("So, ACME wants it!"))], 1).unwrap();
        e.store.replace_meeting_line_sources(quiet, &[row(Some("Ana"), Some("Friday"), Some("send the deck"))], 1).unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        let mut found: Vec<String> = ids(&inv.possible, ItemKind::Meeting);
        found.sort();
        let mut want = vec![by_owner.to_string(), by_due.to_string(), by_anchor.to_string()];
        want.sort();
        assert_eq!(found, want, "the owner, the due date and the anchor each name the client; the quiet one does not");
    }

    #[test]
    fn an_item_with_no_text_that_names_the_client_has_no_excerpt() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let m = meeting(&e, "Weekly", 100);
        file(&e, acme.topic_id, "meeting", m, 100, "seed");
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(inv.linked[0].excerpt, None, "a linked meeting that never says the name");
        let json = serde_json::to_string(&inv.linked[0]).unwrap();
        assert!(!json.contains("excerpt"), "additive: absent when none: {json}");
    }

    #[test]
    fn a_capture_excerpt_comes_from_its_text_and_carries_its_app() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        e.c.execute(
            "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, text, hash) \
             VALUES ('d', 300, 'com.apple.Terminal', 'zsh', 'ls\nclaude --resume for Acme\nexit', 1)",
            [],
        )
        .unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(inv.possible[0].excerpt.as_deref(), Some("claude --resume for Acme"));
        assert_eq!(inv.possible[0].app.as_deref(), Some("com.apple.Terminal"));
    }

    #[test]
    fn excerpt_cuts_on_characters_and_whole_words() {
        let text = format!("{} Zoë-Acme {}", "é".repeat(300), "ü".repeat(300));
        let span = memory::words::first_name_span(&text, &["acme".to_string()]).unwrap();
        let ex = excerpt_around(&text, span);
        assert!(ex.contains("Acme") && ex.chars().count() <= EXCERPT_CHARS + 2, "{ex}");
        let wrapped = "the quick brown fox jumps over the lazy dog and then Acme called back about the quote we sent";
        let span = memory::words::first_name_span(wrapped, &["acme".to_string()]).unwrap();
        assert_eq!(&wrapped[span.0..span.1], "Acme");
    }

    #[test]
    fn a_label_is_cut_by_characters_not_bytes() {
        let s = "é".repeat(200);
        let l = short_label(&s).unwrap();
        assert_eq!(l.chars().count(), LABEL_CHARS);
        assert_eq!(short_label("  \n\t \n"), None);
        assert_eq!(short_label("a\tb   c"), Some("a b c".to_string()));
    }

    #[test]
    fn possible_finds_unlinked_email_capture_and_conversation_by_alias() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &["Initech".to_string()], None).unwrap();
        let linked = email(&e, "g1", "Re: Acme", "linked mail", 100);
        file(&e, acme.topic_id, "email", linked, 100, "seed");
        let loose = email(&e, "g2", "Quarterly", "Notes from the INITECH call", 200);
        email(&e, "g3", "Lunch", "nothing to see", 250);
        e.c.execute(
            "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, text, hash) \
             VALUES ('d', 300, 'app', 'Doc', 'draft for Initech', 1)",
            [],
        )
        .unwrap();
        let cap = e.c.last_insert_rowid();
        let conv = e.store.insert_conversation("t", 1).unwrap();
        e.store
            .append_conversation_exchange(conv, "what did initech say?", "ok", "[]", None, 400)
            .unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(ids(&inv.possible, ItemKind::Email), vec![loose.to_string()], "linked mail is not repeated");
        assert_eq!(ids(&inv.possible, ItemKind::Capture), vec![cap.to_string()]);
        assert_eq!(ids(&inv.possible, ItemKind::Conversation), vec![conv.to_string()]);
        assert_eq!(inv.possible.len(), 3);
        assert_eq!(inv.possible[0].kind, ItemKind::Conversation, "newest first");
    }

    /// Review 5, M2: the delete inventory lists what really names the client, decoded (an
    /// entity-encoded name), and never a row whose text merely cannot be decoded.
    #[test]
    fn possible_lists_encoded_names_and_never_merely_undecodable_text() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let deep = format!("Newsletter R&{}D (unrelated)", "amp;".repeat(memory::words::MAX_DECODE_PASSES + 1));
        assert!(memory::words::readings(&deep).undecidable);
        let undecodable = e.store.insert_conversation("u", 1).unwrap();
        e.store.append_conversation_exchange(undecodable, "what is new?", &deep, "[]", None, 400).unwrap();
        let encoded = e.store.insert_conversation("e", 2).unwrap();
        e.store.append_conversation_exchange(encoded, "what did Ac&#109;e say?", "ok", "[]", None, 500).unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(ids(&inv.possible, ItemKind::Conversation), vec![encoded.to_string()]);
    }

    #[test]
    fn substring_name_does_not_match() {
        let e = env();
        let ann = e.store.create_seed_topic("Ann", &[], None).unwrap();
        email(&e, "g1", "Annual review", "the annual plan", 100);
        let hit = email(&e, "g2", "Hi", "lunch with ann tomorrow", 200);
        meeting(&e, "Annual kickoff", 300);
        let inv = inventory(&e.store, &ann.key).unwrap();
        assert_eq!(inv.possible.len(), 1);
        assert_eq!(inv.possible[0].id, hit.to_string());
    }

    #[test]
    fn possible_finds_a_meeting_by_transcript_and_by_title() {
        let e = env();
        let acme = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let said = meeting(&e, "Weekly", 100);
        let titled = meeting(&e, "Café Acme visit", 200);
        meeting(&e, "Unrelated", 300);
        e.store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: said,
                start_cs: 0,
                speaker_label: "A".into(),
                text: "so ACME wants a quote".into(),
            }])
            .unwrap();
        let inv = inventory(&e.store, &acme.key).unwrap();
        assert_eq!(ids(&inv.possible, ItemKind::Meeting), {
            let mut v = vec![said.to_string(), titled.to_string()];
            v.sort();
            v
        });
    }

    #[test]
    fn shared_meeting_reports_other_client() {
        let e = env();
        let a = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let g = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let m = meeting(&e, "Joint sync", 100);
        file(&e, a.topic_id, "meeting", m, 100, "seed");
        file(&e, g.topic_id, "meeting", m, 100, "seed");
        let only = meeting(&e, "Acme only", 200);
        file(&e, a.topic_id, "meeting", only, 200, "seed");
        let inv = inventory(&e.store, &a.key).unwrap();
        let shared = inv.linked.iter().find(|i| i.id == m.to_string()).unwrap();
        assert_eq!(shared.also_client.as_deref(), Some("Globex"));
        assert_eq!(inv.linked.iter().find(|i| i.id == only.to_string()).unwrap().also_client, None);
        assert!(!format!("{inv:?}").contains("Globex"), "Debug never prints a client name");
    }

    #[test]
    fn left_for_you_lists_media_vault_exports_mailbox_and_backups() {
        let e = env();
        let a = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let folders = KnownFolders {
            media: Some(PathBuf::from("/media")),
            vault: Some(PathBuf::from("/vault")),
            vault_off: false,
        };
        let inv = inventory_in(&e.store, &a.key, &folders).unwrap();
        assert_eq!(
            inv.left_for_you,
            vec![
                LeftForYou::MediaFolder("/media".into()),
                LeftForYou::VaultFolder("/vault".into()),
                LeftForYou::ExportFolders(vec![]),
                LeftForYou::MailboxOriginals,
                LeftForYou::BackupsAndCloud,
                LeftForYou::RelayCopiesBeforeThisRelease,
            ]
        );
        let bare = inventory(&e.store, &a.key).unwrap();
        assert_eq!(bare.left_for_you.len(), 4);
    }

    #[test]
    fn names_with_search_syntax_are_data_not_queries() {
        let e = env();
        for (name, hit, miss) in [
            ("O'Brien", "lunch with O'Brien", "lunch with Brien"),
            ("Acme OR x", "met Acme OR x today", "Acme only"),
            ("say \"hi\"", "they say \"hi\" a lot", "say bye"),
            ("star*", "the star* project", "starship"),
            ("(a) NEAR b:c", "see (a) NEAR b:c now", "near b"),
        ] {
            let seed = e.store.create_seed_topic(name, &[], None).unwrap();
            let h = email(&e, &format!("h-{name}"), "t", hit, 100);
            email(&e, &format!("m-{name}"), "t", miss, 200);
            let inv = inventory(&e.store, &seed.key).unwrap_or_else(|_| panic!("{name}"));
            assert_eq!(ids(&inv.possible, ItemKind::Email), vec![h.to_string()], "{name}");
            e.store.delete_seed_topic(&seed.key).unwrap();
            e.c.execute("DELETE FROM items", []).unwrap();
        }
    }

    #[test]
    fn sharp_s_and_ss_match_each_other_through_the_index() {
        let e = env();
        let a = e.store.create_seed_topic("Strauss", &[], None).unwrap();
        let b = email(&e, "g1", "Hi", "Herr STRAUSS rang an", 100);
        let c = email(&e, "g2", "Hi", "Herr Strauß rang an", 200);
        email(&e, "g3", "Hi", "Strausberg", 300);
        let inv = inventory(&e.store, &a.key).unwrap();
        assert_eq!(ids(&inv.possible, ItemKind::Email), {
            let mut v = vec![b.to_string(), c.to_string()];
            v.sort();
            v
        });
        let d = e.store.create_seed_topic("Strauß", &[], None).unwrap();
        let inv = inventory(&e.store, &d.key).unwrap();
        assert_eq!(inv.possible.len(), 2);
    }

    #[test]
    fn a_possible_match_held_by_another_client_names_it() {
        let e = env();
        let a = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let g = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let m = email(&e, "g1", "Re", "Acme and Globex pricing", 100);
        file(&e, g.topic_id, "email", m, 100, "seed");
        let inv = inventory(&e.store, &a.key).unwrap();
        assert_eq!(inv.possible.len(), 1);
        assert_eq!(inv.possible[0].also_client.as_deref(), Some("Globex"));
    }

    #[test]
    fn a_deleted_client_is_not_reported_as_the_other_holder() {
        let e = env();
        let a = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let g = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let m = meeting(&e, "Joint", 100);
        file(&e, a.topic_id, "meeting", m, 100, "seed");
        file(&e, g.topic_id, "meeting", m, 100, "seed");
        e.store.delete_seed_topic(&g.key).unwrap();
        let inv = inventory(&e.store, &a.key).unwrap();
        assert_eq!(inv.linked[0].also_client, None);
    }

    #[test]
    fn a_slide_is_dated_and_shared_as_its_meeting() {
        let e = env();
        let a = e.store.create_seed_topic("Acme", &[], None).unwrap();
        let g = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let m = meeting(&e, "Review", 1_000);
        e.c.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, thread_id, body_text) VALUES ('d','snapshot',5000,?1,'slide about Acme')",
            [m],
        )
        .unwrap();
        let snap = e.c.last_insert_rowid();
        file(&e, a.topic_id, "snapshot", snap, 5_000, "auto");
        file(&e, g.topic_id, "meeting", m, 1_000, "seed");
        let inv = inventory(&e.store, &a.key).unwrap();
        assert_eq!(inv.linked.len(), 1);
        assert_eq!(inv.linked[0].date_unix_ms, 1_000);
        assert_eq!(inv.linked[0].also_client.as_deref(), Some("Globex"));
        // Unlinked slide matched by text: the meeting's start, not the slide's time.
        let b = e.store.create_seed_topic("Initech", &[], None).unwrap();
        e.c.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, thread_id, body_text) VALUES ('d','snapshot',6000,?1,'Initech roadmap')",
            [m],
        )
        .unwrap();
        let inv = inventory(&e.store, &b.key).unwrap();
        assert_eq!(inv.possible.len(), 1);
        assert_eq!(inv.possible[0].date_unix_ms, 1_000);
    }

    #[test]
    fn truncation_is_reported_when_the_candidate_limit_is_hit() {
        let e = env();
        let a = e.store.create_seed_topic("Acme", &[], None).unwrap();
        email(&e, "g1", "t", "Acme one", 100);
        email(&e, "g2", "t", "Acme two", 200);
        let none = KnownFolders::default();
        let cut = build(&e.store, &a.key, &none, Some(1), true).unwrap();
        assert!(cut.possible_truncated);
        assert_eq!(cut.possible.len(), 1);
        let full = build(&e.store, &a.key, &none, Some(100), true).unwrap();
        assert!(!full.possible_truncated);
        assert_eq!(full.possible.len(), 2);
    }

    #[test]
    fn a_client_that_is_not_a_live_seed_is_not_found() {
        let e = env();
        assert!(matches!(inventory(&e.store, "nope"), Err(ApiError::NotFound)));
        let a = e.store.create_seed_topic("Acme", &[], None).unwrap();
        e.store.delete_seed_topic(&a.key).unwrap();
        assert!(matches!(inventory(&e.store, &a.key), Err(ApiError::NotFound)));
    }
}
