//! Settings-sourced routing/governance configuration for one LLM call, and the
//! provider chain built from it. Moved verbatim from `core/ffi` (M3.0); the only
//! change is that the C-string entry point became [`RoutingConfig::from_json`] over a
//! plain `Option<&str>`, with `core/ffi` keeping a one-line pointer adapter.

/// Decoded from the shell's `routing_config_json` (sub-project 2's Settings UI — see
/// docs/superpowers/specs/2026-07-27-settings-ui-routing-design.md). All fields but
/// `preferred_provider`/`preferred_model` are optional — the shell always sends *some*
/// current selection (defaulting to Ollama, before Settings has ever been touched), but
/// key/cap fields are frequently absent (no key saved yet, cap left at its conservative
/// built-in default). This JSON, sourced from the shell's keychain-backed Settings, is
/// the only way a hosted link is ever added to the chain.
#[derive(Debug, Clone)]
pub struct RoutingConfig {
    pub anthropic_key: Option<String>,
    pub openai_key: Option<String>,
    pub google_key: Option<String>,
    /// The TypeSafe key Reply Priorities already uses; enables the calendar title check.
    pub typesafe_key: Option<String>,
    pub user_daily_cap_cents: Option<i64>,
    pub global_daily_cap_cents: Option<i64>,
    pub sensitive: bool,
    pub preferred_provider: String,
    pub preferred_model: String,
    pub custom_instructions: String,
    pub daily_digest_instructions: String,
    pub weekly_digest_instructions: String,
    pub profile: String,
    /// Light tasks (titles, topic names, profile suggestions) use the cheapest model the
    /// user has instead of the preferred one. Default on; off restores one chain for all.
    pub cheap_first: bool,
    /// Whether Ollama is a link at all. Off drops it from every chain except sensitive
    /// mode, where local is the whole point. Default on, matching every existing install.
    pub use_local_model: bool,
    pub openrouter_key: Option<String>,
    /// The user's OpenRouter model pick, also the light-tier model there. Empty means
    /// none picked yet: OpenRouter is then left out of light chains, and out of the
    /// standard chain too, since there is no fallback model to substitute.
    pub openrouter_model: String,
    /// Prices cached from OpenRouter's model list when the pick was made, keyed by model
    /// id, cents per 1K input and output. Handed to `llm::set_price_overrides` on parse.
    pub openrouter_prices: std::collections::HashMap<String, (f64, f64)>,
}

impl Default for RoutingConfig {
    /// "Ollama-only, no caps configured, not sensitive" — this app's state before
    /// Settings has ever been touched.
    fn default() -> Self {
        RoutingConfig {
            anthropic_key: None,
            openai_key: None,
            google_key: None,
            typesafe_key: None,
            user_daily_cap_cents: None,
            global_daily_cap_cents: None,
            sensitive: false,
            preferred_provider: "ollama".to_string(),
            preferred_model: String::new(),
            custom_instructions: String::new(),
            daily_digest_instructions: String::new(),
            weekly_digest_instructions: String::new(),
            profile: String::new(),
            cheap_first: true,
            use_local_model: true,
            openrouter_key: None,
            openrouter_model: String::new(),
            openrouter_prices: std::collections::HashMap::new(),
        }
    }
}

impl RoutingConfig {
    /// A missing (`None`) or malformed JSON string degrades to [`Default`] — the same
    /// as an empty object `{}` would — keeping every existing call site (including
    /// `core/ffi`'s `#[ignore]`d real-provider tests) working by passing nothing.
    ///
    /// Deliberately parsed field-by-field via `serde_json::Value` rather than
    /// `#[derive(Deserialize)]`: a single wrong-typed field (say `"sensitive":"yes"`)
    /// must degrade *that field* to its default, not reject the whole config. A derive
    /// would fail the entire parse and silently turn a user's hosted-provider setup
    /// into Ollama-only. Keep it this way.
    pub fn from_json(routing_config_json: Option<&str>) -> RoutingConfig {
        let default = RoutingConfig::default();
        let Some(raw) = routing_config_json else {
            return default;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
            return default;
        };
        let openrouter_prices: std::collections::HashMap<String, (f64, f64)> = v
            .get("openrouter_prices")
            .and_then(|x| x.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(model, price)| {
                        let input = price.get("in")?.as_f64()?;
                        let output = price.get("out")?.as_f64()?;
                        Some((model.clone(), (input, output)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        llm::set_price_overrides(openrouter_prices.clone());
        RoutingConfig {
            anthropic_key: v
                .get("anthropic_key")
                .and_then(|x| x.as_str())
                .map(String::from),
            openai_key: v
                .get("openai_key")
                .and_then(|x| x.as_str())
                .map(String::from),
            google_key: v
                .get("google_key")
                .and_then(|x| x.as_str())
                .map(String::from),
            typesafe_key: v
                .get("typesafe_key")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(String::from),
            user_daily_cap_cents: v.get("user_daily_cap_cents").and_then(|x| x.as_i64()),
            global_daily_cap_cents: v.get("global_daily_cap_cents").and_then(|x| x.as_i64()),
            sensitive: v
                .get("sensitive")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            preferred_provider: v
                .get("preferred_provider")
                .and_then(|x| x.as_str())
                .map(String::from)
                .unwrap_or_else(|| "ollama".to_string()),
            preferred_model: v
                .get("preferred_model")
                .and_then(|x| x.as_str())
                .map(String::from)
                .unwrap_or_default(),
            custom_instructions: v
                .get("custom_instructions")
                .and_then(|x| x.as_str())
                .map(String::from)
                .unwrap_or_default()
                .trim()
                .to_string(),
            daily_digest_instructions: v
                .get("daily_digest_instructions")
                .and_then(|x| x.as_str())
                .map(String::from)
                .unwrap_or_default()
                .trim()
                .to_string(),
            weekly_digest_instructions: v
                .get("weekly_digest_instructions")
                .and_then(|x| x.as_str())
                .map(String::from)
                .unwrap_or_default()
                .trim()
                .to_string(),
            profile: v
                .get("profile")
                .and_then(|x| x.as_str())
                .map(String::from)
                .unwrap_or_default()
                .trim()
                .to_string(),
            cheap_first: v
                .get("cheap_first")
                .and_then(|x| x.as_bool())
                .unwrap_or(default.cheap_first),
            use_local_model: v
                .get("use_local_model")
                .and_then(|x| x.as_bool())
                .unwrap_or(default.use_local_model),
            openrouter_key: v
                .get("openrouter_key")
                .and_then(|x| x.as_str())
                .map(String::from),
            openrouter_model: v
                .get("openrouter_model")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .map(String::from)
                .unwrap_or_default(),
            openrouter_prices,
        }
    }

    pub fn spend_cap_config(&self) -> llm::SpendCapConfig {
        llm::SpendCapConfig {
            user_daily_cents: self.user_daily_cap_cents,
            global_daily_cents: self.global_daily_cap_cents,
        }
    }
}

/// Assembles today's provider chain from `routing`. `ollama_model` is always used for
/// Ollama's own link — Ollama has no separate "preferred model" concept, since there's
/// only one Ollama model configured in Settings at a time. When
/// `routing.preferred_provider` names a hosted provider, that provider's link uses
/// `routing.preferred_model` (the user's actual pick); each *other* hosted provider, if
/// it also has a key configured, still gets a fallback link using its hardcoded default
/// model (a safety-net substitution doesn't need to match a model the user never
/// selected for it). `routing.sensitive` (FR-CHAT-5's local-only mode) short-circuits
/// to Ollama-only regardless of `preferred_provider` or which keys are configured. The
/// preferred provider becomes link 0 only if it's actually buildable (Ollama is always
/// buildable; a hosted provider needs its key present) — an unbuildable preference
/// (e.g. Anthropic picked but no Anthropic key saved) falls through to the default
/// Ollama-first order instead of silently dropping a link.
pub fn build_chain(ollama_model: &str, routing: &RoutingConfig) -> Vec<llm::ChainLink> {
    let ollama_link = || ollama_link(ollama_model);
    if routing.sensitive {
        return vec![ollama_link()];
    }

    let anthropic_model = if routing.preferred_provider == "anthropic" {
        routing.preferred_model.as_str()
    } else {
        FALLBACK_ANTHROPIC_MODEL
    };
    let openai_model = if routing.preferred_provider == "openai" {
        routing.preferred_model.as_str()
    } else {
        FALLBACK_OPENAI_MODEL
    };
    let google_model = if routing.preferred_provider == "google" {
        routing.preferred_model.as_str()
    } else {
        FALLBACK_GOOGLE_MODEL
    };

    // OpenRouter has no hardcoded fallback model (their catalogue changes weekly), so
    // its link exists only once the user has picked one; preferred or not, it is that
    // model.
    let openrouter_model = if routing.preferred_provider == "openrouter" && !routing.preferred_model.is_empty() {
        routing.preferred_model.as_str()
    } else {
        routing.openrouter_model.as_str()
    };

    let mut ollama_slot = routing.use_local_model.then(ollama_link);
    let mut anthropic_slot = hosted_link(routing, llm::Provider::Anthropic, anthropic_model);
    let mut openai_slot = hosted_link(routing, llm::Provider::OpenAI, openai_model);
    let mut google_slot = hosted_link(routing, llm::Provider::Google, google_model);
    let mut openrouter_slot = hosted_link(routing, llm::Provider::OpenRouter, openrouter_model);

    let mut chain = Vec::with_capacity(5);
    match routing.preferred_provider.as_str() {
        "anthropic" => {
            if let Some(link) = anthropic_slot.take() {
                chain.push(link);
            }
        }
        "openai" => {
            if let Some(link) = openai_slot.take() {
                chain.push(link);
            }
        }
        "google" => {
            if let Some(link) = google_slot.take() {
                chain.push(link);
            }
        }
        "openrouter" => {
            if let Some(link) = openrouter_slot.take() {
                chain.push(link);
            }
        }
        _ => {
            if let Some(link) = ollama_slot.take() {
                chain.push(link);
            }
        }
    }
    if let Some(link) = ollama_slot.take() {
        chain.push(link);
    }
    if let Some(link) = anthropic_slot.take() {
        chain.push(link);
    }
    if let Some(link) = openai_slot.take() {
        chain.push(link);
    }
    if let Some(link) = google_slot.take() {
        chain.push(link);
    }
    if let Some(link) = openrouter_slot.take() {
        chain.push(link);
    }
    chain
}

const FALLBACK_ANTHROPIC_MODEL: &str = "claude-haiku-4-5";
const FALLBACK_OPENAI_MODEL: &str = "gpt-4o-mini";
const FALLBACK_GOOGLE_MODEL: &str = "gemini-2.5-flash";

/// The model each hosted provider's light link uses: the cheapest chat model in
/// `llm::price_rank`'s table that the provider serves. If a provider's live model list
/// no longer carries it, [`cheap_model_for`] falls back to the provider's fallback
/// model, so a retired id never breaks light tasks.
const CHEAP_ANTHROPIC_MODEL: &str = "claude-haiku-4-5";
const CHEAP_OPENAI_MODEL: &str = "gpt-4o-mini";
const CHEAP_GOOGLE_MODEL: &str = "gemini-2.0-flash";

/// The hosted providers `build_chain` knows, in its default fallback order.
const HOSTED_PROVIDERS: [llm::Provider; 4] =
    [llm::Provider::Anthropic, llm::Provider::OpenAI, llm::Provider::Google, llm::Provider::OpenRouter];

fn fallback_model(provider: llm::Provider) -> &'static str {
    match provider {
        llm::Provider::Anthropic => FALLBACK_ANTHROPIC_MODEL,
        llm::Provider::OpenAI => FALLBACK_OPENAI_MODEL,
        llm::Provider::Google => FALLBACK_GOOGLE_MODEL,
        llm::Provider::Ollama | llm::Provider::OpenRouter => "",
    }
}

/// The light-tier model for `provider`. `available` is the provider's live model list
/// when the caller has one (the key screen fetches it on save); `None` trusts the
/// constant. OpenRouter has no constant: its light model is the user's pick
/// (`RoutingConfig::openrouter_model`), so this answers `""` for it.
pub fn cheap_model_for(provider: llm::Provider, available: Option<&[String]>) -> &'static str {
    let cheap = match provider {
        llm::Provider::Anthropic => CHEAP_ANTHROPIC_MODEL,
        llm::Provider::OpenAI => CHEAP_OPENAI_MODEL,
        llm::Provider::Google => CHEAP_GOOGLE_MODEL,
        llm::Provider::Ollama | llm::Provider::OpenRouter => return "",
    };
    match available {
        Some(list) if !list.iter().any(|m| m == cheap) => fallback_model(provider),
        _ => cheap,
    }
}

/// The model `provider`'s light link runs: the cheap constant, or for OpenRouter the
/// user's pick. Empty means no light link for that provider.
fn light_model(routing: &RoutingConfig, provider: llm::Provider) -> &str {
    match provider {
        llm::Provider::OpenRouter => routing.openrouter_model.as_str(),
        p => cheap_model_for(p, None),
    }
}

fn ollama_link(ollama_model: &str) -> llm::ChainLink {
    llm::ChainLink {
        provider: llm::Provider::Ollama,
        model: ollama_model.to_string(),
        client: Box::new(llm::OllamaClient::new(ollama_model)),
    }
}

/// A hosted link at `model`, or `None` when the user holds no key for `provider` or
/// `model` is empty (OpenRouter before a pick).
fn hosted_link(routing: &RoutingConfig, provider: llm::Provider, model: &str) -> Option<llm::ChainLink> {
    if model.is_empty() {
        return None;
    }
    let client: Box<dyn llm::LlmClient> = match provider {
        llm::Provider::Anthropic => Box::new(llm::AnthropicClient::new(routing.anthropic_key.clone()?, model)),
        llm::Provider::OpenAI => Box::new(llm::OpenAIClient::new(routing.openai_key.clone()?, model)),
        llm::Provider::Google => Box::new(llm::GoogleClient::new(routing.google_key.clone()?, model)),
        llm::Provider::OpenRouter => Box::new(llm::OpenAIClient::openrouter(routing.openrouter_key.clone()?, model)),
        llm::Provider::Ollama => return None,
    };
    Some(llm::ChainLink { provider, model: model.to_string(), client })
}

/// The light chain: the local model first (when enabled), then every hosted provider
/// the user holds a key for at its cheap model, cheapest first by `llm::price_rank`.
fn build_light_chain(ollama_model: &str, routing: &RoutingConfig) -> Vec<llm::ChainLink> {
    let mut hosted: Vec<llm::ChainLink> = HOSTED_PROVIDERS
        .iter()
        .filter_map(|&p| hosted_link(routing, p, light_model(routing, p)))
        .collect();
    hosted.sort_by(|a, b| llm::price_rank(a.provider, &a.model).total_cmp(&llm::price_rank(b.provider, &b.model)));
    let mut chain = Vec::with_capacity(hosted.len() + 1);
    if routing.use_local_model {
        chain.push(ollama_link(ollama_model));
    }
    chain.extend(hosted);
    chain
}

/// The chain for one call of `task`: the light chain for light tasks when cheap-first
/// is on, otherwise [`build_chain`]. Sensitive mode is Ollama-only for every task.
pub fn build_chain_for(task: llm::TaskKind, ollama_model: &str, routing: &RoutingConfig) -> Vec<llm::ChainLink> {
    if routing.sensitive {
        return vec![ollama_link(ollama_model)];
    }
    match task.tier() {
        llm::Tier::Light if routing.cheap_first => build_light_chain(ollama_model, routing),
        _ => build_chain(ollama_model, routing),
    }
}

/// How one routine picks its model: today's chain, the light chain, or one provider and
/// model pinned first. Stored on the `routine` row as `default`, `cheapest`, or
/// `<provider>/<model>`; a value in any other shape is rejected at write time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineModelPref {
    Default,
    Cheapest,
    Pinned { provider: llm::Provider, model: String },
}

impl RoutineModelPref {
    /// A malformed stored value reads as `Default`, so an old row or a hand edit can
    /// never stop a routine from running.
    pub fn parse(raw: &str) -> Self {
        Self::try_parse(raw).unwrap_or(Self::Default)
    }

    /// `None` for any value outside the three shapes.
    pub fn try_parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        match raw {
            "default" => return Some(Self::Default),
            "cheapest" => return Some(Self::Cheapest),
            _ => {}
        }
        // OpenRouter model ids carry their own slash (`vendor/model`), so split on the
        // first one only.
        let (provider, model) = raw.split_once('/')?;
        let provider = llm::Provider::parse(provider)?;
        if model.is_empty() {
            return None;
        }
        Some(Self::Pinned { provider, model: model.to_string() })
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::Default => "default".to_string(),
            Self::Cheapest => "cheapest".to_string(),
            Self::Pinned { provider, model } => format!("{}/{}", provider.as_str(), model),
        }
    }
}

/// The chain for one routine run. `Pinned` puts that provider at that model first when
/// its key exists (a pinned Ollama uses the pinned model instead of `ollama_model`) and
/// the standard chain after it, minus the duplicate; a pinned provider with no key falls
/// through to the standard chain, the way an unbuildable preferred provider does.
pub fn build_chain_for_routine(pref: &RoutineModelPref, ollama_model: &str, routing: &RoutingConfig) -> Vec<llm::ChainLink> {
    if routing.sensitive {
        return vec![ollama_link(ollama_model)];
    }
    match pref {
        RoutineModelPref::Default => build_chain(ollama_model, routing),
        RoutineModelPref::Cheapest if routing.cheap_first => build_light_chain(ollama_model, routing),
        RoutineModelPref::Cheapest => build_chain(ollama_model, routing),
        RoutineModelPref::Pinned { provider, model } => {
            let pinned = match provider {
                llm::Provider::Ollama => Some(ollama_link(model)),
                p => hosted_link(routing, *p, model),
            };
            let Some(pinned) = pinned else {
                return build_chain(ollama_model, routing);
            };
            let mut chain = vec![pinned];
            chain.extend(build_chain(ollama_model, routing).into_iter().filter(|l| l.provider != *provider));
            chain
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::{Provider, TaskKind};

    fn providers(chain: &[llm::ChainLink]) -> Vec<llm::Provider> {
        chain.iter().map(|l| l.provider).collect()
    }

    fn all_keys() -> RoutingConfig {
        RoutingConfig {
            anthropic_key: Some("a".into()),
            openai_key: Some("o".into()),
            google_key: Some("g".into()),
            ..RoutingConfig::default()
        }
    }

    #[test]
    fn from_json_reads_cheap_first_and_use_local_model_and_defaults_both_on() {
        let r = RoutingConfig::from_json(None);
        assert!(r.cheap_first && r.use_local_model);
        let r = RoutingConfig::from_json(Some(r#"{"cheap_first":false,"use_local_model":false}"#));
        assert!(!r.cheap_first && !r.use_local_model);
        let r = RoutingConfig::from_json(Some(r#"{"cheap_first":"no","use_local_model":0}"#));
        assert!(r.cheap_first && r.use_local_model, "a wrong-typed field degrades to its default");
    }

    #[test]
    fn light_chain_is_ollama_then_hosted_cheap_models_by_price() {
        let chain = build_chain_for(TaskKind::ConversationTitle, "gemma4:12b", &all_keys());
        assert_eq!(providers(&chain), vec![Provider::Ollama, Provider::Google, Provider::OpenAI, Provider::Anthropic]);
        assert_eq!(chain[1].model, "gemini-2.0-flash");
        assert_eq!(chain[2].model, "gpt-4o-mini");
        assert_eq!(chain[3].model, "claude-haiku-4-5");
    }

    #[test]
    fn light_chain_ignores_the_preferred_provider_and_skips_providers_without_a_key() {
        let routing = RoutingConfig {
            anthropic_key: Some("a".into()),
            preferred_provider: "anthropic".into(),
            preferred_model: "claude-opus-5".into(),
            ..RoutingConfig::default()
        };
        let chain = build_chain_for(TaskKind::TopicName, "gemma4:12b", &routing);
        assert_eq!(providers(&chain), vec![Provider::Ollama, Provider::Anthropic]);
        assert_eq!(chain[1].model, "claude-haiku-4-5", "the cheap model, not the preferred one");
    }

    #[test]
    fn standard_tasks_and_cheap_first_off_use_the_standard_chain() {
        let routing = RoutingConfig { preferred_provider: "anthropic".into(), preferred_model: "claude-opus-5".into(), ..all_keys() };
        let standard = build_chain_for(TaskKind::Chat, "gemma4:12b", &routing);
        assert_eq!(providers(&standard), vec![Provider::Anthropic, Provider::Ollama, Provider::OpenAI, Provider::Google]);
        assert_eq!(standard[0].model, "claude-opus-5");

        let routing = RoutingConfig { cheap_first: false, ..routing };
        let light_off = build_chain_for(TaskKind::ProfileSuggest, "gemma4:12b", &routing);
        assert_eq!(providers(&light_off), providers(&standard));
        assert_eq!(light_off[0].model, "claude-opus-5");
    }

    #[test]
    fn sensitive_is_ollama_only_for_both_tiers_and_ignores_use_local_model() {
        let routing = RoutingConfig { sensitive: true, use_local_model: false, ..all_keys() };
        for task in [TaskKind::Chat, TaskKind::ConversationTitle] {
            assert_eq!(providers(&build_chain_for(task, "gemma4:12b", &routing)), vec![Provider::Ollama]);
        }
        assert_eq!(
            providers(&build_chain_for_routine(&RoutineModelPref::Cheapest, "gemma4:12b", &routing)),
            vec![Provider::Ollama]
        );
    }

    #[test]
    fn use_local_model_off_drops_ollama_from_standard_and_light_chains() {
        let routing = RoutingConfig { use_local_model: false, ..all_keys() };
        assert_eq!(
            providers(&build_chain("gemma4:12b", &routing)),
            vec![Provider::Anthropic, Provider::OpenAI, Provider::Google]
        );
        assert_eq!(
            providers(&build_chain_for(TaskKind::ConversationTitle, "gemma4:12b", &routing)),
            vec![Provider::Google, Provider::OpenAI, Provider::Anthropic]
        );
        // No keys and no local model: an empty chain, which try_chain reports as a
        // provider failure, the same as every other "nothing to call" case.
        let routing = RoutingConfig { use_local_model: false, ..RoutingConfig::default() };
        assert!(build_chain("gemma4:12b", &routing).is_empty());
    }

    #[test]
    fn cheap_model_falls_back_when_the_live_list_lacks_it() {
        assert_eq!(cheap_model_for(Provider::Google, None), "gemini-2.0-flash");
        let live = vec!["gemini-2.5-flash".to_string(), "gemini-2.5-pro".to_string()];
        assert_eq!(cheap_model_for(Provider::Google, Some(&live)), "gemini-2.5-flash");
        let live = vec!["gemini-2.0-flash".to_string()];
        assert_eq!(cheap_model_for(Provider::Google, Some(&live)), "gemini-2.0-flash");
        assert_eq!(cheap_model_for(Provider::Ollama, None), "");
    }

    #[test]
    fn routine_model_pref_parses_the_three_shapes_and_rejects_the_rest() {
        assert_eq!(RoutineModelPref::try_parse("default"), Some(RoutineModelPref::Default));
        assert_eq!(RoutineModelPref::try_parse(" cheapest "), Some(RoutineModelPref::Cheapest));
        assert_eq!(
            RoutineModelPref::try_parse("anthropic/claude-sonnet-5"),
            Some(RoutineModelPref::Pinned { provider: Provider::Anthropic, model: "claude-sonnet-5".into() })
        );
        assert_eq!(
            RoutineModelPref::try_parse("ollama/gemma4:12b"),
            Some(RoutineModelPref::Pinned { provider: Provider::Ollama, model: "gemma4:12b".into() })
        );
        for bad in ["", "cheap", "anthropic", "anthropic/", "venice/llama", "default/x"] {
            assert_eq!(RoutineModelPref::try_parse(bad), None, "{bad:?}");
            assert_eq!(RoutineModelPref::parse(bad), RoutineModelPref::Default, "{bad:?}");
        }
        for pref in [
            RoutineModelPref::Default,
            RoutineModelPref::Cheapest,
            RoutineModelPref::Pinned { provider: Provider::Google, model: "gemini-2.5-pro".into() },
        ] {
            assert_eq!(RoutineModelPref::try_parse(&pref.as_string()), Some(pref));
        }
    }

    #[test]
    fn routine_chain_follows_the_routine_preference() {
        let routing = all_keys();
        assert_eq!(
            providers(&build_chain_for_routine(&RoutineModelPref::Default, "gemma4:12b", &routing)),
            providers(&build_chain("gemma4:12b", &routing))
        );
        let cheapest = build_chain_for_routine(&RoutineModelPref::Cheapest, "gemma4:12b", &routing);
        assert_eq!(providers(&cheapest), vec![Provider::Ollama, Provider::Google, Provider::OpenAI, Provider::Anthropic]);
        assert_eq!(cheapest[1].model, "gemini-2.0-flash");

        let pinned = RoutineModelPref::Pinned { provider: Provider::OpenAI, model: "gpt-4.1".into() };
        let chain = build_chain_for_routine(&pinned, "gemma4:12b", &routing);
        assert_eq!(providers(&chain), vec![Provider::OpenAI, Provider::Ollama, Provider::Anthropic, Provider::Google]);
        assert_eq!(chain[0].model, "gpt-4.1");

        let pinned_local = RoutineModelPref::Pinned { provider: Provider::Ollama, model: "llama3:8b".into() };
        let chain = build_chain_for_routine(&pinned_local, "gemma4:12b", &routing);
        assert_eq!(chain[0].provider, Provider::Ollama);
        assert_eq!(chain[0].model, "llama3:8b");
        assert_eq!(chain.iter().filter(|l| l.provider == Provider::Ollama).count(), 1);
    }

    #[test]
    fn a_pinned_provider_without_a_key_falls_through_to_the_standard_chain() {
        let routing = RoutingConfig { openai_key: Some("o".into()), ..RoutingConfig::default() };
        let pinned = RoutineModelPref::Pinned { provider: Provider::Anthropic, model: "claude-sonnet-5".into() };
        assert_eq!(
            providers(&build_chain_for_routine(&pinned, "gemma4:12b", &routing)),
            vec![Provider::Ollama, Provider::OpenAI]
        );
    }

    #[test]
    fn from_json_none_and_malformed_both_yield_defaults() {
        for raw in [None, Some("not json"), Some("{")] {
            let r = RoutingConfig::from_json(raw);
            assert_eq!(r.preferred_provider, "ollama");
            assert!(r.anthropic_key.is_none() && r.openai_key.is_none() && r.google_key.is_none());
            assert!(r.user_daily_cap_cents.is_none() && r.global_daily_cap_cents.is_none());
            assert!(!r.sensitive);
            assert!(r.custom_instructions.is_empty());
        }
    }

    #[test]
    fn typesafe_key_parses_and_blank_or_absent_is_none() {
        let key = |raw: &str| RoutingConfig::from_json(Some(raw)).typesafe_key;
        assert_eq!(key(r#"{"typesafe_key":"ts-1"}"#).as_deref(), Some("ts-1"));
        assert_eq!(key(r#"{"typesafe_key":"  "}"#), None);
        assert_eq!(key(r#"{"typesafe_key":""}"#), None);
        assert_eq!(key("{}"), None);
        assert_eq!(RoutingConfig::default().typesafe_key, None);
    }

    #[test]
    fn from_json_wrong_typed_field_degrades_only_that_field() {
        // The reason this stays a Value walk and not a derive: one bad field must not
        // throw away the user's hosted-provider setup.
        let r = RoutingConfig::from_json(Some(
            r#"{"preferred_provider":"anthropic","preferred_model":"claude-sonnet-5","anthropic_key":"k","sensitive":"yes","user_daily_cap_cents":"lots"}"#,
        ));
        assert_eq!(r.preferred_provider, "anthropic");
        assert_eq!(r.preferred_model, "claude-sonnet-5");
        assert_eq!(r.anthropic_key.as_deref(), Some("k"));
        assert!(!r.sensitive);
        assert!(r.user_daily_cap_cents.is_none());
    }

    #[test]
    fn from_json_trims_instruction_fields() {
        let r = RoutingConfig::from_json(Some(
            r#"{"custom_instructions":"  be terse \n","daily_digest_instructions":"   ","weekly_digest_instructions":"\tweekly\t"}"#,
        ));
        assert_eq!(r.custom_instructions, "be terse");
        assert_eq!(r.daily_digest_instructions, "");
        assert_eq!(r.weekly_digest_instructions, "weekly");
    }

    #[test]
    fn build_chain_is_ollama_only_when_sensitive_even_with_keys() {
        let routing = RoutingConfig {
            anthropic_key: Some("a".into()),
            openai_key: Some("o".into()),
            google_key: Some("g".into()),
            sensitive: true,
            preferred_provider: "anthropic".into(),
            ..RoutingConfig::default()
        };
        assert_eq!(
            providers(&build_chain("gemma4:12b", &routing)),
            vec![llm::Provider::Ollama]
        );
    }

    #[test]
    fn build_chain_default_order_is_ollama_anthropic_openai_google() {
        let routing = RoutingConfig {
            anthropic_key: Some("a".into()),
            openai_key: Some("o".into()),
            google_key: Some("g".into()),
            ..RoutingConfig::default()
        };
        assert_eq!(
            providers(&build_chain("gemma4:12b", &routing)),
            vec![
                llm::Provider::Ollama,
                llm::Provider::Anthropic,
                llm::Provider::OpenAI,
                llm::Provider::Google
            ]
        );
    }

    #[test]
    fn build_chain_puts_a_buildable_preferred_provider_first_with_its_model() {
        let routing = RoutingConfig {
            google_key: Some("g".into()),
            preferred_provider: "google".into(),
            preferred_model: "gemini-2.5-pro".into(),
            ..RoutingConfig::default()
        };
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(
            providers(&chain),
            vec![llm::Provider::Google, llm::Provider::Ollama]
        );
        assert_eq!(chain[0].model, "gemini-2.5-pro");
        assert_eq!(chain[1].model, "gemma4:12b");
    }

    #[test]
    fn build_chain_unbuildable_preference_falls_through_to_ollama_first() {
        let routing = RoutingConfig {
            preferred_provider: "anthropic".into(),
            preferred_model: "claude-sonnet-5".into(),
            openai_key: Some("o".into()),
            ..RoutingConfig::default()
        };
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(
            providers(&chain),
            vec![llm::Provider::Ollama, llm::Provider::OpenAI]
        );
        // The non-preferred hosted fallback uses its hardcoded default model.
        assert_eq!(chain[1].model, "gpt-4o-mini");
    }

    // Moved verbatim from core/ffi's test module (M3.0, Task 8); the pointer-adapter
    // half of what they used to cover stays in ffi.
    fn test_routing_config(
        preferred_provider: &str,
        preferred_model: &str,
        anthropic_key: Option<&str>,
        openai_key: Option<&str>,
        sensitive: bool,
    ) -> RoutingConfig {
        test_routing_config_with_google(
            preferred_provider,
            preferred_model,
            anthropic_key,
            openai_key,
            None,
            sensitive,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn test_routing_config_with_google(
        preferred_provider: &str,
        preferred_model: &str,
        anthropic_key: Option<&str>,
        openai_key: Option<&str>,
        google_key: Option<&str>,
        sensitive: bool,
    ) -> RoutingConfig {
        RoutingConfig {
            anthropic_key: anthropic_key.map(str::to_string),
            openai_key: openai_key.map(str::to_string),
            google_key: google_key.map(str::to_string),
            typesafe_key: None,
            user_daily_cap_cents: None,
            global_daily_cap_cents: None,
            sensitive,
            preferred_provider: preferred_provider.to_string(),
            preferred_model: preferred_model.to_string(),
            custom_instructions: String::new(),
            daily_digest_instructions: String::new(),
            weekly_digest_instructions: String::new(),
            profile: String::new(),
            cheap_first: true,
            use_local_model: true,
            openrouter_key: None,
            openrouter_model: String::new(),
            openrouter_prices: Default::default(),
        }
    }

    #[test]
    fn from_json_reads_openrouter_fields_and_hands_prices_to_the_llm_table() {
        let r = RoutingConfig::from_json(Some(
            r#"{"openrouter_key":"sk-or-1","openrouter_model":" vendor/model ","openrouter_prices":{"vendor/parsed-model":{"in":0.01,"out":0.02},"bad":{"in":"x"}}}"#,
        ));
        assert_eq!(r.openrouter_key.as_deref(), Some("sk-or-1"));
        assert_eq!(r.openrouter_model, "vendor/model");
        assert_eq!(r.openrouter_prices.get("vendor/parsed-model"), Some(&(0.01, 0.02)));
        assert!(!r.openrouter_prices.contains_key("bad"), "a malformed price entry is dropped, not the config");
        assert!((llm::price_rank(llm::Provider::OpenRouter, "vendor/parsed-model") - 0.03).abs() < 1e-9);
        let r = RoutingConfig::from_json(Some("{}"));
        assert!(r.openrouter_key.is_none() && r.openrouter_model.is_empty() && r.openrouter_prices.is_empty());
    }

    #[test]
    fn openrouter_is_last_by_default_first_when_preferred_and_absent_without_a_model() {
        let routing = RoutingConfig { openrouter_key: Some("or".into()), openrouter_model: "vendor/model".into(), ..all_keys() };
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(
            providers(&chain),
            vec![Provider::Ollama, Provider::Anthropic, Provider::OpenAI, Provider::Google, Provider::OpenRouter]
        );
        assert_eq!(chain[4].model, "vendor/model");

        let routing = RoutingConfig { preferred_provider: "openrouter".into(), preferred_model: "vendor/big".into(), ..routing };
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain[0].provider, Provider::OpenRouter);
        assert_eq!(chain[0].model, "vendor/big");
        assert_eq!(chain.len(), 5);

        // A key with no pick yet: no link anywhere, preferred or not.
        let routing = RoutingConfig { openrouter_key: Some("or".into()), openrouter_model: String::new(), ..all_keys() };
        assert!(!providers(&build_chain("gemma4:12b", &routing)).contains(&Provider::OpenRouter));
        assert!(!providers(&build_chain_for(TaskKind::TopicName, "gemma4:12b", &routing)).contains(&Provider::OpenRouter));
        let routing = RoutingConfig { preferred_provider: "openrouter".into(), ..routing };
        assert_eq!(build_chain("gemma4:12b", &routing)[0].provider, Provider::Ollama);
    }

    #[test]
    fn light_chain_places_openrouter_by_its_cached_price_and_free_models_first() {
        let routing = RoutingConfig {
            openrouter_key: Some("or".into()),
            openrouter_model: "vendor/model:free".into(),
            ..all_keys()
        };
        let chain = build_chain_for(TaskKind::ConversationTitle, "gemma4:12b", &routing);
        assert_eq!(providers(&chain)[..2], [Provider::Ollama, Provider::OpenRouter]);

        let routing = RoutingConfig {
            openrouter_model: "vendor/pricey".into(),
            openrouter_prices: [("vendor/pricey".to_string(), (1.0, 5.0))].into_iter().collect(),
            ..routing
        };
        llm::set_price_overrides(routing.openrouter_prices.clone());
        let chain = build_chain_for(TaskKind::ConversationTitle, "gemma4:12b", &routing);
        assert_eq!(chain.last().unwrap().provider, Provider::OpenRouter);

        let pinned = RoutineModelPref::try_parse("openrouter/vendor/model:free").unwrap();
        assert_eq!(pinned, RoutineModelPref::Pinned { provider: Provider::OpenRouter, model: "vendor/model:free".into() });
        assert_eq!(pinned.as_string(), "openrouter/vendor/model:free");
    }

    /// Sub-project 2: `preferred_provider`/`preferred_model` reorder the chain instead
    /// of always putting Ollama first; a preference that can't actually be built (no key
    /// saved) falls back to the default Ollama-first order instead of silently dropping
    /// a link; `sensitive` still wins over any preference.
    #[test]
    fn build_chain_honors_preferred_provider_and_falls_back_when_unbuildable() {
        // No keys at all: Ollama-only regardless of preference.
        let routing = test_routing_config("ollama", "", None, None, false);
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].provider, llm::Provider::Ollama);
        assert_eq!(chain[0].model, "gemma4:12b");

        // Both keys present, Ollama preferred: default fixed order (Ollama, Anthropic,
        // OpenAI); hosted links use their hardcoded fallback models since neither is
        // the preference.
        let routing = test_routing_config(
            "ollama",
            "",
            Some("sk-test-fake"),
            Some("sk-test-fake"),
            false,
        );
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].provider, llm::Provider::Ollama);
        assert_eq!(chain[1].provider, llm::Provider::Anthropic);
        assert_eq!(chain[1].model, "claude-haiku-4-5");
        assert_eq!(chain[2].provider, llm::Provider::OpenAI);
        assert_eq!(chain[2].model, "gpt-4o-mini");

        // Anthropic preferred with a key: Anthropic becomes link 0, using the user's
        // actual preferred_model (not the hardcoded fallback) — then the rest in fixed
        // order.
        let routing = test_routing_config(
            "anthropic",
            "claude-sonnet-5",
            Some("sk-test-fake"),
            Some("sk-test-fake"),
            false,
        );
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].provider, llm::Provider::Anthropic);
        assert_eq!(chain[0].model, "claude-sonnet-5");
        assert_eq!(chain[1].provider, llm::Provider::Ollama);
        assert_eq!(chain[2].provider, llm::Provider::OpenAI);
        assert_eq!(chain[2].model, "gpt-4o-mini");

        // Anthropic preferred but NO Anthropic key saved: preference isn't buildable,
        // so it falls back to the default Ollama-first order rather than silently
        // dropping a link.
        let routing = test_routing_config(
            "anthropic",
            "claude-sonnet-5",
            None,
            Some("sk-test-fake"),
            false,
        );
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].provider, llm::Provider::Ollama);
        assert_eq!(chain[1].provider, llm::Provider::OpenAI);

        // sensitive=true wins regardless of preference or keys present (FR-CHAT-5
        // local-only mode).
        let routing = test_routing_config(
            "anthropic",
            "claude-sonnet-5",
            Some("sk-test-fake"),
            Some("sk-test-fake"),
            true,
        );
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].provider, llm::Provider::Ollama);
    }

    /// Google is the 4th/last fallback in the default (Ollama-preferred) order --
    /// today's local-first default (Ollama, Anthropic, OpenAI) stays untouched, Google
    /// only ever comes after all three.
    #[test]
    fn build_chain_places_google_fourth_by_default_and_first_when_preferred() {
        let routing = test_routing_config_with_google(
            "ollama",
            "",
            Some("sk-test-fake"),
            Some("sk-test-fake"),
            Some("goog-test-fake"),
            false,
        );
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0].provider, llm::Provider::Ollama);
        assert_eq!(chain[1].provider, llm::Provider::Anthropic);
        assert_eq!(chain[2].provider, llm::Provider::OpenAI);
        assert_eq!(chain[3].provider, llm::Provider::Google);
        assert_eq!(chain[3].model, "gemini-2.5-flash");

        // Google preferred with a key: Google becomes link 0 using the user's actual
        // preferred_model, then the rest in the same fixed order.
        let routing = test_routing_config_with_google(
            "google",
            "gemini-2.5-pro",
            Some("sk-test-fake"),
            Some("sk-test-fake"),
            Some("goog-test-fake"),
            false,
        );
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0].provider, llm::Provider::Google);
        assert_eq!(chain[0].model, "gemini-2.5-pro");
        assert_eq!(chain[1].provider, llm::Provider::Ollama);
        assert_eq!(chain[2].provider, llm::Provider::Anthropic);
        assert_eq!(chain[3].provider, llm::Provider::OpenAI);

        // Google preferred but no key saved: falls back to the default order, dropping
        // only the unbuildable Google link.
        let routing =
            test_routing_config_with_google("google", "gemini-2.5-pro", None, None, None, false);
        let chain = build_chain("gemma4:12b", &routing);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].provider, llm::Provider::Ollama);
    }

    #[test]
    fn from_json_defaults_to_ollama_only_on_null_or_malformed_json() {
        let routing = RoutingConfig::from_json(None);
        assert_eq!(routing.preferred_provider, "ollama");
        assert!(routing.anthropic_key.is_none());
        assert!(!routing.sensitive);

        let routing = RoutingConfig::from_json(Some("not valid json"));
        assert_eq!(routing.preferred_provider, "ollama");

        let routing = RoutingConfig::from_json(Some(
            "{\"preferred_provider\":\"anthropic\",\"preferred_model\":\"claude-haiku-4-5\"}",
        ));
        assert_eq!(routing.preferred_provider, "anthropic");
        assert_eq!(routing.preferred_model, "claude-haiku-4-5");
        assert!(routing.anthropic_key.is_none());
        assert_eq!(routing.user_daily_cap_cents, None);
    }

    #[test]
    fn from_json_reads_custom_instructions_and_trims_whitespace_only_to_empty() {
        let routing = RoutingConfig::from_json(None);
        assert_eq!(
            routing.custom_instructions, "",
            "null routing_config_json defaults custom_instructions to empty"
        );

        let routing = RoutingConfig::from_json(Some(
            "{\"custom_instructions\":\"Always answer in bullet points.\"}",
        ));
        assert_eq!(
            routing.custom_instructions,
            "Always answer in bullet points."
        );

        let routing = RoutingConfig::from_json(Some("{\"custom_instructions\":\"   \"}"));
        assert_eq!(
            routing.custom_instructions, "",
            "whitespace-only custom_instructions must trim to empty"
        );
    }

    #[test]
    fn from_json_reads_daily_and_weekly_digest_instructions_and_trims_whitespace_only_to_empty() {
        let routing = RoutingConfig::from_json(None);
        assert_eq!(routing.daily_digest_instructions, "");
        assert_eq!(routing.weekly_digest_instructions, "");

        let routing = RoutingConfig::from_json(Some(
            "{\"daily_digest_instructions\":\"be terse\",\"weekly_digest_instructions\":\"group by project\"}",
        ));
        assert_eq!(routing.daily_digest_instructions, "be terse");
        assert_eq!(routing.weekly_digest_instructions, "group by project");

        let routing = RoutingConfig::from_json(Some(
            "{\"daily_digest_instructions\":\"   \",\"weekly_digest_instructions\":\"   \"}",
        ));
        assert_eq!(routing.daily_digest_instructions, "");
        assert_eq!(routing.weekly_digest_instructions, "");
    }

    #[test]
    fn from_json_reads_and_trims_the_profile_and_defaults_to_empty() {
        assert_eq!(RoutingConfig::from_json(Some(r#"{"profile":"  Role: CFO \n"}"#)).profile, "Role: CFO");
        assert_eq!(RoutingConfig::from_json(Some(r#"{"profile":7}"#)).profile, "");
        assert_eq!(RoutingConfig::from_json(None).profile, "");
    }
}
