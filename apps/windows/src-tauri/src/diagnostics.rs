//! Send feedback's diagnostics bundle (docs/superpowers/plans/2026-11-04-windows-diagnostics-bundle.md):
//! a zip with the recent logs and a status report, for the user to attach to the email.
//!
//! Nothing personal leaves in it. Email addresses, the Windows user name in paths and
//! meeting titles are redacted from every line. The report never reads titles, attendees,
//! notes, transcripts, summaries or keys.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::state::AppState;

/// The newest app logs that go in, besides `crash.log` and `supervisor.log`.
const LOG_DAYS: usize = 3;
/// Each log file is cut to its last this-many bytes (one day once reached 54 MB).
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
/// Bundles kept in the diagnostics folder.
pub const KEEP_BUNDLES: usize = 3;
/// Meetings the report lists.
const MEETINGS: usize = 20;

/// Every line of the bundle goes through this, with this PC's Windows user name.
pub fn redact(line: &str) -> String {
    let user = std::env::var("USERPROFILE").ok().and_then(|p| Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()));
    redact_with(line, user.as_deref())
}

/// [`redact`] with `user`, a name that may hold spaces (which a path segment alone can't show).
pub fn redact_with(line: &str, user: Option<&str>) -> String {
    redact_titles(&redact_user_paths(&redact_emails(line), user))
}

fn is_local_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._%+-".contains(c)
}

fn is_domain_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-'
}

/// `name@example.com` → `<email>`.
fn redact_emails(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut start = out.len();
            let local: String = out.chars().rev().take_while(|c| is_local_part(*c)).collect();
            let mut end = i + 1;
            while end < chars.len() && is_domain_part(chars[end]) {
                end += 1;
            }
            let domain: String = chars[i + 1..end].iter().collect();
            let domain = domain.trim_end_matches('.');
            if !local.is_empty() && domain.contains('.') && !domain.starts_with('.') {
                start -= local.len();
                out.truncate(start);
                out.push_str("<email>");
                // A full stop after the domain ended a sentence, so it stays.
                i = i + 1 + domain.chars().count();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `C:\Users\mike\…`, `C:\\Users\\mike\\…` (Debug output) and `/Users/mike/…` → `<user>`.
/// The segment runs to the next separator, quote or space; `user` first covers a name with spaces.
fn redact_user_paths(line: &str, user: Option<&str>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        let lower = rest.to_ascii_lowercase();
        let found = ["\\\\users\\\\", "\\users\\", "/users/"].iter().filter_map(|p| lower.find(p).map(|at| (at, p.len()))).min();
        let Some((at, len)) = found else {
            out.push_str(rest);
            return out;
        };
        let after = at + len;
        out.push_str(&rest[..after]);
        if let Some(user) = user.filter(|u| !u.is_empty()) {
            let end = after + user.len();
            if rest.is_char_boundary(end) && rest.get(after..end).is_some_and(|seg| seg.eq_ignore_ascii_case(user)) {
                out.push_str("<user>");
                rest = &rest[end..];
                continue;
            }
        }
        let name_len = rest[after..].find(|c: char| c == '\\' || c == '/' || c == '"' || c == '\'' || c.is_whitespace()).unwrap_or(rest.len() - after);
        if name_len > 0 {
            out.push_str("<user>");
        }
        rest = &rest[after + name_len..];
    }
}

/// `title="Weekly sync"` (quoted since 2026-11-04) or an older unquoted `title=Weekly sync`,
/// which runs to the next ` key=` → `title=<title>`.
fn redact_titles(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = find_field(rest, "title=") {
        let value_start = at + "title=".len();
        out.push_str(&rest[..value_start]);
        out.push_str("<title>");
        let value = &rest[value_start..];
        let consumed = if let Some(quoted) = value.strip_prefix('"') {
            let mut escaped = false;
            let close = quoted.char_indices().find(|&(_, c)| {
                let end = c == '"' && !escaped;
                escaped = c == '\\' && !escaped;
                end
            });
            close.map(|(i, _)| i + 2).unwrap_or(value.len())
        } else {
            next_field(value).unwrap_or(value.len())
        };
        rest = &value[consumed..];
    }
    out.push_str(rest);
    out
}

/// Where `key` starts as a field: at the line start or after a space.
fn find_field(s: &str, key: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = s[from..].find(key).map(|i| i + from) {
        if at == 0 || s[..at].ends_with(' ') {
            return Some(at);
        }
        from = at + key.len();
    }
    None
}

/// The byte offset of the space before the next ` key=` (a lowercase `[a-z_]+=` word).
fn next_field(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for (i, _) in s.match_indices(' ') {
        let word: Vec<u8> = bytes[i + 1..].iter().copied().take_while(|b| b.is_ascii_lowercase() || *b == b'_').collect();
        if !word.is_empty() && bytes.get(i + 1 + word.len()) == Some(&b'=') {
            return Some(i);
        }
    }
    None
}

/// The last `max` bytes of `path`, starting at a line: a cut mid-line drops that partial line.
pub fn tail(path: &Path, max: u64) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let cut = len > max;
    if cut {
        file.seek(SeekFrom::Start(len - max))?;
    }
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if cut {
        text = match text.find('\n') {
            Some(i) => text[i + 1..].to_string(),
            None => String::new(),
        };
    }
    Ok(text)
}

/// The logs that go in: the newest [`LOG_DAYS`] `whimbrel.*.log`, then `crash.log` and
/// `supervisor.log` when they exist.
pub fn log_files(dir: &Path) -> Vec<PathBuf> {
    let mut daily: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("whimbrel.") && n.ends_with(".log")))
                .collect()
        })
        .unwrap_or_default();
    // `whimbrel.<yyyy-mm-dd>.log`: the name sorts by date.
    daily.sort();
    let mut chosen: Vec<PathBuf> = daily.into_iter().rev().take(LOG_DAYS).collect();
    chosen.reverse();
    chosen.extend(["crash.log", "supervisor.log"].iter().map(|n| dir.join(n)).filter(|p| p.is_file()));
    chosen
}

/// Removes all but the newest `keep` bundles in `dir`.
pub fn prune(dir: &Path, keep: usize) {
    let mut bundles: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Whimbrel-diagnostics-") && n.ends_with(".zip")))
                .collect()
        })
        .unwrap_or_default();
    // `Whimbrel-diagnostics-<yyyymmdd-hhmmss>.zip`: the name sorts by time.
    bundles.sort();
    let excess = bundles.len().saturating_sub(keep);
    for old in bundles.into_iter().take(excess) {
        if let Err(e) = std::fs::remove_file(&old) {
            tracing::warn!(file = %old.display(), error = %e, "could not remove an old diagnostics bundle");
        }
    }
}

/// Writes `entries` (name, text) to a new zip at `path`, each text redacted line by line.
pub fn write_zip(path: &Path, entries: &[(String, String)]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in entries {
        zip.start_file(name.as_str(), options).map_err(|e| e.to_string())?;
        for line in text.lines() {
            writeln!(zip, "{}", redact(line)).map_err(|e| e.to_string())?;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// One meeting's line in the report: no title, attendees, notes, transcript or summary text.
pub struct MeetingFacts {
    pub id: i64,
    pub start_unix_ms: i64,
    pub minutes: Option<i64>,
    pub status: String,
    pub this_pc: bool,
    pub audio_bytes: Option<u64>,
    pub has_transcript: bool,
    pub has_summary: bool,
    pub notice: Option<String>,
}

impl MeetingFacts {
    fn line(&self) -> String {
        let start = chrono::DateTime::from_timestamp_millis(self.start_unix_ms)
            .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "?".into());
        let minutes = self.minutes.map(|m| format!("{m} min")).unwrap_or_else(|| "open".into());
        let audio = match self.audio_bytes {
            Some(b) => format!("audio {} KB", b / 1024),
            None => "no audio".into(),
        };
        let yes = |b: bool| if b { "yes" } else { "no" };
        let mut line = format!(
            "#{} {start} ({minutes}) {} {} · {audio} · transcript {} · summary {}",
            self.id,
            self.status,
            if self.this_pc { "this PC" } else { "other device" },
            yes(self.has_transcript),
            yes(self.has_summary),
        );
        if let Some(notice) = &self.notice {
            line.push_str(&format!(" · {notice}"));
        }
        line
    }
}

/// `diagnostics.txt`: `sections` are (heading, lines), then the meetings.
pub fn report_text(sections: &[(&str, Vec<String>)], meetings: &[MeetingFacts]) -> String {
    let mut out = String::new();
    for (heading, lines) in sections {
        out.push_str(&format!("== {heading}\n"));
        for line in lines {
            out.push_str(&format!("{line}\n"));
        }
        out.push('\n');
    }
    out.push_str(&format!("== Recent meetings ({})\n", meetings.len()));
    for m in meetings {
        out.push_str(&m.line());
        out.push('\n');
    }
    out
}

/// Each file under the models folder (two levels), with its size: names only, no paths.
fn model_files(dir: &Path) -> Vec<String> {
    fn visit(dir: &Path, prefix: &str, lines: &mut Vec<String>) -> Vec<(PathBuf, String)> {
        let mut subdirs = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else { return subdirs };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            match entry.metadata() {
                Ok(m) if m.is_dir() => subdirs.push((entry.path(), format!("{name}/"))),
                Ok(m) => lines.push(format!("{name}: {} MB", m.len() / (1024 * 1024))),
                Err(_) => lines.push(format!("{name}: unreadable")),
            }
        }
        subdirs
    }
    let mut lines = Vec::new();
    for (sub, prefix) in visit(dir, "", &mut lines) {
        visit(&sub, &prefix, &mut lines);
    }
    if lines.is_empty() {
        lines.push("(none)".into());
    }
    lines
}

fn time_label(ms: Option<i64>) -> String {
    ms.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "never this session".into())
}

/// The report for this PC, from the app's state.
pub fn report(state: &AppState) -> String {
    let mut app = vec![
        format!("Whimbrel {}", env!("CARGO_PKG_VERSION")),
        crate::feedback::this_windows(),
        format!("Data folder: {}", state.paths.root.display()),
        format!("Database: {}", if state.store.is_some() { "open" } else { "not open" }),
    ];
    app.extend(state.startup_errors.iter().map(|e| format!("Startup problem: {e}")));
    // Whether this PC checks for updates, and an update that has waited for Restart now.
    app.extend(crate::app::update::status_lines());

    let (routing, capture_on) = {
        let settings = state.settings.lock().unwrap();
        (settings.routing(), settings.capture_enabled())
    };
    let keys = crate::settings::ApiKeys::load(&*state.secrets);
    let mut ai = vec![
        format!("Preferred provider: {:?}", routing.preferred_provider),
        format!("Local only: {}, cheap first: {}, local model: {}", routing.local_only, routing.cheap_first, routing.use_local_model),
        format!("Ollama model: {}", routing.ollama_model),
    ];
    match keys {
        Ok(keys) => {
            for provider in [
                crate::settings::Provider::Anthropic,
                crate::settings::Provider::Openai,
                crate::settings::Provider::Google,
                crate::settings::Provider::Openrouter,
            ] {
                ai.push(format!("{provider:?} key stored: {}", if keys.has(provider) { "yes" } else { "no" }));
            }
        }
        Err(e) => ai.push(format!("Keys unreadable: {e}")),
    }

    let capture = match &state.capture {
        Some(c) => {
            let s = c.status();
            vec![format!(
                "Capture setting on: {capture_on}; running: {}; paused: {}; last error: {}",
                s.running,
                s.paused.as_deref().unwrap_or("no"),
                s.last_error.as_deref().unwrap_or("none")
            )]
        }
        None => vec![format!("Capture setting on: {capture_on}; the service did not start")],
    };

    let sync = match &state.sync {
        Some(sync) => {
            let v = sync.view();
            vec![
                format!("State: {:?}{}", v.state, v.message.map(|m| format!(" ({m})")).unwrap_or_default()),
                format!("Content last synced: {}; error: {}", time_label(v.content_last_synced_ms), v.content_error.as_deref().unwrap_or("none")),
                format!("Profile last synced: {}; error: {}", time_label(v.settings_last_synced_ms), v.settings_error.as_deref().unwrap_or("none")),
            ]
        }
        None => vec!["Sync is not set up".into()],
    };

    let mut pipeline = Vec::new();
    if let Some(p) = &state.pipeline {
        if let Some(now) = p.processing() {
            pipeline.push(format!("Working on meeting #{}: {}", now.meeting_id, now.label));
        }
        if let Some(failed) = p.failure() {
            pipeline.push(format!("Last failure, meeting #{}: {}", failed.meeting_id, failed.message));
        }
    }
    if pipeline.is_empty() {
        pipeline.push("Idle, no failure this session".into());
    }

    let own = state.identity.as_ref().map(|i| i.device_id.clone()).unwrap_or_default();
    let meetings = match &state.store {
        Some(store) => match store.call_blocking(|s| s.recent_meetings(MEETINGS).map_err(|e| e.to_string())) {
            Ok(Ok(rows)) => rows
                .into_iter()
                .map(|m| {
                    let audio = m.audio_path.as_deref().map(Path::new);
                    MeetingFacts {
                        id: m.id,
                        start_unix_ms: m.start_unix_ms,
                        minutes: m.end_unix_ms.map(|end| (end - m.start_unix_ms) / 60_000),
                        status: format!("{:?}", m.status),
                        this_pc: m.device_id == own,
                        audio_bytes: audio.and_then(|a| std::fs::metadata(a).ok()).map(|md| md.len()),
                        has_transcript: m.transcript_path.is_some(),
                        has_summary: m.summary_tldr.is_some(),
                        notice: {
                            use crate::meetings::pipeline::{summary_failure, transcription_notice, TranscriptionNotice};
                            let transcription = audio.and_then(transcription_notice).map(|n| match n {
                                TranscriptionNotice::Failed { message } => format!("transcription failed: {message}"),
                                TranscriptionNotice::NoSpeech => "no speech detected".into(),
                            });
                            let summary = audio.and_then(summary_failure).map(|m| format!("summary failed: {m}"));
                            let both: Vec<String> = transcription.into_iter().chain(summary).collect();
                            (!both.is_empty()).then(|| both.join(" · "))
                        },
                    }
                })
                .collect(),
            Ok(Err(e)) => {
                pipeline.push(format!("Meetings unreadable: {e}"));
                Vec::new()
            }
            Err(e) => {
                pipeline.push(format!("Meetings unreadable: {e}"));
                Vec::new()
            }
        },
        None => Vec::new(),
    };

    report_text(
        &[
            ("App", app),
            ("This PC", crate::perf_log::report_lines()),
            ("AI", ai),
            ("Capture", capture),
            ("Sync", sync),
            ("Meeting pipeline", pipeline),
            ("Models", model_files(&state.paths.models)),
        ],
        &meetings,
    )
}

/// Builds `Whimbrel-diagnostics-<stamp>.zip` in the diagnostics folder and returns its path.
pub fn bundle(state: &AppState) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&state.paths.diagnostics).map_err(|e| e.to_string())?;
    let mut entries = vec![("diagnostics.txt".to_string(), report(state))];
    for log in log_files(&state.paths.logs) {
        let name = log.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match tail(&log, MAX_LOG_BYTES) {
            Ok(text) => entries.push((format!("logs/{name}"), text)),
            Err(e) => entries.push((format!("logs/{name}.unreadable.txt"), e.to_string())),
        }
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = state.paths.diagnostics.join(format!("Whimbrel-diagnostics-{stamp}.zip"));
    write_zip(&path, &entries)?;
    prune(&state.paths.diagnostics, KEEP_BUNDLES);
    Ok(path)
}

/// Opens File Explorer with `file` selected.
pub fn reveal(file: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // `/select,"<path>"` as one raw argument: Explorer parses its own command line, and a
    // user name with a space needs the quotes around the path only.
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", file.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("whimbrel-diag-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn email_addresses_are_redacted() {
        assert_eq!(
            redact("WARN gmail sync failed email=todd.b@precision-wave.com error=Your Google sign-in expired."),
            "WARN gmail sync failed email=<email> error=Your Google sign-in expired."
        );
        assert_eq!(redact("mail a@b.co, then c+d@e.org."), "mail <email>, then <email>.");
        assert_eq!(redact("no address @ here and x@localhost"), "no address @ here and x@localhost");
    }

    #[test]
    fn the_windows_user_name_in_paths_is_redacted() {
        let user = Some("Mike Pointer");
        assert_eq!(redact_with(r"path=C:\Users\Mike Pointer\AppData\x.wav", user), r"path=C:\Users\<user>\AppData\x.wav");
        assert_eq!(redact_with(r"open C:\Users\micha\AppData\Local", user), r"open C:\Users\<user>\AppData\Local");
        assert_eq!(redact_with(r#"error="C:\\Users\\micha\\data""#, None), r#"error="C:\\Users\\<user>\\data""#);
        assert_eq!(redact_with("/Users/mike/Library", None), "/Users/<user>/Library");
    }

    #[test]
    fn meeting_titles_are_redacted_quoted_or_not() {
        assert_eq!(
            redact(r#"INFO meeting detected id=abc title="Weekly \"sync\" with Bob" pid=1234"#),
            "INFO meeting detected id=abc title=<title> pid=1234"
        );
        assert_eq!(redact("INFO meeting detected id=abc title=Weekly sync with Bob pid=1234"), "INFO meeting detected id=abc title=<title> pid=1234");
        assert_eq!(redact("INFO a calendar meeting is about to start id=7 title=Standup"), "INFO a calendar meeting is about to start id=7 title=<title>");
        assert_eq!(redact("subtitle=kept"), "subtitle=kept");
    }

    #[test]
    fn a_tail_starts_at_a_whole_line() {
        let dir = TempDir::new("tail");
        let log = dir.0.join("whimbrel.2026-11-04.log");
        std::fs::write(&log, "first line\nsecond line\nthird\n").unwrap();
        assert_eq!(tail(&log, 1000).unwrap(), "first line\nsecond line\nthird\n");
        assert_eq!(tail(&log, 14).unwrap(), "third\n", "the cut lands inside \"second line\", which is dropped");
    }

    #[test]
    fn the_newest_three_days_and_the_crash_logs_go_in() {
        let dir = TempDir::new("files");
        for day in ["01", "02", "03", "04"] {
            std::fs::write(dir.0.join(format!("whimbrel.2026-11-{day}.log")), "x").unwrap();
        }
        std::fs::write(dir.0.join("installer.2026-10-26.log"), "not the app's log").unwrap();
        std::fs::write(dir.0.join("crash.log"), "boom").unwrap();
        let names: Vec<String> = log_files(&dir.0).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["whimbrel.2026-11-02.log", "whimbrel.2026-11-03.log", "whimbrel.2026-11-04.log", "crash.log"]);
    }

    #[test]
    fn only_the_newest_bundles_are_kept() {
        let dir = TempDir::new("prune");
        for stamp in ["20261101-090000", "20261102-090000", "20261103-090000", "20261104-090000"] {
            std::fs::write(dir.0.join(format!("Whimbrel-diagnostics-{stamp}.zip")), "z").unwrap();
        }
        std::fs::write(dir.0.join("notes.txt"), "not a bundle").unwrap();
        prune(&dir.0, 3);
        let mut left: Vec<String> = std::fs::read_dir(&dir.0).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["Whimbrel-diagnostics-20261102-090000.zip", "Whimbrel-diagnostics-20261103-090000.zip", "Whimbrel-diagnostics-20261104-090000.zip", "notes.txt"]);
    }

    #[test]
    fn the_report_names_no_key_and_the_zip_is_redacted() {
        let dir = TempDir::new("zip");
        let meeting = MeetingFacts {
            id: 12,
            start_unix_ms: 1_793_820_060_000,
            minutes: Some(62),
            status: "Completed".into(),
            this_pc: true,
            audio_bytes: Some(4096),
            has_transcript: true,
            has_summary: false,
            notice: None,
        };
        let text = report_text(&[("AI", vec!["Anthropic key stored: yes".into()])], &[meeting]);
        assert!(text.contains("#12 ") && text.contains("(62 min) Completed this PC · audio 4 KB · transcript yes · summary no"), "{text}");
        assert!(!text.contains("sk-ant"), "a key's value never reaches the report");

        let path = dir.0.join("Whimbrel-diagnostics-20261104-120000.zip");
        write_zip(&path, &[("logs/whimbrel.log".into(), "email=todd@example.com\ntitle=\"Board\" id=1\n".into())]).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut inside = String::new();
        archive.by_name("logs/whimbrel.log").unwrap().read_to_string(&mut inside).unwrap();
        assert_eq!(inside, "email=<email>\ntitle=<title> id=1\n");
    }
}
