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

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub routing: RoutingSettings,
    pub keys: Vec<ProviderKey>,
    /// Extra instructions for every summary (`summary.customPromptInstructions`).
    pub summary_instructions: String,
    pub prompts: Vec<PromptText>,
    pub spend: Option<SpendView>,
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
        Provider::Ollama => None,
    }
}

fn hosted(provider: Provider) -> api::hosted_models::HostedProvider {
    use api::hosted_models::HostedProvider;
    match provider {
        Provider::Anthropic => HostedProvider::Anthropic,
        Provider::Openai => HostedProvider::OpenAi,
        Provider::Google => HostedProvider::Google,
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
    [Provider::Anthropic, Provider::Openai, Provider::Google]
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
    let spend = match &state.store {
        Some(store) => {
            let today = store.call(|s| api::spend::spend_today(s, None, now_ms())).await?;
            Some(SpendView { user_cents: today.user_cents, global_cents: today.global_cents })
        }
        None => None,
    };
    Ok(AiSettings { routing, keys: api_keys(&*state.secrets)?, summary_instructions, prompts, spend })
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
        assert_eq!(key_account(Provider::Ollama), None);
    }
}
