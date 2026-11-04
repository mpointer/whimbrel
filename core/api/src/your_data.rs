//! "Your data": what is on this computer and "export everything" (the trust surface,
//! docs/superpowers/specs/2026-11-04-trust-surface-design.md). The counts are
//! `memory::Store::data_counts`; the export writes one folder a person can read without
//! Whimbrel: meetings as Markdown with their transcripts, the item kinds as JSON lines,
//! conversations and routines, and the Obsidian topics export.
//!
//! Every file name that came from outside (a meeting title) is reduced to a safe stem, and
//! every write is checked to resolve inside the export folder, the way `topics_export`
//! does it. Audio is not copied: the recordings folder is named in the README instead.

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use memory::Store;
use serde::{Deserialize, Serialize};

use crate::envelope::{ApiError, ApiResult};

const PAGE: usize = 500;
const MAX_STEM_CHARS: usize = 80;
/// Meetings are exported newest first up to this many; a library larger than this is
/// not a laptop's.
const MEETING_LIMIT: usize = 1_000_000;
const RUNS_PER_ROUTINE: usize = 100;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportAllReport {
    /// The folder everything was written into.
    pub folder: String,
    pub files: usize,
    pub bytes: u64,
    pub meetings: usize,
    pub transcript_lines: usize,
    pub captures: usize,
    pub emails: usize,
    pub calendar_events: usize,
    pub snapshots: usize,
    pub conversations: usize,
    pub routines: usize,
    pub topics_written: usize,
}

fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

fn ioe(_: io::Error) -> ApiError {
    ApiError::ProviderFailure
}

/// A file stem from untrusted text: letters, digits, spaces and dashes only, trimmed and
/// cut; anything else becomes a space; an empty result is "untitled".
pub fn file_stem(name: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for c in name.chars() {
        let keep = c.is_alphanumeric() || c == '-';
        if keep {
            out.push(c);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
        if out.chars().count() >= MAX_STEM_CHARS {
            break;
        }
    }
    let trimmed = out.trim().trim_matches('-').trim().to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}

/// `yyyy-mm-dd` in UTC for a folder or file name.
fn ymd(unix_ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_millis_opt(unix_ms)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown-date".to_string())
}

fn rfc3339(unix_ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_millis_opt(unix_ms)
        .single()
        .map(|t| t.to_rfc3339())
        .unwrap_or_default()
}

/// Resolves `name` inside `dir`, refusing anything that would escape it.
fn inside(dir: &Path, name: &str) -> ApiResult<PathBuf> {
    let candidate = dir.join(name);
    if name.contains("..") || name.contains('/') || name.contains('\\') || candidate.parent() != Some(dir) {
        return Err(ApiError::ProviderFailure);
    }
    Ok(candidate)
}

struct Writer {
    files: usize,
    bytes: u64,
}

impl Writer {
    fn write(&mut self, path: &Path, text: &str) -> ApiResult<()> {
        fs::write(path, text).map_err(ioe)?;
        self.files += 1;
        self.bytes += text.len() as u64;
        Ok(())
    }
}

fn escape_md(s: &str) -> String {
    s.replace('\r', "").replace("\n#", "\n\\#")
}

fn meeting_markdown(store: &Store, m: &memory::Meeting) -> ApiResult<(String, usize)> {
    let mut md = String::new();
    md.push_str("---\n");
    md.push_str(&format!("whimbrel_meeting_id: {}\n", m.id));
    md.push_str(&format!("title: \"{}\"\n", m.title.replace('"', "'")));
    md.push_str(&format!("start: {}\n", rfc3339(m.start_unix_ms)));
    if let Some(end) = m.end_unix_ms {
        md.push_str(&format!("end: {}\n", rfc3339(end)));
    }
    md.push_str(&format!("trigger: {:?}\n", m.trigger).to_lowercase());
    md.push_str(&format!("attendees: {}\n", serde_json::to_string(&m.attendees).unwrap_or_default()));
    if let Some(audio) = &m.audio_path {
        md.push_str(&format!("audio: \"{}\"\n", audio.replace('"', "'")));
    }
    md.push_str("---\n\n");
    md.push_str(&format!("# {}\n\n", escape_md(&m.title)));
    if let Some(tldr) = &m.summary_tldr {
        md.push_str("## Summary\n\n");
        md.push_str(&escape_md(tldr));
        md.push_str("\n\n");
    }
    for (heading, list) in [
        ("Decisions", &m.summary_decisions),
        ("Action items", &m.summary_action_items),
        ("Open questions", &m.summary_open_questions),
    ] {
        if !list.is_empty() {
            md.push_str(&format!("## {heading}\n\n"));
            for line in list {
                md.push_str(&format!("- {}\n", escape_md(line)));
            }
            md.push('\n');
        }
    }
    if let Some(notes) = &m.notes {
        if !notes.trim().is_empty() {
            md.push_str("## Notes\n\n");
            md.push_str(&escape_md(notes));
            md.push_str("\n\n");
        }
    }
    let mut lines = 0usize;
    let mut offset = 0usize;
    let mut transcript = String::new();
    loop {
        let (page, total) = store.get_transcript_segments_page(m.id, offset, PAGE).map_err(db)?;
        if page.is_empty() {
            break;
        }
        for seg in &page {
            let secs = seg.start_cs / 100;
            transcript.push_str(&format!("[{}:{:02}] {}: {}\n", secs / 60, secs % 60, seg.speaker_label, escape_md(&seg.text)));
            lines += 1;
        }
        offset += page.len();
        if offset >= total {
            break;
        }
    }
    if lines > 0 {
        md.push_str("## Transcript\n\n");
        md.push_str(&transcript);
    }
    Ok((md, lines))
}

fn write_items(store: &Store, w: &mut Writer, dir: &Path, kind: &str, file: &str) -> ApiResult<usize> {
    let path = inside(dir, file)?;
    let f = fs::File::create(&path).map_err(ioe)?;
    let mut out = BufWriter::new(f);
    let mut after = 0i64;
    let mut rows = 0usize;
    let mut bytes = 0u64;
    loop {
        let page = store.export_items_page(kind, after, PAGE).map_err(db)?;
        if page.is_empty() {
            break;
        }
        for row in &page {
            let line = serde_json::to_string(row).map_err(|_| ApiError::ProviderFailure)?;
            out.write_all(line.as_bytes()).map_err(ioe)?;
            out.write_all(b"\n").map_err(ioe)?;
            bytes += line.len() as u64 + 1;
            rows += 1;
            after = row.id;
        }
    }
    out.flush().map_err(ioe)?;
    w.files += 1;
    w.bytes += bytes;
    Ok(rows)
}

/// Writes `root/Whimbrel export <date>/` and returns what it wrote. `root` is the folder
/// the user picked; the dated subfolder is created inside it, and an existing one with
/// the same name is written into (files are replaced, nothing else is deleted).
pub fn export_all(store: &Store, root: &Path, now_unix_ms: i64) -> ApiResult<ExportAllReport> {
    fs::create_dir_all(root).map_err(ioe)?;
    let root = root.canonicalize().map_err(ioe)?;
    let folder = inside(&root, &format!("Whimbrel export {}", ymd(now_unix_ms)))?;
    fs::create_dir_all(&folder).map_err(ioe)?;
    let meetings_dir = inside(&folder, "meetings")?;
    fs::create_dir_all(&meetings_dir).map_err(ioe)?;
    let mut w = Writer { files: 0, bytes: 0 };
    let mut report = ExportAllReport { folder: folder.to_string_lossy().into_owned(), ..Default::default() };

    // Meetings, one Markdown file each, with the transcript.
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in store.recent_meetings(MEETING_LIMIT).map_err(db)? {
        let (md, lines) = meeting_markdown(store, &m)?;
        let base = format!("{} {}", ymd(m.start_unix_ms), file_stem(&m.title));
        let mut name = format!("{base}.md");
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{base} ({n}).md");
            n += 1;
        }
        w.write(&inside(&meetings_dir, &name)?, &md)?;
        report.meetings += 1;
        report.transcript_lines += lines;
    }

    // Items by kind, one JSON lines file each.
    report.captures = write_items(store, &mut w, &folder, "capture", "captures.jsonl")?;
    report.emails = write_items(store, &mut w, &folder, "email", "emails.jsonl")?;
    report.calendar_events = write_items(store, &mut w, &folder, "calendar_event", "calendar.jsonl")?;
    report.snapshots = write_items(store, &mut w, &folder, "snapshot", "snapshots.jsonl")?;

    // Conversations with their exchanges.
    {
        let mut text = String::new();
        for summary in store.list_conversations().map_err(db)? {
            if let Some(c) = store.get_conversation(summary.id).map_err(db)? {
                let value = serde_json::json!({
                    "id": c.id, "title": c.title, "starred": c.starred,
                    "created_at_unix_ms": c.created_at_unix_ms, "updated_at_unix_ms": c.updated_at_unix_ms,
                    "exchanges": c.exchanges,
                });
                text.push_str(&value.to_string());
                text.push('\n');
                report.conversations += 1;
            }
        }
        w.write(&inside(&folder, "conversations.jsonl")?, &text)?;
    }

    // Routines with their recent runs.
    {
        let mut routines = Vec::new();
        for r in store.list_routines().map_err(db)? {
            let runs = store.recent_routine_runs(r.id, RUNS_PER_ROUTINE).map_err(db)?;
            routines.push(serde_json::json!({ "routine": r, "recent_runs": runs }));
            report.routines += 1;
        }
        let text = serde_json::to_string_pretty(&routines).map_err(|_| ApiError::ProviderFailure)?;
        w.write(&inside(&folder, "routines.json")?, &text)?;
    }

    // Connected Topics as Obsidian Markdown, when there are any.
    if store.data_counts().map_err(db)?.topics > 0 {
        let topics_dir = inside(&folder, "topics")?;
        let topics = crate::topics_export::export(store, &topics_dir, "", now_unix_ms)?;
        report.topics_written = topics.written;
        w.files += topics.written;
    }

    let readme = format!(
        "# Whimbrel export\n\nWritten {}.\n\n\
         - `meetings/`: one Markdown file per meeting with its summary, decisions, action items, open questions, notes and transcript.\n\
         - `captures.jsonl`: screen text, one JSON object per line (time, app, window title, URL, text).\n\
         - `emails.jsonl`, `calendar.jsonl`, `snapshots.jsonl`: the same shape for email, calendar events and slide text.\n\
         - `conversations.jsonl`: your Ask conversations with their answers and citations.\n\
         - `routines.json`: your routines and their recent runs.\n\
         - `topics/`: Connected Topics as Obsidian notes, when you have any.\n\n\
         Audio recordings are not copied; they stay in the recordings folder Whimbrel names in Settings.\n",
        rfc3339(now_unix_ms)
    );
    w.write(&inside(&folder, "README.md")?, &readme)?;
    report.files = w.files;
    report.bytes = w.bytes;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{MeetingStatus, MeetingTrigger, NewCaptureItem, NewEmailItem, NewMeeting, NewTranscriptSegment};

    fn seeded() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_record(&[NewCaptureItem {
                device_id: "d".into(),
                ts_unix_ms: 1_700_000_000_000,
                app_bundle_id: "com.apple.Safari".into(),
                window_title: "Falcon notes".into(),
                url: Some("https://example.test/falcon".into()),
                text: "The Falcon budget is 48200 dollars.".into(),
                hash: 1,
                source: "ax".into(),
            }])
            .unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_700_000_000_000,
                end_unix_ms: Some(1_700_001_000_000),
                title: "Falcon / sync: \"planning\"".into(),
                attendees: vec!["Priya <priya@example.test>".into()],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[
                NewTranscriptSegment { meeting_id: id, start_cs: 0, speaker_label: "SPEAKER_00".into(), text: "Hello.".into() },
                NewTranscriptSegment { meeting_id: id, start_cs: 6_500, speaker_label: "SPEAKER_01".into(), text: "# not a heading".into() },
            ])
            .unwrap();
        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "d".into(),
                external_id: "m1".into(),
                ts_unix_ms: 1_700_000_000_000,
                from: "Tomas <tomas@example.test>".into(),
                to: "me@example.test".into(),
                subject: "Heron".into(),
                body_text: "Heron deadline moved.".into(),
                gmail_thread_id: "t1".into(),
                account_email: "me@example.test".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
        store
    }

    #[test]
    fn counts_cover_every_kind() {
        let store = seeded();
        let c = store.data_counts().unwrap();
        assert_eq!((c.captures, c.transcript_lines, c.emails, c.meetings), (1, 2, 1, 1));
        assert_eq!(c.topics, 0);
    }

    #[test]
    fn the_export_folder_holds_readable_files_and_safe_names() {
        let store = seeded();
        let dir = tempfile::tempdir().unwrap();
        let report = export_all(&store, dir.path(), 1_700_000_000_000).unwrap();
        assert_eq!((report.meetings, report.transcript_lines, report.captures, report.emails), (1, 2, 1, 1));
        let folder = Path::new(&report.folder);
        assert!(folder.starts_with(dir.path().canonicalize().unwrap()));
        let meetings: Vec<_> = fs::read_dir(folder.join("meetings")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(meetings, vec!["2023-11-14 Falcon sync planning.md"]);
        let md = fs::read_to_string(folder.join("meetings").join(&meetings[0])).unwrap();
        // A hash inside a line is not a heading and stays as spoken.
        assert!(md.contains("[1:05] SPEAKER_01: # not a heading"), "{md}");
        assert!(md.contains("## Transcript\n\n[0:00] SPEAKER_00: Hello."));
        assert!(md.contains("attendees: [\"Priya <priya@example.test>\"]"));
        let captures = fs::read_to_string(folder.join("captures.jsonl")).unwrap();
        assert!(captures.contains("\"url\":\"https://example.test/falcon\""));
        let emails = fs::read_to_string(folder.join("emails.jsonl")).unwrap();
        assert!(emails.contains("Heron deadline moved."));
        assert!(fs::read_to_string(folder.join("README.md")).unwrap().contains("Audio recordings are not copied"));
        assert!(folder.join("routines.json").exists() && folder.join("conversations.jsonl").exists());
        assert!(!folder.join("topics").exists());
        assert!(report.files >= 7 && report.bytes > 0);
    }

    #[test]
    fn stems_drop_everything_but_letters_digits_and_dashes() {
        assert_eq!(file_stem("../../etc/passwd"), "etc passwd");
        assert_eq!(file_stem("Falcon / sync: \"planning\""), "Falcon sync planning");
        assert_eq!(file_stem("   "), "untitled");
        assert!(file_stem(&"x".repeat(500)).chars().count() <= MAX_STEM_CHARS);
    }

    #[test]
    fn two_meetings_with_the_same_title_and_day_get_distinct_files() {
        let store = seeded();
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_700_000_500_000,
                end_unix_ms: None,
                title: "Falcon / sync: \"planning\"".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let report = export_all(&store, dir.path(), 1_700_000_000_000).unwrap();
        assert_eq!(report.meetings, 2);
        let mut names: Vec<_> = fs::read_dir(Path::new(&report.folder).join("meetings")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["2023-11-14 Falcon sync planning (2).md", "2023-11-14 Falcon sync planning.md"]);
    }
}
