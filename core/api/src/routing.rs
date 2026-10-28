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
    let ollama_link = || llm::ChainLink {
        provider: llm::Provider::Ollama,
        model: ollama_model.to_string(),
        client: Box::new(llm::OllamaClient::new(ollama_model)),
    };
    if routing.sensitive {
        return vec![ollama_link()];
    }

    const FALLBACK_ANTHROPIC_MODEL: &str = "claude-haiku-4-5";
    const FALLBACK_OPENAI_MODEL: &str = "gpt-4o-mini";
    const FALLBACK_GOOGLE_MODEL: &str = "gemini-2.5-flash";

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

    let mut ollama_slot = Some(ollama_link());
    let mut anthropic_slot = routing.anthropic_key.clone().map(|key| llm::ChainLink {
        provider: llm::Provider::Anthropic,
        model: anthropic_model.to_string(),
        client: Box::new(llm::AnthropicClient::new(key, anthropic_model)),
    });
    let mut openai_slot = routing.openai_key.clone().map(|key| llm::ChainLink {
        provider: llm::Provider::OpenAI,
        model: openai_model.to_string(),
        client: Box::new(llm::OpenAIClient::new(key, openai_model)),
    });
    let mut google_slot = routing.google_key.clone().map(|key| llm::ChainLink {
        provider: llm::Provider::Google,
        model: google_model.to_string(),
        client: Box::new(llm::GoogleClient::new(key, google_model)),
    });

    let mut chain = Vec::with_capacity(4);
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
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn providers(chain: &[llm::ChainLink]) -> Vec<llm::Provider> {
        chain.iter().map(|l| l.provider).collect()
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
        }
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
