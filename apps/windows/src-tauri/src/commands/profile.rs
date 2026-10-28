//! Settings → Profile (Profile Task 2): the fields, the learned facts, the two switches, the
//! preview of exactly what is sent, and the daily suggestions waiting to be added or dismissed.
//! The rules (caps, one-line facts, duplicates) are `profile.rs`'s; the daily job is `jobs.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::CommandError;
use crate::profile::{edit_fits, new_fact_id, Fact, FactError, Profile, MAX_CHARS, MAX_FACTS, MAX_FACT_CHARS};
use crate::settings::SettingsFile;
use crate::state::AppState;

/// Everything the pane shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub name: String,
    pub role: String,
    pub organization: String,
    pub about: String,
    pub facts: Vec<Fact>,
    /// "Use my profile in summaries and answers".
    pub enabled: bool,
    /// "Suggest additions from my meetings".
    pub suggest_enabled: bool,
    /// Whether the daily job would run now: the suggest switch is on and something is written.
    pub suggestions_active: bool,
    /// "What Whimbrel sees": exactly the block sent with a prompt. Empty when the switch is off.
    pub preview: String,
    /// The block's length before the cap, for the counter. Above `maxChars`, the rest is not sent.
    pub composed_chars: usize,
    pub max_chars: usize,
    pub max_facts: usize,
    pub max_fact_chars: usize,
}

fn view(p: &Profile) -> ProfileView {
    ProfileView {
        name: p.name.clone(),
        role: p.role.clone(),
        organization: p.organization.clone(),
        about: p.about.clone(),
        facts: p.facts.clone(),
        enabled: p.enabled,
        suggest_enabled: p.suggest_enabled,
        suggestions_active: p.suggestions_active(),
        preview: p.routing_profile(),
        composed_chars: p.composed_chars(),
        max_chars: MAX_CHARS,
        max_facts: MAX_FACTS,
        max_fact_chars: MAX_FACT_CHARS,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFields {
    pub name: String,
    pub role: String,
    pub organization: String,
    pub about: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSwitches {
    pub enabled: bool,
    pub suggest_enabled: bool,
}

/// A fact the daily job proposed, waiting for Add or Dismiss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionView {
    pub id: i64,
    pub text: String,
    pub source_meeting_id: Option<i64>,
    /// The meeting it came from, when that meeting still exists.
    pub source_meeting_title: Option<String>,
    pub created_at_unix_ms: i64,
}

fn refused(e: FactError) -> CommandError {
    CommandError::invalid_input(e.message())
}

/// Applies one change to the Profile in `file`, refusing it (and changing nothing) if it would
/// push the block past the cap. Does not save.
fn apply(file: &mut SettingsFile, change: impl FnOnce(&mut Profile) -> Result<(), CommandError>) -> Result<ProfileView, CommandError> {
    let mut profile = file.profile();
    let before = profile.composed_chars();
    change(&mut profile)?;
    if !edit_fits(before, profile.composed_chars()) {
        return Err(CommandError::invalid_input(format!("That would push your Profile past {MAX_CHARS} characters. Shorten it first.")));
    }
    file.set_profile(&profile);
    Ok(view(&profile))
}

fn update(state: &AppState, change: impl FnOnce(&mut Profile) -> Result<(), CommandError>) -> Result<ProfileView, CommandError> {
    let mut file = state.settings.lock().unwrap();
    let shown = apply(&mut file, change)?;
    file.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    Ok(shown)
}

#[tauri::command]
pub fn profile_get(state: State<'_, AppState>) -> ProfileView {
    view(&state.settings.lock().unwrap().profile())
}

#[tauri::command]
pub fn profile_set_fields(state: State<'_, AppState>, fields: ProfileFields) -> Result<ProfileView, CommandError> {
    update(&state, |p| {
        p.name = fields.name;
        p.role = fields.role;
        p.organization = fields.organization;
        p.about = fields.about;
        Ok(())
    })
}

#[tauri::command]
pub fn profile_set_switches(state: State<'_, AppState>, switches: ProfileSwitches) -> Result<ProfileView, CommandError> {
    update(&state, |p| {
        p.enabled = switches.enabled;
        p.suggest_enabled = switches.suggest_enabled;
        Ok(())
    })
}

#[tauri::command]
pub fn profile_add_fact(state: State<'_, AppState>, text: String) -> Result<ProfileView, CommandError> {
    update(&state, |p| p.add_fact(new_fact_id(), &text, None).map_err(refused))
}

#[tauri::command]
pub fn profile_update_fact(state: State<'_, AppState>, id: String, text: String) -> Result<ProfileView, CommandError> {
    update(&state, |p| p.update_fact(&id, &text).map_err(refused))
}

/// Removing a fact that is already gone is fine: a double click should not be an error.
#[tauri::command]
pub fn profile_remove_fact(state: State<'_, AppState>, id: String) -> Result<ProfileView, CommandError> {
    update(&state, |p| {
        p.remove_fact(&id);
        Ok(())
    })
}

/// The pending suggestions, newest last, each with its source meeting's title.
#[tauri::command]
pub async fn profile_suggestions(state: State<'_, AppState>) -> Result<Vec<SuggestionView>, CommandError> {
    let store = state.store()?.clone();
    store.call(|s| suggestion_views(s)).await?.map_err(CommandError::store)
}

/// A suggestion another device made has no source meeting here (the id would mean nothing on
/// this PC), so it shows with no "From ..." line.
fn suggestion_views(s: &memory::Store) -> Result<Vec<SuggestionView>, String> {
    let pending = s.list_profile_suggestions("pending").map_err(|e| e.to_string())?;
    Ok(pending
        .into_iter()
        .map(|x| SuggestionView {
            source_meeting_title: x.source_meeting_id.and_then(|id| s.get_meeting(id).ok().flatten()).map(|m| m.title),
            id: x.id,
            text: x.text,
            source_meeting_id: x.source_meeting_id,
            created_at_unix_ms: x.created_at_unix_ms,
        })
        .collect())
}

/// Add: the suggestion becomes a fact, then its row is deleted. If the fact is refused (a
/// duplicate, the list is full, over the cap) the row stays so the user can dismiss it.
#[tauri::command]
pub async fn profile_accept_suggestion(state: State<'_, AppState>, id: i64) -> Result<ProfileView, CommandError> {
    let store = state.store()?.clone();
    let suggestion = store
        .call(|s| s.list_profile_suggestions("pending").map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?
        .into_iter()
        .find(|x| x.id == id)
        .ok_or_else(|| CommandError::invalid_input("That suggestion is no longer there."))?;
    let shown = update(&state, |p| p.add_fact(new_fact_id(), &suggestion.text, suggestion.source_meeting_id).map_err(refused))?;
    if let Err(e) = store.call(move |s| s.delete_profile_suggestion(id)).await? {
        // The fact is saved; a leftover row would only show up as a duplicate the next time it is added.
        tracing::warn!(error = %e, id, "profile: the accepted suggestion's row could not be deleted");
    }
    Ok(shown)
}

/// Dismiss: remembered, so the same suggestion is not proposed again.
#[tauri::command]
pub async fn profile_dismiss_suggestion(state: State<'_, AppState>, id: i64) -> Result<(), CommandError> {
    let store = state.store()?.clone();
    store.call(move |s| s.set_profile_suggestion_status(id, "dismissed")).await?.map_err(|e| CommandError::store(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> (SettingsFile, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("whimbrel-profile-commands-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (SettingsFile::load(&dir.join("settings.json")), dir)
    }

    #[test]
    fn a_suggestion_from_another_device_shows_pending_with_no_source_meeting() {
        let store = memory::Store::open_in_memory().unwrap();
        let mine = store.add_profile_suggestion("Prefers mornings", Some(404), 1_000).unwrap();
        let synced = store
            .apply_synced_profile_suggestion("mac", &memory::profile::AppliedProfileSuggestion { source_id: 9, text: "Works in UTC-5".into(), created_at_unix_ms: 2_000 })
            .unwrap()
            .expect("applied");
        let shown = suggestion_views(&store).unwrap();
        assert_eq!(shown.len(), 2);
        let theirs = shown.iter().find(|s| s.id == synced).unwrap();
        assert_eq!((theirs.text.as_str(), theirs.source_meeting_id, theirs.source_meeting_title.as_deref()), ("Works in UTC-5", None, None));
        let ours = shown.iter().find(|s| s.id == mine).unwrap();
        assert_eq!(ours.source_meeting_id, Some(404), "a suggestion made here still names its meeting id");
        assert_eq!(ours.source_meeting_title, None, "and a meeting that is gone has no title");
    }

    fn message(e: CommandError) -> String {
        match e {
            CommandError::InvalidInput { message } => message,
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn a_fact_is_added_stored_and_shown() {
        let (mut f, dir) = file("add");
        let shown = apply(&mut f, |p| p.add_fact("a".into(), "  Works in UTC-5 \n", Some(9)).map_err(refused)).unwrap();
        assert_eq!(shown.facts, vec![Fact { id: "a".into(), text: "Works in UTC-5".into(), source_meeting_id: Some(9) }]);
        assert_eq!(f.profile().facts, shown.facts, "the file holds what the pane was told");
        assert!(shown.suggestions_active, "a fact is content, so the daily job is armed");
        assert_eq!(shown.preview, "Facts:\n- Works in UTC-5");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refused_fact_changes_nothing_and_says_why() {
        let (mut f, dir) = file("refused");
        apply(&mut f, |p| p.add_fact("a".into(), "Likes tea", None).map_err(refused)).unwrap();
        let dup = apply(&mut f, |p| p.add_fact("b".into(), "likes TEA.", None).map_err(refused)).unwrap_err();
        assert_eq!(message(dup), "That is already in your list.");
        let empty = apply(&mut f, |p| p.add_fact("c".into(), "   ", None).map_err(refused)).unwrap_err();
        assert_eq!(message(empty), "A fact can't be empty.");
        assert_eq!(f.profile().facts.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_edit_that_pushes_the_block_past_the_cap_is_refused_and_leaves_the_file_alone() {
        let (mut f, dir) = file("cap");
        f.set_profile(&Profile { about: "x".repeat(MAX_CHARS - 20), ..Profile::default() });
        let before = f.profile();
        let err = apply(&mut f, |p| p.add_fact("a".into(), &"y".repeat(100), None).map_err(refused)).unwrap_err();
        assert!(message(err).contains("2000"));
        assert_eq!(f.profile(), before);
        let err = apply(&mut f, |p| {
            p.role = "z".repeat(100);
            Ok(())
        })
        .unwrap_err();
        assert!(message(err).contains("past 2000 characters"));
        assert_eq!(f.profile(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_profile_that_is_already_over_the_cap_can_still_be_trimmed() {
        let (mut f, dir) = file("trim");
        f.set_profile(&Profile { about: "x".repeat(MAX_CHARS + 500), ..Profile::default() });
        let longer = apply(&mut f, |p| {
            p.about.push('x');
            Ok(())
        });
        assert!(longer.is_err(), "longer is refused");
        let shorter = apply(&mut f, |p| {
            p.about = "x".repeat(MAX_CHARS + 100);
            Ok(())
        })
        .unwrap();
        assert!(shorter.composed_chars > MAX_CHARS, "still over, but shorter, so allowed");
        assert_eq!(shorter.preview.chars().count(), MAX_CHARS, "what is sent still stops at the cap");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_fields_and_switches_are_saved_and_the_view_reports_the_gate_and_the_preview() {
        let (mut f, dir) = file("switches");
        let empty = view(&f.profile());
        assert!(!empty.suggestions_active, "nothing written: the daily job is not armed even with the switch on");
        assert_eq!((empty.max_chars, empty.max_facts, empty.max_fact_chars), (2000, 30, 200));

        let shown = apply(&mut f, |p| {
            p.role = "CTO".into();
            Ok(())
        })
        .unwrap();
        assert!(shown.suggestions_active);
        assert_eq!(shown.preview, "Role: CTO");
        assert_eq!(shown.composed_chars, "Role: CTO".len());

        let off = apply(&mut f, |p| {
            p.suggest_enabled = false;
            Ok(())
        })
        .unwrap();
        assert!(!off.suggestions_active, "the suggest switch closes the gate");
        assert_eq!(off.preview, "Role: CTO", "and does not change what prompts get");

        let unused = apply(&mut f, |p| {
            p.enabled = false;
            Ok(())
        })
        .unwrap();
        assert_eq!(unused.preview, "", "'use my profile' off: nothing is sent");
        assert_eq!(unused.composed_chars, "Role: CTO".len());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removing_and_editing_a_fact_go_through_the_same_rules() {
        let (mut f, dir) = file("edit");
        apply(&mut f, |p| p.add_fact("a".into(), "one", None).map_err(refused)).unwrap();
        apply(&mut f, |p| p.add_fact("b".into(), "two", None).map_err(refused)).unwrap();
        let clash = apply(&mut f, |p| p.update_fact("a", "TWO").map_err(refused)).unwrap_err();
        assert_eq!(message(clash), "That is already in your list.");
        let gone = apply(&mut f, |p| p.update_fact("zzz", "x").map_err(refused)).unwrap_err();
        assert_eq!(message(gone), "That fact no longer exists.");
        let shown = apply(&mut f, |p| {
            p.remove_fact("a");
            p.remove_fact("a");
            Ok(())
        })
        .unwrap();
        assert_eq!(shown.facts.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
