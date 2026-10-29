//! The one table of what the key on-ramp says about each provider: where to get a key,
//! where its data policy lives, what a key looks like, and the data-terms note where a
//! free tier pays for itself with prompts. Both shells and the website read this, so
//! the copy lives in one place. Design:
//! `docs/superpowers/specs/2026-10-29-cheap-first-routing-and-key-on-ramp-design.md`.
//!
//! Copy rules: plain sentences, no em or en dashes, no claim the app does not do. A
//! data-terms note carries the date it was checked, because these policies change and
//! the note must say when it was true.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataTermsNote {
    pub text: &'static str,
    /// ISO date the policy page was last read.
    pub checked: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderInfo {
    /// `llm::Provider::as_str()`.
    pub id: &'static str,
    pub name: &'static str,
    /// Where the user creates a key. `None` for Ollama, which has none.
    pub key_page_url: Option<&'static str>,
    /// The provider's data-use policy for API traffic.
    pub policy_url: Option<&'static str>,
    /// What a key starts with, for the format hint. Prefixes change, so a mismatch
    /// warns and never blocks.
    pub key_prefix: Option<&'static str>,
    /// Present only where the provider's free tier may use prompts.
    pub data_terms: Option<DataTermsNote>,
    /// The light-tier model (`api::routing::cheap_model_for`), empty where the user
    /// picks it (OpenRouter) or there is none (Ollama).
    pub cheap_model: &'static str,
}

pub const PROVIDERS: [ProviderInfo; 5] = [
    ProviderInfo {
        id: "ollama",
        name: "Ollama",
        key_page_url: None,
        policy_url: None,
        key_prefix: None,
        data_terms: None,
        cheap_model: "",
    },
    ProviderInfo {
        id: "anthropic",
        name: "Anthropic",
        key_page_url: Some("https://console.anthropic.com/settings/keys"),
        policy_url: Some("https://www.anthropic.com/legal/commercial-terms"),
        key_prefix: Some("sk-ant-"),
        data_terms: None,
        cheap_model: "claude-haiku-4-5",
    },
    ProviderInfo {
        id: "openai",
        name: "OpenAI",
        key_page_url: Some("https://platform.openai.com/api-keys"),
        policy_url: Some("https://openai.com/policies/business-terms/"),
        key_prefix: Some("sk-"),
        data_terms: None,
        cheap_model: "gpt-4o-mini",
    },
    ProviderInfo {
        id: "google",
        name: "Google Gemini",
        key_page_url: Some("https://aistudio.google.com/apikey"),
        policy_url: Some("https://ai.google.dev/gemini-api/terms"),
        key_prefix: Some("AIza"),
        data_terms: Some(DataTermsNote {
            text: "On Google's free tier, prompts and responses may be used to improve Google's products. The paid tier does not use them that way. Check Google's terms for the current wording.",
            checked: "2026-10-29",
        }),
        cheap_model: "gemini-2.0-flash",
    },
    ProviderInfo {
        id: "openrouter",
        name: "OpenRouter",
        key_page_url: Some("https://openrouter.ai/settings/keys"),
        policy_url: Some("https://openrouter.ai/privacy"),
        key_prefix: Some("sk-or-"),
        data_terms: Some(DataTermsNote {
            text: "Free models on OpenRouter may log prompts. Paid models follow each upstream provider's policy. Check OpenRouter's privacy page for the current wording.",
            checked: "2026-10-29",
        }),
        cheap_model: "",
    },
];

pub fn info(id: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// A one-line warning when a pasted key does not look like `provider`'s, or `None`
/// when it does (or the provider has no known prefix). Advisory only: the key is saved
/// either way, because prefixes change.
pub fn format_hint(provider: &str, key: &str) -> Option<String> {
    let info = info(provider)?;
    let prefix = info.key_prefix?;
    let key = key.trim();
    if key.is_empty() || key.starts_with(prefix) {
        return None;
    }
    Some(format!("This doesn't look like {} key. Keys from {} start with \"{}\".", article(info.name), info.name, prefix))
}

/// "an OpenAI key", "a Google Gemini key".
fn article(name: &str) -> String {
    let first = name.chars().next().map(|c| c.to_ascii_lowercase()).unwrap_or('x');
    if "aeiou".contains(first) {
        format!("an {name}")
    } else {
        format!("a {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_in_the_llm_enum_has_a_row_and_the_ids_match() {
        for p in [llm::Provider::Ollama, llm::Provider::Anthropic, llm::Provider::OpenAI, llm::Provider::Google, llm::Provider::OpenRouter] {
            let row = info(p.as_str()).unwrap_or_else(|| panic!("no row for {}", p.as_str()));
            assert_eq!(row.id, p.as_str());
            if !p.is_local() {
                assert!(row.key_page_url.is_some() && row.policy_url.is_some() && row.key_prefix.is_some(), "{}", row.id);
            }
            assert_eq!(row.cheap_model, crate::routing::cheap_model_for(p, None), "{}", row.id);
        }
        assert!(info("venice").is_none());
    }

    #[test]
    fn copy_has_no_dashes_and_every_data_terms_note_is_dated() {
        for p in &PROVIDERS {
            if let Some(note) = &p.data_terms {
                assert!(!note.text.contains('\u{2014}') && !note.text.contains('\u{2013}'), "{}", p.id);
                assert_eq!(note.checked.len(), 10, "{} note needs an ISO date", p.id);
            }
        }
        assert!(PROVIDERS.iter().filter(|p| p.data_terms.is_some()).map(|p| p.id).eq(["google", "openrouter"]));
    }

    #[test]
    fn format_hint_warns_on_a_wrong_prefix_and_stays_quiet_otherwise() {
        assert_eq!(format_hint("anthropic", "sk-ant-abc"), None);
        assert_eq!(format_hint("anthropic", "  sk-ant-abc  "), None);
        assert_eq!(format_hint("anthropic", ""), None);
        assert_eq!(
            format_hint("anthropic", "sk-abc").as_deref(),
            Some("This doesn't look like an Anthropic key. Keys from Anthropic start with \"sk-ant-\".")
        );
        assert_eq!(
            format_hint("google", "sk-abc").as_deref(),
            Some("This doesn't look like a Google Gemini key. Keys from Google Gemini start with \"AIza\".")
        );
        assert_eq!(format_hint("openrouter", "sk-or-v1-x"), None);
        assert_eq!(format_hint("ollama", "anything"), None);
        assert_eq!(format_hint("venice", "anything"), None);
    }

    #[test]
    fn serializes_for_the_shells() {
        let json = serde_json::to_value(&PROVIDERS[3]).unwrap();
        assert_eq!(json["id"], "google");
        assert_eq!(json["data_terms"]["checked"], "2026-10-29");
        assert_eq!(serde_json::to_value(&PROVIDERS[0]).unwrap()["key_page_url"], serde_json::Value::Null);
    }
}
