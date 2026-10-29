//! Settings → AI & Routing (plan Task 7): provider choice, default models, API keys
//! in Credential Manager, spend caps, local-only mode, per-surface prompts, and
//! today's spend.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
use crate::secrets::{accounts, SecretStore, APP_SERVICE};
use crate::settings::{keys, PromptSurface, Provider, RoutingSettings};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKey {
    pub provider: Provider,
    /// The key with all but its last four characters hidden; `None` when unset.
    pub masked: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptText {
    pub surface: PromptSurface,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpendView {
    pub user_cents: i64,
    pub global_cents: i64,
}

/// One row of the key on-ramp table (core/api::providers): where to get a key, its data
/// policy, what a key looks like, and the dated data-terms note where a free tier may
/// use prompts.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfoView {
    pub id: Provider,
    pub name: String,
    pub key_page_url: Option<String>,
    pub policy_url: Option<String>,
    pub key_prefix: Option<String>,
    pub data_terms: Option<DataTermsView>,
    /// The light-tier model; empty where the user picks it (OpenRouter) or there is none.
    pub cheap_model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataTermsView {
    pub text: String,
    /// ISO date the policy page was last read.
    pub checked: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum KeyCheckStatus {
    Valid,
    /// 401 or 403: the provider rejected the key.
    Refused,
    /// No answer at all. The key is still saved; models load when online.
    Unreachable,
    Other,
}

/// What saving a key found out (core/api::hosted_models::validate_key).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheckView {
    pub status: KeyCheckStatus,
    /// The one line the key row shows.
    pub message: String,
    pub models: Vec<String>,
}

/// Spend in cents for the Model usage section (core/api::usage::totals), UTC days.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageView {
    pub today_cents: i64,
    pub last_30_days_cents: i64,
    pub month_to_date_cents: i64,
    pub by_provider: Vec<ProviderSpendView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSpendView {
    pub provider: String,
    pub cents: i64,
}

/// One call as the Model usage list shows it (core/api::usage::UsageRow).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageRowView {
    pub ts_unix_ms: i64,
    pub task: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub estimated_cost_cents: i64,
    /// A local call is not metered; the list says "local", never a zero.
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub routing: RoutingSettings,
    pub keys: Vec<ProviderKey>,
    /// Extra instructions for every summary (`summary.customPromptInstructions`).
    pub summary_instructions: String,
    pub prompts: Vec<PromptText>,
    pub spend: Option<SpendView>,
    pub providers: Vec<ProviderInfoView>,
    pub usage: Option<UsageView>,
    /// The Brave Search key the skin maker finds images with, masked; `None` when unset.
    pub image_search_key: Option<String>,
}

/// The on-ramp table as the UI reads it, in core's order.
pub fn provider_infos() -> Vec<ProviderInfoView> {
    api::providers::PROVIDERS
        .iter()
        .filter_map(|p| {
            let id = match p.id {
                "ollama" => Provider::Ollama,
                "anthropic" => Provider::Anthropic,
                "openai" => Provider::Openai,
                "google" => Provider::Google,
                "openrouter" => Provider::Openrouter,
                _ => return None,
            };
            Some(ProviderInfoView {
                id,
                name: p.name.to_string(),
                key_page_url: p.key_page_url.map(str::to_string),
                policy_url: p.policy_url.map(str::to_string),
                key_prefix: p.key_prefix.map(str::to_string),
                data_terms: p.data_terms.as_ref().map(|t| DataTermsView { text: t.text.to_string(), checked: t.checked.to_string() }),
                cheap_model: p.cheap_model.to_string(),
            })
        })
        .collect()
}

fn usage_view(totals: api::usage::UsageTotals) -> UsageView {
    UsageView {
        today_cents: totals.today_cents,
        last_30_days_cents: totals.last_30_days_cents,
        month_to_date_cents: totals.month_to_date_cents,
        by_provider: totals.last_30_days_by_provider.into_iter().map(|p| ProviderSpendView { provider: p.provider, cents: p.cents }).collect(),
    }
}

fn key_check_view(check: api::hosted_models::KeyCheck) -> (KeyCheckView, Vec<api::hosted_models::OpenRouterModel>) {
    use api::hosted_models::KeyCheck;
    match check {
        KeyCheck::Valid { models, prices } => {
            let message = format!("Saved. {} model{} available.", models.len(), if models.len() == 1 { "" } else { "s" });
            (KeyCheckView { status: KeyCheckStatus::Valid, message, models }, prices)
        }
        KeyCheck::Refused { message } => (KeyCheckView { status: KeyCheckStatus::Refused, message, models: vec![] }, vec![]),
        KeyCheck::Unreachable { message } => (KeyCheckView { status: KeyCheckStatus::Unreachable, message, models: vec![] }, vec![]),
        KeyCheck::Other { message } => (KeyCheckView { status: KeyCheckStatus::Other, message, models: vec![] }, vec![]),
    }
}

/// Shows only the last four characters, so a pasted key can be recognized but not read.
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".to_string();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("••••••••{tail}")
}

fn key_account(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Anthropic => Some(accounts::ANTHROPIC_API_KEY),
        Provider::Openai => Some(accounts::OPENAI_API_KEY),
        Provider::Google => Some(accounts::GOOGLE_API_KEY),
        Provider::Openrouter => Some(accounts::OPENROUTER_API_KEY),
        Provider::Ollama => None,
    }
}

fn hosted(provider: Provider) -> api::hosted_models::HostedProvider {
    use api::hosted_models::HostedProvider;
    match provider {
        Provider::Anthropic => HostedProvider::Anthropic,
        Provider::Openai => HostedProvider::OpenAi,
        Provider::Google => HostedProvider::Google,
        Provider::Openrouter => HostedProvider::OpenRouter,
        Provider::Ollama => HostedProvider::Ollama,
    }
}

/// Saves `key` for a hosted provider, or removes it when `None` or blank.
fn save_api_key(secrets: &dyn SecretStore, provider: Provider, key: Option<&str>) -> Result<Vec<ProviderKey>, CommandError> {
    let account = key_account(provider).ok_or_else(|| CommandError::invalid_input("Ollama doesn't use an API key."))?;
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => secrets.set_string(APP_SERVICE, account, k)?,
        None => secrets.delete(APP_SERVICE, account)?,
    }
    api_keys(secrets)
}

fn api_keys(secrets: &dyn SecretStore) -> Result<Vec<ProviderKey>, CommandError> {
    Provider::HOSTED
        .into_iter()
        .map(|provider| {
            let stored = secrets.get_string(APP_SERVICE, key_account(provider).unwrap())?;
            Ok(ProviderKey { provider, masked: stored.as_deref().map(mask_key) })
        })
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[tauri::command]
pub async fn ai_settings(state: State<'_, AppState>) -> Result<AiSettings, CommandError> {
    let (routing, summary_instructions, prompts) = {
        let settings = state.settings.lock().unwrap();
        let prompts = PromptSurface::ALL.iter().map(|&surface| PromptText { surface, text: settings.prompt(surface) }).collect();
        (settings.routing(), settings.get(keys::SUMMARY_CUSTOM_PROMPT).unwrap_or_default(), prompts)
    };
    let (spend, usage) = match &state.store {
        Some(store) => {
            let (today, totals) = store.call(|s| (api::spend::spend_today(s, None, now_ms()), api::usage::totals(s, now_ms()))).await?;
            (Some(SpendView { user_cents: today.user_cents, global_cents: today.global_cents }), Some(usage_view(totals)))
        }
        None => (None, None),
    };
    let image_search_key = state.secrets.get_string(APP_SERVICE, accounts::IMAGE_SEARCH_API_KEY)?.as_deref().map(mask_key);
    Ok(AiSettings { routing, keys: api_keys(&*state.secrets)?, summary_instructions, prompts, spend, providers: provider_infos(), usage, image_search_key })
}

/// Checks the saved key for `provider` against its model list; for OpenRouter a valid
/// key also refreshes the cached prices core bills from.
#[tauri::command]
pub async fn key_check(state: State<'_, AppState>, provider: Provider) -> Result<KeyCheckView, CommandError> {
    let account = key_account(provider).ok_or_else(|| CommandError::invalid_input("Ollama doesn't use an API key."))?;
    let key = state
        .secrets
        .get_string(APP_SERVICE, account)?
        .ok_or_else(|| CommandError::not_configured(format!("Add a {} API key first.", provider.as_str())))?;
    let check = tokio::task::spawn_blocking(move || api::hosted_models::validate_key(hosted(provider), &key, None))
        .await
        .map_err(|e| CommandError::network(e.to_string()))?;
    let (view, prices) = key_check_view(check);
    if provider == Provider::Openrouter && !prices.is_empty() {
        let mut settings = state.settings.lock().unwrap();
        settings.set_openrouter_prices(&prices.into_iter().map(|m| (m.id, (m.in_cents_per_1k, m.out_cents_per_1k))).collect());
        settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    }
    Ok(view)
}

/// The one-line warning when `key` does not look like `provider`'s key; `None` when it does.
#[tauri::command]
pub fn key_format_hint(provider: Provider, key: String) -> Option<String> {
    api::providers::format_hint(provider.as_str(), &key)
}

/// The pages the key on-ramp may open: every provider's key and policy page, and the
/// Ollama download. Anything else is refused, so the UI cannot be made to open an
/// arbitrary URL.
pub fn external_url_allowed(url: &str) -> bool {
    url == "https://ollama.com/download"
        || api::providers::PROVIDERS.iter().any(|p| p.key_page_url == Some(url) || p.policy_url == Some(url))
}

/// Opens one of the on-ramp's pages in the default browser.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), CommandError> {
    if !external_url_allowed(&url) {
        return Err(CommandError::invalid_input("That page is not one Whimbrel opens."));
    }
    crate::syncing::controller::open_in_browser(&url).map_err(CommandError::invalid_input)
}

/// The newest calls of the last `days` days (at most 200), newest first.
#[tauri::command]
pub async fn usage_rows(state: State<'_, AppState>, days: Option<i64>) -> Result<Vec<UsageRowView>, CommandError> {
    let now = now_ms();
    let since = now - days.unwrap_or(7).clamp(1, 90) * 24 * 60 * 60 * 1000;
    let rows = state.store()?.call(move |s| api::usage::list(s, since, now + 1, 200)).await?;
    Ok(rows
        .into_iter()
        .map(|r| UsageRowView {
            ts_unix_ms: r.ts_unix_ms,
            task: r.task,
            provider: r.provider,
            model: r.model,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            estimated_cost_cents: r.estimated_cost_cents,
            local: r.local,
        })
        .collect())
}

/// Blank text removes the key (no extra summary instructions).
#[tauri::command]
pub fn set_summary_instructions(state: State<'_, AppState>, text: String) -> Result<(), CommandError> {
    let mut settings = state.settings.lock().unwrap();
    let text = text.trim();
    settings.set(keys::SUMMARY_CUSTOM_PROMPT, (!text.is_empty()).then_some(text));
    settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))
}

/// Saves or (with `None` / blank) removes a hosted provider's key.
#[tauri::command]
pub fn set_api_key(state: State<'_, AppState>, provider: Provider, key: Option<String>) -> Result<Vec<ProviderKey>, CommandError> {
    save_api_key(&*state.secrets, provider, key.as_deref())
}

/// Saves or (with `None` / blank) removes the image search key; returns it masked. It stays on this
/// device, as the provider keys do.
#[tauri::command]
pub fn set_image_search_key(state: State<'_, AppState>, key: Option<String>) -> Result<Option<String>, CommandError> {
    save_image_search_key(&*state.secrets, key.as_deref())
}

fn save_image_search_key(secrets: &dyn SecretStore, key: Option<&str>) -> Result<Option<String>, CommandError> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => secrets.set_string(APP_SERVICE, accounts::IMAGE_SEARCH_API_KEY, k)?,
        None => secrets.delete(APP_SERVICE, accounts::IMAGE_SEARCH_API_KEY)?,
    }
    Ok(secrets.get_string(APP_SERVICE, accounts::IMAGE_SEARCH_API_KEY)?.as_deref().map(mask_key))
}

#[tauri::command]
pub fn set_prompt(state: State<'_, AppState>, surface: PromptSurface, text: String) -> Result<(), CommandError> {
    let mut settings = state.settings.lock().unwrap();
    settings.set_prompt(surface, &text);
    settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))
}

/// The provider's model list (a network call; Ollama is asked on localhost).
#[tauri::command]
pub async fn provider_models(state: State<'_, AppState>, provider: Provider) -> Result<Vec<String>, CommandError> {
    let key = match key_account(provider) {
        Some(account) => state
            .secrets
            .get_string(APP_SERVICE, account)?
            .ok_or_else(|| CommandError::not_configured(format!("Add a {} API key first.", provider.as_str())))?,
        None => String::new(),
    };
    tokio::task::spawn_blocking(move || api::hosted_models::fetch_models(hosted(provider), &key, None))
        .await
        .map_err(|e| CommandError::network(e.to_string()))?
        .map_err(CommandError::network)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_masked_to_their_last_four_characters() {
        assert_eq!(mask_key("sk-ant-api03-abcdefgh1234"), "••••••••1234");
        assert_eq!(mask_key("short"), "••••••••", "short keys reveal nothing");
        assert!(!mask_key("sk-ant-api03-secretpart-1234").contains("secretpart"));
    }

    #[test]
    fn a_saved_key_reads_back_masked_and_blank_removes_it() {
        let secrets = crate::secrets::MemorySecrets::default();
        let keys = save_api_key(&secrets, Provider::Anthropic, Some("  sk-ant-api03-secretpart-9876 \n")).unwrap();
        let anthropic = keys.iter().find(|k| k.provider == Provider::Anthropic).unwrap();
        assert_eq!(anthropic.masked.as_deref(), Some("••••••••9876"));
        assert!(keys.iter().filter(|k| k.provider != Provider::Anthropic).all(|k| k.masked.is_none()));
        // Stored trimmed, under the Mac's account name, where RoutingConfig reads it.
        assert_eq!(secrets.get_string(APP_SERVICE, "anthropic-api-key").unwrap().as_deref(), Some("sk-ant-api03-secretpart-9876"));

        let keys = save_api_key(&secrets, Provider::Anthropic, Some("   ")).unwrap();
        assert!(keys.iter().all(|k| k.masked.is_none()));
        assert_eq!(secrets.get(APP_SERVICE, "anthropic-api-key").unwrap(), None);
        assert!(save_api_key(&secrets, Provider::Ollama, Some("x")).is_err(), "Ollama has no key");
    }

    #[test]
    fn the_image_search_key_saves_under_the_macs_account() {
        let secrets = crate::secrets::MemorySecrets::default();
        assert_eq!(save_image_search_key(&secrets, Some(" BSA-brave-key-1234 ")).unwrap().as_deref(), Some("••••••••1234"));
        assert_eq!(secrets.get_string(APP_SERVICE, "image-search-api-key").unwrap().as_deref(), Some("BSA-brave-key-1234"));
        assert_eq!(save_image_search_key(&secrets, None).unwrap(), None);
        assert_eq!(secrets.get(APP_SERVICE, "image-search-api-key").unwrap(), None);
    }

    /// `cargo test --lib -- --ignored saved_keys_load_model_lists`. Fetches each hosted
    /// provider's model list with the key saved in the real Credential Manager, the way
    /// Settings → Load models does. Prints only a count or the display-ready error,
    /// never a key.
    #[test]
    #[ignore = "reads the real Credential Manager and calls hosted provider APIs"]
    fn saved_keys_load_model_lists() {
        let secrets = crate::secrets::CredentialManager;
        for provider in [Provider::Anthropic, Provider::Openai, Provider::Google] {
            let Some(key) = secrets.get_string(APP_SERVICE, key_account(provider).unwrap()).unwrap() else {
                println!("{}: no key saved", provider.as_str());
                continue;
            };
            let shape = format!("{} chars, trimmed={}", key.chars().count(), key.trim() == key);
            match api::hosted_models::fetch_models(hosted(provider), &key, None) {
                Ok(models) => println!("{} ({shape}): {} models, first {:?}", provider.as_str(), models.len(), models.iter().take(3).collect::<Vec<_>>()),
                Err(e) => println!("{} ({shape}): {e}", provider.as_str()),
            }
        }
    }

    #[test]
    fn every_hosted_provider_has_the_macs_keychain_account() {
        assert_eq!(key_account(Provider::Anthropic), Some("anthropic-api-key"));
        assert_eq!(key_account(Provider::Openai), Some("openai-api-key"));
        assert_eq!(key_account(Provider::Google), Some("google-api-key"));
        assert_eq!(key_account(Provider::Openrouter), Some("openrouter-api-key"));
        assert_eq!(key_account(Provider::Ollama), None);
        let secrets = crate::secrets::MemorySecrets::default();
        assert_eq!(api_keys(&secrets).unwrap().iter().map(|k| k.provider).collect::<Vec<_>>(), Provider::HOSTED.to_vec());
    }

    #[test]
    fn the_provider_table_reaches_the_ui_with_every_core_row() {
        let infos = provider_infos();
        assert_eq!(infos.len(), api::providers::PROVIDERS.len(), "a core row with an id this enum lacks would be dropped silently");
        let google = infos.iter().find(|p| p.id == Provider::Google).unwrap();
        assert!(google.key_page_url.as_deref().unwrap().starts_with("https://"));
        assert_eq!(google.data_terms.as_ref().unwrap().checked.len(), 10);
        assert_eq!(google.cheap_model, "gemini-2.0-flash");
        assert_eq!(infos.iter().find(|p| p.id == Provider::Ollama).unwrap().key_page_url, None);
    }

    #[test]
    fn key_checks_map_to_their_status_and_line() {
        use api::hosted_models::{KeyCheck, OpenRouterModel};
        let (view, prices) = key_check_view(KeyCheck::Valid { models: vec!["a".into()], prices: vec![] });
        assert_eq!((view.status, view.message.as_str(), prices.len()), (KeyCheckStatus::Valid, "Saved. 1 model available.", 0));
        let (view, prices) = key_check_view(KeyCheck::Valid {
            models: vec!["v/m".into(), "v/n".into()],
            prices: vec![OpenRouterModel { id: "v/m".into(), in_cents_per_1k: 0.1, out_cents_per_1k: 0.5 }],
        });
        assert_eq!(view.message, "Saved. 2 models available.");
        assert_eq!(prices[0].id, "v/m");
        let (view, _) = key_check_view(KeyCheck::Refused { message: "no".into() });
        assert_eq!((view.status, view.message.as_str()), (KeyCheckStatus::Refused, "no"));
        let (view, _) = key_check_view(KeyCheck::Unreachable { message: "offline".into() });
        assert_eq!(view.status, KeyCheckStatus::Unreachable);
        assert_eq!(key_format_hint(Provider::Google, "sk-abc".into()).unwrap(), "This doesn't look like a Google Gemini key. Keys from Google Gemini start with \"AIza\".");
        assert_eq!(key_format_hint(Provider::Google, "AIza-abc".into()), None);
    }

    #[test]
    fn only_the_on_ramps_own_pages_may_be_opened() {
        assert!(external_url_allowed("https://ollama.com/download"));
        assert!(external_url_allowed("https://aistudio.google.com/apikey"));
        assert!(external_url_allowed("https://openrouter.ai/privacy"));
        assert!(!external_url_allowed("https://example.com"));
        assert!(!external_url_allowed("http://ollama.com/download"));
        assert!(!external_url_allowed("file:///C:/Windows"));
    }
}
