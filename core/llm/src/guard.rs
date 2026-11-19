//! The one gate every hosted request passes (X6 client controls, spec 7.3): a prompt that
//! names a client the user excluded from hosted models never leaves the device.
//!
//! A gate answers one question, "may this text go to a hosted model?", and its refusal
//! never carries the matched term: no client name reaches a log, an error, or `Debug`.
//! `api::exclusion::current_gate` builds the gate from the store; the chain builders wrap
//! every hosted link with [`guard_hosted`], the streaming entry points wrap their client
//! with [`GuardedStreamingClient`], and `typesafe::TypeSafeClient` checks its request.
//! Local links (Ollama, the bundled model) are never guarded: going local is how an
//! excluded client's request still gets an answer.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::streaming::{ProviderEventStream, StreamRequest, StreamingLlmClient};
use crate::{ChainLink, Completion, LlmClient, LlmError, Provider};

/// What a person sees when a request was held back. Never names the client.
pub const REFUSED_MESSAGE: &str = "This request was not sent: it involves a client you excluded from hosted models.";

/// What a person sees when the exclusion setting could not be read, so every hosted request
/// is held back until it can.
pub const UNREADABLE_MESSAGE: &str =
    "This request was not sent: your hosted-model exclusions could not be read. Try again in a moment.";

/// The refusal reason when the prompt names an excluded client.
pub const REASON_EXCLUDED: &str = "excluded client";
/// The refusal reason when the exclusion setting could not be read (fail closed).
pub const REASON_UNREADABLE: &str = "exclusion setting unreadable";

/// Why a gate held a request back. Only a fixed reason: no client text, ever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateRefusal {
    pub reason: &'static str,
}

impl GateRefusal {
    /// The sentence a person sees for this refusal. Never names the client.
    pub fn user_message(&self) -> &'static str {
        if self.reason == REASON_UNREADABLE { UNREADABLE_MESSAGE } else { REFUSED_MESSAGE }
    }

    /// The failure kind a shell shows for this refusal.
    pub fn failure_kind(&self) -> crate::ProviderFailureKind {
        if self.reason == REASON_UNREADABLE {
            crate::ProviderFailureKind::ExclusionUnreadable
        } else {
            crate::ProviderFailureKind::ExcludedClient
        }
    }
}

impl std::fmt::Display for GateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason)
    }
}

/// Decides whether text may go to a hosted model.
pub trait HostedGate: Send + Sync {
    fn check(&self, prompt: &str) -> Result<(), GateRefusal>;
}

/// Nothing is excluded: every prompt passes, at no cost.
pub struct NoExclusions;

impl HostedGate for NoExclusions {
    fn check(&self, _prompt: &str) -> Result<(), GateRefusal> {
        Ok(())
    }
}

/// The exclusion setting could not be read: every hosted prompt is refused until it can.
pub struct RefuseAllHosted;

impl HostedGate for RefuseAllHosted {
    fn check(&self, _prompt: &str) -> Result<(), GateRefusal> {
        Err(GateRefusal { reason: REASON_UNREADABLE })
    }
}

/// One excluded name or alias, as its lowercase words.
#[derive(Clone)]
struct Term(Vec<String>);

/// The names and aliases of every client excluded from hosted models. Matching is
/// case-insensitive on whole words: the text and each term are split into words (runs
/// of letters and digits) and a term matches where its words appear in a row. So "Acme"
/// matches "Met ACME today" and "acme-related", not "Acmeville"; "Acme Corp" matches
/// across a line break. No pattern is ever built from a term. A term with no letters or
/// digits is dropped (it would match nothing, or everything).
#[derive(Clone)]
pub struct ExclusionSet {
    terms: Vec<Term>,
}

/// The folded words of `text` ([`memory::words::folded_words`]).
fn words(text: &str) -> Vec<String> {
    memory::words::folded_words(text)
}

impl ExclusionSet {
    pub fn new(terms: Vec<String>) -> Self {
        let terms = terms
            .iter()
            .map(|t| words(t))
            .filter(|w| !w.is_empty())
            .map(Term)
            .collect();
        Self { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether the text `fingerprints` ([`word_fingerprints`], sorted) were taken from names any
    /// excluded client. A term longer than [`MAX_FINGERPRINT_WORDS`] words is looked up by its first
    /// [`MAX_FINGERPRINT_WORDS`] (a match counts as found).
    pub fn find_in_fingerprints(&self, fingerprints: &[u32]) -> bool {
        // A name over four words is looked up by its first four (fix round 5, N5): absent means
        // absent; present counts as found.
        self.terms.iter().any(|Term(term)| fingerprints.binary_search(&memory::words::term_fingerprint(term)).is_ok())
    }

    /// Whether `text` names any excluded client.
    pub fn find_in(&self, text: &str) -> bool {
        if self.terms.is_empty() {
            return false;
        }
        let haystack = words(text);
        self.terms
            .iter()
            .any(|Term(term)| haystack.windows(term.len()).any(|w| w == term.as_slice()))
    }
}

pub use memory::words::{word_fingerprints, MAX_FINGERPRINT_WORDS};

/// Only a count: the terms are client names.
impl std::fmt::Debug for ExclusionSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExclusionSet({} terms)", self.terms.len())
    }
}

impl HostedGate for ExclusionSet {
    fn check(&self, prompt: &str) -> Result<(), GateRefusal> {
        if self.find_in(prompt) {
            Err(GateRefusal { reason: REASON_EXCLUDED })
        } else {
            Ok(())
        }
    }
}

/// A hosted client behind a gate: a refused prompt never reaches `inner`.
pub struct GuardedClient {
    inner: Box<dyn LlmClient>,
    gate: Arc<dyn HostedGate>,
}

impl GuardedClient {
    pub fn new(inner: Box<dyn LlmClient>, gate: Arc<dyn HostedGate>) -> Self {
        Self { inner, gate }
    }
}

impl LlmClient for GuardedClient {
    fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
        self.gate.check(prompt).map_err(LlmError::Refused)?;
        self.inner.complete(prompt)
    }

    fn substituted_model(&self) -> Option<String> {
        self.inner.substituted_model()
    }

    /// Limited Use reads an Ollama link's host through the wrapper (an Ollama cloud link is
    /// guarded).
    fn base_url(&self) -> Option<&str> {
        self.inner.base_url()
    }
}

/// `link` with its client behind `gate`. Callers pass hosted links only.
pub fn guard_hosted(link: ChainLink, gate: Arc<dyn HostedGate>) -> ChainLink {
    ChainLink {
        provider: link.provider,
        model: link.model,
        client: Box::new(GuardedClient::new(link.client, gate)),
    }
}

/// A hosted streaming client behind a gate: a refused prompt fails the stream before any
/// connection is made.
pub struct GuardedStreamingClient {
    inner: Arc<dyn StreamingLlmClient>,
    gate: Arc<dyn HostedGate>,
}

impl GuardedStreamingClient {
    pub fn new(inner: Arc<dyn StreamingLlmClient>, gate: Arc<dyn HostedGate>) -> Self {
        Self { inner, gate }
    }
}

#[async_trait]
impl StreamingLlmClient for GuardedStreamingClient {
    fn provider(&self) -> Provider {
        self.inner.provider()
    }

    async fn stream(&self, request: StreamRequest, cancel: CancellationToken) -> Result<ProviderEventStream, LlmError> {
        self.gate.check(&request.prompt).map_err(LlmError::Refused)?;
        self.inner.stream(request, cancel).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenUsage;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fix round 4: a text's word fingerprints answer "does it name this client" like the text
    /// itself, folding included.
    #[test]
    fn word_fingerprints_match_like_the_text() {
        let fp = word_fingerprints("Notes from JOSÉ at Roadrunner Co., re: acme-related pricing");
        let set = |t: &[&str]| ExclusionSet::new(t.iter().map(|s| s.to_string()).collect());
        for term in ["Jose", "roadrunner co", "Acme", "acme related pricing"] {
            assert!(set(&[term]).find_in_fingerprints(&fp), "{term}");
            assert!(set(&[term]).find_in("Notes from JOSÉ at Roadrunner Co., re: acme-related pricing"));
        }
        for term in ["Acmeville", "Globex", "pricing acme"] {
            assert!(!set(&[term]).find_in_fingerprints(&fp), "{term}");
        }
        assert!(!set(&["Globex"]).find_in_fingerprints(&fp));
    }

    /// Fix round 5, N5: a client name over four words is looked up by its first four. A text that
    /// does not hold them does not name the client (before, every text did); one that does, does.
    #[test]
    fn a_long_client_name_is_looked_up_by_its_first_four_words() {
        let set = ExclusionSet::new(vec!["Acme Widgets International Holdings LLC".into()]);
        assert!(!set.find_in_fingerprints(&word_fingerprints("When is the renewal due?")));
        assert!(set.find_in_fingerprints(&word_fingerprints("Mail from Acme Widgets International Holdings LLC")));
        assert!(set.find_in_fingerprints(&word_fingerprints("acme widgets international holdings")), "first four: fail closed");
    }

    struct Counting(Arc<AtomicUsize>);

    impl LlmClient for Counting {
        fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Completion { text: "ok".into(), token_usage: TokenUsage::default() })
        }

        fn substituted_model(&self) -> Option<String> {
            Some("stand-in".into())
        }
    }

    fn acme() -> Arc<dyn HostedGate> {
        Arc::new(ExclusionSet::new(vec!["Acme".into(), "Acme Corp".into()]))
    }

    #[test]
    fn exclusion_set_matches_whole_words_case_insensitively() {
        let set = ExclusionSet::new(vec!["acme".into()]);
        assert!(set.find_in("Met ACME today"));
        assert!(set.find_in("acme-related, (Acme)."));
        assert!(!set.find_in("Acmeville"));
        assert!(!set.find_in("pacme acmes"));
        let two = ExclusionSet::new(vec!["Acme Corp".into()]);
        assert!(two.find_in("the acme\ncorp deal"));
        assert!(!two.find_in("acme alone, corp alone"));
    }

    #[test]
    fn accents_and_sharp_s_fold_alike_in_terms_and_text() {
        let precomposed = "Jos\u{e9}";
        let decomposed = "Jose\u{301}";
        for (term, text) in [(precomposed, decomposed), (decomposed, precomposed), (precomposed, "JOS\u{c9}")] {
            assert!(ExclusionSet::new(vec![term.into()]).find_in(&format!("call with {text} today")), "{term:?} in {text:?}");
        }
        // Folding drops the accent, so the plain spelling matches too (fails closed).
        assert!(ExclusionSet::new(vec![precomposed.into()]).find_in("call with Jose"));
        assert!(ExclusionSet::new(vec!["Strau\u{df}".into()]).find_in("the STRAUSS account"));
        assert!(ExclusionSet::new(vec!["STRAUSS".into()]).find_in("bei Strau\u{df}"));
        assert!(!ExclusionSet::new(vec![precomposed.into()]).find_in("Joseph"), "still whole words");
    }

    #[test]
    fn blank_terms_are_dropped_and_match_nothing() {
        let set = ExclusionSet::new(vec!["".into(), "  ".into(), "--".into()]);
        assert!(set.is_empty());
        assert!(!set.find_in("anything at all"));
        assert!(!ExclusionSet::new(vec!["Acme".into()]).is_empty());
    }

    #[test]
    fn a_term_with_pattern_characters_is_matched_as_words_not_as_a_pattern() {
        let set = ExclusionSet::new(vec!["a.*".into()]);
        assert!(!set.find_in("anything"));
        assert!(set.find_in("plan a. then"));
    }

    #[test]
    fn guarded_client_refuses_prompt_naming_excluded_client_without_calling_inner() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = GuardedClient::new(Box::new(Counting(calls.clone())), acme());
        let err = client.complete("Summarise the ACME call").unwrap_err();
        assert!(matches!(err, LlmError::Refused(GateRefusal { reason: REASON_EXCLUDED })));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn guarded_client_passes_clean_prompt() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = GuardedClient::new(Box::new(Counting(calls.clone())), acme());
        assert_eq!(client.complete("Summarise the Globex call").unwrap().text, "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(client.substituted_model().as_deref(), Some("stand-in"), "usage is priced under the model that answered");
    }

    #[test]
    fn refusal_carries_no_client_text() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = GuardedClient::new(Box::new(Counting(calls)), acme());
        let err = client.complete("Notes from Acme Corp").unwrap_err();
        let LlmError::Refused(refusal) = &err else { panic!("expected a refusal") };
        for shown in [format!("{refusal:?}"), refusal.to_string(), format!("{err:?}"), err.to_string()] {
            assert!(!shown.to_lowercase().contains("acme"), "{shown}");
        }
        assert_eq!(err.to_string(), REFUSED_MESSAGE);
        let set = ExclusionSet::new(vec!["Acme".into()]);
        assert!(!format!("{set:?}").to_lowercase().contains("acme"));
    }

    #[test]
    fn an_unreadable_setting_refuses_every_prompt_and_no_exclusions_passes_every_one() {
        let refusal = RefuseAllHosted.check("hello").unwrap_err();
        assert_eq!(refusal.reason, REASON_UNREADABLE);
        assert_eq!(LlmError::Refused(refusal).to_string(), UNREADABLE_MESSAGE);
        assert_eq!(refusal.failure_kind(), crate::ProviderFailureKind::ExclusionUnreadable);
        assert_eq!(crate::ProviderFailureKind::ExclusionUnreadable.user_message(), UNREADABLE_MESSAGE);
        assert!(NoExclusions.check("Acme").is_ok());
    }

    #[test]
    fn guard_hosted_keeps_provider_and_model() {
        let calls = Arc::new(AtomicUsize::new(0));
        let link = ChainLink { provider: Provider::Anthropic, model: "m".into(), client: Box::new(Counting(calls.clone())) };
        let link = guard_hosted(link, acme());
        assert_eq!((link.provider, link.model.as_str()), (Provider::Anthropic, "m"));
        assert!(matches!(link.client.complete("acme"), Err(LlmError::Refused(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// A refused hosted link falls through, so a local link after it still answers.
    #[test]
    fn a_refused_hosted_link_falls_through_to_a_local_one() {
        let hosted_calls = Arc::new(AtomicUsize::new(0));
        let local_calls = Arc::new(AtomicUsize::new(0));
        let chain = [
            guard_hosted(
                ChainLink { provider: Provider::Anthropic, model: "m".into(), client: Box::new(Counting(hosted_calls.clone())) },
                acme(),
            ),
            ChainLink { provider: Provider::Ollama, model: "l".into(), client: Box::new(Counting(local_calls.clone())) },
        ];
        let (_, link) = crate::try_chain(&chain, |l| l.client.complete("about acme")).expect("the local link answers");
        assert_eq!(link.provider, Provider::Ollama);
        assert_eq!(hosted_calls.load(Ordering::SeqCst), 0);
        assert_eq!(local_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            crate::ProviderFailureKind::classify(Provider::Anthropic, "m", &LlmError::Refused(GateRefusal { reason: REASON_EXCLUDED })),
            crate::ProviderFailureKind::ExcludedClient
        );
        assert_eq!(crate::ProviderFailureKind::ExcludedClient.user_message(), REFUSED_MESSAGE);
    }

    struct CountingStream(Arc<AtomicUsize>);

    #[async_trait]
    impl StreamingLlmClient for CountingStream {
        fn provider(&self) -> Provider {
            Provider::Anthropic
        }

        async fn stream(&self, _request: StreamRequest, _cancel: CancellationToken) -> Result<ProviderEventStream, LlmError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(LlmError::NotImplemented("test"))
        }
    }

    #[tokio::test]
    async fn guarded_stream_refuses_before_connecting() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = GuardedStreamingClient::new(Arc::new(CountingStream(calls.clone())), acme());
        let request = |prompt: &str| StreamRequest { run_id: "r".into(), model: "m".into(), prompt: prompt.into(), started_at_unix_ms: 0 };
        let Err(err) = client.stream(request("the Acme brief"), CancellationToken::new()).await else { panic!("expected a refusal") };
        assert!(matches!(err, LlmError::Refused(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(client.stream(request("the Globex brief"), CancellationToken::new()).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(client.provider(), Provider::Anthropic);
    }
}
