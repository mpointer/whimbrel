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

/// One excluded name, alias or client contact address (or an address's spoken form), as its
/// lowercase words.
#[derive(Clone)]
struct Term(Vec<String>);

/// The names and aliases of every client excluded from hosted models, and the addresses of its
/// known contacts (`exclusion::hosted_terms`). Matching is case-insensitive on whole words: the
/// text and each term are split into words (runs of letters and digits) and a term matches where
/// its words appear in a row. So "Acme" matches "Met ACME today" and "acme-related", not
/// "Acmeville"; "Acme Corp" matches across a line break; `wile@coyote.example` matches
/// `WILE@coyote.example’s`, `mailto:wile@coyote.example?subject=hi` and `wile＠coyote.example`,
/// and, read without its tag, `wile+pricing@coyote.example`; not `wile.e@coyote.example` (another
/// mailbox), while `wile@coyote.example.org` matches (fail closed). Every term, name or address, is
/// also matched in the text decoded (`memory::words::readings`: percent escapes, HTML entities and
/// invisible characters undone, up to four times; `wile%40coyote.example`, `wile&#64;coyote.example`,
/// `Ac&#109;e`). An address is also matched spoken (`spoken_forms`: "wile at coyote dot example"),
/// in text only. No pattern is ever built from a term. A term with no letters or digits is dropped
/// (it would match nothing, or everything).
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
            .flat_map(|t| {
                // As written and without invisible characters (review 5, M3), and an address's
                // spoken forms.
                // Only the spelling without invisible characters (review 6, M3): as written, an alias
                // `Ac\u{AD}me` would fold to "ac me" and match those two words.
                let plain = memory::words::without_invisibles(t);
                std::iter::once(words(&plain)).chain(spoken_forms(&plain)).map(Term).collect::<Vec<_>>()
            })
            .filter(|t| !t.0.is_empty())
            .collect();
        Self { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether the text `fingerprints` ([`word_fingerprints`], sorted) were taken from names any
    /// excluded client. A term longer than [`MAX_FINGERPRINT_WORDS`] words is looked up by its first
    /// [`MAX_FINGERPRINT_WORDS`] (a match counts as found). The fingerprints are taken over the
    /// same readings [`Self::find_in`] matches (`memory::words::word_fingerprints`), and text that
    /// could not be fully decoded matches here too. An address's spoken forms are looked up too, a
    /// form of five words or more by its first four: prose such as "john smith at acme said hi"
    /// matches `john.smith@acme-widgets.com` here (review 4, S1: fail closed; the answer is only
    /// kept off hosted models).
    pub fn find_in_fingerprints(&self, fingerprints: &[u32]) -> bool {
        // A name over four words is looked up by its first four (fix round 5, N5): absent means
        // absent; present counts as found.
        !self.terms.is_empty()
            && (fingerprints.binary_search(&memory::words::UNDECIDABLE_FINGERPRINT).is_ok()
                || self.terms.iter().any(|Term(term)| fingerprints.binary_search(&memory::words::term_fingerprint(term)).is_ok()))
    }

    /// Whether `text` names any excluded client, in any of its readings (`memory::words::readings`):
    /// as written, decoded (percent escapes, HTML entities, invisible characters; up to four passes),
    /// and each of those with plus tags read off (`wile+pricing@coyote.example` is
    /// `wile@coyote.example`). Text still decodable after the last pass matches (fail closed).
    pub fn find_in(&self, text: &str) -> bool {
        if self.terms.is_empty() {
            return false;
        }
        // The same readings the fingerprints are taken over (`memory::words::readings`): as
        // written, each decoding pass, each with plus tags read off. Text still decodable after
        // the last pass matches (fail closed).
        let readings = memory::words::readings(text);
        readings.undecidable || self.names_in_readings(&readings)
    }

    /// Whether `text` really names an excluded client in one of its readings
    /// (`memory::words::visit_readings`: up to `MAX_NAMING_DECODE_PASSES` full passes and
    /// `4 × MAX_NAMING_DECODE_PASSES + 2` single steps per chain, every reading also without plus
    /// tags), never only because it could not be fully decoded. Its work per text is bounded at
    /// `NAMING_READING_BUDGET` times the text's length (plus a floor), so one adversarial row costs
    /// a few linear passes at most. For what acts on the answer and cannot be undone or must not
    /// misfile: deleting a client's derived rows, filing a source under a client, the delete
    /// inventory (review 4, I2). The hosted gate and filters keep [`Self::find_in`], which fails
    /// closed. Known limit: text whose only mention is encoded past that budget reads as not naming
    /// the client here (kept off hosted models all the same).
    pub fn names_in(&self, text: &str) -> bool {
        if self.terms.is_empty() {
            return false;
        }
        // The same readings as `find_in` (`memory::words::visit_readings`), decoded with the deeper
        // cap and the naming budget, visited one at a time, never kept; undecidable alone never
        // matches here.
        memory::words::visit_readings(text, memory::words::MAX_NAMING_DECODE_PASSES, memory::words::NAMING_READING_BUDGET, &mut |t| {
            let haystack = words(t);
            self.terms.iter().any(|Term(term)| haystack.windows(term.len()).any(|w| w == term.as_slice()))
        })
        .0
    }

    fn names_in_readings(&self, readings: &memory::words::Readings) -> bool {
        readings.texts.iter().any(|t| {
            let haystack = words(t);
            self.terms.iter().any(|Term(term)| haystack.windows(term.len()).any(|w| w == term.as_slice()))
        })
    }
}

/// The spoken and bracketed forms of a contact address term (one with an `@`), as words:
/// `wile@coyote.example` is also matched as "wile at coyote dot example" (how speech recognition
/// writes a spoken address; `[at]`, `(at)` and `[dot]` fold to the same words) and as "wile at
/// coyote example" ("at" spelled out, the dots kept). Nothing for a name or alias.
fn spoken_forms(term: &str) -> Vec<Vec<String>> {
    let Some((local, domain)) = term.split_once('@') else { return Vec::new() };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return Vec::new();
    }
    let dotted = |part: &str| -> Vec<String> {
        let mut out = Vec::new();
        for (i, label) in part.split('.').enumerate() {
            if i > 0 {
                out.push("dot".to_string());
            }
            out.extend(words(label));
        }
        out
    };
    let at = vec!["at".to_string()];
    let spoken = [dotted(local), at.clone(), dotted(domain)].concat();
    let at_only = [words(local), at, words(domain)].concat();
    vec![spoken, at_only]
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

    /// Security review: every decoding the text match reads is read by the fingerprints too
    /// (answer stamps), through any number of encodings up to the pass cap; past it, both fail
    /// closed. Another client's contact matches neither way.
    #[test]
    fn layered_encodings_match_alike_as_text_and_as_fingerprints() {
        let set = ExclusionSet::new(vec!["wile@coyote.example".into()]);
        let fp = |t: &str| set.find_in_fingerprints(&word_fingerprints(t));
        for text in [
            "wile%2540coyote.example",
            "wile&amp;#64;coyote.example",
            "wile%2526%2364%253Bcoyote.example",
            "wile%EF%BC%A0coyote.example",
            "wile&#0000064;coyote.example",
            "wile&#x0040;coyote.example",
            "wile&#X00000040;coyote.example",
            "wile&#64coyote.example",
            "wile%26%23x0040%3Bcoyote%2Eexample",
            "wi%E2%80%8Ble%2Bdeal%2540coyote.example",
            "wile+deal&#64;coyote.example",
            "mailto:wile%252Bdeal%2540coyote.example",
            "wile%26#64;coyote.example",
            "wile@coyote.example",
            // Review 4, I1: every default-ignorable character inside the address.
            "wi\u{200E}le@coyote.example",
            "wi\u{200F}le@coyote.example",
            "wi\u{202E}le@coyote.example",
            "wi\u{2066}le@coyote.ex\u{2069}ample",
            "wi\u{2063}le@coyote.example",
            "wi\u{180E}le@coyote.example",
            "wi\u{FE0F}le@coyote.example",
            "wi\u{E0069}le@coyote.example",
            "wi\u{115F}le@coyote.ex\u{3164}ample",
            "wi\u{1D173}le@coyote.example",
            "wi\u{061C}le@coyote.ex\u{17B4}ample\u{FFA0}",
            "wi&#x200E;le@coyote.example",
            // Review 4, M1 and M2: named references with and without `;`, and a UTF-8 sequence split
            // across a percent escape and a reference.
            "wile&commatcoyote&periodexample",
            "wile&amp#64;coyote.example",
        ] {
            assert!(set.find_in(text), "text: {text:?}");
            assert!(fp(text), "fingerprints: {text:?}");
        }
        // The spoken form is matched in text only (review 3, M-a).
        assert!(set.find_in("call wile at coyote dot example"));
        for text in ["carol%2540example.com", "carol&#x0040;example.com", "50% off &amp; more", "plain words"] {
            assert!(!set.find_in(text), "text: {text:?}");
            assert!(!fp(text), "fingerprints: {text:?}");
        }
        // Past the pass cap: both fail closed, and only when something is excluded.
        let deep = format!("wile&{}#64;coyote.example", "amp;".repeat(memory::words::MAX_DECODE_PASSES));
        assert!(memory::words::readings(&deep).undecidable, "{deep}");
        assert!(set.find_in(&deep) && fp(&deep));
        assert!(!ExclusionSet::new(vec![]).find_in(&deep) && !ExclusionSet::new(vec![]).find_in_fingerprints(&word_fingerprints(&deep)));
    }

    /// Review 4, S1: an address's spoken forms are looked up by fingerprints too, a form of five
    /// words or more by its first four, so prose sharing those words makes an answer unclean (fail
    /// closed: the answer only stays off hosted models). The text check still needs every word.
    #[test]
    fn prose_sharing_a_spoken_address_prefix_fails_closed_in_the_recheck() {
        let set = ExclusionSet::new(vec!["john.smith@acme-widgets.com".into()]);
        let fp = |t: &str| set.find_in_fingerprints(&word_fingerprints(t));
        for prose in ["john smith at acme said hi", "john dot smith at acme was there"] {
            assert!(fp(prose), "fail closed: {prose:?}");
            assert!(!set.find_in(prose), "the text check needs the whole form: {prose:?}");
        }
        assert!(!fp("john smith said hi") && !fp("acme said hi"), "a prefix of fewer words does not");
        for written in ["john.smith@acme-widgets.com", "John.Smith%40acme-widgets.com", "john.smith&#64;acme-widgets.com", "john.smith+x@acme-widgets.com"] {
            assert!(fp(written) && set.find_in(written), "{written:?}");
        }
        // In text, the whole spoken form still matches.
        assert!(set.find_in("email john dot smith at acme widgets dot com today"));
    }

    /// Review 5, I1: a name glued to a letter that a later stage of the same pass decodes, or that
    /// the other kind of escape decodes, is still read alone (single-operation chains); as text and
    /// as fingerprints alike.
    #[test]
    fn a_name_glued_to_a_later_decoded_letter_is_read_alone() {
        let wile = ExclusionSet::new(vec!["wile@coyote.example".into()]);
        let acme = ExclusionSet::new(vec!["Acme".into()]);
        let jose = ExclusionSet::new(vec!["José".into()]);
        for (set, text) in [
            (&wile, "wile%40coyote.example%2541"),
            (&acme, "Ac%6De%2541"),
            (&acme, "Ac%6De%25C3%25A9"),
            (&acme, "Ac&#109;e&#37;41"),
            (&acme, "Ac%6De&#65;"),
            (&wile, "wile%40coyote.example&#x41;"),
            (&acme, "Ac&#109;e%41"),
            (&wile, "wile&#64;coyote.example%41"),
            (&acme, "Ac&amp;#109;e%41"),
            (&jose, "Jos%C3&#x25;A9 from Acme"),
            // Needs the chain to switch operation: a reference that yields an escape, then that
            // escape, with a doubly escaped letter glued after the name.
            (&acme, "Ac&#37;6De%2541"),
            // Review 6, I1: percent decoding once, then references (a URL-decoded value shown as
            // HTML), with a doubly escaped letter glued after the name.
            (&acme, "Ac%26%23109%3Be%2541"),
            (&wile, "wile%26%2364%3Bcoyote.example%2541"),
            (&acme, "see Ac%26%23x6D%3Be%2541 today"),
            // A name split only by an invisible character, with escapes on both sides.
            (&acme, "&#65;Ac\u{200B}me%5A"),
            (&acme, "Ac&shy;me signed"),
        ] {
            assert!(set.find_in(text), "text: {text:?}");
            assert!(set.find_in_fingerprints(&word_fingerprints(text)), "fingerprints: {text:?}");
            assert!(set.names_in(text), "names_in: {text:?}");
        }
        for text in ["Acmeville%41", "carol%40example.com%2541", "Ac%6Dez", "R&amp;D %41"] {
            assert!(!acme.find_in(text) && !wile.find_in(text), "{text:?}");
        }
    }

    /// Review 5, M3: a term's invisible characters are dropped too, so an alias pasted with a soft
    /// hyphen or a direction mark matches its plain spelling, and a plain term matches text with
    /// them inside.
    #[test]
    fn a_term_with_invisible_characters_matches_its_plain_spelling() {
        for (term, text) in [
            ("Ac\u{00AD}me", "Acme signed"),
            ("\u{845B}\u{E0100}\u{57CE}", "\u{845B}\u{57CE} office"),
            ("wi\u{200E}le@coyote.example", "mail wile@coyote.example"),
            ("Acme", "Ac\u{00AD}me signed"),
        ] {
            let set = ExclusionSet::new(vec![term.into()]);
            assert!(set.find_in(text) && set.names_in(text), "{term:?} in {text:?}");
            assert!(set.find_in_fingerprints(&word_fingerprints(text)), "fingerprints: {term:?} in {text:?}");
        }
        assert!(memory::words::names_any("Acme signed", &["Ac\u{00AD}me".to_string()]));
        assert!(!ExclusionSet::new(vec!["Ac\u{00AD}me".into()]).find_in("Acmeville"));
        // Review 6, M3: only the plain spelling is a term, so the alias never folds to "ac me".
        let shy = ExclusionSet::new(vec!["Ac\u{00AD}me".into()]);
        assert!(!shy.names_in("an ac me meeting") && !shy.find_in("an ac me meeting"), "two words are not the alias");
        assert!(shy.names_in("Acme signed") && shy.names_in("Ac\u{00AD}me signed"));
    }

    /// Review 6, M2: what deletes or files does bounded work per text, so one adversarial 1 MB row
    /// cannot hold a client delete for long, while an ordinary three-level encoded name is found.
    #[test]
    fn names_in_does_bounded_work_per_text() {
        let set = ExclusionSet::new(vec!["Acme".into(), "wile@coyote.example".into()]);
        assert!(set.names_in("Zorbl Ac%25256De signed"), "three levels of percent");
        assert!(set.names_in("Ac&amp;amp;#109;e signed"), "three levels of references");
        let rows = [
            format!("&{}#64;", "amp;".repeat(250_000)),
            format!("{}40", "%25".repeat(333_000)),
            "x%2540y &amp;#64; ".repeat(1_000_000 / 18),
            (0..20_000).map(|i| if i % 2 == 0 { "%2526" } else { "amp;" }).collect::<String>().repeat(10),
        ];
        let plain = "plain words in a row ".repeat(1_000_000 / 21);
        let started = std::time::Instant::now();
        let _ = set.names_in(&plain);
        let baseline = started.elapsed();
        for row in &rows {
            let started = std::time::Instant::now();
            let _ = set.names_in(row);
            let took = started.elapsed();
            assert!(took < baseline * 40 + std::time::Duration::from_secs(2), "{} bytes: {took:?} against {baseline:?}", row.len());
        }
    }

    /// Review 3, I1: decoding is linear per pass, with no search past an entity's few characters:
    /// 1 MB of `&`, of `&#` and of query-string text each take a small multiple of 1 MB of plain
    /// prose (a quadratic scan takes minutes here, seconds in release).
    #[test]
    fn decoding_a_megabyte_stays_linear() {
        let set = ExclusionSet::new(vec!["wile@coyote.example".into(), "Acme".into()]);
        let time = |text: &str| {
            let started = std::time::Instant::now();
            let _ = set.find_in(text);
            let _ = word_fingerprints(text);
            started.elapsed()
        };
        let baseline = time(&"plain words here ".repeat(1_000_000 / 17));
        for (name, text) in [
            ("&", "&".repeat(1_000_000)),
            ("&#", "&#".repeat(500_000)),
            ("query string", "a=1&b=2".repeat(1_000_000 / 7)),
            ("percent", "%".repeat(1_000_000)),
            ("&am", "&am".repeat(333_333)),
        ] {
            let took = time(&text);
            assert!(took < baseline * 10 + std::time::Duration::from_secs(1), "{name}: {took:?} against {baseline:?} for plain text");
        }
    }

    /// Security review: adversarial input stays linear (a pass cap times linear passes) and never
    /// panics.
    #[test]
    fn adversarial_encodings_are_read_fast_and_never_panic() {
        let set = ExclusionSet::new(vec!["wile@coyote.example".into(), "Acme".into()]);
        let started = std::time::Instant::now();
        for text in ["%".repeat(1_000_000), "&#".repeat(500_000), "at ".repeat(300_000), "&#0".repeat(300_000), "%25".repeat(300_000)] {
            let _ = set.find_in(&text);
            let _ = word_fingerprints(&text);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "took {:?}", started.elapsed());
        for text in ["%", "%4", "%%%", "&", "&#", "&#x", "&#;", "&#x;", "&#99999999;", "&#xFFFFFFFF;", "&commat", "%FF%FE%80", "%C3", "a%ZZb"] {
            let _ = set.find_in(text);
            let _ = word_fingerprints(text);
            let _ = memory::words::decode_once(text);
        }
    }

    /// Review C2: a client contact's address is matched as words, in the forms it is written in.
    #[test]
    fn a_contact_address_is_matched_as_words_in_every_written_form() {
        let set = ExclusionSet::new(vec!["wile@coyote.example".into()]);
        for text in [
            "wile@coyote.example",
            "WILE@Coyote.Example",
            "Wile E <wile@coyote.example>",
            "wile@coyote.example\u{2019}s team",
            "ask wile@coyote.example\u{2014}what pricing",
            "https://mail.example/compose?to=wile@coyote.example",
            "mailto:wile@coyote.example?subject=pricing",
            "see wile@coyote.example/carol",
            "wile\u{ff20}coyote.example",
            "wile+pricing@coyote.example",
            "Wile+A.B-c_d@coyote.example",
            "wile+pricing\u{ff20}coyote.example",
            // Review 2, I1: encoded, spoken and hidden forms.
            "https://mail.google.com/#search/from%3Awile%40coyote.example",
            "mailto:wile%40coyote.example",
            "Reply to wile&#64;coyote.example",
            "Reply to wile&#x40;coyote.example",
            "Reply to wile&commat;coyote&period;example",
            "Call wile at coyote dot example about pricing",
            "Wile At Coyote Dot Example",
            "wile [at] coyote [dot] example",
            "wile (at) coyote (dot) example",
            "wile at coyote.example",
            "wi\u{200B}le@coyote.example",
            "wile@coy\u{00AD}ote.example",
            "wile%2Bdeal%40coyote.example",
        ] {
            assert!(set.find_in(text), "{text:?}");
        }
        for text in [
            "carol@example.com",
            "wile.e@coyote.example",
            "wile@coyote.examples",
            "wile@coyote alone is not the address",
            "a + b @ c",
            "wile at the coyote example",
            "100%4 coyote &#9999999999; &bogus; %zz",
            "carol at example dot com",
        ] {
            assert!(!set.find_in(text), "{text:?}");
        }
        // A tag is read off only where it ends at an `@`: nothing else in the text changes.
        assert_eq!(memory::words::without_plus_tags("a+b@c and 1+2=3"), Some("a@c and 1+2=3".to_string()));
        assert_eq!(memory::words::without_plus_tags("1+2=3"), None);
        assert_eq!(memory::words::decode_once("a%40b &#64; &#x40; &commat; x\u{200B}y"), Some("a@b @ @ @ xy".to_string()));
        assert_eq!(memory::words::decode_once("no escapes"), None);
        // A name gets no spoken form.
        assert!(spoken_forms("Acme Corp").is_empty());
    }

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
