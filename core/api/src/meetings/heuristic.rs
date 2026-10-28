//! FR-MTG-2: turning per-poll "a meeting app looks live" observations into one prompt.
//! Ported from apps/macos `MeetingHeuristicDetector`'s debounce: two consecutive matches
//! for the same app, one prompt per app until nothing matches, and a grace period before
//! a tracked browser meeting counts as gone. What counts as a match is the shell's call
//! (the Mac: a known app running with the default mic and output in use; Windows: a known
//! process with active render and capture audio sessions).

use crate::connectors::calendar::{MeetingProvider, MeetingSource, QualifyingMeeting};

/// Consecutive positive polls before prompting: damps one-poll blips while staying far
/// inside FR-MTG-2's 30 s at a 5 s poll.
pub const REQUIRED_CONSECUTIVE_MATCHES: u32 = 2;
/// How long a tracked browser meeting may be off its meeting URL before it's forgotten.
pub const BROWSER_GONE_GRACE_MS: i64 = 15_000;
/// A detected meeting's provisional length; the end rules decide when it really ends.
pub const DETECTED_MEETING_LENGTH_MS: i64 = 60 * 60_000;

/// One poll's observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// A meeting app (or a browser on a meeting URL) with live call audio.
    /// `app_id` is the shell's app identity: a bundle id on macOS, an exe name on Windows.
    Match { app_id: String, title: String, via_browser: bool },
    /// A meeting app is running, but its call audio isn't live.
    MatchWithoutAudio { app_id: String },
    /// Nothing matched this poll. `tracked_browser_on_meeting` re-checks the browser
    /// window a previous browser match is tracking, if the shell still has it.
    NoMatch { tracked_browser_on_meeting: Option<bool> },
}

/// The debounce state the shell keeps for its detector's lifetime.
#[derive(Debug, Clone, Default)]
pub struct Debounce {
    consecutive_matches: u32,
    last_app_id: Option<String>,
    prompted_app_ids: std::collections::HashSet<String>,
    /// The id of the prompt a browser match produced, while its window is tracked.
    tracked_browser_meeting_id: Option<String>,
    browser_gone_started_unix_ms: Option<i64>,
}

impl Debounce {
    /// Feeds one poll. Returns a meeting to prompt for at most once per app until a poll
    /// with no match (after any browser grace) resets the detector.
    pub fn observe(&mut self, now_unix_ms: i64, observation: Observation) -> Option<QualifyingMeeting> {
        match observation {
            Observation::MatchWithoutAudio { .. } => {
                self.consecutive_matches = 0;
                None
            }
            Observation::Match { app_id, title, via_browser } => {
                if via_browser {
                    self.browser_gone_started_unix_ms = None;
                }
                if self.last_app_id.as_deref() != Some(app_id.as_str()) {
                    self.consecutive_matches = 0;
                    self.last_app_id = Some(app_id.clone());
                }
                self.consecutive_matches += 1;
                if self.consecutive_matches < REQUIRED_CONSECUTIVE_MATCHES || self.prompted_app_ids.contains(&app_id) {
                    return None;
                }
                self.prompted_app_ids.insert(app_id.clone());
                let id = format!("detected-{app_id}-{}", now_unix_ms.div_euclid(1_000));
                if via_browser {
                    self.tracked_browser_meeting_id = Some(id.clone());
                }
                Some(QualifyingMeeting {
                    id,
                    title,
                    start_unix_ms: now_unix_ms,
                    end_unix_ms: now_unix_ms + DETECTED_MEETING_LENGTH_MS,
                    attendees: Vec::new(),
                    conference_link: None,
                    source: MeetingSource::Detected,
                    app_bundle_id: Some(app_id),
                    organizer_email: None,
                    organizer_name: None,
                    self_is_organizer: false,
                    self_response: None,
                })
            }
            Observation::NoMatch { tracked_browser_on_meeting } => {
                if self.tracked_browser_meeting_id.is_some() {
                    match tracked_browser_on_meeting {
                        Some(true) => {
                            self.browser_gone_started_unix_ms = None;
                            return None;
                        }
                        _ => {
                            let since = *self.browser_gone_started_unix_ms.get_or_insert(now_unix_ms);
                            if now_unix_ms - since < BROWSER_GONE_GRACE_MS {
                                return None;
                            }
                        }
                    }
                }
                *self = Self::default();
                None
            }
        }
    }

    /// The prompt id a browser match produced, while that browser window is still tracked
    /// (the shell passes the window on to the end detector for this meeting only).
    pub fn tracked_browser_meeting_id(&self) -> Option<&str> {
        self.tracked_browser_meeting_id.as_deref()
    }
}

/// Which meeting service a browser address-bar value is on, by exact host. Accepts a
/// full URL or the scheme-less form browsers show (`meet.google.com/abc-defg-hij`).
/// Host matching, not substring: a URL that only mentions a meeting host elsewhere, or a
/// look-alike subdomain, is not a match.
pub fn meeting_url_provider(address: &str) -> Option<MeetingProvider> {
    let lowered = address.trim().to_ascii_lowercase();
    let rest = lowered.as_str();
    let rest = rest
        .strip_prefix("https://")
        .or_else(|| rest.strip_prefix("http://"))
        .unwrap_or(rest);
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains(char::is_whitespace) || authority.contains('@') {
        return None;
    }
    let host = authority.split(':').next().unwrap_or("");
    let path = &rest[end..];
    match host {
        "meet.google.com" => Some(MeetingProvider::GoogleMeet),
        "teams.microsoft.com" | "teams.live.com" => Some(MeetingProvider::Teams),
        h if (h == "zoom.us" || h.ends_with(".zoom.us")) && path.starts_with("/wc/") => Some(MeetingProvider::Zoom),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(app: &str) -> Observation {
        Observation::Match { app_id: app.into(), title: "Zoom".into(), via_browser: false }
    }

    fn browser_hit() -> Observation {
        Observation::Match { app_id: "msedge.exe".into(), title: "Google Meet".into(), via_browser: true }
    }

    fn nothing(tracked: Option<bool>) -> Observation {
        Observation::NoMatch { tracked_browser_on_meeting: tracked }
    }

    #[test]
    fn prompts_on_the_second_consecutive_match() {
        let mut d = Debounce::default();
        assert!(d.observe(1_000, hit("zoom.exe")).is_none());
        let m = d.observe(6_000, hit("zoom.exe")).unwrap();
        assert_eq!(m.id, "detected-zoom.exe-6");
        assert_eq!(m.source, MeetingSource::Detected);
        assert_eq!(m.end_unix_ms, 6_000 + DETECTED_MEETING_LENGTH_MS);
        assert_eq!(m.app_bundle_id.as_deref(), Some("zoom.exe"));
    }

    #[test]
    fn prompts_once_per_app_while_it_keeps_matching() {
        let mut d = Debounce::default();
        d.observe(0, hit("zoom.exe"));
        assert!(d.observe(5_000, hit("zoom.exe")).is_some());
        assert!(d.observe(10_000, hit("zoom.exe")).is_none());
    }

    #[test]
    fn a_different_app_restarts_the_count() {
        let mut d = Debounce::default();
        d.observe(0, hit("zoom.exe"));
        assert!(d.observe(5_000, hit("ms-teams.exe")).is_none());
        assert!(d.observe(10_000, hit("ms-teams.exe")).is_some());
    }

    #[test]
    fn audio_going_quiet_restarts_the_count() {
        let mut d = Debounce::default();
        d.observe(0, hit("zoom.exe"));
        d.observe(5_000, Observation::MatchWithoutAudio { app_id: "zoom.exe".into() });
        assert!(d.observe(10_000, hit("zoom.exe")).is_none());
        assert!(d.observe(15_000, hit("zoom.exe")).is_some());
    }

    #[test]
    fn a_poll_with_no_match_lets_the_same_app_prompt_again() {
        let mut d = Debounce::default();
        d.observe(0, hit("zoom.exe"));
        d.observe(5_000, hit("zoom.exe"));
        d.observe(10_000, nothing(None));
        d.observe(15_000, hit("zoom.exe"));
        assert!(d.observe(20_000, hit("zoom.exe")).is_some());
    }

    #[test]
    fn a_tracked_browser_meeting_survives_a_short_tab_switch() {
        let mut d = Debounce::default();
        d.observe(0, browser_hit());
        let id = d.observe(5_000, browser_hit()).unwrap().id;
        assert_eq!(d.tracked_browser_meeting_id(), Some(id.as_str()));
        d.observe(10_000, nothing(Some(false)));
        d.observe(20_000, nothing(Some(false)));
        assert_eq!(d.tracked_browser_meeting_id(), Some(id.as_str()), "10 s is inside the grace");
        assert!(d.observe(25_000, browser_hit()).is_none(), "no second prompt");
    }

    #[test]
    fn a_tracked_browser_meeting_is_forgotten_after_the_grace() {
        let mut d = Debounce::default();
        d.observe(0, browser_hit());
        d.observe(5_000, browser_hit());
        d.observe(10_000, nothing(Some(false)));
        d.observe(25_000, nothing(Some(false)));
        assert_eq!(d.tracked_browser_meeting_id(), None);
    }

    #[test]
    fn a_tracked_browser_still_on_the_meeting_keeps_it() {
        let mut d = Debounce::default();
        d.observe(0, browser_hit());
        d.observe(5_000, browser_hit());
        for t in (10_000..=60_000).step_by(5_000) {
            d.observe(t, nothing(Some(true)));
        }
        assert!(d.tracked_browser_meeting_id().is_some());
    }

    // ---- meeting_url_provider (the Mac's isGoogleMeetURL tests, plus Teams and Zoom web) ----

    #[test]
    fn matches_a_real_meet_url() {
        assert_eq!(meeting_url_provider("https://meet.google.com/abc-defg-hij"), Some(MeetingProvider::GoogleMeet));
    }

    #[test]
    fn matches_with_query_params_and_fragment() {
        assert_eq!(
            meeting_url_provider("https://meet.google.com/abc-defg-hij?authuser=0#fragment"),
            Some(MeetingProvider::GoogleMeet)
        );
    }

    #[test]
    fn matches_the_scheme_less_address_bar_form() {
        assert_eq!(meeting_url_provider("meet.google.com/abc-defg-hij"), Some(MeetingProvider::GoogleMeet));
    }

    #[test]
    fn rejects_empty_and_plain_text() {
        assert_eq!(meeting_url_provider(""), None);
        assert_eq!(meeting_url_provider("not a url at all"), None);
    }

    #[test]
    fn rejects_an_unrelated_domain_that_mentions_the_host_in_its_query() {
        assert_eq!(meeting_url_provider("https://evil.example.com/?redirect=meet.google.com"), None);
    }

    #[test]
    fn rejects_a_subdomain_spoof_and_userinfo_tricks() {
        assert_eq!(meeting_url_provider("https://meet.google.com.evil.example.com/abc"), None);
        assert_eq!(meeting_url_provider("https://meet.google.com@evil.example.com/abc"), None);
    }

    #[test]
    fn rejects_other_google_domains() {
        assert_eq!(meeting_url_provider("https://docs.google.com/document/d/abc"), None);
    }

    #[test]
    fn matches_teams_web_hosts() {
        assert_eq!(meeting_url_provider("https://teams.microsoft.com/v2/"), Some(MeetingProvider::Teams));
        assert_eq!(meeting_url_provider("teams.live.com/meet/123"), Some(MeetingProvider::Teams));
    }

    #[test]
    fn matches_zoom_web_client_only() {
        assert_eq!(meeting_url_provider("https://acme.zoom.us/wc/123456/join"), Some(MeetingProvider::Zoom));
        assert_eq!(meeting_url_provider("https://zoom.us/wc/123/start"), Some(MeetingProvider::Zoom));
        assert_eq!(meeting_url_provider("https://zoom.us/pricing"), None, "Zoom's site, not a call");
        assert_eq!(meeting_url_provider("https://notzoom.us/wc/1"), None);
    }

    #[test]
    fn host_match_ignores_case_and_port() {
        assert_eq!(meeting_url_provider("HTTPS://Meet.Google.com:443/abc"), Some(MeetingProvider::GoogleMeet));
    }
}
