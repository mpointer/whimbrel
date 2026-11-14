//! A meeting's recap (core #815, the Mac's #821): Copy as plain text, Markdown or Slack, and
//! Send recap to the invitees. With Gmail connected the recap goes out through the connector's
//! `send_email` as soon as the user clicks Send; otherwise it opens a draft in the mail app
//! (a `mailto:` link), with the body on the clipboard when it is too long for the link. Core
//! renders every text from the summary as the page shows it, so Mac and Windows match.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::State;

use api::recap::{RecapFormat, RecapLabels, RecapSummary};
use api::write_action::{PlannedAction, ToolOutcome};

use crate::connectors::accounts;
use crate::connectors::write::{self, Connectors};
use crate::error::CommandError;
use crate::l10n::tr;
use crate::state::AppState;

/// The summary as the meeting page shows it (unsaved edits included).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecapSummaryInput {
    pub title: String,
    pub summary: String,
    pub decisions: Vec<String>,
    pub action_items: Vec<String>,
    pub open_questions: Vec<String>,
}

impl RecapSummaryInput {
    fn core(&self) -> RecapSummary {
        RecapSummary {
            title: self.title.clone(),
            tldr: self.summary.clone(),
            decisions: self.decisions.clone(),
            action_items: self.action_items.clone(),
            open_questions: self.open_questions.clone(),
        }
    }
}

/// One invitee the recap can go to.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecapRecipient {
    pub name: String,
    pub email: String,
}

/// What the Send recap sheet opens with.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecapDraft {
    /// The invitees with a usable address, minus the user's own (all ticked to start).
    pub recipients: Vec<RecapRecipient>,
    /// The Gmail account the recap is sent from; `None` opens the mail app instead.
    pub gmail_from: Option<String>,
    /// The email body: the recap with the "Sent with Whimbrel" footer.
    pub body: String,
}

/// "Sent the recap to N people" on the meeting page.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecapSentView {
    pub sent_unix_ms: i64,
    pub recipient_count: i64,
}

/// The mail-app draft: open `url`. When `body_in_url` is false the body was too long for the
/// link, so the UI put it on the clipboard first and the draft says to paste it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecapMailto {
    pub url: String,
    pub body_in_url: bool,
}

fn labels() -> RecapLabels {
    RecapLabels {
        decisions: tr("Decisions", &[]),
        action_items: tr("Action items", &[]),
        open_questions: tr("Open questions", &[]),
        footer: tr("Sent with Whimbrel", &[]),
    }
}

fn format_of(format: &str) -> Result<RecapFormat, CommandError> {
    RecapFormat::parse(format).ok_or_else(|| CommandError::invalid_input(format!("unknown recap format {format}")))
}

/// The recap as text: "plain" (Copy), "markdown", "slack" or "email".
#[tauri::command]
pub fn recap_text(summary: RecapSummaryInput, format: String) -> Result<String, CommandError> {
    Ok(api::recap::render(&summary.core(), &labels(), format_of(&format)?))
}

/// The user's own addresses: every connected Gmail and Calendar account. They are left off.
fn own_emails(s: &memory::Store) -> Vec<String> {
    [accounts::GMAIL, accounts::CALENDAR]
        .iter()
        .filter_map(|kind| api::connectors::accounts::list(s, kind).ok())
        .flat_map(|a| a.emails)
        .collect()
}

/// The Gmail account to send from: the default, else the first connected; `None` when Gmail
/// isn't connected (or can't run here), which opens the mail app instead.
fn gmail_sender(s: &memory::Store) -> Option<String> {
    let a = api::connectors::accounts::list(s, accounts::GMAIL).ok()?;
    a.default_email.or_else(|| a.emails.first().cloned())
}

/// Opens the Send recap sheet: the invitees to tick, where it sends from, and the email body.
#[tauri::command]
pub async fn recap_draft(state: State<'_, AppState>, meeting_id: i64, summary: RecapSummaryInput) -> Result<RecapDraft, CommandError> {
    let gmail_runs = super::connectors::gmail_ready(&state);
    let (recipients, sender) = state
        .store()?
        .call(move |s| -> Result<_, String> {
            let own = own_emails(s);
            let attendees = s.meeting_attendee_details(meeting_id).map_err(|e| e.to_string())?;
            Ok((api::recap::recipients(&attendees, &own), gmail_sender(s)))
        })
        .await?
        .map_err(|e| CommandError::store(e.to_string()))?;
    Ok(RecapDraft {
        recipients: recipients
            .into_iter()
            .filter_map(|a| a.email.map(|email| RecapRecipient { name: a.name, email }))
            .collect(),
        gmail_from: sender.filter(|_| gmail_runs),
        body: api::recap::render(&summary.core(), &labels(), RecapFormat::Email),
    })
}

/// Sends the recap through Gmail right away (the user clicked Send), then records it.
#[tauri::command]
pub async fn recap_send(
    state: State<'_, AppState>,
    meeting_id: i64,
    to: Vec<String>,
    subject: String,
    body: String,
) -> Result<RecapSentView, CommandError> {
    let to: Vec<String> = to.into_iter().map(|e| e.trim().to_string()).filter(|e| api::recap::is_sendable_address(e)).collect();
    if to.is_empty() {
        return Err(CommandError::invalid_input(tr("Add at least one email address.", &[])));
    }
    let store = state.store()?.clone();
    let connectors = write::Real::new(state.secrets.clone(), state.paths.root.clone());
    let count = to.len() as i64;
    tauri::async_runtime::spawn_blocking(move || -> Result<RecapSentView, CommandError> {
        let account = store
            .call_blocking(gmail_sender)?
            .ok_or_else(|| CommandError::invalid_input(tr("Gmail isn't connected, so this opens in your mail app.", &[])))?;
        let failed = |reason: String| CommandError::network(tr("Couldn't send the recap: {error}", &[("error", &reason)]));
        let token = connectors
            .token("gmail", &account)
            .ok_or_else(|| failed(tr("Gmail needs reconnecting in Settings to send.", &[])))?;
        let plan = PlannedAction {
            run_id: -1,
            connector: "gmail".into(),
            tool: "send_email".into(),
            args_json: serde_json::json!({ "to": to.join(", "), "subject": api::recap::header_safe(&subject), "body": body }).to_string(),
            token_env: "GMAIL_ACCESS_TOKEN".into(),
            no_token_message: String::new(),
            success_summary: String::new(),
            terminal_failure_summary: String::new(),
        };
        match connectors.call(&plan, &token) {
            ToolOutcome::Success { .. } => {}
            ToolOutcome::RetryableFailure { reason } => return Err(failed(reason)),
            ToolOutcome::TerminalFailure => return Err(failed(tr("Gmail refused it.", &[]))),
            ToolOutcome::NoToken => return Err(failed(tr("Gmail needs reconnecting in Settings to send.", &[]))),
        }
        tracing::info!(meeting_id, recipients = count, "sent a meeting recap through Gmail");
        let now = chrono::Utc::now().timestamp_millis();
        store
            .call_blocking(move |s| s.record_recap_sent(meeting_id, count, now))?
            .map_err(|e| CommandError::store(e.to_string()))?;
        Ok(RecapSentView { sent_unix_ms: now, recipient_count: count })
    })
    .await
    .map_err(|e| CommandError::store(e.to_string()))?
}

/// The mail-app draft for `to`. Opening it is a separate call, after the UI has put the body
/// on the clipboard when it didn't fit in the link.
#[tauri::command]
pub fn recap_mailto(to: Vec<String>, subject: String, body: String) -> RecapMailto {
    let draft = api::recap::mailto(&to, &subject, &body, &tr("Paste the recap here.", &[]));
    RecapMailto { url: draft.url, body_in_url: draft.body_in_url }
}

/// Opens a `mailto:` link from `recap_mailto` in the user's mail app.
#[tauri::command]
pub fn recap_open_mail(url: String) -> Result<(), CommandError> {
    if !url.starts_with("mailto:") {
        return Err(CommandError::invalid_input("not a mailto link".to_string()));
    }
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    // ShellExecute hands the link to whatever app is registered for mailto:; at or below 32 is
    // a failure (typically none registered).
    let result = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url.as_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        tracing::warn!(code = result.0 as isize, "no mail app opened the recap draft");
        Err(CommandError::invalid_input(tr("No mail app opened. The recap is on the clipboard.", &[])))
    }
}

/// When the recap was last sent from this PC, if ever.
#[tauri::command]
pub async fn recap_sent(state: State<'_, AppState>, meeting_id: i64) -> Result<Option<RecapSentView>, CommandError> {
    let sent = state.store()?.call(move |s| s.recap_sent(meeting_id)).await?.map_err(|e| CommandError::store(e.to_string()))?;
    Ok(sent.map(|r| RecapSentView { sent_unix_ms: r.sent_unix_ms, recipient_count: r.recipient_count }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> RecapSummaryInput {
        RecapSummaryInput {
            title: "Planning".into(),
            summary: "We agreed the plan.".into(),
            decisions: vec!["Ship Friday".into()],
            action_items: vec!["Mike: send notes".into()],
            open_questions: vec![],
        }
    }

    #[test]
    fn the_recap_renders_in_each_format_and_the_email_carries_the_footer() {
        let plain = recap_text(input(), "plain".into()).unwrap();
        assert!(plain.contains("We agreed the plan.") && plain.contains("Ship Friday") && !plain.contains("Sent with Whimbrel"), "{plain}");
        let email = recap_text(input(), "email".into()).unwrap();
        assert!(email.contains("Sent with Whimbrel") && email.contains("whimbrelapp.com"), "{email}");
        assert!(recap_text(input(), "markdown".into()).unwrap().contains("Ship Friday"));
        assert!(recap_text(input(), "slack".into()).unwrap().contains("Ship Friday"));
        assert!(recap_text(input(), "pdf".into()).is_err());
    }

    #[test]
    fn a_long_body_goes_to_the_clipboard_and_the_link_says_to_paste_it() {
        let short = recap_mailto(vec!["a@x.com".into()], "Meeting recap".into(), "Hi".into());
        assert!(short.url.starts_with("mailto:") && short.body_in_url);
        let long = recap_mailto(vec!["a@x.com".into()], "Meeting recap".into(), "x".repeat(5_000));
        assert!(!long.body_in_url && long.url.contains("Paste"), "{}", long.url);
    }

    #[test]
    fn only_a_mailto_link_is_opened() {
        assert!(recap_open_mail("https://example.com".into()).is_err());
    }
}
