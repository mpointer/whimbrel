//! Google Calendar polling and calendar triggers (M3.3 Task 7).
//!
//! Every five minutes, per connected account: ask core what to call
//! (`calendar::sync_request`), run that one tool call through a freshly spawned Node
//! connector, and hand the result back to core (`calendar::apply`). Core owns the sync
//! token, the decoding and the merge; the shell owns the timer, the token refresh and the
//! process. ADR 0001 holds: the network call runs off the store actor, between two short
//! store calls.
//!
//! The same loop raises the lead-time prompt, because it already has today's meetings.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::connectors::calendar::{self, QualifyingMeeting};
use api::meetings::trigger;
use mcp_host::McpHost;
use tauri::AppHandle;

use super::accounts;
use super::node::{self, GOOGLE_CALENDAR};
use crate::error::CommandError;
use crate::meetings::service::Pending;
use crate::secrets::SecretStore;
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;
use crate::syncing::config::SyncConfig;

/// The Mac's 5-minute calendar poll.
pub const POLL: Duration = Duration::from_secs(5 * 60);
/// The Mac's 15-second check for a meeting whose lead window has opened.
pub const TRIGGER_CHECK: Duration = Duration::from_secs(15);
/// What the connector's sync tool is called.
pub const SYNC_TOOL: &str = "sync_events";

/// A meeting qualifies for a prompt when it has a conference link or someone else is
/// invited — the Mac's `qualify`, minus its EventKit calendar opt-in, which Windows has no
/// equivalent for (there is no local calendar to opt in to).
pub fn qualifies(meeting: &QualifyingMeeting) -> bool {
    meeting.conference_link.is_some() || meeting.attendees.len() > 1
}

/// One account's poll, as a pure sequence the caller drives: core builds the request, the
/// caller performs it, core applies the answer. `needs_full_resync` gets one retry, which
/// is what core asks for when Google rejects a stale sync token.
pub struct Poller {
    pub store: StoreHandle,
    pub secrets: Arc<dyn SecretStore>,
    pub config: SyncConfig,
    pub host: Arc<McpHost>,
    pub connectors_root: std::path::PathBuf,
    /// This device's id, for the calendar rows the Library shows.
    pub device_id: String,
}

impl Poller {
    /// Runs the connector's `sync_events` for `email` and applies the result to the
    /// meetings that account already had.
    ///
    /// `existing` matters: after the first sync, Google's connector returns only what
    /// changed since the stored sync token, and core merges that delta into what it is
    /// given. Passing an empty list would throw the calendar away on every poll and leave
    /// the app with whatever the last delta happened to contain.
    pub fn poll_account(&self, email: &str, existing: Vec<QualifyingMeeting>) -> Result<Vec<QualifyingMeeting>, CommandError> {
        self.poll_account_with(email, existing, 0)
    }

    /// Drops the stored sync token so the next request asks for everything. The app keeps
    /// its meetings in memory, so after a launch a delta has nothing to merge into and the
    /// day would look empty until an event happened to change.
    pub fn forget_sync_token(&self, email: &str) -> Result<(), CommandError> {
        let account = email.to_string();
        self.store
            .call_blocking(move |s| s.clear_calendar_sync_token(&account).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)
    }

    fn poll_account_with(
        &self,
        email: &str,
        existing: Vec<QualifyingMeeting>,
        attempt: u8,
    ) -> Result<Vec<QualifyingMeeting>, CommandError> {
        let account = email.to_string();
        let request = self
            .store
            .call_blocking(move |s| calendar::sync_request(s, &account).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?;

        let token = accounts::fresh_access_token(&*self.secrets, &self.config, email)?;
        let result = self.call_tool(&request.tool, &request.args_json, &token)?;

        let (account, now) = (email.to_string(), chrono::Utc::now().timestamp_millis());
        let outcome = self
            .store
            .call_blocking(move |s| calendar::apply(s, &account, &result, &existing, now).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?;

        if outcome.needs_full_resync && attempt == 0 {
            tracing::info!(email, "the calendar sync token was rejected; asking for a full resync");
            return self.poll_account_with(email, Vec::new(), 1);
        }
        tracing::info!(email, meetings = outcome.meetings.len(), cancelled = outcome.cancelled_count, "calendar sync applied");
        self.store_events(&outcome.meetings);
        Ok(outcome.meetings)
    }

    /// Writes the meetings into the store as calendar rows, so they show up in the
    /// Library and in Ask's context like the Mac's do. Upserted on
    /// `(source, external_id)`: re-syncing the same event updates its row.
    fn store_events(&self, meetings: &[QualifyingMeeting]) {
        let items: Vec<memory::NewCalendarEventItem> = meetings
            .iter()
            .filter_map(|m| calendar::calendar_event_item(m, &self.device_id))
            .map(Into::into)
            .collect();
        if items.is_empty() {
            return;
        }
        let count = items.len();
        let stored = self.store.call_blocking(move |s| s.upsert_calendar_event_items(&items).map_err(|e| e.to_string()));
        match stored {
            Ok(Ok(())) => tracing::debug!(count, "stored calendar events"),
            Ok(Err(e)) | Err(CommandError::Store { message: e }) => tracing::warn!(error = %e, "could not store calendar events"),
            Err(e) => tracing::warn!(error = %e, "could not store calendar events"),
        }
    }

    /// One connector call, through the shared spawn-call-disconnect in `node`.
    fn call_tool(&self, tool: &str, args_json: &str, token: &str) -> Result<String, CommandError> {
        node::call_connector(&self.host, &self.connectors_root, GOOGLE_CALENDAR, "calendar", tool, args_json, token)
    }
}

/// The calendar half of the meetings service: polls the connected accounts and prompts
/// when a meeting is about to start.
pub struct CalendarService {
    poller: Poller,
    settings: Arc<Mutex<SettingsFile>>,
    pending: Arc<Pending>,
    app: AppHandle,
    /// The last good meetings per account, so a failed poll doesn't empty the list and
    /// the next delta has something to merge into (the Mac keeps them the same way).
    known: Mutex<HashMap<String, Vec<QualifyingMeeting>>>,
    /// When each account last synced, and why it didn't, for Settings.
    status: Mutex<SyncStatus>,
}

/// What Settings shows about the last round.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStatus {
    pub last_synced_unix_ms: Option<i64>,
    pub error: Option<String>,
}

pub fn start(
    app: AppHandle,
    poller: Poller,
    settings: Arc<Mutex<SettingsFile>>,
    pending: Arc<Pending>,
) -> Arc<CalendarService> {
    let service = Arc::new(CalendarService {
        poller,
        settings,
        pending,
        app,
        known: Mutex::new(HashMap::new()),
        status: Mutex::new(SyncStatus::default()),
    });
    let loops = service.clone();
    let spawned = std::thread::Builder::new().name("whimbrel-calendar".into()).spawn(move || loops.run());
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start calendar polling");
    }
    service
}

impl CalendarService {
    /// Every connected account's meetings, as of their last good sync.
    pub fn meetings(&self) -> Vec<QualifyingMeeting> {
        self.known.lock().unwrap().values().flatten().cloned().collect()
    }

    /// The meetings a prompt could still fire for: qualifying, and not yet over.
    pub fn upcoming(&self, now_unix_ms: i64) -> Vec<QualifyingMeeting> {
        let mut upcoming: Vec<QualifyingMeeting> =
            self.meetings().into_iter().filter(|m| qualifies(m) && m.end_unix_ms > now_unix_ms).collect();
        upcoming.sort_by_key(|m| m.start_unix_ms);
        upcoming
    }

    pub fn status(&self) -> SyncStatus {
        self.status.lock().unwrap().clone()
    }

    /// Polls now, whatever the timer says (after connecting an account, say).
    pub fn poll_now(&self) {
        self.poll(false)
    }

    /// The first poll after launch asks for every event, not the changes since last time.
    fn poll(&self, full: bool) {
        let emails = match self.accounts() {
            Ok(emails) => emails,
            Err(e) => {
                tracing::warn!(error = %e, "could not list calendar accounts");
                return;
            }
        };
        let mut failure = None;
        for email in emails {
            if full {
                if let Err(e) = self.poller.forget_sync_token(&email) {
                    tracing::warn!(email, error = %e, "could not ask for a full calendar sync");
                }
            }
            let existing = self.known.lock().unwrap().get(&email).cloned().unwrap_or_default();
            match self.poller.poll_account(&email, existing) {
                Ok(meetings) => {
                    tracing::info!(email, meetings = meetings.len(), "calendar synced");
                    self.known.lock().unwrap().insert(email, meetings);
                }
                // A failed account keeps whatever it had: a flat network is not an empty day.
                Err(e) => {
                    tracing::warn!(email, error = %e, "calendar sync failed");
                    failure.get_or_insert_with(|| e.to_string());
                }
            }
        }
        let mut status = self.status.lock().unwrap();
        status.error = failure;
        if status.error.is_none() {
            status.last_synced_unix_ms = Some(chrono::Utc::now().timestamp_millis());
        }
    }

    /// Forgets an account's meetings when it is disconnected.
    pub fn forget(&self, email: &str) {
        self.known.lock().unwrap().remove(email);
    }

    fn accounts(&self) -> Result<Vec<String>, CommandError> {
        self.poller
            .store
            .call_blocking(|s| api::connectors::accounts::list(s, accounts::CALENDAR).map_err(|e| e.to_string()))?
            .map(|a| a.emails)
            .map_err(CommandError::store)
    }

    fn run(self: Arc<Self>) {
        let mut prompted = self.settings.lock().unwrap().prompted_meetings(chrono::Utc::now().timestamp_millis());
        let mut acted_on: std::collections::HashSet<String> = prompted.keys().cloned().collect();
        let mut last_poll = std::time::Instant::now() - POLL;
        let mut first = true;
        loop {
            std::thread::sleep(TRIGGER_CHECK);
            if last_poll.elapsed() >= POLL {
                last_poll = std::time::Instant::now();
                self.poll(first);
                first = false;
            }
            self.check_triggers(&mut acted_on, &mut prompted);
        }
    }

    /// Raises the alert for a meeting whose lead window has opened.
    fn check_triggers(
        &self,
        acted_on: &mut std::collections::HashSet<String>,
        prompted: &mut std::collections::HashMap<String, i64>,
    ) {
        let lead_minutes = self.settings.lock().unwrap().lead_minutes();
        let now = chrono::Utc::now().timestamp_millis();
        let meetings = self.upcoming(now);
        let due = trigger::due_prompts(&meetings, acted_on, now, lead_minutes);
        if !due.is_empty() {
            // Persisted so a relaunch during the meeting doesn't prompt (and, with
            // auto-start, record) the same event a second time.
            prompted.extend(due.iter().map(|m| (m.id.clone(), m.end_unix_ms)));
            prompted.retain(|_, end| *end > now);
            let mut settings = self.settings.lock().unwrap();
            settings.set_prompted_meetings(prompted);
            if let Err(e) = settings.save() {
                tracing::warn!(error = %e, "could not save the prompted-meetings list");
            }
        }
        let Some(meeting) = due.into_iter().next_back() else { return };
        if self.pending.get().is_some() {
            return;
        }
        tracing::info!(id = %meeting.id, title = %meeting.title, "a calendar meeting is about to start");
        self.pending.set_calendar(meeting, now);
        crate::alert::show(&self.app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::connectors::calendar::{ConferenceLink, MeetingProvider, MeetingSource};

    fn meeting(id: &str, attendees: &[&str], link: bool) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.into(),
            title: "Standup".into(),
            start_unix_ms: 1_000,
            end_unix_ms: 2_000,
            attendees: attendees.iter().map(|a| a.to_string()).collect(),
            conference_link: link.then(|| ConferenceLink { provider: MeetingProvider::GoogleMeet, url: "https://meet.google.com/x".into() }),
            source: MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            ..Default::default()
        }
    }

    #[test]
    fn a_meeting_qualifies_with_a_link_or_another_person() {
        assert!(qualifies(&meeting("a", &["me@x"], true)), "a conference link is enough");
        assert!(qualifies(&meeting("b", &["me@x", "you@x"], false)), "so is someone else");
        assert!(!qualifies(&meeting("c", &["me@x"], false)), "a solo block with no link is not a meeting");
        assert!(!qualifies(&meeting("d", &[], false)));
    }
}
