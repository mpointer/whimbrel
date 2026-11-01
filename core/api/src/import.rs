//! Bringing existing work in: transcript files and exports as `meeting` rows, a folder of
//! Markdown as items. See docs/superpowers/specs/2026-11-01-import-design.md.

use std::path::{Path, PathBuf};

use memory::{MeetingStatus, MeetingTrigger, NewCaptureItem, NewMeeting, NewTranscriptSegment, Store};
use sha2::{Digest, Sha256};

/// Far above any real transcript; refuses a wrong file before reading it into memory.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MARKDOWN_APP_ID: &str = "import.markdown";
const CHUNK_CHARS: usize = 4000;
const DEFAULT_SPEAKER: &str = "Speaker";
/// A Granola note can be created a little before its meeting starts.
const OVERLAP_LEAD_MS: i64 = 5 * 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub start_cs: i64,
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedMeeting {
    pub title: String,
    pub start_unix_ms: Option<i64>,
    pub lines: Vec<Line>,
    /// The source's own summary, kept as the meeting's notes.
    pub notes: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ImportOutcome {
    /// New meetings, newest first, which still need a summary.
    pub meeting_ids: Vec<i64>,
    pub notes: usize,
    pub skipped: usize,
    /// Export rows skipped because a meeting already here ran at that time.
    pub overlapping: usize,
    pub failed: Vec<String>,
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `00:01:02.500`, `00:01:02,500`, `01:02.500`, in centiseconds.
fn timestamp_cs(raw: &str) -> Option<i64> {
    let (clock, fraction) = raw.trim().split_once(['.', ',']).unwrap_or((raw.trim(), "0"));
    let mut seconds = 0i64;
    let parts: Vec<&str> = clock.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    for part in parts {
        seconds = seconds.checked_mul(60)?.checked_add(part.parse::<u32>().ok()? as i64)?;
    }
    let digits: String = fraction.chars().take(2).collect();
    let cs = format!("{digits:0<2}").parse::<i64>().ok()?;
    seconds.checked_mul(100)?.checked_add(cs)
}

/// A leading `Name:`, for a name of at most four words. A sentence with a colon stays whole.
fn split_speaker(text: &str) -> (Option<&str>, &str) {
    match text.split_once(':') {
        Some((name, rest))
            if !name.is_empty()
                && name.len() <= 40
                && !rest.is_empty()
                && name.split_whitespace().count() <= 4
                && name.chars().all(|c| c.is_alphanumeric() || " .'-_".contains(c)) =>
        {
            (Some(name.trim()), rest.trim())
        }
        _ => (None, text),
    }
}

/// `<v Name>text</v>`, otherwise `Name: text`.
fn cue_line(start_cs: i64, text: &str) -> Option<Line> {
    let mut speaker = None;
    let mut plain = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        plain.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            rest = &rest[open..];
            break;
        };
        if let Some(name) = rest[open + 1..open + close].strip_prefix("v ") {
            speaker.get_or_insert_with(|| name.trim().to_string());
        }
        rest = &rest[open + close + 1..];
    }
    plain.push_str(rest);
    let plain = plain.trim();
    let (named, body) = split_speaker(plain);
    let speaker = speaker.or(named.map(str::to_string)).unwrap_or_else(|| DEFAULT_SPEAKER.to_string());
    let body = if named.is_some() { body } else { plain };
    (!body.is_empty()).then(|| Line { start_cs, speaker, text: body.to_string() })
}

/// WebVTT and SubRip share one shape: blocks with a `start --> end` line, then text.
pub fn parse_cues(text: &str) -> Vec<Line> {
    let text = text.replace("\r\n", "\n");
    let mut lines = Vec::new();
    for block in text.split("\n\n") {
        let mut rows = block.lines();
        let Some(start_cs) = rows
            .by_ref()
            .find_map(|row| row.split_once("-->").and_then(|(start, _)| timestamp_cs(start)))
        else {
            continue;
        };
        let body = rows.collect::<Vec<_>>().join(" ");
        lines.extend(cue_line(start_cs, &body));
    }
    lines
}

/// Otter's header row: `Name  0:05`, or a bare `0:05`.
fn otter_header(row: &str) -> Option<(String, i64)> {
    let row = row.trim();
    let (name, stamp) = row.rsplit_once(char::is_whitespace).unwrap_or(("", row));
    Some((name.trim().to_string(), timestamp_cs(stamp)?))
}

/// Plain text: Otter's blocks, or `Name: text` lines, or anything else line by line.
pub fn parse_text(text: &str) -> Vec<Line> {
    let text = text.replace("\r\n", "\n");
    let mut lines = Vec::new();
    let mut header: Option<(String, i64)> = None;
    for row in text.lines() {
        let row = row.trim();
        if row.is_empty() {
            continue;
        }
        if let Some(found) = otter_header(row) {
            header = Some(found);
            continue;
        }
        if let Some((name, start_cs)) = header.take() {
            let speaker = if name.is_empty() { DEFAULT_SPEAKER.to_string() } else { name };
            lines.push(Line { start_cs, speaker, text: row.to_string() });
            continue;
        }
        let (named, body) = split_speaker(row);
        lines.push(Line {
            start_cs: lines.last().map_or(0, |l: &Line| l.start_cs),
            speaker: named.unwrap_or(DEFAULT_SPEAKER).to_string(),
            text: body.to_string(),
        });
    }
    lines
}

/// RFC 4180: quoted cells may hold commas, new lines and doubled quotes.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut cell, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut cell)),
            ('\n', false) => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            ('\r', false) => {}
            (c, _) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// A date in the forms exports use, as Unix milliseconds in UTC.
fn date_ms(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let date = raw.get(..10)?;
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1970..=2200).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil, Howard Hinnant's algorithm.
    let (y2, m2) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let doy = (153 * (m2 - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let time = raw.get(11..16).and_then(|t| timestamp_cs(t)).map_or(0, |cs| cs * 600);
    Some(days * 86_400_000 + time)
}

/// One meeting per row. Column names aren't documented, so this matches them loosely, against
/// a real export's `document_title`, `document_created`, `summary`, `notes` and `transcript`.
/// With no transcript cell, the note text is the meeting's text.
pub fn parse_granola_csv(text: &str) -> Vec<ParsedMeeting> {
    let mut rows = parse_csv(text).into_iter();
    let Some(header) = rows.next() else { return Vec::new() };
    let column = |names: &[&str]| {
        header.iter().position(|h| {
            let h = h.to_lowercase();
            names.iter().any(|n| h.contains(n))
        })
    };
    let title = column(&["title", "name"]);
    let date = column(&["date", "created", "start"]);
    let transcript = column(&["transcript"]);
    let notes = column(&["summary", "notes", "note", "content"]);
    rows.filter_map(|row| {
        let cell = |i: Option<usize>| i.and_then(|i| row.get(i)).map(|s| s.trim()).unwrap_or("");
        let body = [cell(transcript), cell(notes)].into_iter().find(|s| !s.is_empty())?;
        Some(ParsedMeeting {
            title: cell(title).to_string(),
            start_unix_ms: date_ms(cell(date)),
            lines: parse_text(body),
            notes: Some(cell(notes)).filter(|s| !s.is_empty()).map(str::to_string),
        })
    })
    .collect()
}

/// The meetings in `path`, chosen by extension.
pub fn parse_file(path: &Path) -> Result<Vec<ParsedMeeting>, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE_BYTES {
        return Err("too large".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let single = |lines| vec![ParsedMeeting { title: file_stem(path), start_unix_ms: None, lines, notes: None }];
    match extension.as_str() {
        "vtt" | "srt" => Ok(single(parse_cues(&text))),
        "txt" => Ok(single(parse_text(&text))),
        "csv" => Ok(parse_granola_csv(&text)),
        _ => Err("unsupported file type".into()),
    }
}

/// Adds one meeting; `None` when it's empty or already here.
pub fn add_meeting(store: &Store, device_id: &str, parsed: &ParsedMeeting, fallback_start_ms: i64) -> rusqlite::Result<Option<i64>> {
    let start_unix_ms = parsed.start_unix_ms.unwrap_or(fallback_start_ms);
    if parsed.lines.is_empty() || store.meeting_exists(&parsed.title, start_unix_ms)? {
        return Ok(None);
    }
    let last_cs = parsed.lines.last().map_or(0, |l| l.start_cs);
    let mut attendees: Vec<String> = Vec::new();
    for line in &parsed.lines {
        if line.speaker != DEFAULT_SPEAKER && !attendees.contains(&line.speaker) {
            attendees.push(line.speaker.clone());
        }
    }
    let id = store.insert_meeting(&NewMeeting {
        device_id: device_id.to_string(),
        start_unix_ms,
        end_unix_ms: Some(start_unix_ms + last_cs * 10),
        title: parsed.title.clone(),
        attendees,
        // Not a new trigger: other devices on older versions couldn't read it.
        trigger: MeetingTrigger::Manual,
        audio_path: None,
        status: MeetingStatus::Completed,
    })?;
    let segments: Vec<NewTranscriptSegment> = parsed
        .lines
        .iter()
        .map(|l| NewTranscriptSegment {
            meeting_id: id,
            start_cs: l.start_cs,
            speaker_label: l.speaker.clone(),
            text: l.text.clone(),
        })
        .collect();
    store.insert_transcript_segments(&segments)?;
    if parsed.notes.is_some() {
        store.set_meeting_notes(id, parsed.notes.as_deref())?;
    }
    Ok(Some(id))
}

fn modified_ms(path: &Path, fallback: i64) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(fallback, |d| d.as_millis() as i64)
}

/// Imports each file. One bad file doesn't stop the rest.
pub fn import_files(store: &Store, device_id: &str, paths: &[PathBuf], now_ms: i64) -> ImportOutcome {
    let mut outcome = ImportOutcome::default();
    let mut added: Vec<(i64, i64)> = Vec::new();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Ok(meetings) = parse_file(path) else {
            outcome.failed.push(name);
            continue;
        };
        if meetings.is_empty() {
            outcome.failed.push(name.clone());
        }
        for parsed in &meetings {
            let fallback = modified_ms(path, now_ms);
            // Only a real date can overlap: a file's modified time says nothing about the meeting.
            if parsed.start_unix_ms.is_some_and(|at| store.meeting_overlaps(at, OVERLAP_LEAD_MS).unwrap_or(false)) {
                outcome.overlapping += 1;
                continue;
            }
            match add_meeting(store, device_id, parsed, fallback) {
                Ok(Some(id)) => added.push((parsed.start_unix_ms.unwrap_or(fallback), id)),
                Ok(None) => outcome.skipped += 1,
                Err(_) => outcome.failed.push(name.clone()),
            }
        }
    }
    added.sort_unstable_by(|a, b| b.cmp(a));
    outcome.meeting_ids = added.into_iter().map(|(_, id)| id).collect();
    outcome
}

fn markdown_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        // No hidden folders (.obsidian, .git, .trash), and links can't lead out of the folder.
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        let Ok(kind) = entry.file_type() else { continue };
        if hidden || kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            markdown_files(&path, found);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")) {
            found.push(path);
        }
    }
}

fn chunks(text: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for paragraph in text.split("\n\n") {
        if !out.last().unwrap().is_empty() && out.last().unwrap().chars().count() + paragraph.chars().count() > CHUNK_CHARS {
            out.push(String::new());
        }
        let current = out.last_mut().unwrap();
        current.push_str(paragraph);
        current.push_str("\n\n");
    }
    out.retain(|c| !c.trim().is_empty());
    out
}

/// Each note in `folder` as searchable items. A note already imported unchanged is skipped.
pub fn import_markdown_folder(store: &Store, device_id: &str, folder: &Path, now_ms: i64) -> ImportOutcome {
    let mut outcome = ImportOutcome::default();
    let mut files = Vec::new();
    markdown_files(folder, &mut files);
    files.sort();
    for path in files {
        let relative = path.strip_prefix(folder).unwrap_or(&path).to_string_lossy().into_owned();
        let too_big = std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_FILE_BYTES);
        let Some(text) = (!too_big).then(|| std::fs::read(&path).ok()).flatten() else {
            outcome.failed.push(relative);
            continue;
        };
        let text = String::from_utf8_lossy(&text);
        let digest = Sha256::new().chain_update(relative.as_bytes()).chain_update(text.as_bytes()).finalize();
        let hash = i64::from_le_bytes(digest[..8].try_into().unwrap_or_default());
        let items: Vec<NewCaptureItem> = chunks(&text)
            .into_iter()
            .map(|chunk| NewCaptureItem {
                device_id: device_id.to_string(),
                ts_unix_ms: modified_ms(&path, now_ms),
                app_bundle_id: MARKDOWN_APP_ID.to_string(),
                window_title: file_stem(&path),
                url: None,
                text: chunk,
                hash,
                source: "import".to_string(),
            })
            .collect();
        if items.is_empty() || store.capture_hash_exists(MARKDOWN_APP_ID, hash).unwrap_or(false) {
            outcome.skipped += 1;
            continue;
        }
        match store.insert_capture_record(&items) {
            Ok(_) => outcome.notes += 1,
            Err(_) => outcome.failed.push(relative),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        (tempfile::tempdir().unwrap(), Store::open_in_memory().unwrap())
    }

    #[test]
    fn timestamps_cover_both_separators_and_short_forms() {
        assert_eq!(timestamp_cs("00:01:02.500"), Some(6250));
        assert_eq!(timestamp_cs("00:01:02,500"), Some(6250));
        assert_eq!(timestamp_cs("1:02"), Some(6200));
        assert_eq!(timestamp_cs("1:02:03"), Some(372300));
        assert_eq!(timestamp_cs("hello"), None);
        assert_eq!(timestamp_cs("12"), None);
    }

    #[test]
    fn vtt_reads_voices_and_ignores_the_header() {
        let lines = parse_cues("WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n<v Ana Ruiz>Hello there</v>\n\n00:00:03.000 --> 00:00:04.000\nBen: Hi\nagain\n");
        assert_eq!(lines[0], Line { start_cs: 100, speaker: "Ana Ruiz".into(), text: "Hello there".into() });
        assert_eq!(lines[1], Line { start_cs: 300, speaker: "Ben".into(), text: "Hi again".into() });
    }

    #[test]
    fn srt_with_windows_line_ends() {
        let lines = parse_cues("1\r\n00:00:01,000 --> 00:00:02,000\r\nHello\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nThere\r\n");
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[1].start_cs, lines[1].speaker.as_str()), (300, DEFAULT_SPEAKER));
    }

    #[test]
    fn otter_text_keeps_speakers_and_times() {
        let lines = parse_text("Ana Ruiz  0:05\nWelcome everyone.\n\nBen  1:02:03\nThanks.\n");
        assert_eq!(lines[0], Line { start_cs: 500, speaker: "Ana Ruiz".into(), text: "Welcome everyone.".into() });
        assert_eq!(lines[1].start_cs, 372300);
    }

    #[test]
    fn plain_text_keeps_a_sentence_with_a_colon_whole() {
        let lines = parse_text("Ana: Hello\nHere is the thing we decided last week: ship it\n");
        assert_eq!(lines[0].speaker, "Ana");
        assert_eq!(lines[1].speaker, DEFAULT_SPEAKER);
        assert!(lines[1].text.starts_with("Here is"));
    }

    #[test]
    fn csv_handles_quotes_commas_and_new_lines() {
        let rows = parse_csv("\u{feff}a,b\r\n\"x, y\",\"line 1\nsaid \"\"hi\"\"\"\n");
        assert_eq!(rows, vec![vec!["a", "b"], vec!["x, y", "line 1\nsaid \"hi\""]]);
    }

    #[test]
    fn granola_rows_use_the_transcript_then_the_notes() {
        let csv = "Title,Created At,Summary,Transcript\nPlan,2026-03-02 14:30:00,Short,\"Ana: Hi\nBen: Yes\"\nNo transcript,2026-03-03,Only notes,\nEmpty,2026-03-04,,\n";
        let m = parse_granola_csv(csv);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].title, "Plan");
        assert_eq!(m[0].lines.len(), 2);
        assert_eq!(m[0].start_unix_ms, Some(1_772_461_800_000));
        assert_eq!(m[1].lines[0].text, "Only notes");
        assert_eq!(m[0].notes.as_deref(), Some("Short"));
    }

    fn recorded(store: &Store, start: i64, end: i64) {
        store.insert_meeting(&NewMeeting {
            device_id: "d".into(), start_unix_ms: start, end_unix_ms: Some(end), title: "Recorded".into(),
            attendees: vec![], trigger: MeetingTrigger::Calendar, audio_path: Some("/a.caf".into()), status: MeetingStatus::Completed,
        }).unwrap();
    }

    fn csv_row(dir: &Path, name: &str, date: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("document_title,document_created,transcript\n{name},{date},Hello\n")).unwrap();
        path
    }

    #[test]
    fn an_export_row_during_a_recorded_meeting_is_skipped() {
        let (dir, store) = store();
        let start = date_ms("2026-03-02T14:00").unwrap();
        recorded(&store, start, start + 3_600_000);
        let paths = [
            csv_row(dir.path(), "during.csv", "2026-03-02T14:30:00.000Z"),
            csv_row(dir.path(), "just-before.csv", "2026-03-02T13:56:00.000Z"),
            csv_row(dir.path(), "earlier.csv", "2026-03-02T13:50:00.000Z"),
            csv_row(dir.path(), "next-call.csv", "2026-03-02T15:00:00.000Z"),
        ];
        let out = import_files(&store, "d", &paths, 1);
        assert_eq!((out.overlapping, out.meeting_ids.len()), (2, 2));
    }

    #[test]
    fn a_transcript_file_is_never_an_overlap() {
        let (dir, store) = store();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "Ana: Hello").unwrap();
        let at = modified_ms(&path, 0);
        recorded(&store, at - 1000, at + 1000);
        assert_eq!(import_files(&store, "d", &[path], 1).meeting_ids.len(), 1);
    }

    #[test]
    fn imported_rows_with_no_length_dont_block_each_other() {
        let (dir, store) = store();
        let a = csv_row(dir.path(), "a.csv", "2026-03-02T14:00:00.000Z");
        let b = csv_row(dir.path(), "b.csv", "2026-03-02T14:00:30.000Z");
        assert_eq!(import_files(&store, "d", &[a, b], 1).meeting_ids.len(), 2);
    }

    /// GRANOLA_CSV points at a real export. Prints counts only, never meeting text.
    #[test]
    #[ignore]
    fn a_real_granola_export_imports() {
        let (_dir, store) = store();
        let path = PathBuf::from(std::env::var("GRANOLA_CSV").unwrap());
        let parsed = parse_file(&path).unwrap();
        let lines: usize = parsed.iter().map(|m| m.lines.len()).sum();
        let named = parsed.iter().flat_map(|m| &m.lines).filter(|l| l.speaker != DEFAULT_SPEAKER).count();
        let dated = parsed.iter().filter(|m| m.start_unix_ms.is_some()).count();
        let with_notes = parsed.iter().filter(|m| m.notes.is_some()).count();
        println!("meetings {} lines {lines} named {named} dated {dated} notes {with_notes}", parsed.len());
        let first = import_files(&store, "d", std::slice::from_ref(&path), 1);
        let again = import_files(&store, "d", std::slice::from_ref(&path), 1);
        println!("added {} failed {} skipped {}; again added {} skipped {}", first.meeting_ids.len(), first.failed.len(), first.skipped, again.meeting_ids.len(), again.skipped);
        assert_eq!(first.failed.len(), 0);
        assert_eq!(again.meeting_ids.len(), 0);
    }

    #[test]
    fn bad_dates_are_none_not_a_panic() {
        assert_eq!(date_ms("soon"), None);
        assert_eq!(date_ms("2026-13-40"), None);
        assert_eq!(date_ms("é2026-03-02"), None);
        assert_eq!(date_ms("1970-01-01"), Some(0));
    }

    #[test]
    fn importing_twice_adds_one_meeting_with_its_segments() {
        let (dir, store) = store();
        let path = dir.path().join("Weekly.vtt");
        std::fs::write(&path, "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nAna: Hello\n").unwrap();
        let first = import_files(&store, "d", &[path.clone()], 1000);
        assert_eq!(first.meeting_ids.len(), 1);
        let meeting = store.get_meeting(first.meeting_ids[0]).unwrap().unwrap();
        assert_eq!((meeting.title.as_str(), meeting.trigger, meeting.status), ("Weekly", MeetingTrigger::Manual, MeetingStatus::Completed));
        assert_eq!(meeting.attendees, ["Ana"]);
        let second = import_files(&store, "d", &[path], 1000);
        assert_eq!((second.meeting_ids.len(), second.skipped), (0, 1));
    }

    #[test]
    fn a_bad_file_doesnt_stop_the_next() {
        let (dir, store) = store();
        let good = dir.path().join("a.txt");
        std::fs::write(&good, "Ana: Hello").unwrap();
        let out = import_files(&store, "d", &[dir.path().join("x.pdf"), dir.path().join("gone.txt"), good], 1);
        assert_eq!((out.meeting_ids.len(), out.failed.len()), (1, 2));
    }

    #[test]
    fn markdown_skips_hidden_folders_and_repeats() {
        let (dir, store) = store();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(vault.join("sub")).unwrap();
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        std::fs::write(vault.join("a.md"), "# One\n\nHello").unwrap();
        std::fs::write(vault.join("sub/b.MD"), "Two").unwrap();
        std::fs::write(vault.join(".obsidian/c.md"), "Hidden").unwrap();
        std::fs::write(vault.join("d.txt"), "Other").unwrap();
        assert_eq!(import_markdown_folder(&store, "d", &vault, 1).notes, 2);
        let again = import_markdown_folder(&store, "d", &vault, 1);
        assert_eq!((again.notes, again.skipped), (0, 2));
    }

    #[test]
    fn long_notes_split_at_paragraphs() {
        let text = format!("{}\n\n{}", "a".repeat(3000), "b".repeat(3000));
        let parts = chunks(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts[1].starts_with('b'));
    }
}
