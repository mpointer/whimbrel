//! The hotkey panel's on-demand reader on Windows (spec §4): the [`ForegroundSource`]
//! over UI Automation and OCR. What is read, in what order, the exclusion check and the
//! redaction live in `api::window_read` (platform-independent and unit-tested there,
//! since this crate does not build on a Mac).
//!
//! - Selection: `IUIAutomationTextPattern::GetSelection` on the window's TextPattern
//!   elements. The panel holds keyboard focus by the time this runs, so the focused
//!   element is not the user's; an inactive window keeps its selection, so its text
//!   elements are asked instead. Password fields are left out of the search.
//! - Window text: the capture service's two strategies (`capture/uia.rs`), merged the
//!   same way, both leaving out password fields.
//! - OCR (`capture/ocr.rs`) when that text is short, or when the capture policy says UIA
//!   is showing a browser's chrome rather than its page.
//!
//! Every call here must run on a thread that may block: UIA has no per-call timeout. The
//! caller runs it as `api::window_read::spawn_pending` work waited on with the read's deadline. Window text is never logged.

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;

pub use api::window_read::{ExcludeKind, ForegroundInfo, ForegroundSource, ReadOutcome};

use crate::capture::exclusions::{ExcludeReason, Exclusions};
use crate::capture::window::WindowInfo;
use crate::capture::{app_id, merge, ocr, policy, uia};

/// TextPattern elements asked for a selection, at most (`uia.rs`'s document cap).
const MAX_SELECTION_ELEMENTS: usize = 16;

/// Reads the window recorded when the hotkey was pressed. The capture exclusions run
/// first, and an excluded app has no text read.
pub fn read_window(src: &dyn ForegroundSource, exclusions: &Exclusions) -> ReadOutcome {
    let own_pid = std::process::id();
    api::window_read::read_window(src, &|fg: &ForegroundInfo| {
        exclusions.check(&fg.exe, &fg.app_id, &fg.title, fg.pid, own_pid).map(exclude_kind)
    })
}

/// The capture rule's reason, as core names it for the panel.
pub fn exclude_kind(reason: ExcludeReason) -> ExcludeKind {
    match reason {
        ExcludeReason::Whimbrel => ExcludeKind::OwnApp,
        ExcludeReason::System => ExcludeKind::System,
        ExcludeReason::PasswordManager => ExcludeKind::PasswordManager,
        ExcludeReason::PrivateWindow => ExcludeKind::PrivateWindow,
        ExcludeReason::User => ExcludeKind::User,
    }
}

/// What the panel calls an app: the executable's name without `.exe` (`outlook.exe` →
/// `outlook`), or the identity when there is no exe.
pub fn display_name(exe: &str, app_id: &str) -> String {
    let name = exe.strip_suffix(".exe").unwrap_or(exe);
    if name.is_empty() {
        app_id.to_string()
    } else {
        name.to_string()
    }
}

/// The window that was in front when the hotkey was pressed. Plain data (the handle as
/// an integer), so it can move to the thread that reads it.
#[derive(Debug, Clone)]
pub struct Target {
    pub info: ForegroundInfo,
    pub hwnd: isize,
    pub minimized: bool,
}

impl Target {
    pub fn from_window(w: &WindowInfo) -> Self {
        Target {
            info: ForegroundInfo {
                pid: w.pid,
                exe: w.exe.clone(),
                app_id: app_id::normalize(&w.app_id, &w.exe),
                app_name: display_name(&w.exe, &w.app_id),
                title: w.title.clone(),
            },
            hwnd: w.hwnd.0 as isize,
            minimized: w.minimized,
        }
    }

    pub fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }
}

/// UIA and OCR on one recorded window. Build it on the thread that reads: `Uia::new`
/// initializes COM there.
pub struct UiaSource {
    info: ForegroundInfo,
    hwnd: HWND,
    minimized: bool,
    uia: Option<uia::Uia>,
}

impl UiaSource {
    pub fn new(target: &Target) -> Self {
        let uia = match uia::Uia::new() {
            Ok(u) => Some(u),
            Err(e) => {
                tracing::warn!(error = %e, "hotkey panel: UI Automation unavailable");
                None
            }
        };
        UiaSource { info: target.info.clone(), hwnd: target.hwnd(), minimized: target.minimized, uia }
    }

    fn selection_text(&self, u: &uia::Uia) -> windows::core::Result<Option<String>> {
        unsafe {
            let automation = u.automation();
            let root = automation.ElementFromHandle(self.hwnd)?;
            let has_tp = automation.CreatePropertyCondition(UIA_IsTextPatternAvailablePropertyId, &VARIANT::from(true))?;
            let not_password = automation.CreatePropertyCondition(UIA_IsPasswordPropertyId, &VARIANT::from(false))?;
            let cond = automation.CreateAndCondition(&has_tp, &not_password)?;
            let found = root.FindAll(TreeScope_Subtree, &cond)?;
            let n = found.Length()? as usize;
            for i in 0..n.min(MAX_SELECTION_ELEMENTS) {
                let el = found.GetElement(i as i32)?;
                // Checked again on the element itself: never read a password field.
                if el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(true) {
                    continue;
                }
                let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) else {
                    continue;
                };
                let Ok(ranges) = tp.GetSelection() else { continue };
                let mut text = String::new();
                for j in 0..ranges.Length().unwrap_or(0) {
                    if let Ok(t) = ranges.GetElement(j).and_then(|r| r.GetText(-1)) {
                        text.push_str(&t.to_string());
                    }
                }
                if !text.trim().is_empty() {
                    return Ok(Some(text));
                }
            }
            Ok(None)
        }
    }
}

impl ForegroundSource for UiaSource {
    fn foreground(&self) -> Option<ForegroundInfo> {
        Some(self.info.clone())
    }

    fn selection(&self) -> Option<String> {
        let u = self.uia.as_ref()?;
        if self.minimized {
            return None;
        }
        self.selection_text(u).ok().flatten()
    }

    fn document_text(&self) -> Option<String> {
        let u = self.uia.as_ref()?;
        if self.minimized {
            return None;
        }
        let pattern = u.text_pattern_skipping_passwords(self.hwnd).ok();
        let walk = u.cached_walk_skipping_passwords(self.hwnd).ok();
        let document_found = pattern.as_ref().is_some_and(|e| e.document_found) || walk.as_ref().is_some_and(|e| e.document_found);
        // What is on screen first; everything UIA has when that is (almost) nothing.
        let mut text = union_of(pattern.as_ref(), walk.as_ref(), true);
        if uia::count_chars(&text) < policy::MAC_MIN_CHARS {
            text = union_of(pattern.as_ref(), walk.as_ref(), false);
        }
        // A browser serving only its chrome to UIA: the page is not in `text`, so leave it
        // to OCR (the capture policy's rule).
        let wrong = policy::should_ocr(&policy::FallbackInput {
            app_id: &self.info.app_id,
            exe: &self.info.exe,
            uia_chars: uia::count_chars(&text),
            document_found,
        });
        (!wrong).then_some(text)
    }

    fn ocr(&self) -> Option<String> {
        if self.minimized {
            return None;
        }
        match ocr::Ocr::new().and_then(|o| o.recognize_window(self.hwnd)) {
            Ok(out) => Some(out.text),
            Err(e) => {
                tracing::warn!(error = %e, "hotkey panel: OCR failed");
                None
            }
        }
    }
}

/// The two strategies' text merged as the capture service merges it: the on-screen text
/// (`visible`) or all of it.
fn union_of(pattern: Option<&uia::Extraction>, walk: Option<&uia::Extraction>, visible: bool) -> String {
    let pick = |e: Option<&uia::Extraction>| -> String {
        e.map(|e| if visible { e.visible_text.clone() } else { e.text.clone() }).unwrap_or_default()
    };
    merge::union_text(&pick(pattern), &pick(walk))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_is_named_by_its_exe() {
        assert_eq!(display_name("outlook.exe", "outlook.exe"), "outlook");
        assert_eq!(display_name("", "MSTeams_8wekyb3d8bbwe!MSTeams"), "MSTeams_8wekyb3d8bbwe!MSTeams");
    }

    struct Excluded;
    impl ForegroundSource for Excluded {
        fn foreground(&self) -> Option<ForegroundInfo> {
            Some(ForegroundInfo { pid: 7, exe: "1password.exe".into(), app_id: "1password.exe".into(), app_name: "1password".into(), title: "Vault".into() })
        }
        fn selection(&self) -> Option<String> {
            panic!("an excluded app's selection was read")
        }
        fn document_text(&self) -> Option<String> {
            panic!("an excluded app's text was read")
        }
        fn ocr(&self) -> Option<String> {
            panic!("an excluded app was OCR'd")
        }
    }

    #[test]
    fn the_capture_exclusions_apply_before_any_text_is_read() {
        assert!(matches!(
            read_window(&Excluded, &Exclusions::default()),
            ReadOutcome::Excluded(ref a, ExcludeKind::PasswordManager) if a == "1password"
        ));
    }
}
