//! The cloud runner's configuration, read from the synced settings it pulls. The runner has no
//! keychain and no Settings screen: everything it needs arrives as LWW string fields
//! ([`sync_engine::SettingsField`]), and this module is its own mapping from those fields to typed
//! settings (the apps keep theirs on the device side).
//!
//! Nothing here sets process state: no environment variable, no process-wide switch.

use std::collections::HashMap;

use sync_engine::SettingsField;

use crate::routine_runs::CloudPolicy;
use crate::routing::RoutingConfig;

/// The settings field the runner writes each round; the primary treats the runner as fresh while it
/// is under a grace period old.
pub const HEARTBEAT_KEY: &str = "cloud.runner.lastRun";

/// The settings field holding the user's IANA time zone name.
pub const TIME_ZONE_KEY: &str = "profile.timeZone";

/// The routines that may run in the cloud, by stored template name (`routines::cloud_eligible`).
const CLOUD_TEMPLATES: [&str; 4] = ["daily_brief", "eod_recap", "weekly_digest", "meeting_prep"];

/// The hosted providers whose keys the user may sync.
const KEY_PROVIDERS: [&str; 4] = ["anthropic", "openai", "google", "openrouter"];

/// A live (not deleted) field's value.
fn live<'a>(fields: &'a HashMap<String, SettingsField>, key: &str) -> Option<&'a str> {
    fields.get(key).filter(|f| !f.is_deleted).map(|f| f.value.as_str())
}

fn flag(fields: &HashMap<String, SettingsField>, key: &str) -> Option<bool> {
    match live(fields, key)? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn text(fields: &HashMap<String, SettingsField>, key: &str) -> String {
    live(fields, key).map(|v| v.trim().to_string()).unwrap_or_default()
}

fn cents(fields: &HashMap<String, SettingsField>, key: &str) -> Option<i64> {
    live(fields, key)?.trim().parse::<i64>().ok().filter(|c| *c >= 0)
}

/// A provider's key, only when the user opted in to syncing it (`apiKey.<provider>.syncEnabled` is
/// `true`) and the key field itself is live and not empty. An opted-out key never reaches the runner.
fn synced_key(fields: &HashMap<String, SettingsField>, provider: &str) -> Option<String> {
    if flag(fields, &format!("apiKey.{provider}.syncEnabled")) != Some(true) {
        return None;
    }
    live(fields, &format!("apiKey.{provider}")).map(str::trim).filter(|k| !k.is_empty()).map(String::from)
}

/// The routing the runner uses, from pulled settings: hosted only. There is no local model on the
/// runner, so Ollama is never a link. A synced local-only setting (`routing.localOnlyMode`) turns on
/// the sensitive mode, whose chain holds no hosted link at all, so a call is refused rather than
/// falling back to a hosted model. No Whimbrel AI credentials are ever set; keys come only from
/// opt-in synced provider keys.
pub fn routing_from_settings(fields: &HashMap<String, SettingsField>) -> RoutingConfig {
    let preferred_provider = match live(fields, "routing.preferredProvider").map(str::trim) {
        Some(p) if KEY_PROVIDERS.contains(&p) => p.to_string(),
        _ => "ollama".to_string(),
    };
    let model_of = |provider: &str| text(fields, &format!("routing.{provider}DefaultModel"));
    let preferred_model = if preferred_provider == "ollama" { String::new() } else { model_of(&preferred_provider) };
    RoutingConfig {
        anthropic_key: synced_key(fields, "anthropic"),
        openai_key: synced_key(fields, "openai"),
        google_key: synced_key(fields, "google"),
        openrouter_key: synced_key(fields, "openrouter"),
        openrouter_model: model_of("openrouter"),
        user_daily_cap_cents: cents(fields, "routing.userDailyCapCents"),
        global_daily_cap_cents: cents(fields, "routing.globalDailyCapCents"),
        sensitive: flag(fields, "routing.localOnlyMode").unwrap_or(false),
        preferred_provider,
        preferred_model,
        custom_instructions: text(fields, "summary.customPromptInstructions"),
        daily_digest_instructions: text(fields, "prompts.dailyDigest"),
        weekly_digest_instructions: text(fields, "prompts.weeklyDigest"),
        cheap_first: flag(fields, "routing.cheapFirst").unwrap_or(true),
        use_local_model: false,
        ..RoutingConfig::default()
    }
}

/// The templates the user flagged to run in the cloud (`routine.<template>.cloud` is `true`), among
/// those that may, and each one's ready-by time (`routine.<template>.readyBy`, "HH:MM"; a deleted or
/// unreadable value is left out). `runner_fresh` is false here: it depends on the clock, so use
/// [`cloud_policy_at`] where a device decides whether to stand in.
pub fn cloud_policy_from_settings(fields: &HashMap<String, SettingsField>) -> CloudPolicy {
    let cloud_templates = CLOUD_TEMPLATES
        .iter()
        .filter(|t| flag(fields, &format!("routine.{t}.cloud")) == Some(true))
        .map(|t| t.to_string())
        .collect();
    let ready_by = CLOUD_TEMPLATES
        .iter()
        .filter_map(|t| {
            let at = live(fields, &format!("routine.{t}.readyBy"))?.trim();
            is_hhmm(at).then(|| (t.to_string(), at.to_string()))
        })
        .collect();
    CloudPolicy { cloud_templates, runner_fresh: false, ready_by }
}

/// [`cloud_policy_from_settings`] with the runner's freshness at `now_unix_ms` filled in.
pub fn cloud_policy_at(fields: &HashMap<String, SettingsField>, now_unix_ms: i64, grace_ms: i64) -> CloudPolicy {
    CloudPolicy { runner_fresh: runner_fresh(fields, now_unix_ms, grace_ms), ..cloud_policy_from_settings(fields) }
}

/// The heartbeat the runner pushes each round: the time of the round, stamped with the same time.
pub fn heartbeat_field(now_unix_ms: i64) -> (String, SettingsField) {
    (
        HEARTBEAT_KEY.to_string(),
        SettingsField { value: now_unix_ms.to_string(), updated_at_unix_ms: now_unix_ms, is_deleted: false },
    )
}

/// How far ahead of the reader's clock a heartbeat may be and still count as fresh.
const FUTURE_TOLERANCE_MS: i64 = 5 * 60_000;

/// Whether the runner's heartbeat is under `grace_ms` old at `now_unix_ms`. A missing, deleted or
/// unreadable heartbeat is not fresh.
pub fn runner_fresh(fields: &HashMap<String, SettingsField>, now_unix_ms: i64, grace_ms: i64) -> bool {
    let Some(last) = live(fields, HEARTBEAT_KEY).and_then(|v| v.trim().parse::<i64>().ok()) else {
        return false;
    };
    // A heartbeat from the future (clock skew, or garbage) is fresh only within a small tolerance.
    let age = now_unix_ms.saturating_sub(last);
    (-FUTURE_TOLERANCE_MS..grace_ms).contains(&age)
}

/// The user's IANA time zone name (`profile.timeZone`), or `None` when it is missing, deleted or
/// empty. Not validated here and never written to the process environment.
pub fn timezone_from_settings(fields: &HashMap<String, SettingsField>) -> Option<String> {
    Some(text(fields, TIME_ZONE_KEY)).filter(|z| !z.is_empty())
}

fn is_hhmm(s: &str) -> bool {
    let mut parts = s.split(':');
    let (Some(h), Some(m), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    h.len() == 2
        && m.len() == 2
        && h.bytes().all(|b| b.is_ascii_digit())
        && m.bytes().all(|b| b.is_ascii_digit())
        && h.parse::<u32>().is_ok_and(|h| h < 24)
        && m.parse::<u32>().is_ok_and(|m| m < 60)
}

fn is_model_pref(v: &str) -> bool {
    match v {
        "default" | "cheapest" => true,
        _ => matches!(
            v.split_once('/'),
            Some((p, m)) if matches!(p, "ollama" | "anthropic" | "openai" | "google" | "openrouter") && !m.is_empty()
        ),
    }
}

/// Applies `routine.<template>.enabled|schedule|model_pref` onto the scratch store's routines, for
/// the four cloud-eligible templates only. Routines are matched by template, not id. A deleted
/// field, another template (Reply Priorities included), an unreadable value or a template with no
/// routine is ignored. Seeds the built-in routines first, so a fresh store has them.
pub fn apply_routine_settings(store: &memory::Store, fields: &HashMap<String, SettingsField>) -> rusqlite::Result<()> {
    store.seed_builtin_routines()?;
    let routines = store.list_routines()?;
    for template in CLOUD_TEMPLATES {
        let Some(routine) = routines.iter().find(|r| {
            serde_json::to_value(r.template).ok().and_then(|v| v.as_str().map(str::to_string)).as_deref() == Some(template)
        }) else {
            continue;
        };
        if let Some(on) = flag(fields, &format!("routine.{template}.enabled")) {
            store.set_routine_enabled(routine.id, on)?;
        }
        if let Some(at) = live(fields, &format!("routine.{template}.schedule")).filter(|v| is_hhmm(v)) {
            store.update_routine_schedule(routine.id, at)?;
        }
        if let Some(pref) = live(fields, &format!("routine.{template}.model_pref")).map(str::trim).filter(|v| is_model_pref(v)) {
            store.set_routine_model_pref(routine.id, pref)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn field(value: &str) -> SettingsField {
        SettingsField { value: value.to_string(), updated_at_unix_ms: 1, is_deleted: false }
    }

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, SettingsField> {
        pairs.iter().map(|(k, v)| (k.to_string(), field(v))).collect()
    }

    fn chain_providers(cfg: &RoutingConfig) -> Vec<llm::Provider> {
        let gate: Arc<dyn llm::guard::HostedGate> = Arc::new(llm::guard::NoExclusions);
        crate::routing::build_chain("", cfg, &gate).iter().map(|l| l.provider).collect()
    }

    #[test]
    fn routing_from_settings_uses_only_synced_opt_in_keys() {
        let f = fields(&[
            ("routing.preferredProvider", "openai"),
            ("routing.openaiDefaultModel", "gpt-x"),
            ("apiKey.openai", "sk-openai-secret"),
            ("apiKey.openai.syncEnabled", "false"),
            ("apiKey.anthropic", "sk-anthropic-secret"),
            ("apiKey.anthropic.syncEnabled", "true"),
            ("apiKey.google", "g-secret-without-opt-in"),
            ("routing.userDailyCapCents", "250"),
            ("routing.globalDailyCapCents", "oops"),
        ]);
        let cfg = routing_from_settings(&f);
        assert_eq!(cfg.anthropic_key.as_deref(), Some("sk-anthropic-secret"));
        assert!(cfg.openai_key.is_none());
        assert!(cfg.google_key.is_none());
        assert!(!cfg.use_local_model);
        assert!(cfg.whimbrel_id_token.is_none());
        assert_eq!(cfg.user_daily_cap_cents, Some(250));
        assert_eq!(cfg.global_daily_cap_cents, None);
        assert_eq!(chain_providers(&cfg).into_iter().filter(|p| !p.is_local()).collect::<Vec<_>>(), vec![llm::Provider::Anthropic]);
        let debug = format!("{cfg:?}");
        assert!(!debug.contains("secret"), "{debug}");
    }

    #[test]
    fn a_deleted_key_or_opt_in_is_not_used() {
        let mut f = fields(&[("apiKey.anthropic", "k"), ("apiKey.anthropic.syncEnabled", "true")]);
        f.get_mut("apiKey.anthropic").unwrap().is_deleted = true;
        assert!(routing_from_settings(&f).anthropic_key.is_none());
        let mut f = fields(&[("apiKey.anthropic", "k"), ("apiKey.anthropic.syncEnabled", "true")]);
        f.get_mut("apiKey.anthropic.syncEnabled").unwrap().is_deleted = true;
        assert!(routing_from_settings(&f).anthropic_key.is_none());
    }

    #[test]
    fn a_local_only_setting_yields_a_config_with_no_hosted_link() {
        let f = fields(&[
            ("routing.localOnlyMode", "true"),
            ("routing.preferredProvider", "anthropic"),
            ("apiKey.anthropic", "k"),
            ("apiKey.anthropic.syncEnabled", "true"),
        ]);
        let cfg = routing_from_settings(&f);
        assert!(cfg.sensitive);
        assert!(chain_providers(&cfg).iter().all(|p| p.is_local()), "{:?}", chain_providers(&cfg));
    }

    #[test]
    fn cloud_policy_lists_only_templates_flagged_true() {
        let mut f = fields(&[
            ("routine.daily_brief.cloud", "true"),
            ("routine.eod_recap.cloud", "false"),
            ("routine.weekly_digest.cloud", "yes"),
            ("routine.meeting_prep.cloud", "true"),
            ("routine.reply_priorities.cloud", "true"),
        ]);
        f.get_mut("routine.meeting_prep.cloud").unwrap().is_deleted = true;
        let policy = cloud_policy_from_settings(&f);
        assert_eq!(policy.cloud_templates, vec!["daily_brief".to_string()]);
    }

    #[test]
    fn cloud_policy_reads_each_cloud_templates_ready_by() {
        let mut f = fields(&[
            ("routine.daily_brief.cloud", "true"),
            ("routine.daily_brief.readyBy", "07:00"),
            ("routine.eod_recap.readyBy", " 18:15 "),
            ("routine.weekly_digest.readyBy", "7am"),
            ("routine.meeting_prep.readyBy", "06:00"),
            ("routine.reply_priorities.readyBy", "07:30"),
        ]);
        f.get_mut("routine.meeting_prep.readyBy").unwrap().is_deleted = true;
        let policy = cloud_policy_from_settings(&f);
        let expected: HashMap<String, String> =
            [("daily_brief", "07:00"), ("eod_recap", "18:15")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(policy.ready_by, expected);
        assert_eq!(cloud_policy_at(&f, 0, 1).ready_by, expected);
    }

    #[test]
    fn runner_fresh_is_false_past_the_grace_period_and_when_the_field_is_missing() {
        let grace = 10 * 60_000;
        let now = 1_000_000_000;
        let (key, beat) = heartbeat_field(now - 5 * 60_000);
        assert_eq!(key, "cloud.runner.lastRun");
        let mut f = HashMap::from([(key.clone(), beat)]);
        assert!(runner_fresh(&f, now, grace));
        assert!(cloud_policy_at(&f, now, grace).runner_fresh);
        assert!(!runner_fresh(&f, now + 6 * 60_000, grace));
        assert!(!runner_fresh(&HashMap::new(), now, grace));
        f.get_mut(&key).unwrap().is_deleted = true;
        assert!(!runner_fresh(&f, now, grace));
        f.insert(key, field("not a number"));
        assert!(!runner_fresh(&f, now, grace));
    }

    #[test]
    fn a_heartbeat_far_in_the_future_is_not_fresh() {
        let now = 1_000_000_000;
        for ts in [now + 6 * 60_000, now + 3_600_000, i64::MAX] {
            let f = HashMap::from([heartbeat_field(ts)]);
            assert!(!runner_fresh(&f, now, 600_000), "{ts}");
            assert!(!cloud_policy_at(&f, now, 600_000).runner_fresh);
        }
    }

    #[test]
    fn a_heartbeat_a_few_seconds_in_the_future_is_fresh() {
        let now = 1_000_000_000;
        let f = HashMap::from([heartbeat_field(now + 5_000)]);
        assert!(runner_fresh(&f, now, 600_000));
        assert!(cloud_policy_at(&f, now, 600_000).runner_fresh);
    }

    #[test]
    fn age_equal_to_grace_is_stale() {
        let now = 1_000_000_000;
        let f = HashMap::from([heartbeat_field(now - 600_000)]);
        assert!(!runner_fresh(&f, now, 600_000));
    }

    #[test]
    fn timezone_missing_returns_none() {
        assert_eq!(timezone_from_settings(&HashMap::new()), None);
        assert_eq!(timezone_from_settings(&fields(&[("profile.timeZone", "  ")])), None);
        assert_eq!(timezone_from_settings(&fields(&[("profile.timeZone", "Europe/Paris")])).as_deref(), Some("Europe/Paris"));
    }

    fn routine_of(store: &memory::Store, t: memory::RoutineTemplate) -> memory::Routine {
        store.list_routines().unwrap().into_iter().find(|r| r.template == t).unwrap()
    }

    #[test]
    fn apply_routine_settings_enables_only_flagged_eligible_templates_and_sets_schedule_and_model_pref() {
        let store = memory::Store::open_in_memory().unwrap();
        let f = fields(&[
            ("routine.eod_recap.enabled", "true"),
            ("routine.eod_recap.schedule", "18:15"),
            ("routine.eod_recap.model_pref", "cheapest"),
            ("routine.daily_brief.enabled", "false"),
            ("routine.meeting_prep.model_pref", "anthropic/claude-x"),
        ]);
        apply_routine_settings(&store, &f).unwrap();
        let eod = routine_of(&store, memory::RoutineTemplate::EodRecap);
        assert!(eod.enabled);
        assert_eq!(eod.schedule_hhmm, "18:15");
        assert_eq!(eod.model_pref, "cheapest");
        assert!(!routine_of(&store, memory::RoutineTemplate::DailyBrief).enabled);
        assert_eq!(routine_of(&store, memory::RoutineTemplate::MeetingPrep).model_pref, "anthropic/claude-x");
        assert!(!routine_of(&store, memory::RoutineTemplate::WeeklyDigest).enabled);
    }

    #[test]
    fn a_malformed_schedule_is_ignored() {
        let store = memory::Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        let before = routine_of(&store, memory::RoutineTemplate::DailyBrief).schedule_hhmm;
        for bad in ["25:00", "8:30", "08:60", "0830", "ab:cd", ""] {
            apply_routine_settings(&store, &fields(&[("routine.daily_brief.schedule", bad), ("routine.daily_brief.model_pref", "bogus")])).unwrap();
        }
        let r = routine_of(&store, memory::RoutineTemplate::DailyBrief);
        assert_eq!(r.schedule_hhmm, before);
        assert_eq!(r.model_pref, "default");
    }

    #[test]
    fn reply_priorities_settings_are_ignored() {
        let store = memory::Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        let before = routine_of(&store, memory::RoutineTemplate::ReplyPriorities);
        let f = fields(&[
            ("routine.reply_priorities.enabled", "true"),
            ("routine.reply_priorities.schedule", "01:00"),
            ("routine.reply_priorities.model_pref", "cheapest"),
        ]);
        apply_routine_settings(&store, &f).unwrap();
        let after = routine_of(&store, memory::RoutineTemplate::ReplyPriorities);
        assert_eq!(after.enabled, before.enabled);
        assert_eq!(after.schedule_hhmm, before.schedule_hhmm);
        assert_eq!(after.model_pref, before.model_pref);
    }
}
