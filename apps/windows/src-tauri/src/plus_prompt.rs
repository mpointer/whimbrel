//! The one-time "Use Whimbrel AI?" question for a Plus account (the Mac's `PlusAiPrompt`).
//!
//! Plus promises AI with no key, but nothing selects Whimbrel AI: a fresh install prefers Ollama.
//! Choosing it silently would send request text through Whimbrel's server to Anthropic with no
//! first-use notice, so the user is asked once. Accepting sets `routing.preferredProvider` to
//! `whimbrel` the way Settings → AI's picker does (so it syncs like any change); declining leaves
//! everything as it is. Either answer is remembered on this PC (not synced). Not now, Esc or closing
//! the window records nothing: asked again next launch, not again this session. Client controls
//! and Local-only mode keep working because core gates every hosted link.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The answered flag's file in the data folder.
pub const ANSWERED_FILE: &str = "plus-ai-prompt-answered";

/// What decides whether the question shows on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    /// This session's plan read says Plus (the stored flag doesn't count: like the Mac, only a live
    /// plan read asks).
    pub plus: bool,
    /// Signed in, with the cached ID token Whimbrel AI requests carry.
    pub whimbrel_ready: bool,
    /// `routing.preferredProvider` is already `whimbrel`.
    pub prefers_whimbrel: bool,
    pub local_only: bool,
    /// Answered on this PC before.
    pub answered: bool,
    /// A meeting is starting or recording.
    pub recording: bool,
    /// Core's Whimbrel AI data note is there to show.
    pub data_note: bool,
    /// Closed with Not now (or Esc, or the window closed) earlier this session.
    pub dismissed_this_session: bool,
}

/// Where each input comes from, read by `commands::sync::plus_ai_prompt` and the answer command.
#[derive(Debug, Clone, Default)]
pub struct Sources<'a> {
    /// This session's plan read (`Some(is_plus)`), None before one.
    pub plan_plus: Option<bool>,
    /// Sync is ready (`whimbrel_ai_view().signed_in`).
    pub signed_in: bool,
    /// `whimbrel_ai::cached()` holds credentials.
    pub token_cached: bool,
    pub prefers_whimbrel: bool,
    pub local_only: bool,
    pub answered: bool,
    pub recording: bool,
    /// Core's `whimbrel_ai_passthrough` note, if core has one.
    pub data_note: Option<&'a str>,
}

pub fn inputs(s: &Sources) -> Inputs {
    Inputs {
        plus: s.plan_plus == Some(true),
        whimbrel_ready: s.signed_in && s.token_cached,
        prefers_whimbrel: s.prefers_whimbrel,
        local_only: s.local_only,
        answered: s.answered,
        recording: s.recording,
        data_note: s.data_note.is_some_and(|n| !n.trim().is_empty()),
        dismissed_this_session: dismissed_this_session(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Show,
    /// Plus and already on Whimbrel AI: nothing to ask, so record it as answered.
    MarkAnswered,
    Hide,
}

pub fn decide(i: Inputs) -> Decision {
    if !i.plus || i.answered {
        return Decision::Hide;
    }
    if i.prefers_whimbrel {
        return Decision::MarkAnswered;
    }
    if !i.whimbrel_ready || i.local_only || i.recording || !i.data_note || i.dismissed_this_session {
        return Decision::Hide;
    }
    Decision::Show
}

/// What the window does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlusAiPromptState {
    /// Ask now.
    Show,
    /// Not now (no plan read yet, signing in, a recording, Local-only…): check again in a while.
    Wait,
    /// This session's plan read says not Plus: check again rarely, and when the window regains focus.
    NotPlus,
    /// Answered on this PC, or dismissed for this session: stop checking.
    Done,
}

/// The question's state and, while it shows, core's data note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlusAiPromptView {
    pub state: PlusAiPromptState,
    /// Core's `whimbrel_ai_passthrough` text, while `state` is `show`.
    pub note: Option<String>,
}

/// The three ways out of the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlusAiPromptAnswer {
    /// "Use Whimbrel AI": prefer Whimbrel AI, and remember the answer.
    UseWhimbrel,
    /// "Keep my current setup": remember the answer only.
    Keep,
    /// "Not now", Esc, or the window closed: nothing is stored; not asked again this session.
    NotNow,
}

static DISMISSED: AtomicBool = AtomicBool::new(false);
/// The question is on screen: the window said it displayed it ([`displayed`]), and nothing answered it yet.
static SHOWING: AtomicBool = AtomicBool::new(false);
/// A failed answered-flag write is logged once, not on every check.
static WRITE_FAILURE_LOGGED: AtomicBool = AtomicBool::new(false);

pub fn dismissed_this_session() -> bool {
    DISMISSED.load(Ordering::SeqCst)
}

fn dismiss_for_session() {
    DISMISSED.store(true, Ordering::SeqCst);
    SHOWING.store(false, Ordering::SeqCst);
}

/// The window displayed the question (a check said Show and the dialog is now on screen).
pub fn displayed() {
    if !dismissed_this_session() {
        SHOWING.store(true, Ordering::SeqCst);
    }
}

/// The main window closed (or its page went away) while the question was up: that is Not now.
pub fn window_closed() {
    if SHOWING.swap(false, Ordering::SeqCst) {
        DISMISSED.store(true, Ordering::SeqCst);
    }
}

fn log_write_failure(e: &std::io::Error) {
    if !WRITE_FAILURE_LOGGED.swap(true, Ordering::SeqCst) {
        tracing::warn!(error = %e, "could not record the Whimbrel AI question as answered; not asking again this session");
    }
}

/// The check: whether to ask now. A Plus account already on Whimbrel AI is recorded as answered.
pub fn check(sources: &Sources, flag: &AnsweredFlag) -> PlusAiPromptView {
    let i = inputs(sources);
    let state = match decide(i) {
        Decision::Show => PlusAiPromptState::Show,
        Decision::MarkAnswered => {
            if let Err(e) = flag.set() {
                log_write_failure(&e);
                dismiss_for_session();
            }
            PlusAiPromptState::Done
        }
        Decision::Hide if i.answered || i.dismissed_this_session => PlusAiPromptState::Done,
        // Before this session's plan read nothing is known: wait for it (sync may still be signing in).
        Decision::Hide if sources.plan_plus.is_none() => PlusAiPromptState::Wait,
        Decision::Hide if !i.plus => PlusAiPromptState::NotPlus,
        Decision::Hide => PlusAiPromptState::Wait,
    };
    let note = (state == PlusAiPromptState::Show).then(|| sources.data_note.unwrap_or_default().to_string());
    PlusAiPromptView { state, note }
}

/// The answer. Use Whimbrel AI and Keep check the conditions again first and write nothing if the
/// question no longer applies (a recording started, the plan changed): `Ok(false)`, and like a
/// dialog closed for that reason (the window sends Not now) it isn't asked again this session. A
/// flag that can't be written still closes the question for this session, and is logged once.
pub fn answer<E>(
    answer: PlusAiPromptAnswer,
    sources: &Sources,
    flag: &AnsweredFlag,
    prefer_whimbrel: impl FnOnce() -> Result<(), E>,
) -> Result<bool, E> {
    if answer == PlusAiPromptAnswer::NotNow {
        dismiss_for_session();
        return Ok(true);
    }
    if decide(Inputs { dismissed_this_session: false, ..inputs(sources) }) != Decision::Show {
        dismiss_for_session();
        return Ok(false);
    }
    if answer == PlusAiPromptAnswer::UseWhimbrel {
        // A failed save leaves the question on screen (with the error), so closing the window is
        // still Not now: SHOWING is cleared only once the answer goes through.
        prefer_whimbrel()?;
    }
    SHOWING.store(false, Ordering::SeqCst);
    if let Err(e) = flag.set() {
        log_write_failure(&e);
        dismiss_for_session();
    }
    Ok(true)
}

/// The per-PC answered flag, a file in the data folder.
pub struct AnsweredFlag(PathBuf);

impl AnsweredFlag {
    pub fn under(root: &Path) -> Self {
        AnsweredFlag(root.join(ANSWERED_FILE))
    }

    pub fn is_set(&self) -> bool {
        self.0.exists()
    }

    pub fn set(&self) -> std::io::Result<()> {
        std::fs::write(&self.0, b"1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn ready() -> Inputs {
        Inputs {
            plus: true,
            whimbrel_ready: true,
            prefers_whimbrel: false,
            local_only: false,
            answered: false,
            recording: false,
            data_note: true,
            dismissed_this_session: false,
        }
    }

    const NOTE: &str = "Your request goes through Whimbrel's server to Anthropic.";

    fn sources() -> Sources<'static> {
        Sources { plan_plus: Some(true), signed_in: true, token_cached: true, data_note: Some(NOTE), ..Default::default() }
    }

    /// A fresh temp folder per test; the statics are process-wide, so tests that touch them hold this.
    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("whimbrel-plus-prompt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    static STATICS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn reset_statics() -> std::sync::MutexGuard<'static, ()> {
        let guard = STATICS.lock().unwrap_or_else(|p| p.into_inner());
        DISMISSED.store(false, Ordering::SeqCst);
        SHOWING.store(false, Ordering::SeqCst);
        guard
    }

    #[test]
    fn shows_when_every_condition_holds() {
        assert_eq!(decide(ready()), Decision::Show);
    }

    #[test]
    fn not_on_plus() {
        assert_eq!(decide(Inputs { plus: false, ..ready() }), Decision::Hide);
    }

    #[test]
    fn not_signed_in_or_without_whimbrel_credentials() {
        assert_eq!(decide(Inputs { whimbrel_ready: false, ..ready() }), Decision::Hide);
        assert!(!inputs(&Sources { token_cached: false, ..sources() }).whimbrel_ready, "signed in but no cached token");
        assert!(!inputs(&Sources { signed_in: false, ..sources() }).whimbrel_ready);
    }

    #[test]
    fn already_on_whimbrel_is_marked_answered_silently() {
        assert_eq!(decide(Inputs { prefers_whimbrel: true, ..ready() }), Decision::MarkAnswered);
        assert_eq!(decide(Inputs { prefers_whimbrel: true, whimbrel_ready: false, ..ready() }), Decision::MarkAnswered);
        assert_eq!(decide(Inputs { prefers_whimbrel: true, plus: false, ..ready() }), Decision::Hide);
    }

    #[test]
    fn not_in_local_only_mode() {
        assert_eq!(decide(Inputs { local_only: true, ..ready() }), Decision::Hide);
    }

    #[test]
    fn never_asked_twice() {
        assert_eq!(decide(Inputs { answered: true, ..ready() }), Decision::Hide);
        assert_eq!(decide(Inputs { answered: true, prefers_whimbrel: true, ..ready() }), Decision::Hide);
    }

    #[test]
    fn not_while_recording() {
        assert_eq!(decide(Inputs { recording: true, ..ready() }), Decision::Hide);
    }

    #[test]
    fn not_without_the_data_note() {
        assert_eq!(decide(Inputs { data_note: false, ..ready() }), Decision::Hide);
        assert!(!inputs(&Sources { data_note: None, ..sources() }).data_note);
        assert!(!inputs(&Sources { data_note: Some("  "), ..sources() }).data_note);
    }

    #[test]
    fn not_again_this_session_after_not_now() {
        assert_eq!(decide(Inputs { dismissed_this_session: true, ..ready() }), Decision::Hide);
    }

    #[test]
    fn only_this_sessions_plan_read_counts() {
        assert!(!inputs(&Sources { plan_plus: Some(false), ..sources() }).plus);
        assert!(!inputs(&Sources { plan_plus: None, ..sources() }).plus, "before a plan read the stored flag doesn't ask");
    }

    #[test]
    fn the_check_says_show_wait_not_plus_or_done() {
        let _g = reset_statics();
        let d = dir("check");
        let flag = AnsweredFlag::under(&d);
        let v = check(&sources(), &flag);
        assert_eq!((v.state, v.note.as_deref()), (PlusAiPromptState::Show, Some(NOTE)));
        assert_eq!(check(&Sources { recording: true, ..sources() }, &flag).state, PlusAiPromptState::Wait);
        assert_eq!(check(&Sources { plan_plus: Some(false), ..sources() }, &flag).state, PlusAiPromptState::NotPlus);
        assert_eq!(check(&Sources { answered: true, ..sources() }, &flag).state, PlusAiPromptState::Done);
        // Already on Whimbrel AI: recorded, done.
        assert_eq!(check(&Sources { prefers_whimbrel: true, ..sources() }, &flag).state, PlusAiPromptState::Done);
        assert!(flag.is_set());
    }

    #[test]
    fn accept_sets_the_provider_and_the_flag() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("accept"));
        let set = Cell::new(0);
        let r: Result<bool, ()> = answer(PlusAiPromptAnswer::UseWhimbrel, &sources(), &flag, || {
            set.set(set.get() + 1);
            Ok(())
        });
        assert_eq!((r, set.get(), flag.is_set()), (Ok(true), 1, true));
    }

    #[test]
    fn keep_sets_only_the_flag() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("keep"));
        let set = Cell::new(0);
        let r: Result<bool, ()> = answer(PlusAiPromptAnswer::Keep, &sources(), &flag, || {
            set.set(set.get() + 1);
            Ok(())
        });
        assert_eq!((r, set.get(), flag.is_set()), (Ok(true), 0, true));
    }

    #[test]
    fn not_now_writes_nothing_and_closes_it_for_the_session() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("notnow"));
        let r: Result<bool, ()> = answer(PlusAiPromptAnswer::NotNow, &sources(), &flag, || panic!("no provider change"));
        assert_eq!(r, Ok(true));
        assert!(!flag.is_set());
        assert_eq!(check(&sources(), &flag).state, PlusAiPromptState::Done, "not asked again this session");
    }

    #[test]
    fn an_answer_after_the_conditions_changed_writes_nothing() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("stale"));
        for changed in [Sources { recording: true, ..sources() }, Sources { local_only: true, ..sources() }, Sources { plan_plus: Some(false), ..sources() }] {
            let r: Result<bool, ()> = answer(PlusAiPromptAnswer::UseWhimbrel, &changed, &flag, || panic!("no provider change"));
            assert_eq!(r, Ok(false));
            assert!(!flag.is_set());
            assert!(dismissed_this_session(), "a question that no longer applied counts as closed this session");
            DISMISSED.store(false, Ordering::SeqCst);
        }
    }

    #[test]
    fn before_this_sessions_plan_read_it_waits_and_shows_once_the_read_says_plus() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("planread"));
        // Fresh install, after Delete everything, or a plan bought elsewhere: no read yet.
        assert_eq!(check(&Sources { plan_plus: None, ..sources() }, &flag).state, PlusAiPromptState::Wait);
        // Already on Whimbrel AI, but no read yet: nothing is recorded until the plan says Plus.
        assert_eq!(check(&Sources { plan_plus: None, prefers_whimbrel: true, ..sources() }, &flag).state, PlusAiPromptState::Wait);
        assert!(!flag.is_set());
        // The read lands.
        assert_eq!(check(&Sources { plan_plus: Some(true), ..sources() }, &flag).state, PlusAiPromptState::Show);
        assert_eq!(check(&Sources { plan_plus: Some(false), ..sources() }, &flag).state, PlusAiPromptState::NotPlus);
    }

    #[test]
    fn a_failed_provider_save_records_no_answer() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("savefail"));
        displayed();
        let r = answer(PlusAiPromptAnswer::UseWhimbrel, &sources(), &flag, || Err("disk full"));
        assert_eq!(r, Err("disk full"));
        assert!(!flag.is_set());
        // Still on screen: closing the window now is Not now.
        window_closed();
        assert!(dismissed_this_session());
    }

    #[test]
    fn an_unwritable_flag_still_closes_the_question_for_the_session() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("unwritable").join("missing-folder"));
        let r: Result<bool, ()> = answer(PlusAiPromptAnswer::Keep, &sources(), &flag, || Ok(()));
        assert_eq!(r, Ok(true));
        assert!(dismissed_this_session());
    }

    #[test]
    fn closing_the_window_while_it_shows_is_not_now() {
        let _g = reset_statics();
        let flag = AnsweredFlag::under(&dir("close"));
        window_closed();
        assert!(!dismissed_this_session(), "closing with nothing on screen changes nothing");
        assert_eq!(check(&sources(), &flag).state, PlusAiPromptState::Show);
        window_closed();
        assert!(!dismissed_this_session(), "a Show the window never displayed isn't on screen");
        displayed();
        window_closed();
        assert!(dismissed_this_session() && !flag.is_set());
    }

    #[test]
    fn the_answered_flag_survives_a_relaunch() {
        let d = dir("relaunch");
        assert!(!AnsweredFlag::under(&d).is_set());
        AnsweredFlag::under(&d).set().unwrap();
        assert!(AnsweredFlag::under(&d).is_set(), "a new reader (a relaunch) sees it");
    }
}
