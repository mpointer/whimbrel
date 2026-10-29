use serde::{Deserialize, Serialize};

use crate::{backoff::is_retryable, LlmClient, LlmError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Ollama,
    Anthropic,
    OpenAI,
    Google,
    /// One key, any model on OpenRouter's catalogue, through their OpenAI-compatible
    /// endpoint. Model ids are namespaced (`vendor/model`, `:free` for free ones).
    OpenRouter,
}

impl Provider {
    /// NFR-PRV-1: local-first — only Ollama runs entirely on-device.
    pub fn is_local(&self) -> bool {
        matches!(self, Provider::Ollama)
    }

    /// Lowercase provider name for logging/persistence (e.g. `usage_log.provider`).
    /// Lives here rather than duplicated at the FFI layer, since Settings UI
    /// (sub-project 2) will need this same mapping wherever `Provider` is consumed.
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Ollama => "ollama",
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
            Provider::Google => "google",
            Provider::OpenRouter => "openrouter",
        }
    }

    /// The inverse of [`as_str`](Self::as_str); `None` for an unknown name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ollama" => Some(Provider::Ollama),
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAI),
            "google" => Some(Provider::Google),
            "openrouter" => Some(Provider::OpenRouter),
            _ => None,
        }
    }
}

/// Which feature is making an LLM call. Decides the chain tier (`tier`) and names the
/// `usage_log.route` the call is logged under (`route`), so the two can never drift.
/// Light tasks produce short, low-stakes output where the cheapest model is fine;
/// everything the user reads is Standard. Routines are not a fixed tier: each routine
/// row carries its own model preference (see `api::routing::RoutineModelPref`).
/// Design: `docs/superpowers/specs/2026-10-29-cheap-first-routing-and-key-on-ramp-design.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    Chat,
    MeetingSummary,
    Digest,
    MeetingPrep,
    Routine,
    ConversationTitle,
    TopicName,
    ProfileSuggest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Standard,
    Light,
}

impl TaskKind {
    pub fn tier(self) -> Tier {
        match self {
            TaskKind::Chat
            | TaskKind::MeetingSummary
            | TaskKind::Digest
            | TaskKind::MeetingPrep
            | TaskKind::Routine => Tier::Standard,
            TaskKind::ConversationTitle | TaskKind::TopicName | TaskKind::ProfileSuggest => Tier::Light,
        }
    }

    /// The `usage_log.route` string every caller already logs; kept identical so the
    /// spend sums and the usage screen see one history.
    pub fn route(self) -> &'static str {
        match self {
            TaskKind::Chat => "ask",
            TaskKind::MeetingSummary => "meeting_summary",
            TaskKind::Digest => "digest",
            TaskKind::MeetingPrep => "routine",
            TaskKind::Routine => "routine",
            TaskKind::ConversationTitle => "conversation_title",
            TaskKind::TopicName => "topic_name",
            TaskKind::ProfileSuggest => "profile_suggest",
        }
    }
}

/// One link in a provider-chain (primary → fallback → ...). Holds a live client rather
/// than just a provider/model pair, since constructing a client needs a resolved API
/// key — the caller (FFI layer, per this plan's design) builds each link's client once
/// it has resolved that link's key.
pub struct ChainLink {
    pub provider: Provider,
    pub model: String,
    pub client: Box<dyn LlmClient>,
}

/// Walks `chain` in order (primary → fallback → ...), calling `attempt` for each link:
/// on success, returns immediately with the value and the link that produced it (so the
/// caller can log usage against the right provider/model). A failure that belongs to one
/// link ([`falls_through`]) moves on to the next link; a failure that belongs to the
/// request itself stops the walk, since every other link would reject it too. Returns
/// `None` once every link is exhausted (or `chain` is empty) — the callers report a
/// generic "provider failure" either way, so they never need the last error's contents.
pub fn try_chain<T>(
    chain: &[ChainLink],
    mut attempt: impl FnMut(&ChainLink) -> Result<T, LlmError>,
) -> Option<(T, &ChainLink)> {
    for link in chain {
        match attempt(link) {
            Ok(value) => return Some((value, link)),
            Err(e) if falls_through(&e) => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Whether a link's error is that link's own problem, so the next link may well succeed:
/// the retryable ones (429, 5xx), plus a provider that's unreachable or answered with
/// something unreadable, a key it refuses (401/403), a model it doesn't have (404), a
/// timeout (408), or a capability it lacks. The fallback links exist for exactly these.
/// Before, only 429/5xx moved on, so a local Ollama that wasn't running, or didn't have
/// the configured model, failed every call even with working hosted keys behind it (seen
/// on Windows: Ollama answered 404 for `gemma4:12b`, and Ask failed outright). A
/// request-shaped rejection (400, 413, 422, …) still stops the walk.
pub fn falls_through(err: &LlmError) -> bool {
    match err {
        LlmError::Network(_) | LlmError::Provider(_) | LlmError::NotImplemented(_) => true,
        LlmError::Http { status, .. } => {
            is_retryable(err) || matches!(status, 401 | 403 | 404 | 408)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Completion, TokenUsage};

    /// Answers, or fails with the error `fail` builds.
    struct Fake {
        fail: Option<fn() -> LlmError>,
    }

    impl LlmClient for Fake {
        fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
            match self.fail {
                Some(make) => Err(make()),
                None => Ok(Completion {
                    text: "answer".into(),
                    token_usage: TokenUsage::default(),
                }),
            }
        }
    }

    fn link(provider: Provider, fail: Option<fn() -> LlmError>) -> ChainLink {
        ChainLink {
            provider,
            model: "m".into(),
            client: Box::new(Fake { fail }),
        }
    }

    fn http(status: u16) -> LlmError {
        LlmError::Http {
            status,
            body: String::new(),
        }
    }

    /// Which provider answers when Ollama fails first and a working Anthropic follows.
    fn winner(first: fn() -> LlmError) -> Option<Provider> {
        let chain = [
            link(Provider::Ollama, Some(first)),
            link(Provider::Anthropic, None),
        ];
        try_chain(&chain, |l| l.client.complete("q")).map(|(_, l)| l.provider)
    }

    #[test]
    fn a_failure_specific_to_one_link_moves_on_to_the_next() {
        let cases: [fn() -> LlmError; 9] = [
            || LlmError::Network("connection refused".into()),
            || LlmError::Provider("unreadable response".into()),
            || LlmError::NotImplemented("streaming"),
            || http(401),
            || http(403),
            || http(404),
            || http(408),
            || http(429),
            || http(503),
        ];
        for case in cases {
            assert_eq!(winner(case), Some(Provider::Anthropic), "{}", case());
        }
    }

    #[test]
    fn a_request_shaped_rejection_stops_the_chain() {
        assert_eq!(
            winner(|| http(400)),
            None,
            "the next link would reject the same request"
        );
        for status in [400, 413, 422] {
            assert!(!falls_through(&http(status)), "{status}");
        }
    }

    #[test]
    fn task_kinds_log_under_the_routes_callers_already_use() {
        let cases = [
            (TaskKind::Chat, "ask", Tier::Standard),
            (TaskKind::MeetingSummary, "meeting_summary", Tier::Standard),
            (TaskKind::Digest, "digest", Tier::Standard),
            (TaskKind::MeetingPrep, "routine", Tier::Standard),
            (TaskKind::Routine, "routine", Tier::Standard),
            (TaskKind::ConversationTitle, "conversation_title", Tier::Light),
            (TaskKind::TopicName, "topic_name", Tier::Light),
            (TaskKind::ProfileSuggest, "profile_suggest", Tier::Light),
        ];
        for (kind, route, tier) in cases {
            assert_eq!(kind.route(), route, "{kind:?}");
            assert_eq!(kind.tier(), tier, "{kind:?}");
        }
    }

    #[test]
    fn an_exhausted_or_empty_chain_answers_none() {
        let chain = [
            link(Provider::Ollama, Some(|| http(404))),
            link(Provider::Anthropic, Some(|| http(503))),
        ];
        assert!(try_chain(&chain, |l| l.client.complete("q")).is_none());
        assert!(try_chain(&[], |l: &ChainLink| l.client.complete("q")).is_none());
    }
}
