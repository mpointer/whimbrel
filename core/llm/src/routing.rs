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
    /// The small downloadable model, run by the bundled helper. The last link of a chain.
    Local,
    /// Anthropic's models through Whimbrel's server, paid from prepaid Whimbrel credit.
    /// Used only when it is the user's preferred provider.
    Whimbrel,
}

impl Provider {
    /// NFR-PRV-1: local-first — only Ollama and the bundled model run entirely on-device.
    pub fn is_local(&self) -> bool {
        matches!(self, Provider::Ollama | Provider::Local)
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
            Provider::Local => "local",
            Provider::Whimbrel => "whimbrel",
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
            "local" => Some(Provider::Local),
            "whimbrel" => Some(Provider::Whimbrel),
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
/// `None` once every link is exhausted (or `chain` is empty). Callers that report why
/// use [`try_chain_with_failures`].
pub fn try_chain<T>(
    chain: &[ChainLink],
    attempt: impl FnMut(&ChainLink) -> Result<T, LlmError>,
) -> Option<(T, &ChainLink)> {
    try_chain_with_failures(chain, attempt).ok()
}

/// [`try_chain`], but a failed walk returns each failed link's classified reason in
/// chain order (`[NoProviderConfigured]` for an empty chain). Every link failure is
/// logged either way.
pub fn try_chain_with_failures<T>(
    chain: &[ChainLink],
    mut attempt: impl FnMut(&ChainLink) -> Result<T, LlmError>,
) -> Result<(T, &ChainLink), Vec<ProviderFailureKind>> {
    if chain.is_empty() {
        return Err(vec![ProviderFailureKind::NoProviderConfigured]);
    }
    let mut failures = Vec::new();
    for link in chain {
        match attempt(link) {
            Ok(value) => return Ok((value, link)),
            Err(e) => {
                failures.push(note_link_failure(link.provider, &link.model, &e));
                if !falls_through(&e) {
                    break;
                }
            }
        }
    }
    Err(failures)
}

/// Why one chain link failed, in terms a person can act on. The shells show
/// [`user_message`](Self::user_message) (or their own localized text keyed on `kind`)
/// instead of a bare "every provider failed".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderFailureKind {
    OllamaNotRunning,
    OllamaModelMissing { model: String },
    NoProviderConfigured,
    Unauthorized { #[serde(serialize_with = "provider_id")] provider: Provider },
    RateLimited { #[serde(serialize_with = "provider_id")] provider: Provider },
    Network { #[serde(serialize_with = "provider_id")] provider: Provider },
    /// Whimbrel's server answered 402: the prepaid credit can't cover the request.
    CreditExhausted,
    /// Never the provider's error text: some bodies echo a masked key, and a transport
    /// error's URL can carry one (`?key=`). The HTTP status, when there was one.
    Other { #[serde(serialize_with = "provider_id")] provider: Provider, status: Option<u16> },
    /// The provider answered that `model` does not exist (`crate::is_model_not_found`), and
    /// no fallback answered for it either.
    ModelUnavailable { #[serde(serialize_with = "provider_id")] provider: Provider, model: String },
}

fn provider_id<S: serde::Serializer>(provider: &Provider, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(provider.as_str())
}

impl ProviderFailureKind {
    /// Sorts one link's error into a kind. Ollama's 404 (and any "not found" body, which
    /// is how it says `model "x" not found, try pulling it first`) means the configured
    /// model isn't pulled; a connection that fails (other than a timeout, which means a
    /// running Ollama was slow) means it isn't running.
    pub fn classify(provider: Provider, model: &str, err: &LlmError) -> Self {
        let ollama = provider == Provider::Ollama;
        match err {
            LlmError::Network(msg) if ollama && !msg.to_lowercase().contains("timed out") => Self::OllamaNotRunning,
            LlmError::Http { status, body }
                if ollama && (*status == 404 || body.to_lowercase().contains("not found")) =>
            {
                Self::OllamaModelMissing { model: model.to_string() }
            }
            LlmError::Http { status: 402, .. } if provider == Provider::Whimbrel => Self::CreditExhausted,
            _ if crate::is_model_not_found(provider, err) => Self::ModelUnavailable { provider, model: model.to_string() },
            LlmError::Http { status: 401 | 403, .. } => Self::Unauthorized { provider },
            LlmError::Http { status: 429, .. } => Self::RateLimited { provider },
            LlmError::Network(_) => Self::Network { provider },
            LlmError::Http { status, .. } => Self::Other { provider, status: Some(*status) },
            LlmError::Provider(_) | LlmError::NotImplemented(_) => Self::Other { provider, status: None },
        }
    }

    /// English text for a shell to show; the Mac uses its own localized strings.
    pub fn user_message(&self) -> String {
        let name = |p: &Provider| match p {
            Provider::Ollama => "Ollama",
            Provider::Anthropic => "Anthropic",
            Provider::OpenAI => "OpenAI",
            Provider::Google => "Google",
            Provider::OpenRouter => "OpenRouter",
            Provider::Local => "The downloaded model",
            Provider::Whimbrel => "Whimbrel AI",
        };
        match self {
            Self::OllamaModelMissing { model } => format!(
                "Ollama doesn't have the model {model}. Pick one you have in Settings → AI Providers, or run: ollama pull {model}"
            ),
            Self::OllamaNotRunning => "Ollama isn't running on this computer. Start Ollama, or add an API key in Settings → AI Providers.".into(),
            Self::NoProviderConfigured => "No AI provider is set up. Add one in Settings → AI Providers.".into(),
            Self::Unauthorized { provider } => {
                format!("{} rejected the API key. Check it in Settings → AI Providers.", name(provider))
            }
            Self::RateLimited { provider } => format!("{} is busy or out of credit. Try again shortly.", name(provider)),
            Self::CreditExhausted => "Whimbrel AI credit is used up. Add credit in Settings.".into(),
            Self::ModelUnavailable { provider, model } => format!(
                "{} doesn't offer the model {model} to your key. Pick another in Settings → AI Providers.",
                name(provider)
            ),
            Self::Network { .. } | Self::Other { .. } => "Every AI provider failed.".into(),
        }
    }
}

/// Classifies one link's failure and logs it (provider, model, kind; never the error
/// text, the prompt or a key). Also for chain loops that can't go through [`try_chain`].
pub fn note_link_failure(provider: Provider, model: &str, err: &LlmError) -> ProviderFailureKind {
    let kind = ProviderFailureKind::classify(provider, model, err);
    tracing::warn!(provider = provider.as_str(), model, kind = ?kind, "provider chain link failed");
    kind
}

thread_local! {
    static LAST_CHAIN_FAILURES: std::cell::RefCell<Vec<ProviderFailureKind>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Records why a chain walk failed, right before its caller returns a provider failure;
/// nothing else writes it, so a failure that never walked a chain finds it empty.
pub fn set_last_chain_failures(failures: Vec<ProviderFailureKind>) {
    LAST_CHAIN_FAILURES.with(|f| *f.borrow_mut() = failures);
}

/// Takes the failures [`set_last_chain_failures`] recorded on this thread, leaving none.
// ponytail: thread-local, so it only works because every governed call walks its chain
// and builds its envelope on one thread; a reason travelling inside ApiError would be
// exact, but ApiError is Copy and matched exhaustively by the Windows shell.
pub fn take_last_chain_failures() -> Vec<ProviderFailureKind> {
    LAST_CHAIN_FAILURES.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// Whether a link's error is that link's own problem, so the next link may well succeed:
/// the retryable ones (429, 5xx), plus a provider that's unreachable or answered with
/// something unreadable, a key it refuses (401/403), a model it doesn't have (404), a
/// timeout (408), Whimbrel credit that ran out (402), or a capability it lacks. The fallback links exist for exactly these.
/// Before, only 429/5xx moved on, so a local Ollama that wasn't running, or didn't have
/// the configured model, failed every call even with working hosted keys behind it (seen
/// on Windows: Ollama answered 404 for `gemma4:12b`, and Ask failed outright). A
/// request-shaped rejection (400, 413, 422, …) still stops the walk.
pub fn falls_through(err: &LlmError) -> bool {
    match err {
        LlmError::Network(_) | LlmError::Provider(_) | LlmError::NotImplemented(_) => true,
        LlmError::Http { status, .. } => {
            is_retryable(err)
                || matches!(status, 401 | 402 | 403 | 404 | 408)
                // A model one provider doesn't have says nothing about the next link.
                || (*status == 400 && [Provider::OpenAI, Provider::Anthropic].iter().any(|p| crate::is_model_not_found(*p, err)))
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
        assert_eq!(
            try_chain_with_failures(&[], |l: &ChainLink| l.client.complete("q")).err(),
            Some(vec![ProviderFailureKind::NoProviderConfigured])
        );
    }

    #[test]
    fn link_failures_are_classified() {
        let c = |p, e: LlmError| ProviderFailureKind::classify(p, "gemma4:12b", &e);
        let ollama_404 = LlmError::Http {
            status: 404,
            body: r#"{"error":"model \"gemma4:12b\" not found, try pulling it first"}"#.into(),
        };
        let missing = ProviderFailureKind::OllamaModelMissing { model: "gemma4:12b".into() };
        assert_eq!(c(Provider::Ollama, ollama_404), missing);
        let not_found_500 = LlmError::Http { status: 500, body: "model 'x' not found".into() };
        assert_eq!(c(Provider::Ollama, not_found_500), missing);
        assert_eq!(
            c(Provider::Ollama, LlmError::Network("Connection refused (os error 61)".into())),
            ProviderFailureKind::OllamaNotRunning
        );
        assert_eq!(
            c(Provider::Ollama, LlmError::Network("http://localhost:11434/api/chat: timed out reading response".into())),
            ProviderFailureKind::Network { provider: Provider::Ollama }
        );
        assert_eq!(c(Provider::Anthropic, http(401)), ProviderFailureKind::Unauthorized { provider: Provider::Anthropic });
        assert_eq!(c(Provider::OpenAI, http(403)), ProviderFailureKind::Unauthorized { provider: Provider::OpenAI });
        assert_eq!(c(Provider::Google, http(429)), ProviderFailureKind::RateLimited { provider: Provider::Google });
        assert_eq!(
            c(Provider::Anthropic, LlmError::Network("dns".into())),
            ProviderFailureKind::Network { provider: Provider::Anthropic }
        );
        let keyed = LlmError::Http { status: 400, body: "API key not valid: AIza…xyz".into() };
        let other = c(Provider::Google, keyed);
        assert_eq!(other, ProviderFailureKind::Other { provider: Provider::Google, status: Some(400) });
        assert!(!serde_json::to_string(&other).unwrap().contains("AIza"), "no provider text");
    }

    #[test]
    fn credit_exhausted_falls_through() {
        let chain = [
            link(Provider::Whimbrel, Some(|| http(402))),
            link(Provider::Anthropic, None),
        ];
        assert!(falls_through(&http(402)));
        assert_eq!(try_chain(&chain, |l| l.client.complete("q")).map(|(_, l)| l.provider), Some(Provider::Anthropic));
        let failing = [
            link(Provider::Whimbrel, Some(|| http(402))),
            link(Provider::Anthropic, Some(|| http(503))),
        ];
        assert_eq!(
            try_chain_with_failures(&failing, |l| l.client.complete("q")).err(),
            Some(vec![
                ProviderFailureKind::CreditExhausted,
                ProviderFailureKind::Other { provider: Provider::Anthropic, status: Some(503) },
            ])
        );
        assert_eq!(
            serde_json::to_string(&ProviderFailureKind::CreditExhausted).unwrap(),
            r#"{"kind":"credit_exhausted"}"#
        );
        assert_eq!(
            ProviderFailureKind::CreditExhausted.user_message(),
            "Whimbrel AI credit is used up. Add credit in Settings."
        );
    }

    #[test]
    fn whimbrel_failures_are_classified() {
        let c = |e: LlmError| ProviderFailureKind::classify(Provider::Whimbrel, "m", &e);
        assert_eq!(c(http(402)), ProviderFailureKind::CreditExhausted);
        assert_eq!(c(http(429)), ProviderFailureKind::RateLimited { provider: Provider::Whimbrel });
        assert_eq!(c(http(401)), ProviderFailureKind::Unauthorized { provider: Provider::Whimbrel });
        assert_eq!(c(http(400)), ProviderFailureKind::Other { provider: Provider::Whimbrel, status: Some(400) });
        // Only Whimbrel's server speaks of credit; another provider's 402 stays Other.
        assert_eq!(
            ProviderFailureKind::classify(Provider::Anthropic, "m", &http(402)),
            ProviderFailureKind::Other { provider: Provider::Anthropic, status: Some(402) }
        );
    }

    #[test]
    fn provider_parse_round_trips_whimbrel() {
        assert_eq!(Provider::Whimbrel.as_str(), "whimbrel");
        assert_eq!(Provider::parse("whimbrel"), Some(Provider::Whimbrel));
        assert!(!Provider::Whimbrel.is_local());
        for p in [Provider::Ollama, Provider::Anthropic, Provider::OpenAI, Provider::Google, Provider::OpenRouter, Provider::Local, Provider::Whimbrel] {
            assert_eq!(Provider::parse(p.as_str()), Some(p));
        }
        assert_eq!(
            ProviderFailureKind::RateLimited { provider: Provider::Whimbrel }.user_message(),
            "Whimbrel AI is busy or out of credit. Try again shortly."
        );
    }

    #[test]
    fn try_chain_with_failures_says_why_each_link_failed() {
        let chain = [
            link(Provider::Ollama, Some(|| http(404))),
            link(Provider::Anthropic, Some(|| http(429))),
        ];
        assert_eq!(
            try_chain_with_failures(&chain, |l| l.client.complete("q")).err(),
            Some(vec![
                ProviderFailureKind::OllamaModelMissing { model: "m".into() },
                ProviderFailureKind::RateLimited { provider: Provider::Anthropic },
            ])
        );
        assert!(take_last_chain_failures().is_empty(), "walking a chain records nothing by itself");
    }

    #[test]
    fn a_missing_hosted_model_is_its_own_kind_and_moves_on() {
        let openai_400 = || LlmError::Http { status: 400, body: r#"{"error":{"code":"model_not_found"}}"#.into() };
        let kind = ProviderFailureKind::classify(Provider::OpenAI, "gpt-4o-mini", &openai_400());
        assert_eq!(kind, ProviderFailureKind::ModelUnavailable { provider: Provider::OpenAI, model: "gpt-4o-mini".into() });
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            r#"{"kind":"model_unavailable","provider":"openai","model":"gpt-4o-mini"}"#
        );
        assert_eq!(kind.user_message(), "OpenAI doesn't offer the model gpt-4o-mini to your key. Pick another in Settings → AI Providers.");
        assert_eq!(
            ProviderFailureKind::classify(Provider::Google, "gemini-2.5-flash", &http(404)),
            ProviderFailureKind::ModelUnavailable { provider: Provider::Google, model: "gemini-2.5-flash".into() }
        );
        // Ollama keeps its own kind; OpenRouter's 404 stays Other.
        assert_eq!(ProviderFailureKind::classify(Provider::Ollama, "m", &http(404)), ProviderFailureKind::OllamaModelMissing { model: "m".into() });
        assert_eq!(ProviderFailureKind::classify(Provider::OpenRouter, "m", &http(404)), ProviderFailureKind::Other { provider: Provider::OpenRouter, status: Some(404) });
        // A 400 that only means "no such model" is that link's problem, not the request's.
        assert!(falls_through(&openai_400()));
        assert!(!falls_through(&LlmError::Http { status: 400, body: r#"{"error":{"code":"context_length_exceeded"}}"#.into() }));
    }

    #[test]
    fn failure_kinds_serialize_for_the_envelope_and_read_as_sentences() {
        let missing = ProviderFailureKind::OllamaModelMissing { model: "gemma4:12b".into() };
        assert_eq!(serde_json::to_string(&missing).unwrap(), r#"{"kind":"ollama_model_missing","model":"gemma4:12b"}"#);
        assert_eq!(
            missing.user_message(),
            "Ollama doesn't have the model gemma4:12b. Pick one you have in Settings → AI Providers, or run: ollama pull gemma4:12b"
        );
        let unauthorized = ProviderFailureKind::Unauthorized { provider: Provider::OpenRouter };
        assert_eq!(serde_json::to_string(&unauthorized).unwrap(), r#"{"kind":"unauthorized","provider":"openrouter"}"#);
        assert_eq!(unauthorized.user_message(), "OpenRouter rejected the API key. Check it in Settings → AI Providers.");
        assert_eq!(
            serde_json::to_string(&ProviderFailureKind::NoProviderConfigured).unwrap(),
            r#"{"kind":"no_provider_configured"}"#
        );
    }
}
