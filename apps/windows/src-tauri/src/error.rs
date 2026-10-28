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
    /// Every provider in the chain failed for a reason other than the cap.
    ProviderFailure,
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

impl CommandError {
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
            CommandError::ProviderFailure => write!(f, "every model provider failed"),
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
            api::ApiError::ProviderFailure => CommandError::ProviderFailure,
        }
    }
}

impl From<api::sync_ops::enrollment::WorkerError> for CommandError {
    fn from(err: api::sync_ops::enrollment::WorkerError) -> Self {
        use api::sync_ops::enrollment::WorkerError;
        CommandError::network(match err {
            WorkerError::Http { status, .. } => format!("the sync server returned HTTP {status}"),
            WorkerError::Network => "could not reach the sync server".to_string(),
            WorkerError::MalformedResponse => "the sync server's response was malformed".to_string(),
        })
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
        let e: CommandError = WorkerError::Http { status: 403, body: String::new() }.into();
        assert_eq!(e, CommandError::network("the sync server returned HTTP 403"));
        assert!(matches!(CommandError::from(WorkerError::Network), CommandError::Network { .. }));
    }
}
