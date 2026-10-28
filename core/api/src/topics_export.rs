//! Obsidian-style Markdown export of topics, people and orgs (Connected Topics, Task 7).
//!
//! Everything lands under `root/Whimbrel/`. File names and every piece of text that came
//! from outside (email names/subjects, OCR, window titles, model output) is attacker
//! controllable, so names are sanitized to a safe stem, text is Markdown-escaped, and each
//! write is checked to resolve inside the intended folder. Existing files are found only by
//! scanning `Whimbrel/**/*.md` frontmatter for `whimbrel_id` (no DB table), so a file the
//! user renames or moves in Obsidian is still recognized, and files without a valid
//! `whimbrel_id` are never touched. Symlinks are never followed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use memory::topics::read::{TopicDetail, TopicEntityMention, TopicMember};
use memory::Store;
use serde::{Deserialize, Serialize};

use crate::envelope::{ApiError, ApiResult};

const NOTES_HEADER: &str = "## My notes";
const MAX_STEM_CHARS: usize = 80;
const MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024;
const TOPIC_LIMIT: usize = 1_000_000;
/// Untrusted text is cut to this many chars before escaping (titles, names).
const MAX_TEXT_CHARS: usize = 300;
/// Same for a topic's status line.
const MAX_STATUS_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub written: usize,
    pub unchanged: usize,
    pub archived: usize,
    pub deleted: usize,
    /// Our files left untouched because they have no `## My notes` line.
    pub skipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Owner {
    Topic(i64),
    Person(i64),
    Org(i64),
}

impl Owner {
    fn tag(self) -> String {
        match self {
            Owner::Topic(i) => format!("topic-{i}"),
            Owner::Person(i) => format!("person-{i}"),
            Owner::Org(i) => format!("org-{i}"),
        }
    }

    fn parse(s: &str) -> Option<Owner> {
        let (k, n) = s.split_once('-')?;
        let n: i64 = n.parse().ok().filter(|n| *n >= 0)?;
        match k {
            "topic" => Some(Owner::Topic(n)),
            "person" => Some(Owner::Person(n)),
            "org" => Some(Owner::Org(n)),
            _ => None,
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Owner::Topic(_) => "Topics",
            Owner::Person(_) => "People",
            Owner::Org(_) => "Organizations",
        }
    }
}

// Same convention as `api::topics`: storage/IO failures surface as the one non-cap error.
fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

fn ioe(_: io::Error) -> ApiError {
    ApiError::ProviderFailure
}

fn is_format_char(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}')
}

/// Whitespace-collapsed text with control and format/bidi characters removed.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|&c| !(c.is_control() || is_format_char(c)))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Untrusted text for Markdown body lines: cannot form links, wikilinks, headings, HTML or
/// table cells, and is always a single line. Capped at `MAX_TEXT_CHARS` before escaping.
fn md_text(s: &str) -> String {
    let cleaned = clean(&cap(s, MAX_TEXT_CHARS));
    let chars: Vec<char> = cleaned.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        // Backslash-escape Markdown/Obsidian syntax; defuse bare-URL autolinks by escaping
        // the scheme colon (`https\://`) and the dot of `www.`.
        let url_colon = c == ':' && chars[i + 1..].starts_with(&['/', '/']);
        let www_dot =
            c == '.' && i >= 3 && chars[i - 3..i].iter().all(|w| w.eq_ignore_ascii_case(&'w'));
        if "[]()<>#|\\`*_!%$=".contains(c) || url_colon || www_dot {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// [`md_text`] for a line that stands alone: its first character must not open a list item,
/// rule or similar.
fn md_line(s: &str) -> String {
    let t = md_text(&cap(s, MAX_STATUS_CHARS));
    if t.starts_with(['-', '+', '~']) {
        format!("\\{t}")
    } else {
        t
    }
}

fn cap(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Untrusted text inside a YAML double-quoted scalar.
fn yaml_text(s: &str) -> String {
    clean(s).replace('\\', "\\\\").replace('"', "\\\"")
}

/// A safe file stem (no extension) from an untrusted name.
fn file_stem(name: &str) -> String {
    let kept: String = clean(name)
        .chars()
        .filter(|c| !"[]#^|\\/:*?\"<>".contains(*c))
        .collect();
    let trim = |s: &str| s.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    let capped: String = trim(&kept).chars().take(MAX_STEM_CHARS).collect();
    let mut stem = trim(&capped);
    if stem.is_empty() {
        return "Untitled".into();
    }
    let base = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        stem.push('_');
    }
    stem
}

fn iso_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".into())
}

// ---- reading existing files ----

struct Existing {
    path: PathBuf,
    text: String,
    /// Byte offset just past the `## My notes` line; `None` when the file has no such line.
    notes_at: Option<usize>,
    fm_end: usize,
    /// Byte length of a leading BOM (0 or 3), kept on rewrite.
    bom: usize,
}

impl Existing {
    fn notes(&self) -> Option<&str> {
        self.notes_at.map(|i| &self.text[i..])
    }

    fn title(&self) -> Option<&str> {
        self.text[self.fm_end..]
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .map(str::trim)
    }

    fn stem(&self) -> String {
        self.path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// `(owner, byte offset just past the closing ---)` for a file whose frontmatter has a valid
/// `whimbrel_id`.
fn parse_frontmatter(text: &str) -> Option<(Owner, usize)> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return None;
    }
    let mut off = first.len();
    let mut owner = None;
    for l in lines {
        off += l.len();
        let t = l.trim_end_matches(['\r', '\n']);
        if t == "---" {
            return owner.map(|o| (o, off));
        }
        if owner.is_none() {
            if let Some(v) = t.strip_prefix("whimbrel_id:") {
                let v = v
                    .split(" #")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches(['"', '\'']);
                owner = Owner::parse(v);
            }
        }
    }
    None
}

fn notes_offset(text: &str, from: usize) -> Option<usize> {
    let mut off = from;
    for l in text[from..].split_inclusive('\n') {
        off += l.len();
        if l.trim_end_matches(['\r', '\n']) == NOTES_HEADER {
            return Some(off);
        }
    }
    None
}

struct Scan {
    owned: HashMap<Owner, Existing>,
    /// Lower-cased stems of every `.md` under Whimbrel (ours, archived, foreign): wikilinks
    /// are by stem, so stems stay unique across the whole tree.
    stems: HashSet<String>,
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let ft = e.file_type()?; // does not follow symlinks
        if ft.is_dir() {
            walk(&e.path(), out)?;
        } else if ft.is_file()
            && e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("md"))
        {
            out.push(e.path());
        }
    }
    Ok(())
}

fn scan(wh: &Path) -> io::Result<Scan> {
    let mut files = vec![];
    walk(wh, &mut files)?;
    let archive = wh.join("Archive");
    let mut s = Scan {
        owned: HashMap::new(),
        stems: HashSet::new(),
    };
    for path in files {
        if let Some(stem) = path.file_stem() {
            s.stems.insert(stem.to_string_lossy().to_lowercase());
        }
        if path.starts_with(&archive) || fs::metadata(&path)?.len() > MAX_SCAN_BYTES {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let bom = if text.starts_with('\u{FEFF}') { 3 } else { 0 };
        let Some((owner, fm_end)) = parse_frontmatter(&text[bom..]) else {
            continue;
        };
        let fm_end = fm_end + bom;
        let notes_at = notes_offset(&text, fm_end);
        // A duplicated id (user copied a file): the first path wins, copies stay untouched.
        s.owned.entry(owner).or_insert(Existing {
            path,
            text,
            notes_at,
            fm_end,
            bom,
        });
    }
    Ok(s)
}

// ---- writing ----

/// `p` must be (or become) a real directory, never a symlink.
fn ensure_dir(p: &Path) -> io::Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to write through a symlink or non-directory inside Whimbrel",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(p),
        Err(e) => Err(e),
    }
}

/// The write target must be a single file name directly inside `folder`, and `folder` must
/// be inside the Whimbrel root.
fn check_inside(wh: &Path, folder: &Path, path: &Path) -> io::Result<()> {
    let name = path.file_name().map(Path::new);
    let ok = path.parent() == Some(folder)
        && folder.starts_with(wh)
        && name.is_some_and(|n| n.components().count() == 1)
        && path.file_name().is_some_and(|n| n != "..");
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path escapes the export folder",
        ))
    }
}

fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".whimbrel-{}-{n}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let res = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

fn unique_stem(base: &str, used: &mut HashSet<String>, dir: &Path) -> String {
    let mut n = 1;
    loop {
        let cand = if n == 1 {
            base.to_string()
        } else {
            format!("{base} ({n})")
        };
        let key = cand.to_lowercase();
        if !used.contains(&key) && fs::symlink_metadata(dir.join(format!("{cand}.md"))).is_err() {
            used.insert(key);
            return cand;
        }
        n += 1;
    }
}

// ---- rendering ----

fn link_kind(window: &str) -> Option<&'static str> {
    match window {
        "meeting-detail" => Some("meeting"),
        "thread-detail" => Some("email-thread"),
        "episode-detail" => Some("episode"),
        "capture-detail" => Some("capture"),
        _ => None,
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let mut out = String::new();
        let mut it = v[1..v.len() - 1].chars();
        while let Some(c) = it.next() {
            out.push(if c == '\\' {
                it.next().unwrap_or('\\')
            } else {
                c
            });
        }
        out
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else {
        v.to_string()
    }
}

/// Splits `a, "b, c", 'd'` on commas outside quotes.
fn split_flow(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut q, mut esc) = (vec![], String::new(), None::<char>, false);
    for c in s.chars() {
        match (q, c) {
            (Some(_), _) if esc => esc = false,
            (Some('"'), '\\') => esc = true,
            (Some(x), c) if c == x => q = None,
            (None, '"' | '\'') => q = Some(c),
            (None, ',') => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.iter()
        .map(|v| unquote(v))
        .filter(|v| !v.is_empty())
        .collect()
}

/// The user's frontmatter from an existing file: `(aliases, other keys verbatim)`. Whimbrel
/// owns only `whimbrel_id`, `updated` and its own alias entry.
fn user_frontmatter(ex: &Existing) -> (Vec<String>, String) {
    let fm = &ex.text[ex.bom..ex.fm_end];
    let mut groups: Vec<Vec<&str>> = vec![];
    for l in fm.split_inclusive('\n').skip(1) {
        if l.trim_end_matches(['\r', '\n']) == "---" {
            break;
        }
        let cont = l.starts_with([' ', '\t']) || l.starts_with("- ");
        match groups.last_mut() {
            Some(g) if cont => g.push(l),
            _ => groups.push(vec![l]),
        }
    }
    let (mut aliases, mut extra) = (vec![], String::new());
    for g in groups {
        let key = g[0].split(':').next().unwrap_or("").trim();
        match key {
            "whimbrel_id" | "updated" => {}
            "aliases" => {
                let rest = g[0].split_once(':').map_or("", |x| x.1).trim();
                if let Some(inner) = rest.strip_prefix('[') {
                    aliases = split_flow(inner.trim_end().trim_end_matches(']'));
                } else if rest.is_empty() {
                    aliases = g[1..]
                        .iter()
                        .map(|l| unquote(l.trim().trim_start_matches('-')))
                        .filter(|v| !v.is_empty())
                        .collect();
                } else {
                    aliases = vec![unquote(rest)];
                }
            }
            _ => {
                for l in g {
                    extra.push_str(l);
                    if !l.ends_with('\n') {
                        extra.push('\n');
                    }
                }
            }
        }
    }
    (aliases, extra)
}

fn head(
    owner: Owner,
    title: &str,
    updated_ms: i64,
    status: Option<&str>,
    sections: &[(&str, Vec<String>)],
    existing: Option<&Existing>,
) -> String {
    let own = clean(&cap(title, MAX_TEXT_CHARS));
    let (user_aliases, extra) = existing.map(user_frontmatter).unwrap_or_default();
    // Drop the alias Whimbrel wrote last time (it matches the old heading), keep the user's.
    let old_heading = existing.and_then(|e| e.title()).unwrap_or("");
    let mut aliases = vec![own.clone()];
    for a in user_aliases {
        let a = clean(&a);
        if !a.is_empty() && !aliases.contains(&a) && md_text(&a) != old_heading {
            aliases.push(a);
        }
    }
    let aliases: Vec<String> = aliases
        .iter()
        .map(|a| format!("\"{}\"", yaml_text(a)))
        .collect();
    let mut s = format!(
        "---\nwhimbrel_id: {}\naliases: [{}]\nupdated: {}\n{}---\n# {}\n",
        owner.tag(),
        aliases.join(", "),
        iso_date(updated_ms),
        extra,
        md_text(title)
    );
    if let Some(st) = status.map(md_line).filter(|t| !t.is_empty()) {
        s.push_str(&st);
        s.push('\n');
    }
    for (h, lines) in sections.iter().filter(|(_, l)| !l.is_empty()) {
        s.push_str(&format!("## {h}\n"));
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
    }
    s
}

struct TopicData {
    detail: TopicDetail,
    mentions: Vec<TopicEntityMention>,
}

struct Entity {
    org: bool,
    display: String,
    topics: Vec<i64>,
}

/// Moves a vanished owner's file into `Archive/` as frontmatter, heading, archive note and
/// `## My notes`: the generated sections may name content the user deleted, so they go.
/// `strip_notes` empties the notes too (merge: notes moved). A file with no `## My notes`
/// line keeps its body, since its notes can't be told apart from generated text.
fn archive_file(wh: &Path, ex: &Existing, strip_notes: bool, now_ms: i64) -> io::Result<()> {
    let dir = wh.join("Archive");
    ensure_dir(&dir)?;
    let (fm, rest) = ex.text.split_at(ex.fm_end);
    let note = format!(
        "> Archived {}: this topic no longer exists in Whimbrel.\n",
        iso_date(now_ms)
    );
    let text = match ex.notes() {
        Some(notes) => {
            let heading = rest
                .lines()
                .find(|l| l.starts_with("# "))
                .map(|l| format!("{l}\n"))
                .unwrap_or_default();
            let notes = if strip_notes { "" } else { notes };
            format!("{fm}{heading}{note}{NOTES_HEADER}\n{notes}")
        }
        None => format!("{fm}{note}{rest}"),
    };
    // Archived files are never wikilink targets, so only the folder itself must be collision-free.
    let stem = unique_stem(&file_stem(&ex.stem()), &mut HashSet::new(), &dir);
    let target = dir.join(format!("{stem}.md"));
    check_inside(wh, &dir, &target)?;
    write_atomic(&target, &text)?;
    fs::remove_file(&ex.path)
}

/// `root/Whimbrel`, resolved once if it is a symlink (iCloud/Dropbox vaults). The link is
/// followed only to a directory inside `root`, or to one that is empty or holds nothing but
/// our own folders (dot-files aside), so a planted link can't aim the export at `$HOME` or a
/// system folder. Symlinks anywhere inside it stay refused (see [`ensure_dir`]).
fn resolve_whimbrel(root: &Path) -> io::Result<PathBuf> {
    let wh = root.join("Whimbrel");
    if !fs::symlink_metadata(&wh).is_ok_and(|m| m.file_type().is_symlink()) {
        return Ok(wh);
    }
    let target = fs::canonicalize(&wh)?;
    if target.starts_with(fs::canonicalize(root)?) {
        return Ok(target);
    }
    for e in fs::read_dir(&target)? {
        let name = e?.file_name();
        let name = name.to_string_lossy();
        let ours = ["People", "Organizations", "Topics", "Archive"].contains(&&*name);
        if !name.starts_with('.') && !ours {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to follow the Whimbrel symlink: it leads outside the export folder \
                 to a directory holding other files",
            ));
        }
    }
    Ok(target)
}

pub fn export(store: &Store, root: &Path, link_base: &str, now_ms: i64) -> ApiResult<ExportReport> {
    fs::create_dir_all(root).map_err(ioe)?;
    let wh = resolve_whimbrel(root).map_err(ioe)?;
    ensure_dir(&wh).map_err(ioe)?;
    let mut report = ExportReport::default();

    // Live owners.
    let mut topics: BTreeMap<i64, TopicData> = BTreeMap::new();
    let mut entities: BTreeMap<i64, Entity> = BTreeMap::new();
    for t in store.list_topics(false, TOPIC_LIMIT).map_err(db)? {
        let Some(detail) = store.topic_detail(t.id).map_err(db)? else {
            continue;
        };
        let mentions = store.topic_entity_mentions(t.id).map_err(db)?;
        for m in &mentions {
            let e = entities.entry(m.entity_id).or_insert_with(|| Entity {
                org: m.kind == "org",
                display: m.display.clone(),
                topics: vec![],
            });
            if !e.topics.contains(&t.id) {
                e.topics.push(t.id);
            }
        }
        topics.insert(t.id, TopicData { detail, mentions });
    }
    let owner_of = |id: i64, e: &Entity| {
        if e.org {
            Owner::Org(id)
        } else {
            Owner::Person(id)
        }
    };
    let mut live: Vec<Owner> = topics.keys().map(|&i| Owner::Topic(i)).collect();
    live.extend(entities.iter().map(|(&i, e)| owner_of(i, e)));
    let live_set: HashSet<Owner> = live.iter().copied().collect();

    // Existing files and stems.
    let Scan {
        owned,
        stems: mut used,
    } = scan(&wh).map_err(ioe)?;
    let mut paths: HashMap<Owner, PathBuf> = HashMap::new();
    let mut stems: HashMap<Owner, String> = HashMap::new();
    for &o in &live {
        if let Some(ex) = owned.get(&o) {
            paths.insert(o, ex.path.clone());
            stems.insert(o, ex.stem());
            continue;
        }
        let name = match o {
            Owner::Topic(i) => topics[&i].detail.summary.name.clone(),
            Owner::Person(i) | Owner::Org(i) => entities[&i].display.clone(),
        };
        let dir = wh.join(o.folder());
        let stem = unique_stem(&file_stem(&name), &mut used, &dir);
        paths.insert(o, dir.join(format!("{stem}.md")));
        stems.insert(o, stem);
    }

    // Vanished owners: merged into a live topic, deleted, or archived.
    let mut vanished: Vec<(&Owner, &Existing)> = owned
        .iter()
        .filter(|(o, _)| !live_set.contains(o))
        .collect();
    vanished.sort_by_key(|(_, ex)| ex.path.clone());
    let mut appended: HashMap<Owner, Vec<String>> = HashMap::new();
    let mut to_archive: Vec<(&Existing, bool)> = vec![];
    let mut to_delete: Vec<&Existing> = vec![];
    for (&o, ex) in vanished {
        let survivor = match o {
            Owner::Topic(id) => store
                .topic_detail(id)
                .map_err(db)?
                .map(|d| Owner::Topic(d.summary.id))
                .filter(|s| *s != o && live_set.contains(s))
                // Without a `## My notes` line in the survivor we cannot place notes safely.
                .filter(|s| owned.get(s).is_none_or(|e| e.notes_at.is_some())),
            _ => None,
        };
        let notes = ex.notes();
        match (survivor, notes) {
            // Nothing of the user's to keep (merged or not): delete (spec §7.3).
            (_, Some(n)) if n.trim().is_empty() => to_delete.push(ex),
            (Some(s), Some(n)) => {
                if !n.trim().is_empty() {
                    let name = ex.title().map(clean).unwrap_or_else(|| o.tag());
                    let nl = if n.ends_with('\n') { "" } else { "\n" };
                    appended
                        .entry(s)
                        .or_default()
                        .push(format!("> From merged topic \"{name}\":\n{n}{nl}"));
                }
                to_archive.push((ex, true));
            }
            _ => to_archive.push((ex, false)),
        }
    }

    // Render and write live owners.
    let rel_link = |m: &TopicMember| {
        let kind = link_kind(&m.open.as_ref()?.window)?;
        Some(format!(
            "{link_base}open?kind={kind}&id={}",
            m.open.as_ref()?.id
        ))
    };
    let wikilink = |o: Owner| format!("- [[{}]]", stems[&o]);
    for &o in &live {
        let body = match o {
            Owner::Topic(id) => {
                let td = &topics[&id];
                let sm = &td.detail.summary;
                let members: Vec<String> = td
                    .detail
                    .members
                    .iter()
                    .filter(|m| m.origin != "removed")
                    .map(|m| {
                        let mut line = format!(
                            "- {} · {} · ",
                            iso_date(m.ts_unix_ms),
                            md_text(&capitalize(&m.kind))
                        );
                        let t = md_text(&m.title);
                        let t = if t.is_empty() {
                            "(untitled)".to_string()
                        } else {
                            t
                        };
                        match rel_link(m) {
                            Some(u) => line.push_str(&format!("[{t}]({u})")),
                            None => line.push_str(&t),
                        }
                        let mut with: Vec<i64> = td
                            .mentions
                            .iter()
                            .filter(|x| x.source_id == m.source_id && x.kind == "person")
                            .map(|x| x.entity_id)
                            .collect();
                        with.dedup();
                        if !with.is_empty() {
                            let names: Vec<String> = with
                                .iter()
                                .map(|&i| format!("[[{}]]", stems[&Owner::Person(i)]))
                                .collect();
                            line.push_str(&format!(" · with {}", names.join(", ")));
                        }
                        line
                    })
                    .collect();
                let ids = |kind: &str| -> Vec<i64> {
                    let mut v: Vec<i64> = td
                        .mentions
                        .iter()
                        .filter(|x| x.kind == kind)
                        .map(|x| x.entity_id)
                        .collect();
                    v.dedup();
                    v
                };
                let people: Vec<String> = ids("person")
                    .into_iter()
                    .map(|i| wikilink(Owner::Person(i)))
                    .collect();
                let orgs: Vec<String> = ids("org")
                    .into_iter()
                    .map(|i| wikilink(Owner::Org(i)))
                    .collect();
                head(
                    o,
                    &sm.name,
                    sm.last_member_ts_unix_ms,
                    sm.status_line.as_deref(),
                    &[
                        ("Members", members),
                        ("People", people),
                        ("Organizations", orgs),
                    ],
                    owned.get(&o),
                )
            }
            Owner::Person(id) | Owner::Org(id) => {
                let e = &entities[&id];
                let updated = e
                    .topics
                    .iter()
                    .map(|t| topics[t].detail.summary.last_member_ts_unix_ms)
                    .max()
                    .unwrap_or(0);
                let mut ts = e.topics.clone();
                ts.sort();
                let list: Vec<String> = ts.iter().map(|&t| wikilink(Owner::Topic(t))).collect();
                let status = if e.org { "Organization" } else { "Person" };
                head(
                    o,
                    &e.display,
                    updated,
                    Some(status),
                    &[("Topics", list)],
                    owned.get(&o),
                )
            }
        };
        let path = &paths[&o];
        let existing = owned.get(&o);
        let notes = match existing {
            Some(ex) => match ex.notes() {
                Some(n) => n.to_string(),
                None => {
                    // No `## My notes` line: we cannot separate user text, so leave it.
                    report.skipped += 1;
                    continue;
                }
            },
            None => String::new(),
        };
        let mut notes = notes;
        for block in appended.get(&o).into_iter().flatten() {
            if notes.contains(block.as_str()) {
                continue;
            }
            if !notes.is_empty() && !notes.ends_with('\n') {
                notes.push('\n');
            }
            if !notes.is_empty() {
                notes.push('\n');
            }
            notes.push_str(block);
        }
        let bom = if existing.is_some_and(|e| e.bom > 0) {
            "\u{FEFF}"
        } else {
            ""
        };
        let content = format!("{bom}{body}{NOTES_HEADER}\n{notes}");
        let folder = path.parent().ok_or(ApiError::ProviderFailure)?;
        if existing.is_none() {
            ensure_dir(folder).map_err(ioe)?;
        }
        check_inside(&wh, folder, path).map_err(ioe)?;
        if existing.is_some_and(|ex| ex.text == content) {
            report.unchanged += 1;
        } else {
            write_atomic(path, &content).map_err(ioe)?;
            report.written += 1;
        }
    }

    for ex in to_delete {
        fs::remove_file(&ex.path).map_err(ioe)?;
        report.deleted += 1;
    }
    for (ex, strip) in to_archive {
        archive_file(&wh, ex, strip, now_ms).map_err(ioe)?;
        report.archived += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::fs;

    const T0: i64 = 1_700_000_000_000;
    const DAY: i64 = 86_400_000;
    const NOW: i64 = T0 + 30 * DAY;
    const BASE: &str = "https://app.test/";

    struct Env {
        _d: tempfile::TempDir,
        store: Store,
        c: Connection,
        root: PathBuf,
    }

    fn env() -> Env {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("m.db");
        let store = Store::open(&path, d.path()).unwrap();
        let c = Connection::open(&path).unwrap();
        let root = d.path().join("vault");
        fs::create_dir_all(&root).unwrap();
        Env {
            _d: d,
            store,
            c,
            root,
        }
    }

    /// Topic with members `(kind, ref_id, title)`; returns (topic id, source ids).
    fn topic(e: &Env, name: &str, items: &[(&str, Option<i64>, &str)]) -> (i64, Vec<i64>) {
        e.c.execute(
            "INSERT INTO topic (name, name_dirty, status_line, created_at_unix_ms, updated_at_unix_ms) VALUES (?1, 0, 'Going well', 0, 0)",
            [name],
        )
        .unwrap();
        let t = e.c.last_insert_rowid();
        let mut sids = vec![];
        for (i, (kind, r, title)) in items.iter().enumerate() {
            e.c.execute(
                "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 0)",
                params![kind, r, T0 + i as i64 * DAY, title],
            )
            .unwrap();
            let sid = e.c.last_insert_rowid();
            e.c.execute(
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')",
                [t, sid],
            )
            .unwrap();
            sids.push(sid);
        }
        (t, sids)
    }

    fn two(e: &Env, name: &str) -> (i64, Vec<i64>) {
        topic(
            e,
            name,
            &[
                ("meeting", Some(12), "Q3 pricing sync"),
                ("episode", Some(5), "Draft deck"),
            ],
        )
    }

    fn mention(e: &Env, sid: i64, kind: &str, display: &str) {
        e.c.execute(
            "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES (?1, ?2, ?2, 0) ON CONFLICT(kind, key) DO NOTHING",
            params![kind, display],
        )
        .unwrap();
        e.c.execute(
            "UPDATE entity SET display = ?2 WHERE kind = ?1 AND key = ?2",
            params![kind, display],
        )
        .unwrap();
        e.c.execute(
            "INSERT INTO entity_mention (entity_id, source_id) SELECT id, ?3 FROM entity WHERE kind = ?1 AND key = ?2",
            params![kind, display, sid],
        )
        .unwrap();
    }

    fn run(e: &Env) -> ExportReport {
        export(&e.store, &e.root, BASE, NOW).unwrap()
    }

    fn wh(e: &Env, rel: &str) -> PathBuf {
        e.root.join("Whimbrel").join(rel)
    }

    fn read(e: &Env, rel: &str) -> String {
        fs::read_to_string(wh(e, rel)).unwrap()
    }

    /// Pricing topic with Sarah Lee (person) and Acme (org) on its first member.
    fn pricing(e: &Env) -> (i64, Vec<i64>) {
        let (t, s) = two(e, "Pricing");
        mention(e, s[0], "person", "Sarah Lee");
        mention(e, s[0], "org", "Acme");
        (t, s)
    }

    #[test]
    fn first_export_writes_people_orgs_topics() {
        let e = env();
        pricing(&e);
        let r = run(&e);
        assert_eq!(
            (r.written, r.unchanged, r.archived, r.deleted),
            (3, 0, 0, 0)
        );
        let t = read(&e, "Topics/Pricing.md");
        assert!(
            t.starts_with("---\nwhimbrel_id: topic-1\naliases: [\"Pricing\"]\nupdated: 2023-11-"),
            "{t}"
        );
        assert!(
            t.contains("\n---\n# Pricing\nGoing well\n## Members\n"),
            "{t}"
        );
        assert!(
            t.contains("- 2023-11-14 · Meeting · [Q3 pricing sync](https://app.test/open?kind=meeting&id=12) · with [[Sarah Lee]]\n"),
            "{t}"
        );
        assert!(
            t.contains(
                "- 2023-11-15 · Episode · [Draft deck](https://app.test/open?kind=episode&id=5)\n"
            ),
            "{t}"
        );
        assert!(
            t.contains("## People\n- [[Sarah Lee]]\n## Organizations\n- [[Acme]]\n## My notes\n"),
            "{t}"
        );
        assert!(t.ends_with("## My notes\n"));
        let p = read(&e, "People/Sarah Lee.md");
        assert!(
            p.contains("whimbrel_id: person-1\n") && p.contains("## Topics\n- [[Pricing]]\n"),
            "{p}"
        );
        assert!(read(&e, "Organizations/Acme.md").contains("whimbrel_id: org-2\n"));
    }

    #[test]
    fn second_export_is_all_unchanged() {
        let e = env();
        pricing(&e);
        run(&e);
        let r = run(&e);
        assert_eq!(
            (r.written, r.unchanged, r.archived, r.deleted),
            (0, 3, 0, 0)
        );
    }

    #[test]
    fn rename_keeps_file_name_and_updates_alias() {
        let e = env();
        let (t, _) = pricing(&e);
        run(&e);
        e.store.rename_topic(t, "Fresh Name").unwrap();
        let r = run(&e);
        assert_eq!((r.written, r.unchanged), (1, 2));
        assert!(!wh(&e, "Topics/Fresh Name.md").exists());
        let f = read(&e, "Topics/Pricing.md");
        assert!(
            f.contains("aliases: [\"Fresh Name\"]") && f.contains("\n# Fresh Name\n"),
            "{f}"
        );
        // People link by the unchanged file name.
        assert!(read(&e, "People/Sarah Lee.md").contains("[[Pricing]]"));
    }

    #[test]
    fn my_notes_survive_rewrite() {
        let e = env();
        let (t, _) = pricing(&e);
        run(&e);
        let notes = "my line\r\n\n  indented  \n- [ ] todo\n\n\n## Not special\ntail no newline";
        let mut f = read(&e, "Topics/Pricing.md");
        f.push_str(notes);
        fs::write(wh(&e, "Topics/Pricing.md"), &f).unwrap();
        e.store.rename_topic(t, "Other").unwrap();
        run(&e);
        let f = read(&e, "Topics/Pricing.md");
        assert!(f.contains("# Other"));
        assert!(f.ends_with(&format!("## My notes\n{notes}")), "{f:?}");
    }

    #[test]
    fn vanished_topic_without_notes_is_deleted() {
        let e = env();
        let (t, s) = pricing(&e);
        run(&e);
        e.store.remove_from_topic(t, s[1]).unwrap();
        let r = run(&e);
        // Topic, and the person/org that only it mentioned.
        assert_eq!((r.deleted, r.archived, r.written), (3, 0, 0));
        assert!(!wh(&e, "Topics/Pricing.md").exists() && !wh(&e, "People/Sarah Lee.md").exists());
    }

    #[test]
    fn vanished_topic_with_notes_is_archived() {
        let e = env();
        let (t, s) = pricing(&e);
        run(&e);
        let mut f = read(&e, "Topics/Pricing.md");
        f.push_str("keep me\n");
        fs::write(wh(&e, "Topics/Pricing.md"), f).unwrap();
        e.store.remove_from_topic(t, s[1]).unwrap();
        let r = run(&e);
        assert_eq!((r.archived, r.deleted), (1, 2));
        assert!(!wh(&e, "Topics/Pricing.md").exists());
        let a = read(&e, "Archive/Pricing.md");
        assert!(
            a.contains("> Archived 2023-12-14: this topic no longer exists in Whimbrel.\n"),
            "{a}"
        );
        assert!(a.ends_with("## My notes\nkeep me\n"));
        // Archived files are never touched again.
        let r = run(&e);
        assert_eq!((r.written, r.archived, r.deleted), (0, 0, 0));
    }

    #[test]
    fn merge_moves_notes_to_survivor() {
        let e = env();
        let (a, _) = two(&e, "Alpha");
        let (b, _) = topic(
            &e,
            "Beta",
            &[("meeting", Some(30), "B1"), ("meeting", Some(31), "B2")],
        );
        run(&e);
        let mut f = read(&e, "Topics/Beta.md");
        f.push_str("beta notes\n");
        fs::write(wh(&e, "Topics/Beta.md"), f).unwrap();
        let mut f = read(&e, "Topics/Alpha.md");
        f.push_str("alpha notes\n");
        fs::write(wh(&e, "Topics/Alpha.md"), f).unwrap();
        e.store.merge_topics(a, b).unwrap();
        let r = run(&e);
        assert_eq!(r.archived, 1);
        let alpha = read(&e, "Topics/Alpha.md");
        assert!(
            alpha.ends_with(
                "## My notes\nalpha notes\n\n> From merged topic \"Beta\":\nbeta notes\n"
            ),
            "{alpha}"
        );
        assert!(alpha.contains("[B1]"));
        assert!(!wh(&e, "Topics/Beta.md").exists());
        let arch = read(&e, "Archive/Beta.md");
        assert!(
            arch.contains("> Archived ") && !arch.contains("beta notes"),
            "{arch}"
        );
        // Idempotent: nothing appended twice.
        let r = run(&e);
        assert_eq!((r.written, r.archived, r.deleted), (0, 0, 0));
        assert_eq!(read(&e, "Topics/Alpha.md"), alpha);
    }

    #[test]
    fn foreign_files_are_never_touched() {
        let e = env();
        pricing(&e);
        fs::create_dir_all(wh(&e, "Topics")).unwrap();
        let files = [
            ("Topics/mine.md", "# mine\nmy own\n"),
            ("Topics/fm.md", "---\ntitle: x\n---\nbody\n"),
            ("Topics/bogus.md", "---\nwhimbrel_id: bogus\n---\nbody\n"),
            ("Topics/Pricing.md", "user's file with the same name\n"),
        ];
        for (p, c) in files {
            fs::write(wh(&e, p), c).unwrap();
        }
        run(&e);
        for (p, c) in files {
            assert_eq!(read(&e, p), c, "{p}");
        }
        // The topic took a suffixed name instead of overwriting the user's file.
        assert!(read(&e, "Topics/Pricing (2).md").contains("whimbrel_id: topic-1"));
    }

    #[test]
    fn file_renamed_by_user_is_still_found() {
        let e = env();
        let (t, _) = pricing(&e);
        run(&e);
        fs::create_dir_all(wh(&e, "Mine/Deep")).unwrap();
        fs::rename(
            wh(&e, "Topics/Pricing.md"),
            wh(&e, "Mine/Deep/Renamed By Me.md"),
        )
        .unwrap();
        e.store.rename_topic(t, "Newer").unwrap();
        run(&e);
        assert!(read(&e, "Mine/Deep/Renamed By Me.md").contains("# Newer"));
        assert!(!wh(&e, "Topics/Pricing.md").exists());
        assert!(!wh(&e, "Topics/Newer.md").exists());
        // Wikilinks follow the user's file name.
        assert!(read(&e, "People/Sarah Lee.md").contains("[[Renamed By Me]]"));
    }

    #[test]
    fn link_base_is_used_verbatim() {
        let e = env();
        pricing(&e);
        export(&e.store, &e.root, "whimbrel://host/x?", NOW).unwrap();
        let t = read(&e, "Topics/Pricing.md");
        assert!(
            t.contains("(whimbrel://host/x?open?kind=meeting&id=12)"),
            "{t}"
        );
    }

    #[test]
    fn members_without_target_get_no_link() {
        let e = env();
        topic(
            &e,
            "NoLink",
            &[("email", None, "Plain"), ("email", Some(99), "Orphan")],
        );
        run(&e);
        let t = read(&e, "Topics/NoLink.md");
        assert!(
            t.contains("· Email · Plain\n")
                && t.contains("· Email · Orphan\n")
                && !t.contains("]("),
            "{t}"
        );
    }

    #[test]
    fn hostile_titles_cannot_inject_markdown() {
        let e = env();
        let evil = "x](http://evil) [[Secrets]] \n## My notes\n---";
        let (_, s) = topic(
            &e,
            "Safe",
            &[("meeting", Some(1), evil), ("meeting", Some(2), "ok")],
        );
        mention(&e, s[0], "person", "Eve](http://evil)\n# H");
        run(&e);
        let f = fs::read_dir(wh(&e, "People"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        for text in [read(&e, "Topics/Safe.md"), fs::read_to_string(f).unwrap()] {
            // Frontmatter aliases are plain YAML text; the Markdown body is what must be inert.
            let (fm, text) = text[4..].split_once("\n---\n").unwrap();
            assert!(fm.starts_with("whimbrel_id:"));
            let text = format!("---\n---\n{text}");
            assert!(
                !text.contains("[[Secrets]]") && !text.contains("](http://evil"),
                "{text}"
            );
            assert_eq!(
                text.lines().filter(|l| *l == "## My notes").count(),
                1,
                "{text}"
            );
            assert_eq!(
                text.lines().filter(|l| l.starts_with("---")).count(),
                2,
                "{text}"
            );
            assert!(!text.lines().any(|l| l.starts_with("# H")));
        }
    }

    #[test]
    fn file_stems_are_safe() {
        for (raw, want) in [
            ("../../etc/passwd", "etcpasswd"),
            ("a/b", "ab"),
            (".hidden", "hidden"),
            ("..", "Untitled"),
            ("CON", "CON_"),
            ("[[x]]", "x"),
            ("  a \n\t b\u{202E}c ", "a bc"),
            ("?*:\"<>|^#\\", "Untitled"),
            ("", "Untitled"),
        ] {
            assert_eq!(file_stem(raw), want, "{raw:?}");
        }
        let long = "é".repeat(200);
        assert_eq!(file_stem(&long).chars().count(), 80);
    }

    #[test]
    fn hostile_names_stay_inside_whimbrel_and_collide_with_suffixes() {
        let e = env();
        for (i, n) in [
            "../../etc/passwd",
            "a/b",
            ".hidden",
            "CON",
            "[[x]]",
            "ab",
            "a/b",
        ]
        .iter()
        .enumerate()
        {
            let i = 100 + i as i64 * 2;
            topic(
                &e,
                n,
                &[("meeting", Some(i), "m"), ("meeting", Some(i + 1), "n")],
            );
        }
        run(&e);
        let mut names: Vec<String> = fs::read_dir(wh(&e, "Topics"))
            .unwrap()
            .map(|f| f.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "CON_.md",
                "ab (2).md",
                "ab (3).md",
                "ab.md",
                "etcpasswd.md",
                "hidden.md",
                "x.md"
            ]
        );
        let top: Vec<_> = fs::read_dir(&e.root).unwrap().collect();
        assert_eq!(top.len(), 1, "{top:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        use std::os::unix::fs::symlink;
        let e = env();
        let (t, _) = pricing(&e);
        run(&e);
        let outside = e._d.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("v.md");
        let body = "---\nwhimbrel_id: topic-1\n---\n# old\n## My notes\n";
        fs::write(&victim, body).unwrap();
        symlink(&outside, wh(&e, "Link")).unwrap();
        symlink(&victim, wh(&e, "Topics/Sneaky.md")).unwrap();
        e.store.rename_topic(t, "Changed").unwrap();
        run(&e);
        assert_eq!(fs::read_to_string(&victim).unwrap(), body);
        assert!(read(&e, "Topics/Pricing.md").contains("# Changed"));
        // A symlinked output folder is refused outright.
        fs::remove_dir_all(wh(&e, "Topics")).unwrap();
        symlink(&outside, wh(&e, "Topics")).unwrap();
        assert!(export(&e.store, &e.root, BASE, NOW).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    }

    #[test]
    fn user_frontmatter_survives_rewrite() {
        let e = env();
        let (t, _) = pricing(&e);
        run(&e);
        let f = read(&e, "Topics/Pricing.md");
        let f = f.replacen(
            "aliases: [\"Pricing\"]\n",
            "aliases: [\"Pricing\", \"Pricey, the plan\"]\ntags: [x]\nrating: 5\nprops:\n  - a\n  - b\n",
            1,
        );
        fs::write(wh(&e, "Topics/Pricing.md"), f).unwrap();
        e.store.rename_topic(t, "Fresh").unwrap();
        run(&e);
        let f = read(&e, "Topics/Pricing.md");
        assert!(f.contains("aliases: [\"Fresh\", \"Pricey, the plan\"]\ntags: [x]\nrating: 5\nprops:\n  - a\n  - b\nupdated: ") || f.contains("\ntags: [x]\nrating: 5\nprops:\n  - a\n  - b\n---\n"), "{f}");
        assert!(
            f.contains("aliases: [\"Fresh\", \"Pricey, the plan\"]"),
            "{f}"
        );
        assert!(!f.contains("\"Pricing\""));
        let r = run(&e);
        assert_eq!((r.written, r.unchanged), (0, 3));
    }

    #[test]
    fn hostile_text_cannot_autolink_or_embed() {
        let e = env();
        let evil = "https://evil.example www.evil.com ![[x]] %%c%% $x$ ==h== *b* _i_ http://a.b";
        let (t, _) = topic(
            &e,
            "Safe",
            &[("meeting", Some(1), evil), ("meeting", Some(2), "ok")],
        );
        for (i, st) in ["---", "- item", "= x", "+ y", "> q", evil]
            .iter()
            .enumerate()
        {
            e.c.execute(
                "UPDATE topic SET status_line = ?1 WHERE id = ?2",
                params![st, t],
            )
            .unwrap();
            fs::remove_dir_all(wh(&e, "Topics")).ok();
            run(&e);
            let text = read(&e, "Topics/Safe.md");
            let body = text.split_once("\n---\n").unwrap().1.replace(BASE, "");
            for bad in [
                "https://", "http://", "www.", "![", "%%", "$x$", "==", "*b*", "_i_",
            ] {
                assert!(!body.contains(bad), "{i}: {bad} in {body}");
            }
            let status = body.lines().nth(1).unwrap();
            assert!(!status.starts_with(['-', '+', '=', '>']), "{i}: {status}");
            assert!(!body.lines().any(|l| l.starts_with("---")));
        }
    }

    #[test]
    fn file_without_notes_heading_is_skipped_and_counted() {
        let e = env();
        pricing(&e);
        fs::create_dir_all(wh(&e, "Topics")).unwrap();
        let body = "---\nwhimbrel_id: topic-1\n---\n# Old\nuser text\n";
        fs::write(wh(&e, "Topics/Odd.md"), body).unwrap();
        let r = run(&e);
        assert_eq!(r.skipped, 1);
        assert_eq!(read(&e, "Topics/Odd.md"), body);
        assert!(!wh(&e, "Topics/Pricing.md").exists());
    }

    #[test]
    fn bom_and_single_quoted_id_are_recognized() {
        let e = env();
        pricing(&e);
        fs::create_dir_all(wh(&e, "Topics")).unwrap();
        fs::write(
            wh(&e, "Topics/Mine.md"),
            "\u{FEFF}---\nwhimbrel_id: 'topic-1'\n---\n# old\n## My notes\nmine\n",
        )
        .unwrap();
        run(&e);
        let f = read(&e, "Topics/Mine.md");
        assert!(
            f.starts_with("\u{FEFF}---\nwhimbrel_id: topic-1\n")
                && f.contains("# Pricing")
                && f.ends_with("## My notes\nmine\n"),
            "{f:?}"
        );
        assert!(!wh(&e, "Topics/Pricing.md").exists());
        assert_eq!(run(&e).written, 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_whimbrel_root_is_followed_but_inner_links_are_not() {
        use std::os::unix::fs::symlink;
        let e = env();
        pricing(&e);
        let real = e._d.path().join("cloud");
        fs::create_dir_all(&real).unwrap();
        let link = e.root.join("Whimbrel");
        symlink(&real, &link).unwrap();
        let r = run(&e);
        assert_eq!(r.written, 3);
        assert!(real.join("Topics/Pricing.md").exists());
        let other = e._d.path().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::remove_dir_all(real.join("Topics")).unwrap();
        symlink(&other, real.join("Topics")).unwrap();
        assert!(export(&e.store, &e.root, BASE, NOW).is_err());
        assert_eq!(fs::read_dir(&other).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn whimbrel_symlink_is_followed_only_inside_root_or_into_our_own_folders() {
        use std::os::unix::fs::symlink;
        let e = env();
        let link = e.root.join("Whimbrel");
        let to = |target: &Path| {
            let _ = fs::remove_file(&link);
            symlink(target, &link).unwrap();
            resolve_whimbrel(&e.root)
        };
        // Inside the root: followed whatever it holds.
        let inside = e.root.join("Synced");
        fs::create_dir_all(&inside).unwrap();
        fs::write(inside.join("notes.txt"), "x").unwrap();
        assert!(to(&inside).is_ok());
        // Outside, empty or only our folders (dot-files ignored): followed.
        let empty = e._d.path().join("cloud");
        fs::create_dir_all(&empty).unwrap();
        assert!(to(&empty).is_ok());
        fs::create_dir_all(empty.join("Topics")).unwrap();
        fs::write(empty.join(".DS_Store"), "").unwrap();
        assert!(to(&empty).is_ok());
        // Outside with anything else: refused with its own message.
        let busy = e._d.path().join("home");
        fs::create_dir_all(&busy).unwrap();
        fs::write(busy.join("notes.txt"), "x").unwrap();
        let err = to(&busy).unwrap_err();
        assert!(err.to_string().contains("Whimbrel symlink"), "{err}");
        pricing(&e);
        assert!(export(&e.store, &e.root, BASE, NOW).is_err());
        assert_eq!(fs::read_dir(&busy).unwrap().count(), 1);
    }

    #[test]
    fn archived_file_keeps_only_heading_and_notes() {
        let e = env();
        let (t, s) = topic(
            &e,
            "Pricing",
            &[
                ("meeting", Some(12), "Secret subject"),
                ("episode", Some(5), "Draft deck"),
            ],
        );
        mention(&e, s[0], "person", "Sarah Lee");
        run(&e);
        let mut f = read(&e, "Topics/Pricing.md");
        f.push_str("keep me\n");
        fs::write(wh(&e, "Topics/Pricing.md"), f).unwrap();
        e.store.remove_from_topic(t, s[1]).unwrap();
        assert_eq!(run(&e).archived, 1);
        let a = read(&e, "Archive/Pricing.md");
        for gone in ["Secret subject", "Going well", "## Members", "## People", "Sarah Lee]]"] {
            assert!(!a.contains(gone), "{gone} in {a}");
        }
        assert!(
            a.contains("\n---\n# Pricing\n> Archived 2023-12-14: ")
                && a.ends_with("\n## My notes\nkeep me\n"),
            "{a}"
        );
    }

    #[test]
    fn merged_topic_with_empty_notes_is_deleted() {
        let e = env();
        let (a, _) = two(&e, "Alpha");
        let (b, _) = topic(
            &e,
            "Beta",
            &[("meeting", Some(30), "B1"), ("meeting", Some(31), "B2")],
        );
        run(&e);
        e.store.merge_topics(a, b).unwrap();
        let r = run(&e);
        assert_eq!((r.archived, r.deleted), (0, 1));
        assert!(!wh(&e, "Topics/Beta.md").exists() && !wh(&e, "Archive/Beta.md").exists());
    }

    #[test]
    fn untrusted_text_is_capped_and_escaped_in_linear_time() {
        let big = "w.w.w.".repeat(33_334);
        let t = std::time::Instant::now();
        let out = md_text(&big);
        assert!(out.chars().count() <= 2 * MAX_TEXT_CHARS, "{}", out.len());
        assert!(md_line(&big).chars().count() <= 2 * MAX_STATUS_CHARS);
        assert!(t.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(md_text("see WWW.x.com"), "see WWW\\.x.com");
    }
}
