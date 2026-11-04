//! Finding live questions on this PC (parity item b): the names alerts listen for, and the
//! per-recording [`Watcher`] that feeds core's rules (`llm::live_question`). Plan:
//! `docs/superpowers/plans/2026-11-03-windows-live-question-detection.md`. The Mac's
//! counterparts are `ProfileSettings.names` and `LiveQuestionWatcher`.
//!
//! The live transcriber hands over its whole state after every slice (`live::Listener`).
//! The watcher keeps the meeting's lines, judges each candidate once 30 s of audio past it is
//! transcribed (or the recording stopped), one at a time, and keeps dedupe and the ceiling.
//! Times are recording-relative centiseconds, the saved transcript's clock; Windows meetings
//! have no parts, so there is no offset to add.

use std::collections::HashSet;
use std::sync::{mpsc, Arc, Mutex};

use llm::live_question::{candidate_indices, is_duplicate, LiveLine, LiveQuestionVerdict};
use memory::live_alert::{LiveAlert, NewLiveAlert};

use crate::live_alerts::{local_notice, more_notice, QuestionAlerts};
use crate::meetings::live::{self, Accumulated};
use crate::secrets::SecretStore;
use crate::settings::{ApiKeys, Instructions, SettingsFile};
use crate::store_actor::StoreHandle;

/// At most this many alerts per meeting get a popup and a draft (the Mac's `maxAlerts`); the
/// rest are stored for the meeting page and counted.
pub const MAX_ALERTS: usize = 6;
/// Core's trailing context (`FOLLOWING_CS`, 30 s) and lead context (`CONTEXT_CS`, 2 min).
const FOLLOWING_CS: i64 = 3_000;
const CONTEXT_CS: i64 = 12_000;
/// The draft gets the last 5 minutes of the meeting as background.
const RECENT_CS: i64 = 30_000;
/// Mic line ids sit above every system line id, so ids stay stable as both channels grow.
const MIC_ID_BASE: i64 = 1 << 40;

/// The names live question alerts listen for: the full name, the first name and each
/// comma-separated other name, with whitespace runs collapsed, without repeats (ignoring
/// case). Empty when nothing is set, which keeps alerts off.
pub fn names(profile_name: &str, other_names: &str) -> Vec<String> {
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = squash(profile_name);
    let first = full.split(' ').next().unwrap_or_default().to_string();
    let others = other_names.split(',').map(squash);
    let mut seen = HashSet::new();
    [full, first]
        .into_iter()
        .chain(others)
        .filter(|n| !n.is_empty() && seen.insert(n.to_lowercase()))
        .collect()
}

/// What the watcher needs from outside: the model and the store. Injected so the rules are
/// tested without either.
pub trait Hooks {
    /// Core's `api::live_question::detect` over `window`, judging `window[index]`.
    fn detect(&mut self, window: &[LiveLine], index: usize) -> Option<LiveQuestionVerdict>;
    /// Stores the alert (origin NULL, so it syncs out); `None` when the write failed.
    fn insert(&mut self, alert: &NewLiveAlert) -> Option<i64>;
    fn now_ms(&self) -> i64;
}

/// An alert under the ceiling: it gets a popup now and a draft next.
#[derive(Debug, Clone)]
pub struct Alerted {
    pub alert: LiveAlert,
    pub verdict: LiveQuestionVerdict,
    /// The minutes before it, for the draft's background.
    pub recent: Vec<LiveLine>,
}

/// One recording's question watcher.
#[derive(Debug)]
pub struct Watcher {
    meeting_id: i64,
    meeting_title: String,
    names: Vec<String>,
    lines: Vec<LiveLine>,
    /// Segments already taken from each channel; the transcriber only ever appends.
    seen_system: usize,
    seen_mic: usize,
    /// How far each channel's audio has been transcribed, in centiseconds.
    covered_cs: Option<i64>,
    dispatched: HashSet<i64>,
    earlier: Vec<(String, i64)>,
    alert_count: usize,
    extra: usize,
    /// The latest stored alert's `at_cs` when this watcher was built: candidates at or before
    /// it were judged by an earlier watcher for this meeting.
    high_water_cs: i64,
    stopped: bool,
}

impl Watcher {
    /// `stored` is the meeting's alerts so far, so a watcher rebuilt mid-meeting keeps dedupe,
    /// the ceiling and what was already judged.
    pub fn new(meeting_id: i64, meeting_title: String, names: Vec<String>, stored: &[LiveAlert]) -> Watcher {
        let alert_count = stored.len().min(MAX_ALERTS);
        Watcher {
            meeting_id,
            meeting_title,
            names,
            lines: Vec::new(),
            seen_system: 0,
            seen_mic: 0,
            covered_cs: None,
            dispatched: HashSet::new(),
            earlier: stored.iter().map(|a| (a.restatement.clone(), a.created_at_unix_ms)).collect(),
            alert_count,
            extra: stored.len() - alert_count,
            high_water_cs: stored.iter().map(|a| a.at_cs).max().unwrap_or(-1),
            stopped: false,
        }
    }

    pub fn meeting_id(&self) -> i64 {
        self.meeting_id
    }

    /// Takes the transcriber's latest state and judges every candidate that is ready.
    pub fn ingest(&mut self, state: &Accumulated, hooks: &mut dyn Hooks) -> Vec<Alerted> {
        if self.stopped {
            return Vec::new();
        }
        self.take_lines(state);
        self.judge_ready(hooks)
    }

    /// The recording stopped (its live transcript is complete): judges every candidate still
    /// waiting for later lines. Returns those alerts and how many went over the ceiling.
    pub fn flush(&mut self, hooks: &mut dyn Hooks) -> (Vec<Alerted>, usize) {
        if self.stopped {
            return (Vec::new(), 0);
        }
        self.stopped = true;
        let alerted = self.judge_ready(hooks);
        (alerted, self.extra)
    }

    fn take_lines(&mut self, state: &Accumulated) {
        // A channel the transcriber gave up on comes back empty; its lines stay as they were.
        let fresh_system = state.system.iter().enumerate().skip(self.seen_system).map(|(i, s)| line(i as i64, "system", s));
        let fresh_mic = state.mic.iter().enumerate().skip(self.seen_mic).map(|(i, s)| line(MIC_ID_BASE + i as i64, "mic", s));
        let fresh: Vec<LiveLine> = fresh_system.chain(fresh_mic).collect();
        self.seen_system = self.seen_system.max(state.system.len());
        self.seen_mic = self.seen_mic.max(state.mic.len());
        self.lines.extend(fresh);
        self.lines.sort_by_key(|l| (l.start_cs, l.id));
        // Both channels are sliced in turn; time has passed only as far as the slower one got.
        let covered = [state.system_cursor, state.mic_cursor].into_iter().filter(|c| *c > 0).map(live::cs_at).min();
        self.covered_cs = covered.or(self.lines.last().map(|l| l.start_cs));
    }

    /// Judges, one at a time, each candidate with 30 s of transcribed audio after it (every
    /// one once stopped).
    fn judge_ready(&mut self, hooks: &mut dyn Hooks) -> Vec<Alerted> {
        let Some(covered) = self.covered_cs else { return Vec::new() };
        // Room audio: the recording ended with no system audio at all. Core also treats a minute
        // of mic-only lines as room audio, until the first system line.
        let room_audio = self.stopped && !self.lines.iter().any(|l| l.channel == "system");
        let mut out = Vec::new();
        for index in candidate_indices(&self.lines, &self.names, room_audio) {
            let candidate = self.lines[index].clone();
            if self.dispatched.contains(&candidate.id) || !(self.stopped || covered >= candidate.start_cs + FOLLOWING_CS) {
                continue;
            }
            self.dispatched.insert(candidate.id);
            let near = |from: i64| {
                self.lines
                    .iter()
                    .filter(|l| l.start_cs >= candidate.start_cs - from && l.start_cs <= candidate.start_cs + FOLLOWING_CS)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let window = near(CONTEXT_CS);
            let recent = near(RECENT_CS);
            let Some(at) = window.iter().position(|l| l.id == candidate.id) else { continue };
            if let Some(alerted) = self.judge(&candidate, &window, at, recent, hooks) {
                out.push(alerted);
            }
        }
        out
    }

    fn judge(&mut self, candidate: &LiveLine, window: &[LiveLine], at: usize, recent: Vec<LiveLine>, hooks: &mut dyn Hooks) -> Option<Alerted> {
        if candidate.start_cs <= self.high_water_cs {
            return None;
        }
        let verdict = hooks.detect(window, at)?;
        let now = hooks.now_ms();
        if is_duplicate(&verdict.restatement, &self.earlier, now) {
            return None;
        }
        self.earlier.push((verdict.restatement.clone(), now));
        let new = NewLiveAlert {
            meeting_id: self.meeting_id,
            meeting_title: self.meeting_title.clone(),
            quote: verdict.quote.clone(),
            restatement: verdict.restatement.clone(),
            asker: verdict.asker.clone(),
            at_cs: candidate.start_cs,
            created_at_unix_ms: now,
        };
        let id = hooks.insert(&new);
        if id.is_none() {
            tracing::error!("live question alert not stored; delivering anyway");
        }
        // Over the ceiling: stored for the meeting page, no popup and no draft.
        if self.alert_count >= MAX_ALERTS {
            self.extra += 1;
            return None;
        }
        self.alert_count += 1;
        let alert = LiveAlert {
            id: id.unwrap_or(-1),
            meeting_id: Some(self.meeting_id),
            origin_device_id: None,
            source_id: id.unwrap_or(-1),
            source_meeting_id: self.meeting_id,
            meeting_title: new.meeting_title,
            quote: new.quote,
            restatement: new.restatement,
            asker: new.asker,
            at_cs: new.at_cs,
            created_at_unix_ms: now,
            draft_json: None,
            email_status: None,
            notified_at_unix_ms: None,
        };
        Some(Alerted { alert, verdict, recent })
    }
}

fn line(id: i64, channel: &str, s: &transcribe::TranscriptSegment) -> LiveLine {
    LiveLine { id, channel: channel.into(), start_cs: s.start_cs, text: s.text.trim().to_string() }
}

/// The app's side of the watcher: the model through the slow store (Ask's routing, so
/// local-only mode and the spend cap apply), alerts through the main store, popups through
/// the question alert queue. [`Runtime::listener`] starts one recording's watcher.
#[derive(Clone)]
pub struct Runtime {
    pub store: StoreHandle,
    pub slow: StoreHandle,
    pub settings: Arc<Mutex<SettingsFile>>,
    pub secrets: Arc<dyn SecretStore>,
    pub alerts: Arc<QuestionAlerts>,
    pub device_id: String,
}

impl Runtime {
    /// Starts a watcher thread for this recording and returns the transcriber's listener. Each
    /// snapshot goes over a channel, so the transcriber never waits on a model call. When the
    /// transcriber drops the listener, the recording's live transcript is complete and the
    /// watcher flushes.
    pub fn listener(&self, meeting_id: i64, meeting_title: String) -> Option<live::Listener> {
        let (tx, rx) = mpsc::channel::<Accumulated>();
        let runtime = self.clone();
        std::thread::Builder::new()
            .name("whimbrel-live-question".into())
            .spawn(move || runtime.watch(meeting_id, &meeting_title, rx))
            .map_err(|e| tracing::error!(error = %e, "could not start the live question watcher"))
            .ok()?;
        Some(live::Listener(Arc::new(move |state: &Accumulated| {
            let _ = tx.send(state.clone());
        })))
    }

    fn armed(&self) -> bool {
        self.settings.lock().unwrap().live_question_armed()
    }

    /// The meeting's stored alerts, for a watcher built mid-meeting.
    fn seed(&self, meeting_id: i64, meeting_title: &str) -> Watcher {
        let names = self.settings.lock().unwrap().live_question_names();
        let stored = match self.store.call_blocking(move |s| s.list_live_alerts(meeting_id)) {
            Ok(Ok(stored)) => stored,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "could not read the meeting's question alerts");
                Vec::new()
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not read the meeting's question alerts");
                Vec::new()
            }
        };
        Watcher::new(meeting_id, meeting_title.to_string(), names, &stored)
    }

    /// Ask's Ollama model and routing, with the keys read now.
    fn routing(&self) -> Option<(String, api::RoutingConfig)> {
        let routing = self.settings.lock().unwrap().routing();
        let keys = ApiKeys::load(&*self.secrets).map_err(|e| tracing::warn!(error = %e, "live questions: API keys unreadable")).ok()?;
        Some((routing.ollama_model.clone(), routing.routing_config(&keys, &Instructions::default())))
    }

    fn watch(&self, meeting_id: i64, meeting_title: &str, rx: mpsc::Receiver<Accumulated>) {
        // Drafts run one at a time on their own thread, so a slow draft never holds up the next
        // detection, and the model never runs two at once on this PC.
        let (drafts, queued) = mpsc::channel::<Alerted>();
        let drafter = self.clone();
        let drafter = std::thread::Builder::new().name("whimbrel-live-draft".into()).spawn(move || {
            for job in queued {
                drafter.draft(job);
            }
        });
        if let Err(e) = &drafter {
            tracing::error!(error = %e, "could not start the live answer drafter; alerts go without drafts");
        }
        let mut watcher: Option<Watcher> = None;
        while let Ok(mut state) = rx.recv() {
            // Each snapshot is the whole state, so only the newest matters.
            while let Ok(newer) = rx.try_recv() {
                state = newer;
            }
            if !self.armed() {
                // Turned off (or the name cleared): judge nothing more. Turned back on, a new
                // watcher seeds from what was stored.
                watcher = None;
                continue;
            }
            let w = watcher.get_or_insert_with(|| self.seed(meeting_id, meeting_title));
            let names = w.names.clone();
            let alerted = w.ingest(&state, &mut LiveHooks::new(self, &names));
            self.deliver(alerted, &drafts);
        }
        if let Some(w) = watcher.as_mut().filter(|_| self.armed()) {
            let names = w.names.clone();
            let (alerted, extra) = w.flush(&mut LiveHooks::new(self, &names));
            self.deliver(alerted, &drafts);
            if extra > 0 {
                self.alerts.push(vec![more_notice(&self.device_id, meeting_id, Some(meeting_id), meeting_title, extra)]);
            }
        }
        tracing::info!(meeting_id, "live question watcher finished");
    }

    fn deliver(&self, alerted: Vec<Alerted>, drafts: &mpsc::Sender<Alerted>) {
        for job in alerted {
            tracing::info!(meeting_id = job.alert.source_meeting_id, at_cs = job.alert.at_cs, "a live question for the user");
            self.alerts.push(vec![local_notice(&job.alert, &self.device_id)]);
            let _ = drafts.send(job);
        }
    }

    /// Drafts the answer, stores it (which syncs it out) and updates the popup if it's still up.
    fn draft(&self, job: Alerted) {
        let Some((model, routing)) = self.routing() else { return };
        let Alerted { mut alert, verdict, recent } = job;
        let answer = match self.slow.call_blocking(move |s| api::live_question::draft(s, &verdict, &recent, &model, &routing)) {
            Ok(Ok(answer)) => answer,
            Ok(Err(e)) => return tracing::warn!(error = ?e, "live answer draft failed"),
            Err(e) => return tracing::warn!(error = %e, "live answer draft failed"),
        };
        let Ok(json) = serde_json::to_string(&answer) else { return };
        if alert.id >= 0 {
            let (id, stored) = (alert.id, json.clone());
            match self.store.call_blocking(move |s| s.set_live_alert_draft(id, &stored)) {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "live answer draft not stored"),
                Err(e) => tracing::warn!(error = %e, "live answer draft not stored"),
            }
        }
        alert.draft_json = Some(json);
        self.alerts.replace(local_notice(&alert, &self.device_id));
    }
}

/// [`Hooks`] over the real model and store, with routing read once per snapshot.
struct LiveHooks<'a> {
    runtime: &'a Runtime,
    names: Vec<String>,
    routing: Option<(String, api::RoutingConfig)>,
}

impl<'a> LiveHooks<'a> {
    fn new(runtime: &'a Runtime, names: &[String]) -> Self {
        LiveHooks { runtime, names: names.to_vec(), routing: runtime.routing() }
    }
}

impl Hooks for LiveHooks<'_> {
    fn detect(&mut self, window: &[LiveLine], index: usize) -> Option<LiveQuestionVerdict> {
        let (model, routing) = self.routing.clone()?;
        let (lines, names) = (window.to_vec(), self.names.clone());
        match self.runtime.slow.call_blocking(move |s| api::live_question::detect(s, &lines, index, &names, &model, &routing)) {
            Ok(Ok(verdict)) => verdict,
            Ok(Err(e)) => {
                tracing::warn!(error = ?e, "live question detect failed");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "live question detect failed");
                None
            }
        }
    }

    fn insert(&mut self, alert: &NewLiveAlert) -> Option<i64> {
        let alert = alert.clone();
        match self.runtime.store.call_blocking(move |s| s.insert_live_alert(&alert)) {
            Ok(Ok(id)) => Some(id),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "live question alert not stored");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "live question alert not stored");
                None
            }
        }
    }

    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe::TranscriptSegment;

    /// One second of 16 kHz mono 16-bit audio, past the 44-byte header.
    fn cursor_at(seconds: i64) -> u64 {
        44 + seconds as u64 * 32_000
    }

    fn seg(start_s: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment { start_cs: start_s * 100, end_cs: start_s * 100 + 200, text: text.into() }
    }

    fn state(system: Vec<TranscriptSegment>, mic: Vec<TranscriptSegment>, covered_s: i64) -> Accumulated {
        Accumulated { system, mic, system_cursor: cursor_at(covered_s), mic_cursor: cursor_at(covered_s), ..Accumulated::default() }
    }

    /// Says "high" to every line that contains `?`, and records what it was asked.
    #[derive(Default)]
    struct Fake {
        asked: Vec<String>,
        stored: Vec<NewLiveAlert>,
        now: i64,
        fail_insert: bool,
    }

    impl Hooks for Fake {
        fn detect(&mut self, window: &[LiveLine], index: usize) -> Option<LiveQuestionVerdict> {
            let text = window[index].text.clone();
            self.asked.push(text.clone());
            text.contains('?').then(|| LiveQuestionVerdict {
                is_for_user: true,
                confidence: "high".into(),
                quote: text.clone(),
                asker: None,
                restatement: text,
            })
        }
        fn insert(&mut self, alert: &NewLiveAlert) -> Option<i64> {
            self.stored.push(alert.clone());
            (!self.fail_insert).then_some(self.stored.len() as i64)
        }
        fn now_ms(&self) -> i64 {
            self.now
        }
    }

    fn watcher() -> Watcher {
        Watcher::new(7, "Pricing review".into(), names("Mike Pointer", ""), &[])
    }

    #[test]
    fn names_are_full_first_then_others_without_repeats() {
        assert_eq!(names("  Mike   Pointer ", "Mikey, MP ,mike,, "), ["Mike Pointer", "Mike", "Mikey", "MP"]);
        assert_eq!(names("Cher", ""), ["Cher"], "a one-word name isn't listed twice");
        assert_eq!(names("", "Mikey"), ["Mikey"], "other names alone still count");
        assert!(names(" \n", " , ").is_empty());
    }

    #[test]
    fn a_question_late_in_a_slice_waits_for_the_next_slice() {
        let mut w = watcher();
        let mut fake = Fake::default();
        // Asked at 20 s of a 30 s slice: only 10 s of audio after it so far.
        let first = state(vec![seg(20, "Mike, can you send the numbers?")], vec![], 30);
        assert!(w.ingest(&first, &mut fake).is_empty());
        assert!(fake.asked.is_empty(), "not judged before 30 s of later audio");

        // The next slice covers to 60 s, even with nobody speaking in it.
        let second = state(vec![seg(20, "Mike, can you send the numbers?")], vec![], 60);
        let alerted = w.ingest(&second, &mut fake);
        assert_eq!(alerted.len(), 1);
        assert_eq!(alerted[0].alert.at_cs, 2_000, "on the recording's clock");
        assert_eq!(alerted[0].alert.origin_device_id, None);
        assert_eq!(fake.stored[0].meeting_id, 7);

        // Seen once, judged once.
        w.ingest(&second, &mut fake);
        assert_eq!(fake.asked.len(), 1);
    }

    #[test]
    fn the_slower_channel_sets_how_far_time_has_got() {
        let mut w = watcher();
        let mut fake = Fake::default();
        let mut s = state(vec![seg(5, "Mike, thoughts?")], vec![], 60);
        s.mic_cursor = cursor_at(30); // the mic slice of this round hasn't run yet
        assert!(w.ingest(&s, &mut fake).is_empty());
        s.mic_cursor = cursor_at(60);
        assert_eq!(w.ingest(&s, &mut fake).len(), 1);
    }

    #[test]
    fn the_users_own_mic_never_triggers_while_there_is_system_audio() {
        let mut w = watcher();
        let mut fake = Fake::default();
        let s = state(vec![seg(1, "Hello everyone")], vec![seg(10, "Mike, can you hear me?")], 120);
        assert!(w.ingest(&s, &mut fake).is_empty());
        assert!(w.flush(&mut fake).0.is_empty());
        assert!(fake.asked.is_empty());
    }

    #[test]
    fn stopping_judges_what_was_still_waiting() {
        let mut w = watcher();
        let mut fake = Fake::default();
        assert!(w.ingest(&state(vec![seg(50, "Mike, could you take this one?")], vec![], 60), &mut fake).is_empty());
        let (alerted, extra) = w.flush(&mut fake);
        assert_eq!((alerted.len(), extra), (1, 0));
        // After the stop, nothing more is taken.
        assert!(w.ingest(&state(vec![seg(50, "x"), seg(55, "Mike, again?")], vec![], 120), &mut fake).is_empty());
    }

    #[test]
    fn a_rephrased_question_within_five_minutes_alerts_once() {
        let mut w = watcher();
        let mut fake = Fake::default();
        let s = state(vec![seg(10, "Mike, can you send the Q3 numbers?"), seg(40, "Mike, can you send the Q3 numbers today?")], vec![], 120);
        assert_eq!(w.ingest(&s, &mut fake).len(), 1);
        assert_eq!(fake.asked.len(), 2, "both judged; the second is a duplicate");
    }

    #[test]
    fn after_six_alerts_the_rest_are_stored_and_counted() {
        let mut w = watcher();
        let mut fake = Fake::default();
        let questions: Vec<TranscriptSegment> =
            (0..8).map(|i| seg(i * 400, &format!("Mike, can you handle item {}?", ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"][i as usize]))).collect();
        let mut alerted = 0;
        for (i, _) in questions.iter().enumerate() {
            fake.now = i as i64 * 400_000; // past the dedupe window each time
            alerted += w.ingest(&state(questions[..=i].to_vec(), vec![], i as i64 * 400 + 60), &mut fake).len();
        }
        assert_eq!(alerted, MAX_ALERTS);
        assert_eq!(fake.stored.len(), 8, "every one is stored for the meeting page");
        assert_eq!(w.flush(&mut fake).1, 2);
    }

    #[test]
    fn a_rebuilt_watcher_skips_what_was_already_judged() {
        let stored = LiveAlert {
            id: 1,
            meeting_id: Some(7),
            origin_device_id: None,
            source_id: 1,
            source_meeting_id: 7,
            meeting_title: "Pricing review".into(),
            quote: "q".into(),
            restatement: "Send the numbers".into(),
            asker: None,
            at_cs: 2_000,
            created_at_unix_ms: 0,
            draft_json: None,
            email_status: None,
            notified_at_unix_ms: None,
        };
        let mut w = Watcher::new(7, "Pricing review".into(), names("Mike", ""), &[stored]);
        let mut fake = Fake::default();
        let s = state(vec![seg(20, "Mike, can you send the numbers?"), seg(90, "Mike, will you join Friday?")], vec![], 150);
        let alerted = w.ingest(&s, &mut fake);
        assert_eq!(fake.asked, ["Mike, will you join Friday?"], "the line at the high-water mark isn't judged again");
        assert_eq!(alerted.len(), 1);
    }

    #[test]
    fn an_alert_that_did_not_store_still_pops_up() {
        let mut w = watcher();
        let mut fake = Fake { fail_insert: true, ..Fake::default() };
        let alerted = w.ingest(&state(vec![seg(1, "Mike, thoughts?")], vec![], 60), &mut fake);
        assert_eq!(alerted.len(), 1);
        assert_eq!(alerted[0].alert.id, -1);
    }

    #[test]
    fn a_mention_that_isnt_aimed_at_the_user_is_not_sent_to_the_model() {
        let mut w = watcher();
        let mut fake = Fake::default();
        let s = state(vec![seg(1, "Mikey's team owns that."), seg(5, "Can anyone take this?")], vec![], 60);
        assert!(w.ingest(&s, &mut fake).is_empty());
        assert!(fake.asked.is_empty(), "no name, no candidate");
    }
}
