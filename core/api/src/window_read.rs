//! The hotkey panel's on-demand window reader, platform-independent part (spec §4). The
//! platform supplies a [`ForegroundSource`] (Windows: UI Automation and OCR in
//! `apps/windows/src-tauri/src/hotkey_panel`); this module decides what is read and in
//! what order, so the rules are tested on every CI host:
//!
//! - the exclusion check runs first, and an excluded app has no text method called;
//! - a focused password field is never read;
//! - a non-blank selection wins, else the window's text, else OCR when that text is
//!   under [`MIN_TEXT_CHARS`] (the Mac's `OCRFallback.minimumAccessibilityTextLength`);
//! - text and title are redacted (`memory::redact::redact_secrets`) before they leave.
//!
//! Window text is never logged here or by any caller.

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::window_ask::{Destination, WindowInput, WINDOW_TEXT_MAX_CHARS};

/// Below this many trimmed characters, the window's own text is treated as nothing and
/// OCR is tried (the Mac's rule).
pub const MIN_TEXT_CHARS: usize = 8;

/// The overall read deadline (the Mac panel's `readTimeout`).
pub const READ_DEADLINE: Duration = Duration::from_secs(3);

/// The window that was in front before the panel opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundInfo {
    pub pid: u32,
    /// Lowercased executable basename, e.g. `outlook.exe`.
    pub exe: String,
    /// The platform's app identity (Windows: §4.3 AUMID or exe).
    pub app_id: String,
    /// What the panel calls the app.
    pub app_name: String,
    pub title: String,
}

/// Why an app is excluded, so the panel can say so. The platform maps its own rule's
/// reasons onto these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcludeKind {
    /// Whimbrel itself.
    OwnApp,
    /// The secure desktop, credential prompts, UAC.
    System,
    PasswordManager,
    /// A browser's private window.
    PrivateWindow,
    /// The user excluded the app.
    User,
}

pub enum ReadOutcome {
    Read(WindowInput),
    /// The app is excluded from capture: the app's name and why. No text was read.
    Excluded(String, ExcludeKind),
    /// Nothing could be read: the app's name (empty when there was no window).
    Unreadable(String),
}

impl std::fmt::Debug for ReadOutcome {
    /// Never prints window text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadOutcome::Read(w) => write!(f, "Read({:?}, {} chars, selection: {})", w.app_name, w.text.chars().count(), w.is_selection),
            ReadOutcome::Excluded(a, why) => write!(f, "Excluded({a:?}, {why:?})"),
            ReadOutcome::Unreadable(a) => write!(f, "Unreadable({a:?})"),
        }
    }
}

/// What the platform can read from the foreground window. Every text method is called
/// only after the exclusion check passed.
pub trait ForegroundSource {
    fn foreground(&self) -> Option<ForegroundInfo>;
    /// Whether the focused element is a password field; its selection is then never read.
    fn focused_is_password(&self) -> bool {
        false
    }
    /// The focused element's selected text.
    fn selection(&self) -> Option<String>;
    /// The window's text, with password fields skipped.
    fn document_text(&self) -> Option<String>;
    fn ocr(&self) -> Option<String>;
}

fn trimmed_chars(s: &str) -> usize {
    s.trim().chars().count()
}

/// Reads the foreground window. `is_excluded` is the platform's capture-exclusion rule.
pub fn read_window(src: &dyn ForegroundSource, is_excluded: &dyn Fn(&ForegroundInfo) -> Option<ExcludeKind>) -> ReadOutcome {
    let Some(fg) = src.foreground() else {
        return ReadOutcome::Unreadable(String::new());
    };
    if let Some(why) = is_excluded(&fg) {
        return ReadOutcome::Excluded(fg.app_name, why);
    }
    let selection = if src.focused_is_password() { None } else { src.selection() };
    let (text, is_selection) = match selection {
        Some(s) if trimmed_chars(&s) > 0 => (s, true),
        _ => {
            let doc = src.document_text().unwrap_or_default();
            if trimmed_chars(&doc) < MIN_TEXT_CHARS {
                match src.ocr() {
                    Some(o) if trimmed_chars(&o) > 0 => (o, false),
                    _ => (doc, false),
                }
            } else {
                (doc, false)
            }
        }
    };
    let text = memory::redact::redact_secrets(&text).into_owned();
    if text.trim().is_empty() {
        return ReadOutcome::Unreadable(fg.app_name);
    }
    let title = memory::redact::redact_secrets(&fg.title).into_owned();
    ReadOutcome::Read(WindowInput { app_name: fg.app_name, title, text, is_selection })
}

/// Whether `window_ask` will cut the (already redacted) text to fit.
pub fn will_truncate(input: &WindowInput) -> bool {
    input.text.chars().count() > WINDOW_TEXT_MAX_CHARS
}

/// Work running on its own thread, to be waited for with a deadline.
pub struct Pending<T>(Option<mpsc::Receiver<T>>);

/// Starts `f` on its own thread, named `name`, and returns at once.
pub fn spawn_pending<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Pending<T> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let _ = tx.send(f());
    });
    Pending(spawned.ok().map(|_| rx))
}

impl<T> Pending<T> {
    /// The result if it arrives by `deadline`. `None` on timeout: the work cannot be
    /// cancelled (a hung UI Automation call), so its late result is dropped.
    pub fn wait_until(self, deadline: Instant) -> Option<T> {
        let rx = self.0?;
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()
    }
}

/// Runs `f` on its own thread and waits at most `deadline`.
pub fn run_with_deadline<T: Send + 'static>(deadline: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    spawn_pending("whimbrel-window-read", f).wait_until(Instant::now() + deadline)
}

/// The panel's one request at a time, under one lock: the recorded window, what was read
/// from it, the destination the panel showed, and the answer streaming for it. Each open
/// starts a new id; anything carrying an older id is stale. Because a stream is recorded
/// under the same lock that ends a request, a close can never miss a stream that started.
pub struct PanelRequests<T> {
    inner: Mutex<RequestState<T>>,
}

struct RequestState<T> {
    id: u64,
    target: Option<T>,
    input: Option<WindowInput>,
    shown: Option<Destination>,
    run: Option<String>,
}

impl<T> Default for PanelRequests<T> {
    fn default() -> Self {
        PanelRequests { inner: Mutex::new(RequestState { id: 0, target: None, input: None, shown: None, run: None }) }
    }
}

/// What ending a request leaves to the caller: the recorded window, and the stream to
/// cancel.
#[derive(Debug, PartialEq)]
pub struct Dropped<T> {
    pub target: T,
    pub run: Option<String>,
}

impl<T: Clone> PanelRequests<T> {
    /// Starts a request for `target` (`None`: no window was in front), ending any earlier
    /// one. Returns the new id and an earlier stream to cancel.
    pub fn begin(&self, target: Option<T>) -> (u64, Option<String>) {
        let mut g = self.inner.lock().unwrap();
        g.id += 1;
        g.target = target;
        g.input = None;
        g.shown = None;
        (g.id, g.run.take())
    }

    /// The current request's id, while it has a window.
    pub fn current_id(&self) -> Option<u64> {
        let g = self.inner.lock().unwrap();
        g.target.as_ref().map(|_| g.id)
    }

    /// The recorded window for request `id`, if it is current.
    pub fn target(&self, id: u64) -> Option<T> {
        let g = self.inner.lock().unwrap();
        if g.id == id { g.target.clone() } else { None }
    }

    pub fn is_current(&self, id: u64) -> bool {
        self.target(id).is_some()
    }

    /// Keeps the read for request `id`; false (nothing kept) when it is not current.
    pub fn store_read(&self, id: u64, input: WindowInput) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.id != id || g.target.is_none() {
            return false;
        }
        g.input = Some(input);
        true
    }

    pub fn input(&self, id: u64) -> Option<WindowInput> {
        let g = self.inner.lock().unwrap();
        if g.id == id { g.input.clone() } else { None }
    }

    /// Records the destination the panel shows for request `id`; false when not current.
    pub fn set_shown(&self, id: u64, destination: Destination) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.id != id || g.target.is_none() {
            return false;
        }
        g.shown = Some(destination);
        true
    }

    pub fn shown(&self, id: u64) -> Option<Destination> {
        let g = self.inner.lock().unwrap();
        if g.id == id { g.shown.clone() } else { None }
    }

    /// Records the stream answering request `id`. False when the request is no longer
    /// current: the caller cancels the run.
    pub fn start_run(&self, id: u64, run_id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.id != id || g.target.is_none() {
            return false;
        }
        g.run = Some(run_id.to_string());
        true
    }

    /// The stream `run_id` ended.
    pub fn end_run(&self, run_id: &str) {
        let mut g = self.inner.lock().unwrap();
        if g.run.as_deref() == Some(run_id) {
            g.run = None;
        }
    }

    /// Ends request `id` if it is current: the read and the shown destination are dropped.
    /// A later request is left alone.
    pub fn drop_request(&self, id: u64) -> Option<Dropped<T>> {
        let mut g = self.inner.lock().unwrap();
        if g.id != id {
            return None;
        }
        let target = g.target.take()?;
        g.input = None;
        g.shown = None;
        Some(Dropped { target, run: g.run.take() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const KEY: &str = "sk-ant-abcdefghijklmnopqrstuvwxyz0123456789";

    #[derive(Default)]
    struct Fake {
        fg: Option<ForegroundInfo>,
        password: bool,
        selection: Option<String>,
        document: Option<String>,
        ocr: Option<String>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                fg: Some(ForegroundInfo {
                    pid: 42,
                    exe: "outlook.exe".into(),
                    app_id: "outlook.exe".into(),
                    app_name: "Outlook".into(),
                    title: "Re: Q3 rollout".into(),
                }),
                ..Default::default()
            }
        }
        fn text_calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
    }

    impl ForegroundSource for Fake {
        fn foreground(&self) -> Option<ForegroundInfo> {
            self.fg.clone()
        }
        fn focused_is_password(&self) -> bool {
            self.calls.borrow_mut().push("password");
            self.password
        }
        fn selection(&self) -> Option<String> {
            self.calls.borrow_mut().push("selection");
            self.selection.clone()
        }
        fn document_text(&self) -> Option<String> {
            self.calls.borrow_mut().push("document");
            self.document.clone()
        }
        fn ocr(&self) -> Option<String> {
            self.calls.borrow_mut().push("ocr");
            self.ocr.clone()
        }
    }

    fn never(_: &ForegroundInfo) -> Option<ExcludeKind> {
        None
    }

    fn read(f: &Fake) -> WindowInput {
        match read_window(f, &never) {
            ReadOutcome::Read(w) => w,
            other => panic!("expected Read, got {other:?}"),
        }
    }

    #[test]
    fn a_selection_is_preferred_and_marked() {
        let mut f = Fake::new();
        f.selection = Some("the selected words".into());
        f.document = Some("the whole window text".into());
        let w = read(&f);
        assert_eq!(w.text, "the selected words");
        assert!(w.is_selection);
        assert_eq!((w.app_name.as_str(), w.title.as_str()), ("Outlook", "Re: Q3 rollout"));
        assert!(!f.text_calls().contains(&"document"));
    }

    #[test]
    fn a_blank_selection_falls_back_to_the_window_text() {
        let mut f = Fake::new();
        f.selection = Some("  \n ".into());
        f.document = Some("the whole window text".into());
        let w = read(&f);
        assert_eq!(w.text, "the whole window text");
        assert!(!w.is_selection);
        assert!(!f.text_calls().contains(&"ocr"));
    }

    #[test]
    fn an_excluded_app_reads_no_text_at_all() {
        let mut f = Fake::new();
        f.selection = Some("secret".into());
        f.document = Some("secret window".into());
        let outcome = read_window(&f, &|fg: &ForegroundInfo| (fg.exe == "outlook.exe").then_some(ExcludeKind::User));
        assert!(matches!(outcome, ReadOutcome::Excluded(ref app, ExcludeKind::User) if app == "Outlook"), "{outcome:?}");
        assert_eq!(format!("{outcome:?}"), r#"Excluded("Outlook", User)"#);
        assert!(f.text_calls().is_empty(), "text methods called: {:?}", f.text_calls());
    }

    #[test]
    fn nothing_readable_is_unreadable() {
        let f = Fake::new();
        assert!(matches!(read_window(&f, &never), ReadOutcome::Unreadable(ref app) if app == "Outlook"));
        assert_eq!(f.text_calls(), vec!["password", "selection", "document", "ocr"]);
        let mut none = Fake::new();
        none.fg = None;
        assert!(matches!(read_window(&none, &never), ReadOutcome::Unreadable(ref app) if app.is_empty()));
    }

    #[test]
    fn short_window_text_falls_back_to_ocr() {
        let mut f = Fake::new();
        f.document = Some("File".into());
        f.ocr = Some("text OCR read off the screen".into());
        assert_eq!(read(&f).text, "text OCR read off the screen");
        // OCR found nothing: the short text is still better than nothing.
        f.ocr = None;
        assert_eq!(read(&f).text, "File");
    }

    #[test]
    fn a_password_field_is_never_read() {
        let mut f = Fake::new();
        f.password = true;
        f.selection = Some("hunter2-password".into());
        f.document = Some("the login page around it".into());
        let w = read(&f);
        assert_eq!(w.text, "the login page around it");
        assert!(!f.text_calls().contains(&"selection"));
    }

    #[test]
    fn secrets_are_redacted_before_text_leaves_the_reader() {
        let mut f = Fake::new();
        f.selection = Some(format!("my key is {KEY} ok"));
        f.fg.as_mut().unwrap().title = format!("token {KEY}");
        let w = read(&f);
        assert!(!w.text.contains(KEY) && w.text.contains("[REDACTED"), "{}", w.text);
        assert!(!w.title.contains(KEY));
        // A selection that is only a secret still reads as the redaction marker.
        let mut doc = Fake::new();
        doc.document = Some(format!("api key: {KEY}"));
        assert!(!read(&doc).text.contains(KEY));
    }

    #[test]
    fn truncation_is_reported_past_the_cap() {
        let mut w = WindowInput { app_name: "a".into(), title: "t".into(), text: "x".repeat(WINDOW_TEXT_MAX_CHARS), is_selection: false };
        assert!(!will_truncate(&w));
        w.text.push('é');
        assert!(will_truncate(&w));
    }

    #[test]
    fn the_deadline_drops_a_late_result() {
        assert_eq!(run_with_deadline(Duration::from_secs(2), || 7), Some(7));
        let late = run_with_deadline(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(500));
            7
        });
        assert_eq!(late, None);
    }

    #[test]
    fn pending_work_shares_one_deadline() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(100);
        let fast = spawn_pending("test-fast", || 1);
        let slow = spawn_pending("test-slow", || {
            std::thread::sleep(Duration::from_millis(600));
            2
        });
        assert_eq!(fast.wait_until(deadline), Some(1));
        assert_eq!(slow.wait_until(deadline), None);
        assert!(start.elapsed() < Duration::from_millis(400), "waited past the deadline");
    }

    fn body(text: &str) -> WindowInput {
        WindowInput { app_name: "a".into(), title: "t".into(), text: text.into(), is_selection: false }
    }

    #[test]
    fn a_stale_request_neither_stores_nor_reads() {
        let r = PanelRequests::<&str>::default();
        let (first, _) = r.begin(Some("w1"));
        let (second, _) = r.begin(Some("w2"));
        assert!(!r.store_read(first, body("old")), "a read for an older open is dropped");
        assert!(!r.set_shown(first, Destination::Local), "an older read can't replace the shown line");
        assert!(r.store_read(second, body("new")));
        assert!(r.set_shown(second, Destination::LocalOnly));
        assert!(r.input(first).is_none() && r.shown(first).is_none() && r.target(first).is_none());
        assert_eq!(r.input(second).unwrap().text, "new");
        assert_eq!(r.shown(second), Some(Destination::LocalOnly));
        assert_eq!(r.current_id(), Some(second));
    }

    #[test]
    fn no_window_means_no_current_request() {
        let r = PanelRequests::<&str>::default();
        let (id, _) = r.begin(None);
        assert!(!r.is_current(id) && r.current_id().is_none());
        assert!(!r.store_read(id, body("x")));
    }

    #[test]
    fn dropping_a_request_returns_its_stream_and_clears_everything() {
        let r = PanelRequests::<&str>::default();
        let (id, _) = r.begin(Some("w"));
        r.store_read(id, body("x"));
        r.set_shown(id, Destination::Local);
        assert!(r.start_run(id, "run-1"));
        assert_eq!(r.drop_request(id), Some(Dropped { target: "w", run: Some("run-1".into()) }));
        assert!(r.input(id).is_none() && r.shown(id).is_none() && !r.is_current(id));
        assert_eq!(r.drop_request(id), None, "a second close is a no-op");
    }

    #[test]
    fn a_stream_starting_after_the_close_is_refused() {
        let r = PanelRequests::<&str>::default();
        let (id, _) = r.begin(Some("w"));
        r.drop_request(id);
        assert!(!r.start_run(id, "run-1"), "the caller must cancel it");
    }

    #[test]
    fn a_late_close_leaves_the_newer_request_alone() {
        let r = PanelRequests::<&str>::default();
        let (old, _) = r.begin(Some("w1"));
        let (new, _) = r.begin(Some("w2"));
        assert_eq!(r.drop_request(old), None);
        assert!(r.is_current(new));
    }

    #[test]
    fn a_new_request_hands_back_the_old_stream_and_end_run_forgets_only_its_own() {
        let r = PanelRequests::<&str>::default();
        let (id, _) = r.begin(Some("w"));
        r.start_run(id, "run-1");
        r.end_run("run-other");
        let (_, old_run) = r.begin(Some("w2"));
        assert_eq!(old_run.as_deref(), Some("run-1"));
        let (id, _) = r.begin(Some("w3"));
        r.start_run(id, "run-2");
        r.end_run("run-2");
        assert_eq!(r.drop_request(id).unwrap().run, None);
    }
}
