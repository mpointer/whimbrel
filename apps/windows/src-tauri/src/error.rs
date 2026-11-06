//! The one error type every Tauri command returns (plan Decision 10).
//!
//! `api::ApiError`, `llm::SpendCapError` and the sync error types have no serde, so
//! they are mapped here. `SpendCap` and `ProviderFailure` serialize to exactly the
//! `error` object of `api::envelope_json`'s envelope, so the UI handles a spend cap the
//! way the Mac does; the other kinds carry a human-readable `message`.

use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpendScope {
    User,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandError {
    /// A daily spend cap rejected the call.
    SpendCap { scope: SpendScope, spent_cents: i64, cap_cents: i64 },
    /// Every provider in the chain failed for a reason other than the cap. `reason` is the one
    /// worth acting on (core #575, `api::last_provider_failure`), when core knows it.
    ProviderFailure {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<ProviderFailureReason>,
    },
    /// Every provider the call could use is over its monthly budget (`api::budget`).
    /// `provider` is `llm::Provider::as_str` ("openai", "anthropic", ...).
    BudgetSpent { provider: String, spent_cents: i64, budget_cents: i64 },
    /// The store actor failed: it could not open, it stopped, or a job panicked.
    Store { message: String },
    /// Credential Manager refused a read or write.
    Secrets { message: String },
    /// A network call (Worker, sync database, provider model list) failed.
    Network { message: String },
    /// A required setting or build-time secret is missing.
    NotConfigured { message: String },
    /// The UI sent something the shell can't act on.
    InvalidInput { message: String },
}

/// Why a provider failed, as core's `llm::ProviderFailureKind` serializes it (`provider` is
/// `Provider::as_str`). Never the provider's own error text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderFailureReason {
    OllamaNotRunning,
    OllamaModelMissing { model: String },
    NoProviderConfigured,
    Unauthorized { provider: String },
    RateLimited { provider: String },
    Network { provider: String },
    Other { provider: String, status: Option<u16> },
}

impl From<llm::ProviderFailureKind> for ProviderFailureReason {
    fn from(kind: llm::ProviderFailureKind) -> Self {
        use llm::ProviderFailureKind as K;
        match kind {
            K::OllamaNotRunning => Self::OllamaNotRunning,
            K::OllamaModelMissing { model } => Self::OllamaModelMissing { model },
            K::NoProviderConfigured => Self::NoProviderConfigured,
            K::Unauthorized { provider } => Self::Unauthorized { provider: provider.as_str().into() },
            K::RateLimited { provider } => Self::RateLimited { provider: provider.as_str().into() },
            K::Network { provider } => Self::Network { provider: provider.as_str().into() },
            K::Other { provider, status } => Self::Other { provider: provider.as_str().into(), status },
        }
    }
}

impl ProviderFailureReason {
    /// The sentence a log line, a summary-failure marker or another non-UI surface shows, in the UI
    /// language. Network and other failures keep the general line.
    pub fn message(&self) -> Option<String> {
        use crate::l10n::tr;
        let name = |p: &str| match p {
            "anthropic" => "Anthropic".to_string(),
            "openai" => "OpenAI".to_string(),
            "google" => "Google".to_string(),
            "openrouter" => "OpenRouter".to_string(),
            other => other.to_string(),
        };
        Some(match self {
            Self::OllamaModelMissing { model } => tr(
                "Ollama doesn't have the model {model}. Pick one you have in Settings → AI, or run: ollama pull {model}",
                &[("model", model)],
            ),
            Self::OllamaNotRunning => tr("Ollama isn't running on this computer. Start Ollama, or add an API key in Settings → AI.", &[]),
            Self::NoProviderConfigured => tr("No AI provider is set up. Add one in Settings → AI.", &[]),
            Self::Unauthorized { provider } => tr("{provider} rejected the API key. Check it in Settings → AI.", &[("provider", &name(provider))]),
            Self::RateLimited { provider } => tr("{provider} is busy or out of credit. Try again shortly.", &[("provider", &name(provider))]),
            Self::Network { .. } | Self::Other { .. } => return None,
        })
    }
}

impl CommandError {
    /// A provider failure with no reason known.
    pub fn provider_failure() -> Self {
        CommandError::ProviderFailure { reason: None }
    }

    pub fn store(message: impl Into<String>) -> Self {
        CommandError::Store { message: message.into() }
    }

    pub fn secrets(message: impl Into<String>) -> Self {
        CommandError::Secrets { message: message.into() }
    }

    pub fn network(message: impl Into<String>) -> Self {
        CommandError::Network { message: message.into() }
    }

    pub fn not_configured(message: impl Into<String>) -> Self {
        CommandError::NotConfigured { message: message.into() }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        CommandError::InvalidInput { message: message.into() }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::SpendCap { scope, spent_cents, cap_cents } => {
                write!(f, "{scope:?} daily spend cap reached: {spent_cents}¢ of {cap_cents}¢")
            }
            CommandError::ProviderFailure { reason } => match reason.as_ref().and_then(ProviderFailureReason::message) {
                Some(message) => f.write_str(&message),
                None => write!(f, "every model provider failed"),
            },
            CommandError::BudgetSpent { provider, spent_cents, budget_cents } => {
                write!(f, "{provider} monthly budget spent: {spent_cents}¢ of {budget_cents}¢")
            }
            CommandError::Store { message }
            | CommandError::Secrets { message }
            | CommandError::Network { message }
            | CommandError::NotConfigured { message }
            | CommandError::InvalidInput { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<api::ApiError> for CommandError {
    fn from(err: api::ApiError) -> Self {
        match err {
            api::ApiError::SpendCap(cap) => CommandError::SpendCap {
                scope: match cap.scope {
                    llm::SpendCapScope::User => SpendScope::User,
                    llm::SpendCapScope::Global => SpendScope::Global,
                },
                spent_cents: cap.spent_cents,
                cap_cents: cap.cap_cents,
            },
            // Take-once: core leaves the walk's reason right before it returns this.
            api::ApiError::ProviderFailure => CommandError::ProviderFailure { reason: api::last_provider_failure().map(Into::into) },
            api::ApiError::BudgetSpent(spent) => CommandError::BudgetSpent {
                provider: spent.provider.as_str().to_string(),
                spent_cents: spent.spent_cents,
                budget_cents: spent.budget_cents,
            },
        }
    }
}

impl From<api::sync_ops::enrollment::WorkerError> for CommandError {
    fn from(err: api::sync_ops::enrollment::WorkerError) -> Self {
        CommandError::network(api::sync_ops::enrollment::user_message_in(&err, &crate::l10n::ui_language()))
    }
}

impl From<api::sync_ops::coordinators::SyncEngineError> for CommandError {
    fn from(err: api::sync_ops::coordinators::SyncEngineError) -> Self {
        CommandError::network(err.to_string())
    }
}

impl From<crate::secrets::SecretError> for CommandError {
    fn from(err: crate::secrets::SecretError) -> Self {
        CommandError::secrets(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `error` object inside `api::envelope_json`'s `{"ok":false,…}`.
    fn envelope_error(err: api::ApiError) -> serde_json::Value {
        let result: api::ApiResult<()> = Err(err);
        let envelope: serde_json::Value = serde_json::from_str(&api::envelope::envelope_json(&result).unwrap()).unwrap();
        envelope["error"].clone()
    }

    #[test]
    fn spend_cap_matches_the_envelope_the_mac_decodes() {
        for scope in [llm::SpendCapScope::User, llm::SpendCapScope::Global] {
            let err = api::ApiError::SpendCap(llm::SpendCapError { spent_cents: 42, cap_cents: 200, scope });
            assert_eq!(serde_json::to_value(CommandError::from(err)).unwrap(), envelope_error(err));
        }
    }

    #[test]
    fn budget_spent_matches_the_envelope() {
        let spent = || api::ApiError::BudgetSpent(api::budget::BudgetSpent { provider: llm::Provider::OpenRouter, spent_cents: 2512, budget_cents: 2500 });
        assert_eq!(serde_json::to_value(CommandError::from(spent())).unwrap(), envelope_error(spent()));
    }

    #[test]
    fn a_provider_failure_carries_why_and_says_it() {
        let missing: ProviderFailureReason = llm::ProviderFailureKind::OllamaModelMissing { model: "gemma4:12b".into() }.into();
        let err = CommandError::ProviderFailure { reason: Some(missing) };
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            serde_json::json!({"kind": "provider_failure", "reason": {"kind": "ollama_model_missing", "model": "gemma4:12b"}})
        );
        assert_eq!(
            err.to_string(),
            "Ollama doesn't have the model gemma4:12b. Pick one you have in Settings → AI, or run: ollama pull gemma4:12b"
        );
        let key: ProviderFailureReason = llm::ProviderFailureKind::Unauthorized { provider: llm::Provider::OpenAI }.into();
        assert_eq!(serde_json::to_value(&key).unwrap(), serde_json::json!({"kind": "unauthorized", "provider": "openai"}));
        assert_eq!(key.message().unwrap(), "OpenAI rejected the API key. Check it in Settings → AI.");
        let net = CommandError::ProviderFailure { reason: Some(ProviderFailureReason::Network { provider: "google".into() }) };
        assert_eq!(net.to_string(), "every model provider failed", "a network blip keeps the general line");
        assert_eq!(serde_json::to_value(CommandError::provider_failure()).unwrap(), serde_json::json!({"kind": "provider_failure"}));
    }

    #[test]
    fn provider_failure_matches_the_envelope() {
        let err = api::ApiError::ProviderFailure;
        assert_eq!(serde_json::to_value(CommandError::from(err)).unwrap(), envelope_error(err));
    }

    #[test]
    fn other_kinds_carry_a_message() {
        assert_eq!(
            serde_json::to_value(CommandError::store("stopped")).unwrap(),
            serde_json::json!({"kind": "store", "message": "stopped"})
        );
        assert_eq!(
            serde_json::to_value(CommandError::not_configured("no Secrets.toml")).unwrap(),
            serde_json::json!({"kind": "not_configured", "message": "no Secrets.toml"})
        );
    }

    #[test]
    fn worker_errors_become_network_errors() {
        use api::sync_ops::enrollment::WorkerError;
        let err = WorkerError::Http { status: 500, body: String::new() };
        let e: CommandError = err.clone().into();
        assert_eq!(e, CommandError::network(api::sync_ops::enrollment::user_message(&err)));
        assert_eq!(e, CommandError::network("Sync is temporarily unavailable (server error 500)."));
        assert!(matches!(CommandError::from(WorkerError::Network), CommandError::Network { .. }));
    }
}
