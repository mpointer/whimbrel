//! The topic page (topics piece 2): the Topics section, its page and step actions, the ways in
//! (Today, the meeting page, the prep brief, search and Ask, the tray), and Settings → Topics'
//! "Names I go by" and "Find steps with TypeSafe". Plan:
//! `docs/superpowers/plans/2026-11-08-windows-topic-page.md`.
//!
//! Core's page types carry no JSON schema, so the views here mirror them for the UI.

use std::sync::atomic::Ordering;

use memory::topics::page::{NextMeeting, StepView, SummaryLead, TopicListEntry, TopicPage, TopicRef};
use memory::topics::read::{OpenTarget, TopicMember};
use memory::topics::steps::StepAction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::chat::LibraryTarget;
use crate::commands::topics::TopicSortEstimate;
use crate::error::CommandError;
use crate::l10n::tr;
use crate::secrets::{accounts, SecretStore, APP_SERVICE};
use crate::settings::SettingsFile;
use crate::state::AppState;
use crate::topics::{self, TopicSortStatus, USD_PER_MILLION};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn store_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::store(e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicRefView {
    pub topic_id: i64,
    pub key: String,
    pub name: String,
}

impl From<TopicRef> for TopicRefView {
    fn from(t: TopicRef) -> Self {
        TopicRefView { topic_id: t.topic_id, key: t.key, name: t.name }
    }
}

/// A row in the Topics list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicListItemView {
    pub topic: TopicRefView,
    /// customer, product, project or subject; none for Other.
    pub kind: Option<String>,
    pub next_meeting_unix_ms: Option<i64>,
    /// Open steps the user owes (the hint's "2 steps").
    pub open_mine: i64,
}

impl From<TopicListEntry> for TopicListItemView {
    fn from(e: TopicListEntry) -> Self {
        TopicListItemView { topic: e.topic.into(), kind: e.kind, next_meeting_unix_ms: e.next_meeting_unix_ms, open_mine: e.open_mine }
    }
}

/// Where a link on the page goes: a Library detail or the meeting page. `None` (an email
/// thread, which Windows has no page for) shows as plain text.
fn open_target(open: Option<OpenTarget>) -> Option<LibraryTarget> {
    let open = open?;
    match open.window.as_str() {
        "meeting-detail" => Some(LibraryTarget::Meeting { id: open.id }),
        "capture-detail" => Some(LibraryTarget::Capture { id: open.id }),
        "episode-detail" => Some(LibraryTarget::Episode { id: open.id }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicStepView {
    /// Pass to `topic_step_action`.
    pub key: String,
    pub text: String,
    /// `mine` (You owe) or `owed` (Owed to you).
    pub direction: String,
    /// email, meeting or episode.
    pub source_kind: String,
    pub source_title: String,
    /// Found on a screen (a Teams episode): captures are partial, so the page says so.
    pub from_screen: bool,
    pub open: Option<LibraryTarget>,
    pub asked_at_unix_ms: i64,
    /// `YYYY-MM-DD` when one was stated.
    pub due_date: Option<String>,
    /// open, done or dismissed.
    pub status: String,
    pub confirmed: bool,
    pub note: Option<String>,
    /// What closed it, for a step a newer item marked done.
    pub closed_by: Option<String>,
}

impl From<StepView> for TopicStepView {
    fn from(s: StepView) -> Self {
        TopicStepView {
            key: s.key,
            text: s.text,
            direction: s.direction,
            source_kind: s.source_kind,
            source_title: s.source_title,
            from_screen: s.from_screen,
            open: open_target(s.open),
            asked_at_unix_ms: s.asked_at_unix_ms,
            due_date: s.due_date,
            status: s.status,
            confirmed: s.confirmed,
            note: s.note,
            closed_by: s.closed_by,
        }
    }
}

/// What the summary line leads with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SummaryLeadView {
    /// A key date: "SOW due Fri Nov 14".
    Date { label: String, date: String },
    /// The next open step.
    #[serde(rename_all = "camelCase")]
    Step { text: String, due_date: Option<String> },
}

impl From<SummaryLead> for SummaryLeadView {
    fn from(l: SummaryLead) -> Self {
        match l {
            SummaryLead::Date { label, date } => SummaryLeadView::Date { label, date },
            SummaryLead::Step { text, due_date } => SummaryLeadView::Step { text, due_date },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgCount {
    /// An email domain.
    pub org: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicNextMeetingView {
    pub title: String,
    pub start_unix_ms: i64,
    /// Attendees other than the user, by email domain, largest first.
    pub attendees_by_org: Vec<OrgCount>,
    /// Attendees whose address is one of the user's: "1 you".
    pub me: i64,
}

impl From<NextMeeting> for TopicNextMeetingView {
    fn from(m: NextMeeting) -> Self {
        TopicNextMeetingView {
            title: m.title,
            start_unix_ms: m.start_unix_ms,
            attendees_by_org: m.attendees_by_org.into_iter().map(|(org, count)| OrgCount { org, count }).collect(),
            me: m.me,
        }
    }
}

/// A Recent row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicItemView {
    /// email, meeting, calendar, episode…
    pub kind: String,
    pub title: String,
    pub ts_unix_ms: i64,
    pub open: Option<LibraryTarget>,
}

impl From<TopicMember> for TopicItemView {
    fn from(m: TopicMember) -> Self {
        TopicItemView { kind: m.kind, title: m.title, ts_unix_ms: m.ts_unix_ms, open: open_target(m.open) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicPageView {
    pub topic: TopicRefView,
    pub kind: Option<String>,
    /// "lead consultant", "supporting"…
    pub role: String,
    /// The user wrote the role.
    pub role_edited: bool,
    pub lead: Option<SummaryLeadView>,
    pub next_meeting: Option<TopicNextMeetingView>,
    pub you_owe: Vec<TopicStepView>,
    pub owed_to_you: Vec<TopicStepView>,
    pub done: Vec<TopicStepView>,
    /// Open steps of a topic quiet for 60 days.
    pub older: Vec<TopicStepView>,
    /// The newest 20 items.
    pub recent: Vec<TopicItemView>,
    pub recent_total: i64,
}

impl From<TopicPage> for TopicPageView {
    fn from(p: TopicPage) -> Self {
        let steps = |v: Vec<StepView>| v.into_iter().map(TopicStepView::from).collect();
        TopicPageView {
            topic: p.topic.into(),
            kind: p.kind,
            role: p.role.text,
            role_edited: p.role.edited,
            lead: p.lead.map(Into::into),
            next_meeting: p.next_meeting.map(Into::into),
            you_owe: steps(p.you_owe),
            owed_to_you: steps(p.owed_to_you),
            done: steps(p.done),
            older: steps(p.older),
            recent: p.recent.into_iter().map(Into::into).collect(),
            recent_total: p.recent_total,
        }
    }
}

/// What the user does to a step: core's `StepAction`, with a schema.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum StepActionInput {
    Done,
    Reopen,
    /// `reason`: `not_mine` or `not_task`.
    Dismiss { reason: String },
    Confirm { confirmed: bool },
    Note { text: String },
    #[serde(rename_all = "camelCase")]
    Edit { text: String, due_date: Option<String> },
}

pub fn step_action(input: StepActionInput) -> Result<StepAction, CommandError> {
    Ok(match input {
        StepActionInput::Done => StepAction::Done,
        StepActionInput::Reopen => StepAction::Reopen,
        StepActionInput::Dismiss { reason } => {
            if reason != "not_mine" && reason != "not_task" {
                return Err(CommandError::invalid_input(format!("unknown dismiss reason {reason:?}")));
            }
            StepAction::Dismiss { reason }
        }
        StepActionInput::Confirm { confirmed } => StepAction::Confirm { confirmed },
        StepActionInput::Note { text } => StepAction::Note { text: text.trim().to_string() },
        StepActionInput::Edit { text, due_date } => {
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err(CommandError::invalid_input(tr("A step needs some text.", &[])));
            }
            let due_date = due_date.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
            if let Some(d) = &due_date {
                if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
                    return Err(CommandError::invalid_input(format!("not a date: {d:?}")));
                }
            }
            StepAction::Edit { text, due_date }
        }
    })
}

#[tauri::command]
pub async fn topics_page_list(state: State<'_, AppState>) -> Result<Vec<TopicListItemView>, CommandError> {
    let now = now_ms();
    let list = state.store()?.call(move |s| s.topic_page_list(now)).await?.map_err(store_error)?;
    Ok(list.into_iter().map(Into::into).collect())
}

/// `None` when the topic is gone (deleted, or merged away).
#[tauri::command]
pub async fn topic_page(state: State<'_, AppState>, topic_id: i64) -> Result<Option<TopicPageView>, CommandError> {
    let now = now_ms();
    let page = state.store()?.call(move |s| s.topic_page(topic_id, now)).await?.map_err(store_error)?;
    Ok(page.map(Into::into))
}

/// `false` when the step no longer exists.
#[tauri::command]
pub async fn topic_step_action(state: State<'_, AppState>, key: String, action: StepActionInput) -> Result<bool, CommandError> {
    let action = step_action(action)?;
    state.store()?.call(move |s| s.apply_step_action(&key, &action)).await?.map_err(store_error)
}

/// The user's wording of their role; blank goes back to the inferred one.
#[tauri::command]
pub async fn topic_role_set(state: State<'_, AppState>, topic_key: String, text: String) -> Result<(), CommandError> {
    let text = text.trim().to_string();
    state.store()?.call(move |s| s.set_topic_role(&topic_key, &text)).await?.map_err(store_error)
}

/// One thing a page shows a topic chip for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopicLookup {
    /// A recorded meeting.
    Meeting { id: i64 },
    /// A calendar event: its series' topic, else the first topic its title names.
    Event { id: String, title: String },
}

/// Each lookup's topic, in order, in one store call (Today asks for every row at once).
#[tauri::command]
pub async fn topic_lookup(state: State<'_, AppState>, items: Vec<TopicLookup>) -> Result<Vec<Option<TopicRefView>>, CommandError> {
    let found = state
        .store()?
        .call(move |s| {
            items
                .iter()
                .map(|item| match item {
                    TopicLookup::Meeting { id } => s.topic_for_meeting(*id),
                    TopicLookup::Event { id, title } => s.topic_for_calendar_event(id, title),
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .map_err(store_error)?;
    Ok(found.into_iter().map(|t| t.map(Into::into)).collect())
}

/// Topics a search or question names: Library and Ask offer their pages first.
#[tauri::command]
pub async fn topics_named_in(state: State<'_, AppState>, text: String) -> Result<Vec<TopicRefView>, CommandError> {
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    let found = state.store()?.call(move |s| s.topics_named_in(&text)).await?.map_err(store_error)?;
    Ok(found.into_iter().map(Into::into).collect())
}

/// "Names I go by", and names from the user's accounts to offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicNamesView {
    pub names: Vec<String>,
    pub suggested: Vec<String>,
}

#[tauri::command]
pub async fn topic_names(state: State<'_, AppState>) -> Result<TopicNamesView, CommandError> {
    let (names, suggested) = state
        .store()?
        .call(|s| s.self_names().and_then(|names| Ok((names, s.suggested_self_names()?))))
        .await?
        .map_err(store_error)?;
    Ok(TopicNamesView { names, suggested })
}

/// Saves the list (trimmed, blanks and repeats dropped) and returns it.
#[tauri::command]
pub async fn set_topic_names(state: State<'_, AppState>, names: Vec<String>) -> Result<TopicNamesView, CommandError> {
    let names = clean_names(names);
    state.store()?.call(move |s| s.set_self_names(&names)).await?.map_err(store_error)?;
    topic_names(state).await
}

pub fn clean_names(names: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let n = n.trim().to_string();
        if !n.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
            out.push(n);
        }
    }
    out
}

/// Settings → Topics' "Find steps with TypeSafe".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicStepsView {
    pub enabled: bool,
    /// Start was pressed on the estimate.
    pub started: bool,
    /// A TypeSafe key is saved (Settings → AI); the switch needs one.
    pub has_key: bool,
    pub local_only: bool,
    pub key_rejected: bool,
    pub over_budget: bool,
    /// The last local run found no local model: "Finding steps needs a local model (Settings → AI)."
    pub no_model: bool,
}

fn steps_view(file: &SettingsFile, secrets: &dyn SecretStore, status: &TopicSortStatus) -> Result<TopicStepsView, CommandError> {
    Ok(TopicStepsView {
        enabled: file.topic_steps_enabled(),
        started: file.topic_steps_started(),
        has_key: secrets.get_string(APP_SERVICE, accounts::TYPESAFE_API_KEY)?.is_some_and(|k| !k.trim().is_empty()),
        local_only: file.routing().local_only,
        key_rejected: status.steps_key_rejected.load(Ordering::SeqCst),
        over_budget: status.steps_over_budget.load(Ordering::SeqCst),
        no_model: status.steps_no_model.load(Ordering::SeqCst),
    })
}

fn save(file: &SettingsFile) -> Result<(), CommandError> {
    file.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))
}

#[tauri::command]
pub fn topic_steps_state(state: State<'_, AppState>) -> Result<TopicStepsView, CommandError> {
    steps_view(&state.settings.lock().unwrap(), &*state.secrets, &state.topic_sort)
}

/// The step switch. Turning it on after the budget ran out asks for a fresh Start; turning it
/// off goes back to the local model and deletes nothing.
#[tauri::command]
pub fn set_topic_steps_enabled(state: State<'_, AppState>, enabled: bool) -> Result<TopicStepsView, CommandError> {
    {
        let mut file = state.settings.lock().unwrap();
        steps_switch(&mut file, &state.topic_sort, enabled);
        save(&file)?;
    }
    topic_steps_state(state)
}

fn steps_switch(file: &mut SettingsFile, status: &TopicSortStatus, enabled: bool) {
    if enabled && status.steps_over_budget.swap(false, Ordering::SeqCst) {
        file.set_topic_steps_started(false);
    }
    file.set_topic_steps_enabled(enabled);
}

#[tauri::command]
pub async fn topic_steps_estimate(state: State<'_, AppState>) -> Result<TopicSortEstimate, CommandError> {
    let e = state.store()?.call(|s| s.topic_steps_estimate(USD_PER_MILLION)).await?.map_err(store_error)?;
    Ok(TopicSortEstimate { sources: e.sources, tokens: e.tokens, usd: e.usd, indexing: e.indexing })
}

/// Start on the estimate: allows TypeSafe up to 1.5 × `tokens`.
#[tauri::command]
pub async fn start_topic_steps(state: State<'_, AppState>, tokens: i64) -> Result<TopicStepsView, CommandError> {
    let budget = topics::start_budget(tokens);
    state.store()?.call(move |s| s.start_topic_steps_judging(budget)).await?.map_err(store_error)?;
    {
        let mut file = state.settings.lock().unwrap();
        file.set_topic_steps_started(true);
        save(&file)?;
    }
    state.topic_sort.steps_over_budget.store(false, Ordering::SeqCst);
    topic_steps_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_arrive_as_the_ui_sends_them_and_are_checked() {
        let parse = |v: serde_json::Value| step_action(serde_json::from_value(v).unwrap());
        assert_eq!(parse(serde_json::json!({"action": "done"})).unwrap(), StepAction::Done);
        assert_eq!(
            parse(serde_json::json!({"action": "dismiss", "reason": "not_mine"})).unwrap(),
            StepAction::Dismiss { reason: "not_mine".into() }
        );
        assert!(parse(serde_json::json!({"action": "dismiss", "reason": "bored"})).is_err());
        assert_eq!(
            parse(serde_json::json!({"action": "edit", "text": " Send the SOW ", "dueDate": " "})).unwrap(),
            StepAction::Edit { text: "Send the SOW".into(), due_date: None }
        );
        assert!(parse(serde_json::json!({"action": "edit", "text": "x", "dueDate": "Friday"})).is_err());
        assert!(parse(serde_json::json!({"action": "edit", "text": "  ", "dueDate": null})).is_err());
        assert_eq!(
            parse(serde_json::json!({"action": "confirm", "confirmed": true})).unwrap(),
            StepAction::Confirm { confirmed: true }
        );
    }

    #[test]
    fn links_open_library_details_and_the_meeting_page_but_not_threads() {
        let t = |w: &str| open_target(Some(OpenTarget { window: w.into(), id: 7 }));
        assert_eq!(t("meeting-detail"), Some(LibraryTarget::Meeting { id: 7 }));
        assert_eq!(t("episode-detail"), Some(LibraryTarget::Episode { id: 7 }));
        assert_eq!(t("capture-detail"), Some(LibraryTarget::Capture { id: 7 }));
        assert_eq!(t("thread-detail"), None);
        assert_eq!(open_target(None), None);
    }

    #[test]
    fn names_are_trimmed_without_blanks_or_repeats() {
        let names = clean_names(vec![" Mike ".into(), "".into(), "mike".into(), "Michael".into()]);
        assert_eq!(names, vec!["Mike".to_string(), "Michael".to_string()]);
    }

    #[test]
    fn the_steps_switch_asks_for_a_new_start_after_the_budget_ran_out() {
        let path = std::env::temp_dir().join(format!("whimbrel-topic-page-{}-{:?}.json", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_file(&path);
        let mut f = SettingsFile::load(&path);
        let status = TopicSortStatus::default();
        steps_switch(&mut f, &status, true);
        f.set_topic_steps_started(true);
        steps_switch(&mut f, &status, false);
        assert!(f.topic_steps_started(), "off keeps the budget");
        status.steps_over_budget.store(true, Ordering::SeqCst);
        steps_switch(&mut f, &status, true);
        assert!(!f.topic_steps_started(), "a spent budget shows the estimate again");
        let secrets = crate::secrets::MemorySecrets::default();
        let v = steps_view(&f, &secrets, &status).unwrap();
        assert!(v.enabled && !v.has_key && !v.over_budget);
        let _ = std::fs::remove_file(path);
    }
}
