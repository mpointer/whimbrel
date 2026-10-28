//! Windows never captured — docs/12 §4 exclusions row (FR-CAP-4).

/// The secure desktop, credential prompts and UAC: never captured, not configurable.
const ALWAYS: &[&str] = &["lockapp.exe", "logonui.exe", "credentialuibroker.exe", "consent.exe"];
/// Password managers: excluded unless the user removes them (in a later task).
const DEFAULT: &[&str] = &["1password.exe", "bitwarden.exe", "dashlane.exe", "enpass.exe"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcludeReason {
    Whimbrel,
    System,
    PasswordManager,
    PrivateWindow,
    User,
}

/// Browser private windows, by title. Edge marks InPrivate windows in the title
/// ("… - [InPrivate] - Microsoft Edge"), and Firefox appends "Private Browsing". Chrome
/// marks only its new-tab page ("New Incognito tab"), so a Chrome Incognito window on
/// an ordinary page is NOT detected; a known gap, noted in the plan.
pub fn is_private_window(title: &str) -> bool {
    title.contains("[InPrivate]")
        || title.ends_with("InPrivate")
        || title.contains("Incognito")
        || title.ends_with("Private Browsing")
}

#[derive(Debug, Clone, Default)]
pub struct Exclusions {
    /// User-excluded §4.3 identities or exe names, compared case-insensitively
    /// (settings `captureExclusions.excludedBundleIds`).
    pub user: Vec<String>,
}

impl Exclusions {
    pub fn check(&self, exe: &str, app_id: &str, title: &str, pid: u32, own_pid: u32) -> Option<ExcludeReason> {
        if pid == own_pid {
            return Some(ExcludeReason::Whimbrel);
        }
        if ALWAYS.contains(&exe) {
            return Some(ExcludeReason::System);
        }
        if DEFAULT.contains(&exe) || exe.starts_with("keepass") {
            return Some(ExcludeReason::PasswordManager);
        }
        if is_private_window(title) {
            return Some(ExcludeReason::PrivateWindow);
        }
        if self.user.iter().any(|u| u.eq_ignore_ascii_case(app_id) || u.eq_ignore_ascii_case(exe)) {
            return Some(ExcludeReason::User);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u32 = 1000;

    fn check(exe: &str, app_id: &str, title: &str) -> Option<ExcludeReason> {
        Exclusions { user: vec!["Spotify.exe".into(), "MSTeams_8wekyb3d8bbwe!MSTeams".into()] }.check(exe, app_id, title, 42, ME)
    }

    #[test]
    fn whimbrel_never_captures_itself() {
        assert_eq!(Exclusions::default().check("whimbrel-windows.exe", "x", "Whimbrel", ME, ME), Some(ExcludeReason::Whimbrel));
    }

    #[test]
    fn system_and_password_manager_windows_are_excluded() {
        assert_eq!(check("lockapp.exe", "x", ""), Some(ExcludeReason::System));
        assert_eq!(check("consent.exe", "x", "User Account Control"), Some(ExcludeReason::System));
        assert_eq!(check("1password.exe", "x", "1Password"), Some(ExcludeReason::PasswordManager));
        assert_eq!(check("keepassxc.exe", "x", "KeePassXC"), Some(ExcludeReason::PasswordManager));
    }

    #[test]
    fn private_browser_windows_are_excluded_by_title() {
        assert_eq!(check("msedge.exe", "MSEdge", "New InPrivate tab - [InPrivate] - Microsoft Edge"), Some(ExcludeReason::PrivateWindow));
        assert_eq!(check("firefox.exe", "firefox.exe", "Example — Mozilla Firefox Private Browsing"), Some(ExcludeReason::PrivateWindow));
        assert_eq!(check("chrome.exe", "Chrome", "New Incognito tab - Google Chrome"), Some(ExcludeReason::PrivateWindow));
        assert_eq!(check("msedge.exe", "MSEdge", "Inbox - Gmail - Microsoft Edge"), None);
    }

    #[test]
    fn user_exclusions_match_identity_or_exe_case_insensitively() {
        assert_eq!(check("spotify.exe", "spotify.exe", "Spotify"), Some(ExcludeReason::User));
        assert_eq!(check("ms-teams.exe", "MSTeams_8wekyb3d8bbwe!MSTeams", "Chat | Microsoft Teams"), Some(ExcludeReason::User));
        assert_eq!(check("winword.exe", "winword.exe", "Report.docx - Word"), None);
    }
}
