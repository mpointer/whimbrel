//! The Tauri presenter (brief 2.7, 3.3): the coordinator's thread blocks in `present` or
//! `run_onboarding` until a launch command answers; the commands read the current stage.
//!
//! The webview never holds the launch's state: it asks `launch_state` on every mount (the
//! main window can be closed and rebuilt) and listens to `launch://stage` and
//! `launch://progress`. Answers the current screen does not offer are refused here, so a stale
//! or forged click changes nothing.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use api::launch_decision::RecoveryReason;

use super::{LaunchPresenter, LaunchResponse, LaunchScreen};
use crate::db_key::KeyBytes;

/// Where the launch is, for the webview.
#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Deciding, opening or running a step the user does not answer.
    Starting,
    /// The recovery setup of a new install; `reuses_key` when it wraps `db-key-pending`.
    Onboarding { reuses_key: bool },
    Migrating { overall: f64 },
    Recovery { reason: RecoveryReason },
    /// `still_unavailable`: Retry came back to this same screen.
    KeyUnavailable { still_unavailable: bool },
    /// Any other blocking screen, drawn from the typed screen.
    Screen { screen: LaunchScreen },
    Opened,
    OpenedWithoutStore,
    Quitting,
}

impl Stage {
    pub fn is_final(&self) -> bool {
        matches!(self, Stage::Opened | Stage::OpenedWithoutStore | Stage::Quitting)
    }
}

/// Where stage changes go (live: Tauri events and the main window; tests: a log).
/// Called outside the presenter's lock (a sink may block on the main thread).
pub trait StageSink: Send + Sync {
    /// `waiting`: the launch waits for the user.
    fn stage_changed(&self, stage: &Stage, waiting: bool);
    fn progress(&self, overall: f64);
}

/// What the coordinator thread is blocked on.
enum Waiting {
    Nothing,
    Screen(LaunchScreen),
    /// The setup; the pending key it must wrap, if any.
    Onboarding(Option<KeyBytes>),
}

enum Answer {
    Response(LaunchResponse),
    Onboarding(Option<KeyBytes>),
}

struct State {
    stage: Stage,
    /// Bumped by every stage change (not by progress within a migration).
    generation: u64,
    waiting: Waiting,
    answer: Option<Answer>,
}

/// What closing the main window before the store opened does (brief 2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAnswer {
    /// Closed as the screen's Quit.
    Quit,
    /// Refused: the migration (or a short step) is running.
    Refuse,
    /// Refused; the webview asks "Quit anyway?" first (a fresh key whose words exist).
    AskFirst,
}

pub struct TauriPresenter {
    st: Mutex<State>,
    cv: Condvar,
    sink: Box<dyn StageSink>,
}

impl TauriPresenter {
    pub fn new(sink: Box<dyn StageSink>) -> Self {
        TauriPresenter {
            st: Mutex::new(State { stage: Stage::Starting, generation: 0, waiting: Waiting::Nothing, answer: None }),
            cv: Condvar::new(),
            sink,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Changes the stage under the lock; the caller tells the sink once it dropped the lock
    /// ([`Self::tell`]).
    fn set_stage(&self, st: &mut State, stage: Stage) -> (Stage, bool) {
        st.stage = stage;
        st.generation += 1;
        self.cv.notify_all();
        (st.stage.clone(), !matches!(st.waiting, Waiting::Nothing))
    }

    fn tell(&self, (stage, waiting): (Stage, bool)) {
        self.sink.stage_changed(&stage, waiting);
    }

    /// The stage, its generation, and whether the coordinator waits for the user.
    pub fn snapshot(&self) -> (Stage, u64, bool) {
        let st = self.lock();
        (st.stage.clone(), st.generation, !matches!(st.waiting, Waiting::Nothing))
    }

    /// Whether the launch needs the window (anything but the silent start).
    pub fn needs_window(&self) -> bool {
        !matches!(self.lock().stage, Stage::Starting)
    }

    /// The launch ended (`Opened`, `OpenedWithoutStore` or `Quitting`).
    pub fn finish(&self, stage: Stage) {
        let mut st = self.lock();
        st.waiting = Waiting::Nothing;
        let told = self.set_stage(&mut st, stage);
        drop(st);
        self.tell(told);
    }

    /// The webview's answer to the current screen; refused (false) unless that screen
    /// offers it. Quit also ends the setup.
    pub fn respond(&self, r: LaunchResponse) -> bool {
        let mut st = self.lock();
        let accepted = match &st.waiting {
            Waiting::Screen(screen) => screen.offers(&r),
            Waiting::Onboarding(_) => r == LaunchResponse::Quit,
            Waiting::Nothing => false,
        };
        if !accepted {
            return false;
        }
        let answer = match (&st.waiting, r) {
            (Waiting::Onboarding(_), _) => Answer::Onboarding(None),
            (_, r) => Answer::Response(r),
        };
        st.answer = Some(answer);
        st.waiting = Waiting::Nothing;
        self.cv.notify_all();
        true
    }

    /// A restored key for the recovery screen (never written here: the coordinator proves it
    /// first). The generation to wait past, or `None` outside the recovery screen.
    pub fn restored_key(&self, key: KeyBytes) -> Option<u64> {
        let mut st = self.lock();
        if !matches!(&st.waiting, Waiting::Screen(LaunchScreen::Recovery(_))) {
            return None;
        }
        st.answer = Some(Answer::Response(LaunchResponse::RestoredKey(key)));
        st.waiting = Waiting::Nothing;
        self.cv.notify_all();
        Some(st.generation)
    }

    /// Waits until the stage moves past `generation` to something the user sees or the end
    /// (not the silent start or a migration's first report), or `timeout`. The stage then.
    pub fn wait_past(&self, generation: u64, timeout: Duration) -> Stage {
        let deadline = Instant::now() + timeout;
        let mut st = self.lock();
        loop {
            let settled = !matches!(st.waiting, Waiting::Nothing) || st.stage.is_final();
            if st.generation > generation && settled {
                return st.stage.clone();
            }
            let now = Instant::now();
            if now >= deadline {
                return st.stage.clone();
            }
            st = self.cv.wait_timeout(st, deadline - now).map(|(g, _)| g).unwrap_or_else(|p| p.into_inner().0);
        }
    }

    /// The pending key the setup must wrap, while the launch waits in the setup.
    pub fn onboarding(&self) -> Option<Option<KeyBytes>> {
        match &self.lock().waiting {
            Waiting::Onboarding(existing) => Some(existing.clone()),
            _ => None,
        }
    }

    /// The setup's key for the launch; refused (false) unless the launch waits in the setup.
    pub fn deliver_onboarding_key(&self, key: KeyBytes) -> bool {
        let mut st = self.lock();
        if !matches!(st.waiting, Waiting::Onboarding(_)) {
            return false;
        }
        st.answer = Some(Answer::Onboarding(Some(key)));
        st.waiting = Waiting::Nothing;
        self.cv.notify_all();
        true
    }

    /// Closing the main window before the store opened. `fresh_material`: the setup made
    /// words for a fresh key (its Quit asks first).
    pub fn close_requested(&self, fresh_material: bool) -> CloseAnswer {
        let waiting_screen = {
            let st = self.lock();
            match (&st.waiting, &st.stage) {
                (Waiting::Onboarding(_), _) if fresh_material => return CloseAnswer::AskFirst,
                (Waiting::Onboarding(_), _) | (Waiting::Screen(_), _) => true,
                (_, stage) if stage.is_final() => return CloseAnswer::Quit,
                _ => false,
            }
        };
        if waiting_screen && self.respond(LaunchResponse::Quit) {
            CloseAnswer::Quit
        } else {
            CloseAnswer::Refuse
        }
    }

    fn wait_answer(&self, mut st: MutexGuard<'_, State>) -> Answer {
        loop {
            if let Some(a) = st.answer.take() {
                return a;
            }
            st = self.cv.wait(st).unwrap_or_else(|p| p.into_inner());
        }
    }
}

impl LaunchPresenter for TauriPresenter {
    fn present(&self, screen: &LaunchScreen) -> LaunchResponse {
        let mut st = self.lock();
        let stage = match screen {
            LaunchScreen::Recovery(reason) => Stage::Recovery { reason: *reason },
            LaunchScreen::KeyUnavailable(_) => {
                Stage::KeyUnavailable { still_unavailable: matches!(st.stage, Stage::KeyUnavailable { .. }) }
            }
            other => Stage::Screen { screen: other.clone() },
        };
        st.answer = None;
        st.waiting = Waiting::Screen(screen.clone());
        let told = self.set_stage(&mut st, stage);
        drop(st);
        self.tell(told);
        match self.wait_answer(self.lock()) {
            Answer::Response(r) => r,
            Answer::Onboarding(_) => LaunchResponse::Quit,
        }
    }

    fn run_onboarding(&self, existing_key: Option<&KeyBytes>) -> Option<KeyBytes> {
        let mut st = self.lock();
        st.answer = None;
        st.waiting = Waiting::Onboarding(existing_key.cloned());
        let told = self.set_stage(&mut st, Stage::Onboarding { reuses_key: existing_key.is_some() });
        drop(st);
        self.tell(told);
        match self.wait_answer(self.lock()) {
            Answer::Onboarding(k) => k,
            Answer::Response(_) => None,
        }
    }

    fn migration_progress(&self, progress: &api::encrypt_migration::Progress) {
        let mut st = self.lock();
        let told = match st.stage {
            Stage::Migrating { overall } if progress.overall <= overall => return,
            Stage::Migrating { .. } => {
                st.stage = Stage::Migrating { overall: progress.overall };
                None
            }
            _ => Some(self.set_stage(&mut st, Stage::Migrating { overall: progress.overall })),
        };
        drop(st);
        if let Some(told) = told {
            self.tell(told);
        }
        self.sink.progress(progress.overall);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Log(Mutex<(u32, Vec<f64>)>);
    impl StageSink for Arc<Log> {
        fn stage_changed(&self, _: &Stage, _: bool) {
            self.0.lock().unwrap().0 += 1;
        }
        fn progress(&self, overall: f64) {
            self.0.lock().unwrap().1.push(overall);
        }
    }

    fn presenter() -> (Arc<TauriPresenter>, Arc<Log>) {
        let log = Arc::new(Log::default());
        (Arc::new(TauriPresenter::new(Box::new(log.clone()))), log)
    }

    fn wait_for_waiting(p: &TauriPresenter) {
        let t = Instant::now();
        while !p.snapshot().2 {
            assert!(t.elapsed() < Duration::from_secs(5), "the launch thread never waited");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_screen_takes_only_the_answers_it_offers() {
        let (p, _) = presenter();
        let t = {
            let p = p.clone();
            std::thread::spawn(move || p.present(&LaunchScreen::Damaged(api::launch_decision::DamageReason::Damaged)))
        };
        wait_for_waiting(&p);
        assert!(!p.respond(LaunchResponse::Retry), "Damaged offers only Quit");
        assert!(!p.respond(LaunchResponse::ContinueWithoutStore));
        assert!(p.restored_key(KeyBytes::new([1; 32])).is_none(), "a key only on the recovery screen");
        assert!(p.respond(LaunchResponse::Quit));
        assert_eq!(t.join().unwrap(), LaunchResponse::Quit);
        assert!(!p.respond(LaunchResponse::Quit), "nothing waits now");
    }

    #[test]
    fn continue_without_the_store_only_after_a_failed_plain_open() {
        let (p, _) = presenter();
        for (error, offered) in [
            (super::super::LaunchError::PlainOpenFailed, true),
            (super::super::LaunchError::KeyedOpenFailed(super::super::OpenKind::NeedsKey), false),
        ] {
            let t = {
                let p = p.clone();
                std::thread::spawn(move || p.present(&LaunchScreen::Error(error)))
            };
            wait_for_waiting(&p);
            assert_eq!(p.respond(LaunchResponse::ContinueWithoutStore), offered);
            if !offered {
                assert!(p.respond(LaunchResponse::Quit));
            }
            t.join().unwrap();
        }
    }

    #[test]
    fn retry_back_to_key_unavailable_says_still_unavailable() {
        let (p, _) = presenter();
        let screen = LaunchScreen::KeyUnavailable(super::super::LaunchDiagnostic("Access is denied.".into()));
        let t = {
            let (p, screen) = (p.clone(), screen.clone());
            std::thread::spawn(move || {
                let first = p.present(&screen);
                let second = p.present(&screen);
                (first, second)
            })
        };
        wait_for_waiting(&p);
        assert_eq!(p.snapshot().0, Stage::KeyUnavailable { still_unavailable: false });
        assert!(p.respond(LaunchResponse::Retry));
        let t0 = Instant::now();
        while p.snapshot().0 != (Stage::KeyUnavailable { still_unavailable: true }) {
            assert!(t0.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(p.respond(LaunchResponse::Quit));
        assert_eq!(t.join().unwrap(), (LaunchResponse::Retry, LaunchResponse::Quit));
    }

    #[test]
    fn a_restored_key_waits_for_what_the_launch_does_next() {
        let (p, _) = presenter();
        let t = {
            let p = p.clone();
            std::thread::spawn(move || {
                let r = p.present(&LaunchScreen::Recovery(RecoveryReason::NeedsKey));
                assert!(matches!(r, LaunchResponse::RestoredKey(_)));
                // The key was refused: the recovery screen again.
                p.present(&LaunchScreen::Recovery(RecoveryReason::KeyRejected))
            })
        };
        wait_for_waiting(&p);
        let generation = p.restored_key(KeyBytes::new([3; 32])).unwrap();
        assert_eq!(p.wait_past(generation, Duration::from_secs(5)), Stage::Recovery { reason: RecoveryReason::KeyRejected });
        assert!(p.respond(LaunchResponse::Quit));
        t.join().unwrap();
    }

    #[test]
    fn the_setup_answers_with_its_key_or_quit_and_nothing_else() {
        let (p, _) = presenter();
        let t = {
            let p = p.clone();
            std::thread::spawn(move || p.run_onboarding(None))
        };
        wait_for_waiting(&p);
        assert_eq!(p.onboarding(), Some(None));
        assert!(!p.respond(LaunchResponse::Retry));
        assert!(p.deliver_onboarding_key(KeyBytes::new([5; 32])));
        assert_eq!(t.join().unwrap(), Some(KeyBytes::new([5; 32])));
        assert!(!p.deliver_onboarding_key(KeyBytes::new([6; 32])), "nothing waits now");

        let t = {
            let p = p.clone();
            std::thread::spawn(move || p.run_onboarding(Some(&KeyBytes::new([7; 32]))))
        };
        wait_for_waiting(&p);
        assert_eq!(p.snapshot().0, Stage::Onboarding { reuses_key: true });
        assert!(p.respond(LaunchResponse::Quit));
        assert_eq!(t.join().unwrap(), None);
    }

    #[test]
    fn migration_progress_never_goes_back() {
        let (p, log) = presenter();
        let at = |overall| api::encrypt_migration::Progress { step: api::encrypt_migration::Step::Copy, step_fraction: 0.0, overall, note: None };
        for v in [0.1, 0.4, 0.3, 0.4, 0.9] {
            p.migration_progress(&at(v));
        }
        assert_eq!(log.0.lock().unwrap().1, vec![0.1, 0.4, 0.9]);
        assert_eq!(p.snapshot().0, Stage::Migrating { overall: 0.9 });
        assert_eq!(log.0.lock().unwrap().0, 1, "one stage change for the whole run");
    }

    #[test]
    fn closing_the_window_quits_a_screen_refuses_a_migration_and_asks_with_fresh_words() {
        let (p, _) = presenter();
        assert_eq!(p.close_requested(false), CloseAnswer::Refuse, "the silent start");
        p.migration_progress(&api::encrypt_migration::Progress { step: api::encrypt_migration::Step::Copy, step_fraction: 0.0, overall: 0.2, note: None });
        assert_eq!(p.close_requested(false), CloseAnswer::Refuse, "never during the migration");

        let t = {
            let p = p.clone();
            std::thread::spawn(move || p.run_onboarding(None))
        };
        wait_for_waiting(&p);
        assert_eq!(p.close_requested(true), CloseAnswer::AskFirst);
        assert_eq!(p.close_requested(false), CloseAnswer::Quit);
        assert_eq!(t.join().unwrap(), None);

        let t = {
            let p = p.clone();
            std::thread::spawn(move || p.present(&LaunchScreen::MigrationInterrupted))
        };
        wait_for_waiting(&p);
        assert_eq!(p.close_requested(false), CloseAnswer::Quit);
        assert_eq!(t.join().unwrap(), LaunchResponse::Quit);
    }
}
