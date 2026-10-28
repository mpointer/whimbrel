//! The routine scheduler (M3.4 Task 1). The engine is `api::routine_runs::tick`: it seeds the
//! built-in routines, runs whichever are due, and parks anything that writes as a pending
//! approval. This module is the shell around it: a tick every minute on the slow lane, the inputs
//! the tick needs, and what the Automations tab may offer.
//!
//! Plan: docs/superpowers/plans/2026-10-07-m3.4-automations-windows.md.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::connectors::calendar::QualifyingMeeting;
use api::routine_runs::RunGate;
use chrono::{DateTime, Local, TimeZone};
use memory::RoutineTemplate;

use crate::connectors::calendar::CalendarService;
use crate::connectors::write::{self, Connectors};
use crate::secrets::SecretStore;
use crate::settings::{ApiKeys, PromptSurface, SettingsFile};
use crate::store_actor::StoreHandle;
use crate::syncing::controller::DeviceView;

/// How often the scheduler looks. Core decides what is due; the Mac ticks at the same pace.
const TICK: Duration = Duration::from_secs(60);
/// The first look comes sooner, so the routines exist (a tick seeds them) shortly after launch,
/// but not before the calendar has had a moment to sync the day's events.
const FIRST_TICK: Duration = Duration::from_secs(20);

// --- what a routine may do on this PC ----------------------------------------------------------

/// What this PC can do right now, so the tab never offers a switch that would do nothing. Some of
/// it is what this build has (the approvals inbox), some is what the user has connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// The approvals inbox exists, so a routine that writes can be approved (Task 2).
    pub approvals: bool,
    /// A Google Calendar account is connected, so an event can be added (Task 2).
    pub calendar: bool,
    /// A Gmail account is connected, so an email can be sent (Task 3).
    pub gmail_send: bool,
    /// Gmail mail is kept in memory, so there are threads to rank (Task 4).
    pub gmail_mail: bool,
}

impl Capabilities {
    /// Nothing: the base the tests build on.
    pub const NONE: Capabilities = Capabilities { approvals: false, calendar: false, gmail_send: false, gmail_mail: false };
}

/// What this PC can do right now: the approvals inbox is built in; Calendar and sending depend on
/// whether an account is connected. Reading Gmail arrives with Task 4.
pub fn capabilities(store: &memory::Store) -> Capabilities {
    let connected = |connector: &str| api::connectors::accounts::list(store, connector).map(|a| !a.emails.is_empty()).unwrap_or(false);
    Capabilities { approvals: true, calendar: connected("calendar"), gmail_send: connected("gmail"), gmail_mail: false }
}

/// The two things the email routines need besides a model: whether Gmail is connected, and whom
/// the email is for. The recipient is the user's own address, as on the Mac: the default Calendar
/// account's. A Gmail-only user has no Calendar, so their Gmail account's address stands in; that
/// is the one place this differs from the Mac, which would send nothing.
pub fn tick_inputs(store: &memory::Store) -> (bool, Option<String>) {
    let accounts = |connector: &str| api::connectors::accounts::list(store, connector).ok();
    let address = |found: Option<api::connectors::accounts::Accounts>| {
        found.and_then(|a| a.default_email.or_else(|| a.emails.first().cloned())).filter(|email| email.contains('@'))
    };
    let gmail = accounts("gmail");
    let connected = gmail.as_ref().is_some_and(|a| !a.emails.is_empty());
    let recipient = address(accounts("calendar")).or_else(|| address(gmail));
    (connected, recipient)
}

/// Whether a routine ends in something that needs approving: an event added, an email sent.
pub fn writes(template: &RoutineTemplate) -> bool {
    matches!(template, RoutineTemplate::FocusTime | RoutineTemplate::EodRecap | RoutineTemplate::WeeklyDigest)
}

/// Why `template` cannot run here yet, in words the tab can show, or `None` when it can.
pub fn unavailable(template: &RoutineTemplate, can: Capabilities) -> Option<&'static str> {
    match template {
        RoutineTemplate::DailyBrief => None,
        // Event-driven (`prep_tick`), and with no calendar there is no meeting to prepare for.
        RoutineTemplate::MeetingPrep if !can.calendar => Some("Needs a Google Calendar account connected (Settings)."),
        RoutineTemplate::FocusTime if !can.approvals => Some("Needs the approvals inbox, which Windows does not have yet."),
        RoutineTemplate::FocusTime if !can.calendar => Some("Needs a Google Calendar account connected (Settings)."),
        RoutineTemplate::EodRecap | RoutineTemplate::WeeklyDigest if !can.approvals => {
            Some("Needs the approvals inbox and Gmail, which Windows does not have yet.")
        }
        RoutineTemplate::EodRecap | RoutineTemplate::WeeklyDigest if !can.gmail_send => {
            Some("Needs Gmail connected (Settings).")
        }
        RoutineTemplate::ReplyPriorities if !can.gmail_mail => Some("Needs your Gmail mail, which Windows does not read yet."),
        _ => None,
    }
}

/// `HH:MM` on a 24-hour clock, or `None`. Returned in the canonical two-digit form core stores.
pub fn valid_schedule(text: &str) -> Option<String> {
    let text = text.trim();
    let (h, m) = text.split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

// --- the tick's inputs ---------------------------------------------------------------------------

/// The events the Daily Brief is told about: those on today's calendar that are not over yet,
/// soonest first. "Today" is the local day of `now`.
pub fn todays_events(meetings: &[QualifyingMeeting], now: DateTime<Local>) -> Vec<routines::CalendarEventSummary> {
    let day_end = now
        .date_naive()
        .succ_opt()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|d| Local.from_local_datetime(&d).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or(i64::MAX);
    let now_ms = now.timestamp_millis();
    let mut today: Vec<&QualifyingMeeting> = meetings.iter().filter(|m| m.end_unix_ms > now_ms && m.start_unix_ms < day_end).collect();
    today.sort_by_key(|m| m.start_unix_ms);
    today.into_iter().map(|m| routines::CalendarEventSummary { title: m.title.clone(), start_unix_ms: m.start_unix_ms }).collect()
}

// --- which device runs routines ------------------------------------------------------------------------

/// The Mac's fallback id for rows from before devices had real identities; never a real device.
const LEGACY_SPIKE_DEVICE: &str = "spike-device";

/// What this device may run, with the Mac's rule (`RoutineHost.gate`). `Full` unless the user chose a
/// **different** device that is known to be approved, in which case this one is a `Failover`: it runs
/// only the read-only routines, and only once they are three hours overdue. A choice that is blank, is
/// this device, names a device that is unknown or revoked, or arrives while the approved list is not
/// known yet, all mean `Full`: a stale setting can never silently stop routines.
pub fn gate(primary: &str, own: Option<&str>, approved: &HashSet<String>) -> RunGate {
    let primary = primary.trim();
    let own = own.unwrap_or("");
    if primary.is_empty() || own.is_empty() || primary == own || own == LEGACY_SPIKE_DEVICE {
        return RunGate::Full;
    }
    if approved.contains(primary) {
        RunGate::Failover
    } else {
        RunGate::Full
    }
}

/// The public keys of the approved devices: what a choice must name to count.
pub fn approved_ids(devices: &[DeviceView]) -> HashSet<String> {
    devices.iter().filter(|d| d.status == "approved").map(|d| d.public_key.clone()).collect()
}

// --- the loop --------------------------------------------------------------------------------------

/// Starts the scheduler on the slow lane (a due routine holds the store across a model call).
/// It runs for the life of the process and stops when the store actor does.
/// Where the scheduler learns which devices are approved right now (the sync controller's cache).
pub type ApprovedDevices = Arc<dyn Fn() -> HashSet<String> + Send + Sync>;

/// The gate for work that follows the primary device but is not a routine (the daily profile
/// suggestions): the stored choice, this PC's identity and the cached approved devices, read now
/// so a change the user makes takes effect on the next turn.
pub fn gate_now(settings: &Mutex<SettingsFile>, own_device_id: Option<&str>, approved: &ApprovedDevices) -> RunGate {
    let primary = settings.lock().unwrap().primary_device_id();
    gate(&primary, own_device_id, &approved())
}

/// Meeting Prep for the meetings on the calendar, under the same gate as the routines. Called every
/// tick, whether or not any routine was due: prep is event-driven and core decides what is due (the
/// lead window, a brief already made here or by a peer, the Meeting Prep routine being on). `Off`
/// still shows a peer's brief under this PC's own calendar id.
pub fn prep_tick(
    store: &memory::Store,
    now_unix_ms: i64,
    lead_minutes: f64,
    ollama_model: &str,
    config: &api::RoutingConfig,
    meetings: &[QualifyingMeeting],
    gate: RunGate,
) -> Result<Vec<memory::RoutineRun>, String> {
    api::meeting_prep::run_with_chain(store, meetings, now_unix_ms, lead_minutes, ollama_model, config, gate).map_err(|e| e.to_string())
}

pub fn start(
    store: StoreHandle,
    settings: Arc<Mutex<SettingsFile>>,
    secrets: Arc<dyn SecretStore>,
    calendar: Option<Arc<CalendarService>>,
    connectors: Arc<dyn Connectors>,
    own_device_id: Option<String>,
    approved: ApprovedDevices,
) {
    let spawned = std::thread::Builder::new().name("whimbrel-routines".into()).spawn(move || {
        // A run left mid-approval by a crash goes back to waiting, as the Mac's store open does.
        if let Err(e) = store.call_blocking(|s| s.release_stale_executing_routine_runs()).and_then(|r| r.map_err(|e| crate::error::CommandError::store(e.to_string()))) {
            tracing::warn!(error = %e, "routines: could not release stale runs");
        }
        let mut delay = FIRST_TICK;
        loop {
            std::thread::sleep(delay);
            delay = TICK;
            // One short lock; nothing below holds it.
            let (routing, instructions, primary, prep_lead) = {
                let file = settings.lock().unwrap();
                (file.routing(), file.instructions(PromptSurface::DailyDigest), file.primary_device_id(), file.prep_lead_minutes())
            };
            // Read each tick, so a change the user makes takes effect within a minute.
            let run_gate = gate(&primary, own_device_id.as_deref(), &approved());
            let keys = ApiKeys::load(&*secrets).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "routines: could not read the API keys; only a local model is available this tick");
                ApiKeys::default()
            });
            let config = routing.routing_config(&keys, &instructions);
            let ollama_model = routing.ollama_model.clone();
            let now = Local::now();
            let events = calendar.as_ref().map(|c| todays_events(&c.meetings(), now)).unwrap_or_default();
            let upcoming = calendar.as_ref().map(|c| c.upcoming(now.timestamp_millis())).unwrap_or_default();
            // The gate: which routines this device may run (the user's choice of device). Core seeds the
            // built-in routines whatever it is.
            let outcome = store.call_blocking(move |s| {
                let (gmail_connected, recipient) = tick_inputs(s);
                let now_ms = now.timestamp_millis();
                let ran = api::routine_runs::tick(s, now_ms, &ollama_model, &config, &events, gmail_connected, recipient.as_deref(), run_gate).map_err(|e| e.to_string())?;
                // Meeting Prep is event-driven, so it is asked every tick, after the routines have seeded.
                let prepared = prep_tick(s, now_ms, prep_lead, &ollama_model, &config, &upcoming, run_gate).unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "meeting prep: could not run");
                    Vec::new()
                });
                Ok::<_, String>((ran, prepared.len()))
            });
            match outcome {
                Ok(Ok((runs, prepared))) if !runs.is_empty() || prepared > 0 => {
                    tracing::info!(runs = runs.len(), briefs = prepared, "routines: ran what was due")
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "routines: the tick could not read the routine table"),
                Err(e) => {
                    tracing::warn!(error = %e, "routines: the store is gone; stopping");
                    break;
                }
            }
            // Anything waiting whose routine is on and opted in to acting without a click, including
            // a draft left from before the user turned that on. Everything else waits for Approve.
            write::auto_approve(&store, &*connectors);
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the routine scheduler");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::connectors::calendar::MeetingSource;

    fn local(h: u32, m: u32, day: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, day, h, m, 0).earliest().unwrap()
    }

    fn meeting(title: &str, start: DateTime<Local>, minutes: i64) -> QualifyingMeeting {
        QualifyingMeeting {
            id: title.into(),
            title: title.into(),
            start_unix_ms: start.timestamp_millis(),
            end_unix_ms: start.timestamp_millis() + minutes * 60_000,
            attendees: vec![],
            conference_link: None,
            source: MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            ..Default::default()
        }
    }

    const MIN: i64 = 60_000;
    const NOW: i64 = 1_000_000_000_000;

    /// A store where the user has switched Meeting Prep on (it is off until they do).
    fn prep_store(enabled: bool) -> memory::Store {
        let store = memory::Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        let id = store.list_routines().unwrap().into_iter().find(|r| r.template == RoutineTemplate::MeetingPrep).unwrap().id;
        store.set_routine_enabled(id, enabled).unwrap();
        store
    }

    fn prep_meeting(id: &str, starts_in_min: i64) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.into(),
            title: "Acme sync".into(),
            start_unix_ms: NOW + starts_in_min * MIN,
            end_unix_ms: NOW + (starts_in_min + 30) * MIN,
            attendees: vec!["ann@acme.test".into(), "bo@acme.test".into()],
            conference_link: None,
            source: MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            ..Default::default()
        }
    }

    fn prep(store: &memory::Store, meetings: &[QualifyingMeeting], gate: RunGate) -> usize {
        prep_tick(store, NOW, 30.0, "no-such-model", &api::RoutingConfig::default(), meetings, gate).unwrap().len()
    }

    #[test]
    fn the_tick_prepares_a_meeting_whose_lead_has_opened_and_not_one_that_is_further_off() {
        let store = prep_store(true);
        assert_eq!(prep(&store, &[prep_meeting("far", 60)], RunGate::Full), 0, "60 min out, lead 30");
        assert!(store.get_meeting_prep("far").unwrap().is_none());
        assert_eq!(prep(&store, &[prep_meeting("near", 20)], RunGate::Full), 1);
        assert!(store.get_meeting_prep("near").unwrap().is_some());
    }

    #[test]
    fn a_secondary_covers_only_in_the_last_ten_minutes() {
        let store = prep_store(true);
        assert_eq!(prep(&store, &[prep_meeting("e", 20)], RunGate::Failover), 0, "the primary has until 10 min before");
        assert_eq!(prep(&store, &[prep_meeting("e", 8)], RunGate::Failover), 1);
    }

    #[test]
    fn off_prepares_nothing_but_still_shows_a_brief_a_peer_made_under_this_pcs_event_id() {
        let store = prep_store(true);
        let peer = memory::prep::MeetingPrepRow {
            event_id: "peer-calendar-id".into(),
            title: "Acme sync".into(),
            start_unix_ms: NOW + 20 * MIN,
            generated_at_unix_ms: NOW - MIN,
            brief_json: "{}".into(),
            provider: "ollama".into(),
            model: "m".into(),
        };
        assert!(store.apply_synced_meeting_prep(&peer).unwrap());
        assert!(store.get_meeting_prep("local-calendar-id").unwrap().is_none());
        assert_eq!(prep(&store, &[prep_meeting("local-calendar-id", 20)], RunGate::Off), 0);
        assert_eq!(store.get_meeting_prep("local-calendar-id").unwrap().expect("aliased").provider, "ollama");
    }

    #[test]
    fn nothing_is_prepared_with_the_routine_off() {
        let store = prep_store(false);
        assert_eq!(prep(&store, &[prep_meeting("e", 20)], RunGate::Full), 0);
    }

    #[test]
    fn todays_events_are_this_local_days_that_are_not_over_soonest_first() {
        let now = local(8, 30, 8);
        let meetings = vec![
            meeting("Standup", local(9, 0, 8), 15),
            meeting("Planning", local(8, 0, 8), 120), // started, still on
            meeting("Yesterday's retro", local(8, 0, 7), 60),
            meeting("Already over", local(7, 0, 8), 30),
            meeting("Tomorrow", local(9, 0, 9), 30),
            meeting("Late", local(23, 30, 8), 20),
        ];
        let titles: Vec<String> = todays_events(&meetings, now).into_iter().map(|e| e.title).collect();
        assert_eq!(titles, ["Planning", "Standup", "Late"]);
    }

    #[test]
    fn a_meeting_that_starts_at_midnight_belongs_to_the_next_day() {
        let now = local(8, 30, 8);
        assert!(todays_events(&[meeting("Midnight", local(0, 0, 9), 30)], now).is_empty());
    }

    #[test]
    fn no_calendar_means_no_events_not_an_error() {
        assert!(todays_events(&[], local(8, 30, 8)).is_empty());
    }

    #[test]
    fn the_event_carries_its_title_and_start() {
        let start = local(9, 0, 8);
        let events = todays_events(&[meeting("Standup", start, 15)], local(8, 30, 8));
        assert_eq!((events[0].title.as_str(), events[0].start_unix_ms), ("Standup", start.timestamp_millis()));
    }

    #[test]
    fn a_schedule_is_two_digit_hours_and_minutes_on_a_24_hour_clock() {
        for (text, want) in [("08:30", "08:30"), (" 17:05 ", "17:05"), ("00:00", "00:00"), ("23:59", "23:59")] {
            assert_eq!(valid_schedule(text).as_deref(), Some(want), "{text}");
        }
        for bad in ["", "8:30", "24:00", "12:60", "12-30", "12:3", "ab:cd", "12:30:00", "+1:30", "1230"] {
            assert_eq!(valid_schedule(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_daily_brief_runs_with_nothing_connected_and_meeting_prep_needs_a_calendar() {
        let now = Capabilities::NONE;
        assert_eq!(unavailable(&RoutineTemplate::DailyBrief, now), None);
        for t in [RoutineTemplate::FocusTime, RoutineTemplate::EodRecap, RoutineTemplate::WeeklyDigest, RoutineTemplate::ReplyPriorities, RoutineTemplate::MeetingPrep] {
            assert!(unavailable(&t, now).is_some(), "{t:?} should wait for a later task");
        }
        let everything = Capabilities { approvals: true, calendar: true, gmail_send: true, gmail_mail: true };
        assert_eq!(unavailable(&RoutineTemplate::MeetingPrep, now), Some("Needs a Google Calendar account connected (Settings)."));
        let calendar_only = Capabilities { calendar: true, ..Capabilities::NONE };
        assert_eq!(unavailable(&RoutineTemplate::MeetingPrep, calendar_only), None, "no approvals or Gmail needed");
        for t in [RoutineTemplate::DailyBrief, RoutineTemplate::FocusTime, RoutineTemplate::EodRecap, RoutineTemplate::WeeklyDigest, RoutineTemplate::ReplyPriorities, RoutineTemplate::MeetingPrep] {
            assert_eq!(unavailable(&t, everything), None, "{t:?}");
        }
    }

    fn store() -> memory::Store {
        memory::Store::open_in_memory().unwrap()
    }

    fn connect(store: &memory::Store, connector: &str, email: &str) {
        api::connectors::accounts::add(store, connector, email, 1).unwrap();
    }

    #[test]
    fn with_nothing_connected_the_email_routines_get_no_gmail_and_no_recipient() {
        assert_eq!(tick_inputs(&store()), (false, None));
    }

    #[test]
    fn the_recipient_is_the_default_calendar_account_as_on_the_mac() {
        let s = store();
        connect(&s, "calendar", "me@example.com");
        connect(&s, "calendar", "other@example.com");
        connect(&s, "gmail", "sender@example.com");
        assert_eq!(tick_inputs(&s), (true, Some("me@example.com".to_string())), "mail is for the user, not the account that sends it");
    }

    #[test]
    fn a_gmail_only_user_gets_the_recap_at_their_gmail_address() {
        let s = store();
        connect(&s, "gmail", "me@gmail.com");
        assert_eq!(tick_inputs(&s), (true, Some("me@gmail.com".to_string())));
    }

    #[test]
    fn calendar_without_gmail_is_not_connected_for_sending_even_though_there_is_a_recipient() {
        let s = store();
        connect(&s, "calendar", "me@example.com");
        assert_eq!(tick_inputs(&s), (false, Some("me@example.com".to_string())), "core drafts nothing unless Gmail is connected");
    }

    #[test]
    fn an_address_that_is_not_one_is_never_used_as_a_recipient() {
        let s = store();
        connect(&s, "calendar", "not-an-address");
        connect(&s, "gmail", "me@gmail.com");
        assert_eq!(tick_inputs(&s).1.as_deref(), Some("me@gmail.com"));
    }

    #[test]
    fn what_this_pc_can_do_follows_what_is_connected() {
        let s = store();
        let none = capabilities(&s);
        assert!(none.approvals && !none.calendar && !none.gmail_send && !none.gmail_mail);
        connect(&s, "gmail", "me@gmail.com");
        let sending = capabilities(&s);
        assert!(sending.gmail_send && !sending.calendar);
        assert_eq!(unavailable(&RoutineTemplate::EodRecap, sending), None, "connecting Gmail is what lets the recap routines run");
        assert!(unavailable(&RoutineTemplate::FocusTime, sending).unwrap().contains("Calendar"), "and does nothing for Focus Time");
    }

    // --- which device runs routines: the Mac's RoutineHostTests table ---------------------------------

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_unset_primary_means_every_device_runs_everything() {
        for primary in ["", "  ", "\t\n"] {
            assert_eq!(gate(primary, Some("me"), &ids(&["me", "other"])), RunGate::Full, "{primary:?}");
        }
    }

    #[test]
    fn the_chosen_device_runs_everything_and_another_approved_device_stands_by() {
        assert_eq!(gate("me", Some("me"), &ids(&["me", "other"])), RunGate::Full);
        assert_eq!(gate("other", Some("me"), &ids(&["me", "other"])), RunGate::Failover);
    }

    #[test]
    fn a_revoked_or_unknown_primary_never_stops_routines() {
        assert_eq!(gate("gone", Some("me"), &ids(&["me"])), RunGate::Full, "revoked or unknown behaves as unset");
    }

    #[test]
    fn the_fallback_device_id_is_never_treated_as_another_device() {
        assert_eq!(gate("other", Some("spike-device"), &ids(&["other"])), RunGate::Full);
    }

    #[test]
    fn a_pc_with_no_identity_runs_everything_because_it_cannot_tell_who_it_is() {
        assert_eq!(gate("other", None, &ids(&["other"])), RunGate::Full);
    }

    #[test]
    fn until_the_approved_list_is_known_a_choice_cannot_stop_routines() {
        assert_eq!(gate("other", Some("me"), &ids(&[])), RunGate::Full, "no fetch yet, or sync is not set up");
    }

    #[test]
    fn the_choice_is_compared_without_surrounding_whitespace() {
        assert_eq!(gate("  other\n", Some("me"), &ids(&["me", "other"])), RunGate::Failover);
    }

    #[test]
    fn the_profile_job_reads_the_choice_each_time_it_asks() {
        let dir = std::env::temp_dir().join(format!("whimbrel-gate-now-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Mutex::new(SettingsFile::load(&dir.join("settings.json")));
        let approved: ApprovedDevices = Arc::new(|| ids(&["me", "mac"]));
        assert_eq!(gate_now(&settings, Some("me"), &approved), RunGate::Full, "no choice yet");
        settings.lock().unwrap().set_primary_device_id("mac");
        assert_eq!(gate_now(&settings, Some("me"), &approved), RunGate::Failover, "another approved device is primary");
        settings.lock().unwrap().set_primary_device_id("me");
        assert_eq!(gate_now(&settings, Some("me"), &approved), RunGate::Full, "this PC chosen");
        settings.lock().unwrap().set_primary_device_id("");
        assert_eq!(gate_now(&settings, Some("me"), &approved), RunGate::Full, "back to any device");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_here_ever_turns_routines_off() {
        for (primary, own, approved) in [("", Some("me"), vec![]), ("me", Some("me"), vec!["me"]), ("other", Some("me"), vec!["other"]), ("x", None, vec![])] {
            assert_ne!(gate(primary, own, &ids(&approved)), RunGate::Off);
        }
    }

    fn device(public_key: &str, status: &str) -> DeviceView {
        DeviceView {
            id: format!("id-{public_key}"),
            public_key: public_key.into(),
            name: None,
            platform: None,
            status: status.into(),
            this_device: false,
            enrolled_at_unix_ms: 0,
            comparison_code: None,
        }
    }

    #[test]
    fn only_approved_devices_count_and_they_are_identified_by_public_key() {
        let devices = [device("key-a", "approved"), device("key-b", "pending"), device("key-c", "revoked"), device("key-d", "approved")];
        assert_eq!(approved_ids(&devices), ids(&["key-a", "key-d"]));
        assert!(approved_ids(&[]).is_empty());
    }

    #[test]
    fn a_choice_naming_a_revoked_device_runs_everything_end_to_end() {
        let devices = [device("me", "approved"), device("old", "revoked")];
        assert_eq!(gate("old", Some("me"), &approved_ids(&devices)), RunGate::Full);
        let devices = [device("me", "approved"), device("mac", "approved")];
        assert_eq!(gate("mac", Some("me"), &approved_ids(&devices)), RunGate::Failover);
    }

    #[test]
    fn each_later_task_opens_exactly_what_it_delivers() {
        let approvals = Capabilities { approvals: true, ..Capabilities::NONE };
        assert!(unavailable(&RoutineTemplate::FocusTime, approvals).unwrap().contains("Calendar"), "approvals alone are not enough: the event needs a Calendar");
        let with_calendar = Capabilities { calendar: true, ..approvals };
        assert_eq!(unavailable(&RoutineTemplate::FocusTime, with_calendar), None);
        let approvals = with_calendar;
        assert!(unavailable(&RoutineTemplate::EodRecap, approvals).unwrap().contains("Gmail"));
        let sending = Capabilities { gmail_send: true, ..approvals };
        assert_eq!(unavailable(&RoutineTemplate::EodRecap, sending), None);
        assert_eq!(unavailable(&RoutineTemplate::WeeklyDigest, sending), None);
        assert!(unavailable(&RoutineTemplate::ReplyPriorities, sending).is_some(), "ranking needs mail, not just sending");
        assert_eq!(unavailable(&RoutineTemplate::ReplyPriorities, Capabilities { gmail_mail: true, ..sending }), None);
    }
}
