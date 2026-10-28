//! Background job loops (plan Task 4). They mirror the Mac's
//! `AppState.scheduleJobsBatchStep` and `scheduleEpisodeSegmentStep`
//! (`AppState.swift:548–606`). Without them, Windows captures never form episodes, and
//! neither those rows nor rows pulled by sync ever get embeddings, so semantic search
//! and Ask's semantic fallback find nothing.
//!
//! - **Jobs loop:** `run_jobs_batch` (the `reindex_sweep` embedding backfill and
//!   `thread_state_sync`) plus thread summaries. Gated like the Mac's `PowerIdleGate`:
//!   on AC power and idle for at least 60 s, because embedding compute is the heavy part.
//! - **Episode loop:** episode segmentation plus episode summaries, ungated (the Mac's
//!   #52: gating episodes behind AC + idle left them barely running).
//!
//! Each loop sends one job per turn to the store actor, so capture inserts interleave,
//! and reschedules after 0.1 s when it did work, 30 s when it didn't.
//!
//! - **Profile loop:** the daily suggestion job (`api::profile_suggest::run`), which reads the
//!   day's meeting summaries and asks a model for facts about the user. It is the one loop that
//!   can send meeting content to a model on its own schedule, so [`profile_turn`] checks the
//!   privacy gate first and nothing else runs while it is closed.
//!
//! The summarize batches call local Ollama while holding the store (plan Decision 9's
//! debt). Two guards keep that from freezing the actor: they run only when a 300 ms
//! TCP probe finds Ollama listening, since a Windows connect to a closed port retries
//! for seconds; and they process one job per turn.

use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::capture::session_gate::idle_duration;
use crate::profile::Profile;
use crate::secrets::SecretStore;
use crate::settings::{ApiKeys, Instructions, SettingsFile};
use crate::store_actor::StoreHandle;

pub const BUSY_DELAY: Duration = Duration::from_millis(100);
pub const IDLE_DELAY: Duration = Duration::from_secs(30);
/// The Mac's `PowerIdleGate` idle threshold for the heavy loop.
pub const HEAVY_IDLE: Duration = Duration::from_secs(60);
const OLLAMA_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
/// A job turn that holds the store longer than this is logged with its parts: the main
/// lane twice had one job wait over a minute with no sync or Ask to explain it (M3.3
/// Tasks 2 and 3), and the job loops are the remaining suspects.
const SLOW_TURN: Duration = Duration::from_secs(2);
const OLLAMA_PROBE_TTL: Duration = Duration::from_secs(30);
/// How often the profile loop looks at its gate. While the gate is closed that is one read of the
/// in-memory settings.
const PROFILE_TICK: Duration = Duration::from_secs(60);
/// After today's batch is done, nothing is left to do until tomorrow.
const PROFILE_DONE_REST: Duration = Duration::from_secs(30 * 60);
/// After an attempt that stored nothing (no summarized meeting yet today, or a failed call; core
/// holds a failed call back for an hour itself).
const PROFILE_RETRY_REST: Duration = Duration::from_secs(10 * 60);

/// When a loop runs next: soon if it just did work, otherwise in 30 s.
pub fn next_delay(did_work: bool) -> Duration {
    if did_work {
        BUSY_DELAY
    } else {
        IDLE_DELAY
    }
}

/// The heavy (embedding) loop's gate: on AC power and idle for at least 60 s.
pub fn heavy_jobs_allowed(on_ac: bool, idle: Duration) -> bool {
    on_ac && idle >= HEAVY_IDLE
}

/// `GetSystemPowerStatus`. A desktop with no battery reports AC; an unknown status
/// counts as AC too, rather than blocking the embedding backfill forever.
pub fn on_ac_power() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe {
        if GetSystemPowerStatus(&mut status).is_err() {
            return true;
        }
    }
    status.ACLineStatus != 0
}

/// Whether local Ollama is accepting connections, cached for 30 s.
struct OllamaProbe {
    last: Option<(Instant, bool)>,
}

impl OllamaProbe {
    fn reachable(&mut self) -> bool {
        if let Some((at, up)) = self.last {
            if at.elapsed() < OLLAMA_PROBE_TTL {
                return up;
            }
        }
        let addr: SocketAddr = "127.0.0.1:11434".parse().unwrap();
        let up = TcpStream::connect_timeout(&addr, OLLAMA_PROBE_TIMEOUT).is_ok();
        self.last = Some((Instant::now(), up));
        up
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn ollama_model(settings: &Mutex<SettingsFile>) -> String {
    settings.lock().unwrap().routing().ollama_model
}

/// Starts both loops. They run for the life of the process and stop when the store
/// actor does.
pub fn start(store: StoreHandle, settings: Arc<Mutex<SettingsFile>>) {
    {
        let (store, settings) = (store.clone(), settings.clone());
        spawn("whimbrel-jobs", move || {
            let mut probe = OllamaProbe { last: None };
            let mut delay = BUSY_DELAY;
            loop {
                std::thread::sleep(delay);
                if !heavy_jobs_allowed(on_ac_power(), idle_duration()) {
                    delay = IDLE_DELAY;
                    continue;
                }
                let summarize = probe.reachable().then(|| ollama_model(&settings));
                let turn = store.call_blocking(move |s| {
                    let started = Instant::now();
                    let jobs = s.run_jobs_batch(1, now_ms()).unwrap_or(0);
                    let jobs_ms = started.elapsed().as_millis() as u64;
                    let threads = summarize.map_or(0, |model| api::jobs::run_thread_summarize_batch(s, &model, 1, "").unwrap_or(0));
                    if started.elapsed() >= SLOW_TURN {
                        tracing::info!(jobs, jobs_ms, threads, total_ms = started.elapsed().as_millis() as u64, "slow jobs turn");
                    }
                    jobs + threads
                });
                match turn {
                    Ok(n) => delay = next_delay(n > 0),
                    Err(_) => break,
                }
            }
        });
    }
    spawn("whimbrel-episodes", move || {
        let mut probe = OllamaProbe { last: None };
        let mut delay = BUSY_DELAY;
        loop {
            std::thread::sleep(delay);
            let summarize = probe.reachable().then(|| ollama_model(&settings));
            let turn = store.call_blocking(move |s| {
                let started = Instant::now();
                let segmented = s.run_episode_segment_batch(1, now_ms()).unwrap_or(0);
                let segment_ms = started.elapsed().as_millis() as u64;
                let summarized = summarize.map_or(0, |model| api::jobs::run_episode_summarize_batch(s, &model, 1, "").unwrap_or(0));
                if started.elapsed() >= SLOW_TURN {
                    tracing::info!(segmented, segment_ms, summarized, total_ms = started.elapsed().as_millis() as u64, "slow episode turn");
                }
                segmented + summarized
            });
            match turn {
                Ok(n) => delay = next_delay(n > 0),
                Err(_) => break,
            }
        }
    });
}

/// What one look at the profile loop's gate came to.
#[derive(Debug, PartialEq, Eq)]
pub enum ProfileTurn {
    /// The switch is off or the Profile is empty: nothing was read and no model was called.
    Closed,
    /// Today's batch was already made.
    Done,
    /// The job ran and stored this many suggestions (0 is fine: nothing new, or no meetings yet).
    Ran(usize),
    /// The job could not run (keys unreadable, store or model failed); nothing changed.
    Failed,
}

/// One turn of the daily suggestion job. The gate is **the Profile's own**
/// ([`Profile::suggestions_active`]: the suggest switch is on AND something is written) and it is
/// checked before anything else, so a closed gate never reads the store and never reaches a model.
/// `last_day` is the local day the job last completed; `run` makes the model call with the facts
/// the user already has and returns how many suggestions it stored, or `None` if it could not run.
pub fn profile_turn(
    profile: &Profile,
    today: &str,
    last_day: impl FnOnce() -> Option<String>,
    run: impl FnOnce(Vec<String>) -> Option<usize>,
) -> ProfileTurn {
    if !profile.suggestions_active() {
        return ProfileTurn::Closed;
    }
    if last_day().as_deref() == Some(today) {
        return ProfileTurn::Done;
    }
    match run(profile.fact_texts()) {
        Some(added) => ProfileTurn::Ran(added),
        None => ProfileTurn::Failed,
    }
}

fn profile_delay(turn: &ProfileTurn) -> Duration {
    match turn {
        ProfileTurn::Closed => PROFILE_TICK,
        ProfileTurn::Done => PROFILE_DONE_REST,
        ProfileTurn::Ran(_) | ProfileTurn::Failed => PROFILE_RETRY_REST,
    }
}

/// Starts the daily profile-suggestion loop on the slow lane, because the model call holds the
/// store. It runs for the life of the process. Routing is the user's own (Local-only, the spend
/// cap and secret redaction apply as for every other call), per the plan's decision.
pub fn start_profile_suggestions(
    store: StoreHandle,
    settings: Arc<Mutex<SettingsFile>>,
    secrets: Arc<dyn SecretStore>,
    own_device_id: Option<String>,
    approved: crate::routines::ApprovedDevices,
) {
    spawn("whimbrel-profile", move || {
        let mut delay = PROFILE_TICK;
        loop {
            std::thread::sleep(delay);
            // One short lock for both; nothing below holds it.
            let (profile, routing) = {
                let file = settings.lock().unwrap();
                (file.profile(), file.routing())
            };
            // Follows the primary device like the routines: a PC that is not the chosen one only
            // covers a day the primary did not, and never when a suggestion already arrived.
            let gate = crate::routines::gate_now(&settings, own_device_id.as_deref(), &approved);
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let turn = profile_turn(
                &profile,
                &today,
                || store.call_blocking(|s| s.profile_suggest_last_day()).ok().and_then(Result::ok).flatten(),
                |facts| {
                    let keys = match ApiKeys::load(&*secrets) {
                        Ok(keys) => keys,
                        Err(e) => {
                            tracing::warn!(error = %e, "profile suggestions: could not read the API keys");
                            return None;
                        }
                    };
                    // The profile is not part of this call's routing; core clears it anyway.
                    let config = routing.routing_config(&keys, &Instructions::default());
                    let model = routing.ollama_model.clone();
                    match store.call_blocking(move |s| api::profile_suggest::run(s, now_ms(), &model, &config, &facts, gate)) {
                        Ok(Ok(added)) => Some(added),
                        Ok(Err(e)) => {
                            tracing::warn!(error = %e, "profile suggestions: the store refused");
                            None
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "profile suggestions: the store is unavailable");
                            None
                        }
                    }
                },
            );
            if let ProfileTurn::Ran(added) = &turn {
                if *added > 0 {
                    tracing::info!(added, "profile suggestions: new suggestions are waiting in Settings");
                }
            }
            delay = profile_delay(&turn);
        }
    });
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        tracing::error!(error = %e, name, "could not start a job loop");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written() -> Profile {
        Profile { role: "CTO".into(), ..Profile::default() }
    }

    /// One turn, counting how often the store was read and the model was reached.
    fn turn(profile: &Profile, today: &str, last_day: Option<&str>, run: Option<usize>) -> (ProfileTurn, std::cell::Cell<u32>, std::cell::Cell<u32>) {
        let (reads, runs) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
        let t = profile_turn(
            profile,
            today,
            || {
                reads.set(reads.get() + 1);
                last_day.map(str::to_string)
            },
            |_| {
                runs.set(runs.get() + 1);
                run
            },
        );
        (t, reads, runs)
    }

    #[test]
    fn a_closed_gate_never_reads_the_store_or_reaches_a_model() {
        for closed in [
            Profile::default(),
            Profile { suggest_enabled: false, ..written() },
            Profile { name: "  ".into(), ..Profile::default() },
        ] {
            let (t, reads, runs) = turn(&closed, "2026-10-07", None, Some(3));
            assert_eq!(t, ProfileTurn::Closed);
            assert_eq!((reads.get(), runs.get()), (0, 0), "{closed:?}");
        }
    }

    #[test]
    fn an_open_gate_runs_the_job_once_a_day() {
        let (t, reads, runs) = turn(&written(), "2026-10-07", Some("2026-10-06"), Some(2));
        assert_eq!(t, ProfileTurn::Ran(2));
        assert_eq!((reads.get(), runs.get()), (1, 1));

        let (t, _, runs) = turn(&written(), "2026-10-07", Some("2026-10-07"), Some(2));
        assert_eq!(t, ProfileTurn::Done, "today's batch is already made");
        assert_eq!(runs.get(), 0);

        let (t, _, runs) = turn(&written(), "2026-10-07", None, Some(0));
        assert_eq!(t, ProfileTurn::Ran(0), "a first run, even with nothing to add");
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn the_job_is_told_the_facts_the_user_already_has() {
        let mut p = written();
        p.add_fact("a".into(), "Works in UTC-5", None).unwrap();
        let mut told = Vec::new();
        profile_turn(&p, "d", || None, |facts| {
            told = facts;
            Some(0)
        });
        assert_eq!(told, vec!["Works in UTC-5"]);
    }

    #[test]
    fn the_use_my_profile_switch_does_not_close_the_suggestion_gate() {
        let (t, _, runs) = turn(&Profile { enabled: false, ..written() }, "d", None, Some(1));
        assert_eq!(t, ProfileTurn::Ran(1));
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn a_job_that_could_not_run_is_reported_and_each_outcome_sets_its_own_pace() {
        let (t, _, _) = turn(&written(), "d", None, None);
        assert_eq!(t, ProfileTurn::Failed);
        assert_eq!(profile_delay(&ProfileTurn::Closed), PROFILE_TICK);
        assert_eq!(profile_delay(&ProfileTurn::Done), PROFILE_DONE_REST);
        assert_eq!(profile_delay(&ProfileTurn::Ran(0)), PROFILE_RETRY_REST);
        assert_eq!(profile_delay(&ProfileTurn::Failed), PROFILE_RETRY_REST);
        assert!(PROFILE_TICK < PROFILE_RETRY_REST && PROFILE_RETRY_REST < PROFILE_DONE_REST);
    }

    /// `cargo test --release --lib -- --ignored embedding_cost --nocapture`. Times the
    /// inline embedding every captured or synced row pays once the MiniLM model is
    /// installed (it sets how fast `sync-engine`'s time-limited pulls drain a backlog),
    /// against the same insert with no model.
    #[test]
    #[ignore = "needs the downloaded MiniLM model under %LOCALAPPDATA%\\Whimbrel\\data\\models\\minilm"]
    fn embedding_cost_per_row() {
        let paths = crate::paths::Paths::from_env().unwrap();
        let dir = std::env::temp_dir().join(format!("whimbrel-embed-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let with_model = memory::Store::open(&dir.join("bench.sqlite"), &paths.minilm).unwrap();
        assert!(with_model.embedding_model_loaded(), "download the search model first");
        let without = memory::Store::open_in_memory().unwrap();
        // About 2,000 characters: the ingest chunk size, so a full-size row.
        let text = "quarterly roadmap review with the platform team ".repeat(42);
        let time = |store: &memory::Store, n: i64| {
            let started = Instant::now();
            for i in 0..n {
                store
                    .insert_capture_item(&memory::NewCaptureItem {
                        device_id: "bench".into(),
                        ts_unix_ms: i,
                        app_bundle_id: "bench.exe".into(),
                        window_title: "Bench".into(),
                        url: None,
                        text: format!("{i} {text}"),
                        hash: i,
                        source: "uia".into(),
                    })
                    .unwrap();
            }
            started.elapsed() / n as u32
        };
        time(&with_model, 2); // warm-up: the first embed loads the model
        let n = 20;
        println!("per row with the model: {:?}; without: {:?}", time(&with_model, n), time(&without, n));
        drop(with_model);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loop_that_did_work_comes_back_quickly() {
        assert_eq!(next_delay(true), Duration::from_millis(100));
        assert_eq!(next_delay(false), Duration::from_secs(30));
    }

    #[test]
    fn heavy_jobs_need_ac_power_and_a_minute_idle() {
        assert!(heavy_jobs_allowed(true, Duration::from_secs(60)));
        assert!(!heavy_jobs_allowed(true, Duration::from_secs(59)));
        assert!(!heavy_jobs_allowed(false, Duration::from_secs(600)));
    }

    #[test]
    fn the_ollama_probe_answers_within_its_timeout_and_caches() {
        let mut probe = OllamaProbe { last: None };
        let t = Instant::now();
        let first = probe.reachable();
        assert!(t.elapsed() < Duration::from_secs(2), "a closed port must not stall the loop");
        let t = Instant::now();
        assert_eq!(probe.reachable(), first);
        assert!(t.elapsed() < Duration::from_millis(5), "the second answer comes from the cache");
    }
}
