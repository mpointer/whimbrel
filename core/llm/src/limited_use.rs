//! Google API Services "Limited Use": which provider links may receive Gmail and Google
//! Calendar text. Whimbrel sends it only to a model on the user's own device, or to
//! services whose terms say they do not train on API data by default. Which rows count as
//! Google user data is `memory::limited_use`; this module is the per-link rule, applied
//! wherever a prompt is built for one link of a chain (the chain can fall back between
//! providers, so the rule is per link, never once per request).
//!
//! | Link | Gmail and Calendar text |
//! |---|---|
//! | Anthropic (user key), Whimbrel AI, OpenAI | yes |
//! | Bundled local model | yes |
//! | Ollama on loopback, not a cloud model | yes |
//! | Ollama elsewhere, or a `:cloud` / `-cloud` model | only with `remote_ollama_google_data` |
//! | Google (Gemini through an AI Studio key) | only with `google_ai_billing_confirmed` |
//! | OpenRouter | never (every request also carries `data_collection: "deny"`) |
//!
//! Text that a link may not receive is left out of its prompt; the request still runs and
//! the answer just doesn't use those items.

use crate::{ChainLink, ContextItem, Provider};

/// The two user switches that widen the rule; both default to off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoogleDataPolicy {
    /// The user's Google AI Studio key is on a paid (billing-enabled) project, where
    /// Google's terms say prompts are not used to improve its products.
    pub google_ai_billing_confirmed: bool,
    /// The user accepts sending Gmail and Calendar text to a non-loopback Ollama or an
    /// Ollama cloud model.
    pub remote_ollama_google_data: bool,
}

/// Ollama's default host, what [`ChainLink`]s built in core use.
const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

/// Whether `url`'s host is loopback: `localhost`, `127.0.0.0/8` or `::1`.
pub fn is_loopback_url(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Ollama's cloud models run on Ollama's servers even through a local Ollama; their names
/// end in `:cloud` or `-cloud` (`gpt-oss:120b-cloud`, `kimi-k2:1t-cloud`).
pub fn is_ollama_cloud_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.ends_with(":cloud") || m.ends_with("-cloud")
}

/// Whether a link to `provider` at `model` may receive Gmail and Google Calendar text.
/// `ollama_base_url` is the Ollama host (`None` means Ollama's default, localhost).
pub fn provider_may_receive_google_data(
    provider: Provider,
    model: &str,
    ollama_base_url: Option<&str>,
    policy: GoogleDataPolicy,
) -> bool {
    match provider {
        Provider::Anthropic | Provider::Whimbrel | Provider::OpenAI | Provider::Local => true,
        Provider::Google => policy.google_ai_billing_confirmed,
        Provider::OpenRouter => false,
        Provider::Ollama => {
            let on_device = is_loopback_url(ollama_base_url.unwrap_or(DEFAULT_OLLAMA_BASE_URL)) && !is_ollama_cloud_model(model);
            on_device || policy.remote_ollama_google_data
        }
    }
}

/// [`provider_may_receive_google_data`] for one chain link, reading an Ollama link's host
/// from its client.
pub fn link_may_receive_google_data(link: &ChainLink, policy: GoogleDataPolicy) -> bool {
    provider_may_receive_google_data(link.provider, &link.model, link.client.base_url(), policy)
}

/// Whether every link of `chain` may receive Google user data: the rule for a prompt that
/// is built once, before the chain is walked.
pub fn chain_may_receive_google_data(chain: &[ChainLink], policy: GoogleDataPolicy) -> bool {
    chain.iter().all(|link| link_may_receive_google_data(link, policy))
}

impl ContextItem {
    /// Whether this context item carries Gmail or Google Calendar text. A thread is a
    /// Gmail thread (its summary is written from the messages, its text may carry one);
    /// a routine run is text generated from them. Captures, transcripts, episodes and
    /// slide snapshots come from the screen and meeting audio, which are not Google data.
    pub fn is_google_user_data(&self) -> bool {
        match self {
            ContextItem::Thread(t) => memory::is_google_user_data("thread", Some(&t.source)),
            ContextItem::RoutineRun(_) => memory::is_google_user_data("routine_run", None),
            ContextItem::Capture(_)
            | ContextItem::Transcript(_)
            | ContextItem::Episode(_)
            | ContextItem::Snapshot(_) => false,
        }
    }
}

/// `items`, minus the Google user data unless `allow_google` (then a plain copy).
pub fn filter_context(items: &[ContextItem], allow_google: bool) -> Vec<ContextItem> {
    items
        .iter()
        .filter(|item| allow_google || !item.is_google_user_data())
        .cloned()
        .collect()
}

/// What replaces a meeting's title in a prompt that may not hold Google Calendar text.
pub const WITHHELD_TITLE: &str = "(title withheld)";

/// [`filter_context`] that also looks at the store: unless `allow_google`, a transcript or
/// slide item from a meeting that may hold Google Calendar text (its title came from the
/// event, see `memory::Store::meeting_may_hold_google_calendar_data`) keeps its text but
/// not its title, which is [`WITHHELD_TITLE`]; a Focus Time routine run stays.
pub fn filter_context_in(store: &memory::Store, items: &[ContextItem], allow_google: bool) -> Vec<ContextItem> {
    if allow_google {
        return items.to_vec();
    }
    let mut routines: Option<Vec<memory::Routine>> = None;
    let mut out: Vec<ContextItem> = items
        .iter()
        .filter(|item| match item {
            // Focus Time makes no model call and reads no Gmail or Calendar text, so its
            // runs are not Google data; every other routine's output is.
            ContextItem::RoutineRun(r) => {
                let routines = routines.get_or_insert_with(|| store.list_routines().unwrap_or_default());
                routines.iter().any(|x| x.id == r.routine_id && x.template == memory::RoutineTemplate::FocusTime)
            }
            _ => !item.is_google_user_data(),
        })
        .cloned()
        .collect();
    for item in &mut out {
        match item {
            ContextItem::Transcript(t) if store.meeting_may_hold_google_calendar_data(t.meeting_id) => {
                t.meeting_title = WITHHELD_TITLE.to_string();
            }
            ContextItem::Snapshot(h) if store.meeting_may_hold_google_calendar_data(h.meeting_id) => {
                h.meeting_title = WITHHELD_TITLE.to_string();
            }
            _ => {}
        }
    }
    out
}

/// The context one link may receive: `items` minus the Google user data it may not.
pub fn context_for_link(items: &[ContextItem], link: &ChainLink, policy: GoogleDataPolicy) -> Vec<ContextItem> {
    filter_context(items, link_may_receive_google_data(link, policy))
}

/// [`context_for_link`] with [`filter_context_in`]'s meeting titles.
pub fn context_for_link_in(store: &memory::Store, items: &[ContextItem], link: &ChainLink, policy: GoogleDataPolicy) -> Vec<ContextItem> {
    filter_context_in(store, items, link_may_receive_google_data(link, policy))
}

/// The error a link returns when the prompt it would get is all Google user data it may
/// not receive: the chain walk moves on to the next link (`falls_through`), and the
/// reason is logged with the link's other failures.
pub fn withheld_from_link() -> crate::LlmError {
    crate::LlmError::NotImplemented("limited use: this link may not receive Gmail or Google Calendar text")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Completion, LlmClient, LlmError, OllamaClient};

    struct Nop;
    impl LlmClient for Nop {
        fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
            Err(LlmError::NotImplemented("nop"))
        }
    }

    fn link(provider: Provider, model: &str) -> ChainLink {
        ChainLink { provider, model: model.into(), client: Box::new(Nop) }
    }

    fn ollama(base: &str, model: &str) -> ChainLink {
        ChainLink { provider: Provider::Ollama, model: model.into(), client: Box::new(OllamaClient::with_base_url(base, model)) }
    }

    const OFF: GoogleDataPolicy = GoogleDataPolicy { google_ai_billing_confirmed: false, remote_ollama_google_data: false };

    #[test]
    fn loopback_hosts() {
        for url in ["http://localhost:11434", "http://127.0.0.1:11434", "http://[::1]:11434", "http://127.0.0.2", "localhost:11434", "http://LOCALHOST"] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in ["http://192.168.1.10:11434", "https://ollama.example.com", "http://localhost.evil.com:11434", "http://[2001:db8::1]:11434", "http://10.0.0.1"] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    #[test]
    fn cloud_model_names() {
        assert!(is_ollama_cloud_model("gpt-oss:120b-cloud"));
        assert!(is_ollama_cloud_model("deepseek-v3.1:cloud"));
        assert!(!is_ollama_cloud_model("gemma4:12b"));
        assert!(!is_ollama_cloud_model("cloudy:7b"));
    }

    #[test]
    fn anthropic_openai_whimbrel_and_local_always_receive_it() {
        for p in [Provider::Anthropic, Provider::Whimbrel, Provider::OpenAI, Provider::Local] {
            assert!(link_may_receive_google_data(&link(p, "m"), OFF), "{p:?}");
        }
    }

    #[test]
    fn local_ollama_receives_it() {
        assert!(link_may_receive_google_data(&ollama("http://localhost:11434", "gemma4:12b"), OFF));
        assert!(link_may_receive_google_data(&ollama("http://127.0.0.1:11434", "gemma4:12b"), OFF));
        assert!(link_may_receive_google_data(&link(Provider::Ollama, "gemma4:12b"), OFF), "no host reported means the default, localhost");
    }

    #[test]
    fn remote_or_cloud_ollama_receives_it_only_with_the_flag() {
        let on = GoogleDataPolicy { remote_ollama_google_data: true, ..OFF };
        for l in [ollama("http://192.168.1.10:11434", "gemma4:12b"), ollama("http://localhost:11434", "gpt-oss:120b-cloud"), ollama("http://localhost:11434", "qwen3:cloud")] {
            assert!(!link_may_receive_google_data(&l, OFF), "{}", l.model);
            assert!(link_may_receive_google_data(&l, on), "{}", l.model);
        }
    }

    #[test]
    fn gemini_receives_it_only_once_billing_is_confirmed() {
        let l = link(Provider::Google, "gemini-2.5-flash");
        assert!(!link_may_receive_google_data(&l, OFF));
        assert!(link_may_receive_google_data(&l, GoogleDataPolicy { google_ai_billing_confirmed: true, ..OFF }));
        assert!(!link_may_receive_google_data(&l, GoogleDataPolicy { remote_ollama_google_data: true, ..OFF }));
    }

    #[test]
    fn openrouter_never_receives_it() {
        let all = GoogleDataPolicy { google_ai_billing_confirmed: true, remote_ollama_google_data: true };
        assert!(!link_may_receive_google_data(&link(Provider::OpenRouter, "anthropic/claude-sonnet-4"), all));
    }

    #[test]
    fn a_chain_receives_it_only_when_every_link_does() {
        let chain = vec![link(Provider::Anthropic, "m"), ollama("http://localhost:11434", "gemma4:12b")];
        assert!(chain_may_receive_google_data(&chain, OFF));
        let chain = vec![link(Provider::Anthropic, "m"), link(Provider::Google, "g")];
        assert!(!chain_may_receive_google_data(&chain, OFF));
    }

    fn routine_run(summary: &str) -> ContextItem {
        ContextItem::RoutineRun(memory::RoutineRunWithName {
            id: 1,
            routine_id: 1,
            routine_name: "Daily Brief".into(),
            ran_at_unix_ms: 1,
            status: memory::RunStatus::Success,
            output_summary: summary.into(),
            output_json: "{}".into(),
            origin_device_id: None,
        })
    }

    /// A routine's output is written from Gmail threads and Calendar events (and the
    /// run's row does not say which), so it is Google user data where it is reused.
    #[test]
    fn routine_run_output_is_google_user_data() {
        let items = vec![routine_run("BRIEFTEXT")];
        assert!(items[0].is_google_user_data());
        assert!(filter_context(&items, false).is_empty());
        assert_eq!(filter_context(&items, true).len(), 1);
    }
}
