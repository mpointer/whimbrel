//! Normalizing the §4.3 app identity before it is stored as `app_bundle_id`.
//!
//! M3.1 U4: Edge stamps a per-profile AUMID. The default profile's window reports
//! `MSEdge`, a second profile `MSEdge.<profile-id>.Default`. Stored raw, one browser
//! would become one app per profile in exclusions and per-app views. For browsers the
//! AUMID is cut at its first `.`; the profile is not kept in `app_bundle_id`.
//!
//! Only browsers: packaged AUMIDs legitimately contain dots
//! (`Microsoft.WindowsTerminal_8wekyb3d8bbwe!App`).

use crate::capture::policy;

pub fn normalize(app_id: &str, exe: &str) -> String {
    // An identity that fell back to the exe name (`msedge.exe`) has a dot that is not
    // a profile separator.
    if policy::is_browser(exe) && !app_id.contains('!') && !app_id.eq_ignore_ascii_case(exe) {
        if let Some((base, _profile)) = app_id.split_once('.') {
            if !base.is_empty() {
                return base.to_string();
            }
        }
    }
    app_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_profiles_collapse_to_one_identity() {
        assert_eq!(normalize("MSEdge", "msedge.exe"), "MSEdge");
        assert_eq!(normalize("MSEdge.abc123.Default", "msedge.exe"), "MSEdge");
        assert_eq!(normalize("Chrome.XYZ.UserData.Profile1", "chrome.exe"), "Chrome");
    }

    #[test]
    fn non_browser_identities_are_untouched() {
        assert_eq!(normalize("MSTeams_8wekyb3d8bbwe!MSTeams", "ms-teams.exe"), "MSTeams_8wekyb3d8bbwe!MSTeams");
        assert_eq!(
            normalize("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App", "windowsterminal.exe"),
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"
        );
        assert_eq!(normalize("code.exe", "code.exe"), "code.exe");
    }

    #[test]
    fn an_exe_fallback_identity_for_a_browser_is_kept() {
        // No AUMID: the identity is the exe name itself, whose dot is not a profile.
        assert_eq!(normalize("msedge.exe", "msedge.exe"), "msedge.exe");
        assert_eq!(normalize("firefox.exe", "firefox.exe"), "firefox.exe");
    }
}
