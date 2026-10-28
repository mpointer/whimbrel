//! When the OCR fallback fires — FR-CAP-2, docs/12 §4.1.
//!
//! The macOS shell OCRs a window only when Accessibility returns essentially nothing
//! (`ScreenTextCaptureService.swift`: trimmed AX text shorter than 8 characters). M3.1
//! found the case that rule cannot see on Windows: an Edge instance that serves only its
//! browser chrome to UIA (~1,000 characters of tab titles and toolbar labels) and none
//! of the page. UIA is not *empty*, it is *wrong* (M3.1 report, U6). Such a tree has no
//! Document element; every page-showing Chromium/WebView2/Electron window measured had
//! one. The M3.1 table records raw UIA and OCR numbers per app, so any rule here can be
//! re-evaluated against the measurements.

/// macOS parity threshold (non-whitespace characters here; the Mac counts trimmed
/// characters including inner whitespace — the difference is immaterial at 8).
pub const MAC_MIN_CHARS: usize = 8;

/// The Mac's rule, verbatim: fall back only when UIA produced (almost) no text.
pub fn mac_parity_fires(uia_chars: usize) -> bool {
    uia_chars < MAC_MIN_CHARS
}

/// What the capture loop knows at the moment it decides.
#[derive(Debug, Clone, Copy)]
pub struct FallbackInput<'a> {
    /// §4.3 identity: `MSEdge`, `MSTeams_8wekyb3d8bbwe!MSTeams`, `code.exe`, …
    pub app_id: &'a str,
    /// Lowercased executable basename: `msedge.exe`, `ms-teams.exe`, `winword.exe`, …
    pub exe: &'a str,
    /// Non-whitespace characters UIA returned for the window.
    pub uia_chars: usize,
    /// Whether the window's UIA tree contains a Document element.
    pub document_found: bool,
}

/// Browsers: windows with a tab strip, where a tree with no Document element means UIA
/// is serving the browser chrome and not the page (U6). Only Edge was measured in M3.1;
/// the others are Chromium or Gecko browsers expected to fail the same way.
/// Deliberately excluded:
/// - WebView2 and Electron hosts (new Outlook, Teams, Slack, VS Code). They always had a
///   Document in M3.1.
/// - Windows Terminal and native apps. They legitimately have no Document, and OCR there
///   would cost ~0.6-1 s per capture for nothing.
const BROWSER_EXES: &[&str] = &[
    "msedge.exe",
    "chrome.exe",
    "brave.exe",
    "vivaldi.exe",
    "opera.exe",
    "arc.exe",
    "firefox.exe",
];

pub fn is_browser(exe: &str) -> bool {
    BROWSER_EXES.contains(&exe)
}

/// The Windows shell's OCR-fallback decision for one capture: the Mac rule (UIA gave
/// essentially nothing), plus a browser window whose tree has no Document (UIA gave the
/// wrong thing — U6).
///
/// The stuck-browser case mostly hits the first capture after a session restore. It
/// clears once the user interacts with the window (Gmail went from 957 chars and no
/// Document to 9,903 chars and a Document after one click). So this fires rarely, and
/// when it does the page really is invisible to UIA.
///
/// Known gap: Google Docs *has* a Document but draws its body on a canvas (UIA recall
/// 0.32-0.38 with or without Docs' screen-reader support). Catching it needs the page
/// URL, and `FallbackInput` has none yet — M3.2.
pub fn should_ocr(input: &FallbackInput) -> bool {
    mac_parity_fires(input.uia_chars) || (is_browser(input.exe) && !input.document_found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(exe: &str, uia_chars: usize, document_found: bool) -> FallbackInput<'_> {
        FallbackInput { app_id: exe, exe, uia_chars, document_found }
    }

    #[test]
    fn empty_uia_fires_anywhere() {
        assert!(should_ocr(&input("someapp.exe", 3, false)));
    }

    #[test]
    fn browser_showing_only_chrome_fires() {
        // Edge instance B / stuck Gmail: ~1,000 chars of tab titles, no Document.
        assert!(should_ocr(&input("msedge.exe", 957, false)));
    }

    #[test]
    fn browser_with_a_document_does_not_fire() {
        assert!(!should_ocr(&input("msedge.exe", 9_903, true)));
    }

    #[test]
    fn non_browsers_without_a_document_do_not_fire() {
        assert!(!should_ocr(&input("windowsterminal.exe", 3_183, false)));
        assert!(!should_ocr(&input("slack.exe", 1_200, false)));
    }
}
