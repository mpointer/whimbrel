//! The Today tab (plan Task 9), useful before M3.3 exists: the day's recap (the core's
//! daily digest, cached per local day in the store), open loops, the latest meeting, and
//! today's captures per device, a glanceable form of the M3.2 gate.
//!
//! `today_overview` never calls a model, so the UI re-runs it whenever the tab is shown or
//! the window gains focus. Only `today_recap` can call one, and it holds the store for the
//! provider call (Decision 9), so the UI runs it once when there's no cached recap, and
//! on Refresh.

use chrono::{DateTime, TimeZone};
use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::commands::library::{meeting_row, tag, DeviceTag, MeetingRow};
use crate::device_labels::DeviceLabeler;
use crate::error::CommandError;
use crate::settings::{ApiKeys, PromptSurface};
use crate::state::AppState;

/// What `api::digest::daily_digest` returns for a window with nothing in it, without a
/// model call or a spend check. Pinned by a test against the core.
const EMPTY_DAY_SUMMARY: &str = "No activity captured today.";

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TodayRecap {
    /// The local calendar day, `YYYY-MM-DD`: the digest cache's key.
    pub day: String,
    pub summary: String,
    pub open_loops: Vec<String>,
    /// Served from the cache, so no model was called this time.
    pub cached: bool,
    /// Nothing to summarize yet today: no model was called and nothing was cached.
    pub empty_day: bool,
}

/// Today's Daily Brief as the recap: the routine's own summary, from this PC or a synced device
/// (docs/superpowers/plans/2026-10-17-today-recap-daily-brief-windows.md).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BriefRecap {
    pub summary: String,
    pub ran_at_unix_ms: i64,
    /// The device that ran it, when that is not this PC.
    pub from: Option<DeviceTag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRow {
    pub id: i64,
    pub subject: Option<String>,
    pub last_activity_ts_unix_ms: i64,
    pub last_message_from: String,
    /// `waiting_on_me` or `waiting_on_them`.
    pub status: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCount {
    pub device: DeviceTag,
    pub captures: i64,
    /// The newest capture today; `None` when the device captured nothing today.
    pub last_ts_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TodayOverview {
    pub day: String,
    /// Today's cached recap; `None` until one is generated.
    pub recap: Option<TodayRecap>,
    /// Today's Daily Brief, when one has run on any device. It is shown in place of the recap.
    pub brief_recap: Option<BriefRecap>,
    pub open_loops: Vec<ThreadRow>,
    pub latest_meeting: Option<MeetingRow>,
    pub devices: Vec<DeviceCount>,
}

/// The cache key and `[start, end)` in unix ms for the calendar day `now` falls in, in
/// `now`'s time zone (the Mac's `Calendar.startOfDay`). Midnight is the earliest instant
/// that reads 00:00; where a DST change skips midnight, it's 01:00.
pub fn day_bounds<Tz: TimeZone>(now: &DateTime<Tz>) -> (String, i64, i64) {
    let tz = now.timezone();
    let date = now.date_naive();
    let midnight = |d: chrono::NaiveDate| {
        let at = |h| tz.from_local_datetime(&d.and_hms_opt(h, 0, 0).unwrap()).earliest();
        at(0).or_else(|| at(1)).map(|t| t.timestamp_millis()).unwrap_or_default()
    };
    let tomorrow = date.succ_opt().unwrap_or(date);
    (date.format("%Y-%m-%d").to_string(), midnight(date), midnight(tomorrow))
}

/// Today's captures per device, counted in core (`Store::capture_activity_by_device`):
/// this PC first, listed even at zero because the gate is about both machines, then the
/// most active. Labeling stays here, since only the shell knows the synced device names.
pub fn device_counts(activity: &[memory::DeviceActivity], labeler: &DeviceLabeler) -> Vec<DeviceCount> {
    let mut counts: Vec<DeviceCount> = activity
        .iter()
        .map(|a| DeviceCount { device: tag(labeler, &a.device_id), captures: a.captures, last_ts_unix_ms: Some(a.last_ts_unix_ms) })
        .collect();
    if let Some(own) = &labeler.own_device_id {
        if !counts.iter().any(|c| &c.device.device_id == own) {
            counts.push(DeviceCount { device: tag(labeler, own), captures: 0, last_ts_unix_ms: None });
        }
    }
    counts.sort_by(|a, b| {
        b.device
            .this_device
            .cmp(&a.device.this_device)
            .then(b.captures.cmp(&a.captures))
            .then_with(|| a.device.label.cmp(&b.device.label))
    });
    counts
}

/// The Daily Brief Today shows: the newest successful run of the routine named "Daily Brief" whose
/// `ran_at` falls in this device's local day `[start, end)`, from any device. Matched by name, as the
/// Mac does, since a persisted run carries no template. A run with no summary is skipped, so an empty
/// brief never hides the digest.
pub fn daily_brief_recap(runs: &[memory::RoutineRunWithName], start: i64, end: i64, labeler: &DeviceLabeler) -> Option<BriefRecap> {
    runs.iter()
        .filter(|r| {
            r.routine_name == "Daily Brief"
                && r.status == memory::RunStatus::Success
                && r.ran_at_unix_ms >= start
                && r.ran_at_unix_ms < end
                && !r.output_summary.trim().is_empty()
        })
        .max_by_key(|r| r.ran_at_unix_ms)
        .map(|r| BriefRecap {
            summary: r.output_summary.clone(),
            ran_at_unix_ms: r.ran_at_unix_ms,
            from: r.origin_device_id.as_deref().map(|id| tag(labeler, id)),
        })
}

/// A cache row in some other shape reads as "no cache", never as an error. The MCP
/// server reads and writes the same rows.
fn recap_from_cache(day: &str, json: &str) -> Option<TodayRecap> {
    let digest: llm::DailyDigest = serde_json::from_str(json).ok()?;
    Some(TodayRecap { day: day.to_string(), summary: digest.summary, open_loops: digest.open_loops, cached: true, empty_day: false })
}

/// The core's answer for a window with nothing in it (no model was called).
fn is_empty_day(digest: &llm::DailyDigest) -> bool {
    digest.summary == EMPTY_DAY_SUMMARY && digest.open_loops.is_empty() && digest.token_usage == llm::TokenUsage::default()
}

fn thread_row(t: memory::Thread) -> ThreadRow {
    ThreadRow {
        id: t.id,
        subject: t.subject,
        last_activity_ts_unix_ms: t.last_activity_ts_unix_ms,
        last_message_from: t.last_message_from,
        status: t.status,
        summary: t.summary,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Everything on the tab except generating a recap: one store call, no model.
#[tauri::command]
pub async fn today_overview(state: State<'_, AppState>) -> Result<TodayOverview, CommandError> {
    let labeler = state.device_labeler();
    let (day, start, end) = day_bounds(&chrono::Local::now());
    let key = day.clone();
    let (cached, threads, meetings, activity, runs) = state
        .store()?
        .call(move |s| -> Result<_, String> {
            Ok((
                s.get_cached_daily_digest(&key).map_err(|e| e.to_string())?,
                s.threads_needing_attention(5).map_err(|e| e.to_string())?,
                s.recent_meetings(1).map_err(|e| e.to_string())?,
                s.capture_activity_by_device(start, end).map_err(|e| e.to_string())?,
                s.recent_routine_runs_across_all(50).map_err(|e| e.to_string())?,
            ))
        })
        .await?
        .map_err(CommandError::store)?;
    Ok(TodayOverview {
        recap: cached.as_deref().and_then(|json| recap_from_cache(&day, json)),
        brief_recap: daily_brief_recap(&runs, start, end, &labeler),
        open_loops: threads.into_iter().map(thread_row).collect(),
        latest_meeting: meetings.first().map(|m| meeting_row(m, &labeler)),
        devices: device_counts(&activity, &labeler),
        day,
    })
}

/// Today's recap: from the cache unless `refresh`, otherwise generated with the preferred
/// provider and the daily-digest instructions, then cached for the day. The core's
/// empty-day placeholder is returned but not cached, so a recap asked for before anything
/// was captured doesn't stick for the rest of the day. (The Mac caches it.)
#[tauri::command]
pub async fn today_recap(state: State<'_, AppState>, refresh: bool) -> Result<TodayRecap, CommandError> {
    let (day, start, end) = day_bounds(&chrono::Local::now());
    let (settings, instructions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::DailyDigest))
    };
    let routing = settings.routing_config(&ApiKeys::load(&*state.secrets)?, &instructions);
    let (provider, model) = (settings.preferred_provider.as_str(), settings.preferred_model().to_string());
    let ollama_model = settings.ollama_model.clone();
    let key = day.clone();
    let started = std::time::Instant::now();
    // The slow lane: generating a recap holds a store for the provider call.
    let outcome = state
        .slow_store()?
        .call(move |s| -> Result<TodayRecap, CommandError> {
            if !refresh {
                let cached = s.get_cached_daily_digest(&key).map_err(|e| CommandError::store(e.to_string()))?;
                if let Some(recap) = cached.as_deref().and_then(|json| recap_from_cache(&key, json)) {
                    return Ok(recap);
                }
            }
            let digest = api::digest::daily_digest(s, &ollama_model, start, end, &routing)?;
            if is_empty_day(&digest) {
                return Ok(TodayRecap { day: key, summary: digest.summary, open_loops: Vec::new(), cached: false, empty_day: true });
            }
            // Stored verbatim: the MCP server reads these rows as `llm::DailyDigest`.
            let json = serde_json::to_string(&digest).map_err(|e| CommandError::store(e.to_string()))?;
            s.cache_daily_digest(&key, &json, now_ms()).map_err(|e| CommandError::store(e.to_string()))?;
            Ok(TodayRecap { day: key, summary: digest.summary, open_loops: digest.open_loops, cached: false, empty_day: false })
        })
        .await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match &outcome {
        Ok(recap) => {
            let how = if recap.cached { "cached" } else if recap.empty_day { "empty day" } else { "generated" };
            tracing::info!(day = %recap.day, how, provider, model = %model, refresh, elapsed_ms, "today recap");
        }
        Err(error) => tracing::warn!(error = %error, provider, model = %model, refresh, elapsed_ms, "today recap failed"),
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn labeler() -> DeviceLabeler {
        DeviceLabeler { own_device_id: Some("PC".into()), names: HashMap::from([("MAC".to_string(), "MacBook Pro".to_string())]) }
    }

    fn activity(device: &str, captures: i64, last_ts_unix_ms: i64) -> memory::DeviceActivity {
        memory::DeviceActivity { device_id: device.into(), captures, last_ts_unix_ms }
    }

    #[test]
    fn a_day_runs_from_local_midnight_to_the_next() {
        let edt = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        let now = edt.with_ymd_and_hms(2026, 9, 11, 7, 30, 0).unwrap();
        let (day, start, end) = day_bounds(&now);
        assert_eq!(day, "2026-09-11");
        assert_eq!(start, edt.with_ymd_and_hms(2026, 9, 11, 0, 0, 0).unwrap().timestamp_millis());
        assert_eq!(end - start, 24 * 3600 * 1000);
        // 23:59 local is still the 11th, though it's already the 12th in UTC.
        let late = edt.with_ymd_and_hms(2026, 9, 11, 23, 59, 0).unwrap();
        assert_eq!(day_bounds(&late), (day, start, end));
    }

    #[test]
    fn device_counts_put_this_pc_first_even_at_zero() {
        let counts = device_counts(&[activity("MAC", 2, 9), activity("OTHER", 1, 3)], &labeler());
        let shape: Vec<(bool, i64, Option<i64>)> =
            counts.iter().map(|c| (c.device.this_device, c.captures, c.last_ts_unix_ms)).collect();
        assert_eq!(shape, [(true, 0, None), (false, 2, Some(9)), (false, 1, Some(3))]);
        assert_eq!((counts[0].device.label.as_str(), counts[1].device.label.as_str()), ("This PC", "MacBook Pro"));
    }

    /// End to end through the store: captures outside the day aren't counted, and this
    /// PC still comes first though the Mac was busier.
    #[test]
    fn device_counts_come_from_the_core_query_over_the_day() {
        let store = memory::Store::open_in_memory().unwrap();
        let record = |ts: i64, text: &str| ingest::CaptureRecord {
            ts_unix_ms: ts,
            bundle_id: "app".into(),
            window_title: "w".into(),
            url: None,
            text: text.into(),
            source: "uia".into(),
        };
        ingest::ingest_record(&store, "MAC", &record(1_000, "first mac capture"));
        ingest::ingest_record(&store, "MAC", &record(2_000, "second mac capture"));
        ingest::ingest_record(&store, "PC", &record(3_000, "a capture on this pc"));
        ingest::ingest_record(&store, "MAC", &record(9_000, "tomorrow's mac capture"));
        let counts = device_counts(&store.capture_activity_by_device(0, 5_000).unwrap(), &labeler());
        let shape: Vec<(&str, i64, Option<i64>)> =
            counts.iter().map(|c| (c.device.device_id.as_str(), c.captures, c.last_ts_unix_ms)).collect();
        assert_eq!(shape, [("PC", 1, Some(3_000)), ("MAC", 2, Some(2_000))]);
    }

    #[test]
    fn a_cached_digest_round_trips_and_a_foreign_row_reads_as_no_cache() {
        let digest = llm::DailyDigest {
            summary: "Wrote the plan.".into(),
            open_loops: vec!["Send the draft".into()],
            token_usage: llm::TokenUsage { input_tokens: Some(900), output_tokens: Some(80) },
        };
        let recap = recap_from_cache("2026-09-11", &serde_json::to_string(&digest).unwrap()).unwrap();
        assert_eq!(
            recap,
            TodayRecap {
                day: "2026-09-11".into(),
                summary: "Wrote the plan.".into(),
                open_loops: vec!["Send the draft".into()],
                cached: true,
                empty_day: false
            }
        );
        // The shape the MCP server's own tests write, too.
        assert!(recap_from_cache("d", r#"{"summary":"cached","open_loops":[],"token_usage":{"input_tokens":null,"output_tokens":null}}"#).is_some());
        assert!(recap_from_cache("d", "not json").is_none());
    }

    /// Pins the coupling to the core's placeholder: if its wording changes, empty mornings
    /// would start caching it again.
    #[test]
    fn the_cores_empty_day_digest_is_recognized() {
        let store = memory::Store::open_in_memory().unwrap();
        let digest = api::digest::daily_digest(&store, "gemma4:12b", 0, 1, &api::RoutingConfig::default()).unwrap();
        assert!(is_empty_day(&digest), "the core's empty-day digest changed: {digest:?}");
        let real = llm::DailyDigest { summary: "Wrote the plan.".into(), open_loops: vec![], token_usage: llm::TokenUsage::default() };
        assert!(!is_empty_day(&real));
    }

    // --- the Daily Brief as the recap ---

    fn run(name: &str, status: memory::RunStatus, ran_at: i64, summary: &str, origin: Option<&str>) -> memory::RoutineRunWithName {
        memory::RoutineRunWithName {
            id: ran_at,
            routine_id: 1,
            routine_name: name.into(),
            ran_at_unix_ms: ran_at,
            status,
            output_summary: summary.into(),
            output_json: "{}".into(),
            origin_device_id: origin.map(String::from),
        }
    }

    fn brief(ran_at: i64, summary: &str, origin: Option<&str>) -> memory::RoutineRunWithName {
        run("Daily Brief", memory::RunStatus::Success, ran_at, summary, origin)
    }

    #[test]
    fn the_newest_daily_brief_of_the_day_is_the_recap() {
        let runs = [brief(2_000, "older", None), brief(4_000, "newest", None), brief(3_000, "middle", None)];
        let shown = daily_brief_recap(&runs, 1_000, 10_000, &labeler()).expect("a brief");
        assert_eq!((shown.summary.as_str(), shown.ran_at_unix_ms), ("newest", 4_000));
    }

    #[test]
    fn only_a_successful_daily_brief_inside_the_local_day_counts() {
        let runs = [
            run("Weekly Review", memory::RunStatus::Success, 5_000, "other routine", None),
            run("Daily Brief", memory::RunStatus::Failed, 6_000, "it failed", None),
            brief(999, "yesterday", None),
            brief(10_000, "tomorrow (the end is exclusive)", None),
        ];
        assert_eq!(daily_brief_recap(&runs, 1_000, 10_000, &labeler()), None);
        assert_eq!(daily_brief_recap(&[brief(1_000, "first instant", None)], 1_000, 10_000, &labeler()).unwrap().summary, "first instant");
        assert_eq!(daily_brief_recap(&[], 1_000, 10_000, &labeler()), None);
    }

    #[test]
    fn a_brief_with_no_summary_does_not_hide_the_digest() {
        let runs = [brief(4_000, "   ", None), brief(3_000, "the real one", None)];
        assert_eq!(daily_brief_recap(&runs, 1_000, 10_000, &labeler()).unwrap().summary, "the real one");
    }

    #[test]
    fn a_peers_brief_names_its_device_and_this_pcs_own_does_not() {
        let peer = daily_brief_recap(&[brief(4_000, "from the mac", Some("MAC"))], 0, 10_000, &labeler()).unwrap();
        assert_eq!(peer.from.map(|d| (d.label, d.this_device)), Some(("MacBook Pro".to_string(), false)));
        assert_eq!(daily_brief_recap(&[brief(4_000, "mine", None)], 0, 10_000, &labeler()).unwrap().from, None);
    }

    /// Through the store: a run applied from sync (what the Mac's brief looks like here) is found.
    #[test]
    fn a_daily_brief_synced_from_another_device_is_found_in_the_store() {
        let store = memory::Store::open_in_memory().unwrap();
        let applied = memory::AppliedRoutineRun {
            source_run_id: 7,
            template: "daily_brief".into(),
            ran_at_unix_ms: 4_000,
            status: memory::RunStatus::Success,
            output_summary: "Three meetings today.".into(),
            output_json: "{}".into(),
        };
        store.apply_synced_routine_run("MAC", &applied).unwrap().expect("applied");
        let runs = store.recent_routine_runs_across_all(50).unwrap();
        let shown = daily_brief_recap(&runs, 0, 10_000, &labeler()).expect("the synced brief");
        assert_eq!((shown.summary.as_str(), shown.from.map(|d| d.device_id)), ("Three meetings today.", Some("MAC".to_string())));
    }
}
