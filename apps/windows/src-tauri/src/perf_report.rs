//! "Copy performance report" (Settings → About) and the performance lines in the diagnostics file.
//!
//! A tester's laptop had choppy call video with Whimbrel running (2026-11-13), and the log could not
//! say which part of Whimbrel was busy. This keeps the last hour of content-free performance lines in
//! memory: resource use (every minute while a meeting records, else every 5 minutes, split into the
//! app, its WebView2 windows and the local model helper), each live-transcription slice, each
//! snapshot tick and each background job turn that did work.
//!
//! Content-free by construction: every line is built here from numbers, fixed words and model file
//! names, never from a title, transcript, name, path or error text. Nothing is sent anywhere; the
//! user copies the report or attaches the diagnostics file.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// How long lines are kept.
pub const KEEP: Duration = Duration::from_secs(60 * 60);
/// The most lines kept, whatever their age (a busy hour of short job turns).
const MAX_LINES: usize = 4_000;

static LINES: Mutex<VecDeque<(i64, String)>> = Mutex::new(VecDeque::new());
static RECORDING: AtomicBool = AtomicBool::new(false);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Keeps `line` (built only from numbers and fixed words) and writes it to the log under `perf`.
pub fn note(line: String) {
    tracing::info!(target: "perf", "{line}");
    push(&mut LINES.lock().unwrap(), now_ms(), line);
}

fn push(lines: &mut VecDeque<(i64, String)>, at_ms: i64, line: String) {
    lines.push_back((at_ms, line));
    let oldest = at_ms - KEEP.as_millis() as i64;
    while lines.front().is_some_and(|(t, _)| *t < oldest) || lines.len() > MAX_LINES {
        lines.pop_front();
    }
}

/// One background job turn that did work. Turns can come every 100 ms, so they are summed per loop
/// and written once a minute: turns, items done, and the time they took.
pub fn job_turn(loop_name: &'static str, items: usize, took: Duration) {
    let line = {
        let mut totals = JOB_TOTALS.lock().unwrap();
        let entry = match totals.iter_mut().find(|t| t.name == loop_name) {
            Some(t) => t,
            None => {
                totals.push(JobTotals { name: loop_name, since: std::time::Instant::now(), turns: 0, items: 0, busy: Duration::ZERO });
                totals.last_mut().unwrap()
            }
        };
        entry.turns += 1;
        entry.items += items;
        entry.busy += took;
        (entry.since.elapsed() >= JOB_EVERY).then(|| {
            let line = format!(
                "jobs loop={} turns={} items={} busy_ms={} over_s={}",
                entry.name,
                entry.turns,
                entry.items,
                entry.busy.as_millis(),
                entry.since.elapsed().as_secs()
            );
            *entry = JobTotals { name: entry.name, since: std::time::Instant::now(), turns: 0, items: 0, busy: Duration::ZERO };
            line
        })
    };
    if let Some(line) = line {
        note(line);
    }
}

const JOB_EVERY: Duration = Duration::from_secs(60);

struct JobTotals {
    name: &'static str,
    since: std::time::Instant,
    turns: u32,
    items: usize,
    busy: Duration,
}

static JOB_TOTALS: Mutex<Vec<JobTotals>> = Mutex::new(Vec::new());

/// A meeting started or stopped recording: resource use is sampled every minute while one records.
/// Also takes or clears core's recording lease (`memory::background_gate`), which core's own
/// background entry points check; `meeting_load` renews it while the recording lasts.
pub fn set_recording(on: bool) {
    memory::background_gate::set_recording_active(on);
    if RECORDING.swap(on, Ordering::Relaxed) != on {
        note(format!("meeting recording={}", if on { "started" } else { "stopped" }));
    }
}

pub fn recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// The kept lines, oldest first, each with its local time.
pub fn lines() -> Vec<String> {
    let lines = LINES.lock().unwrap();
    lines.iter().map(|(t, l)| format!("{} {l}", local_time(*t))).collect()
}

fn local_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map_or_else(|| ms.to_string(), |t| t.format("%H:%M:%S").to_string())
}

/// The whole report: the version, this PC, then the last hour's lines.
pub fn report(version: &str) -> String {
    let mut out = format!("Whimbrel {version} performance report\n");
    for line in crate::perf_log::report_lines() {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("\nLast hour:\n");
    let kept = lines();
    if kept.is_empty() {
        out.push_str("(nothing yet: lines start when the app starts)\n");
    }
    for line in kept {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The report for Settings → About's "Copy performance report".
#[tauri::command]
pub fn perf_report_text() -> String {
    report(env!("CARGO_PKG_VERSION"))
}

/// A model file's name (never its folder, which holds the user's name).
pub fn file_name(path: &std::path::Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_older_than_an_hour_drop_off() {
        let mut lines = VecDeque::new();
        let hour = KEEP.as_millis() as i64;
        push(&mut lines, 0, "old".into());
        push(&mut lines, hour / 2, "middle".into());
        push(&mut lines, hour + 1, "new".into());
        assert_eq!(lines.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), ["middle", "new"]);
    }

    #[test]
    fn at_most_max_lines_are_kept() {
        let mut lines = VecDeque::new();
        for i in 0..(MAX_LINES as i64 + 10) {
            push(&mut lines, i, i.to_string());
        }
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines.front().unwrap().1, "10");
    }

    #[test]
    fn a_model_path_reports_only_its_file_name() {
        let p = std::path::Path::new(r"C:\Users\someone\AppData\Local\Whimbrel\models\ggml-base.bin");
        assert_eq!(file_name(p), "ggml-base.bin");
    }
}
