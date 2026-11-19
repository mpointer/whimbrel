//! Retrieval that leaves an excluded client's material out (X6 client controls, spec 7.2).
//!
//! [`ContextFilter`] decides, for each retrieved item, whether it may go into a prompt that can
//! reach a hosted model. An item is left out when it is filed under an excluded client (its
//! meeting, episode, email, thread, slide or capture is a member, following merges; an item in two
//! topics is out when either is excluded) or when its text or label names an excluded client as
//! whole words (unlinked material, fail closed). When the setting cannot be read, everything is
//! left out. Both `gather_and_assemble_context*` take a filter, so no retrieval can skip the
//! decision: a caller whose prompt stays on this computer passes [`ContextFilter::none`].
//!
//! The filter never says why: callers show "left out material" locally, and the hosted prompt
//! carries neither the client's name nor the reason.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::guard::ExclusionSet;
use crate::{ContextItem, ContextItemRef, Provider};

/// Whether an Ollama model runs on Ollama's servers rather than this computer: Ollama names its
/// cloud models with a `cloud` tag or suffix (`gpt-oss:120b-cloud`, `qwen3-coder:480b-cloud`,
/// `kimi-k2:1t-cloud`, `deepseek-v3.1:671b-cloud`, `glm-4.6:cloud`). Matched fail closed: any
/// model whose tag or name has a `cloud` word counts.
pub fn is_ollama_cloud_model(model: &str) -> bool {
    model
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == "cloud")
}

/// Whether a request to `provider` at `model` leaves this computer. The one place that decides
/// it: an Ollama model that runs in Ollama's cloud is hosted, like any other provider's.
pub fn is_hosted(provider: Provider, model: &str) -> bool {
    match provider {
        Provider::Local => false,
        Provider::Ollama => is_ollama_cloud_model(model),
        _ => true,
    }
}

/// Decides which retrieved items may go into a hosted prompt; counts the ones it leaves out.
pub struct ContextFilter {
    mode: Mode,
    left_out: RefCell<HashSet<ContextItemRef>>,
}

enum Mode {
    /// Everything passes: the prompt stays on this computer, or nothing is excluded.
    Open,
    /// The setting cannot be read: nothing passes.
    Closed,
    Excluding { set: ExclusionSet, material: memory::topics::exclusion::ExcludedMaterial },
}

impl ContextFilter {
    /// Lets everything through: for a prompt that stays on this computer.
    pub fn none() -> Self {
        Self { mode: Mode::Open, left_out: RefCell::default() }
    }

    /// Leaves everything out: the exclusion setting cannot be read.
    pub fn closed() -> Self {
        Self { mode: Mode::Closed, left_out: RefCell::default() }
    }

    /// The filter for hosted prompts, read from `store` now: one predicate for every path
    /// (`memory::Store::excluded_material_with`: membership, names anywhere in a meeting or thread,
    /// client contacts' addresses, meetings recorded from the client's events, clean routine
    /// runs). Cached on the store until anything it reads changes.
    pub fn load(store: &memory::Store) -> Self {
        let terms = match store.excluded_terms() {
            Ok(t) => t,
            Err(_) => return Self::closed(),
        };
        let set = ExclusionSet::new(terms);
        let material = match store.excluded_material_with(&|t| set.find_in(t)) {
            Ok(m) => m,
            Err(_) => return Self::closed(),
        };
        if material.is_empty() {
            return Self::none();
        }
        Self { mode: Mode::Excluding { set, material }, left_out: RefCell::default() }
    }

    /// A fingerprint of the exclusion state this filter was read under: `Some("open")` when
    /// nothing is excluded, `None` when the setting could not be read (nothing may be cached
    /// then). A cache built under one version is never served under another. Never names a client.
    pub fn state_version(&self) -> Option<String> {
        match &self.mode {
            Mode::Open => Some("open".to_string()),
            Mode::Closed => None,
            Mode::Excluding { material, .. } => Some(material.material_version.clone()),
        }
    }

    /// Whether this filter can leave anything out.
    pub fn is_active(&self) -> bool {
        !matches!(self.mode, Mode::Open)
    }

    /// Whether the setting could not be read (every item is left out).
    pub fn is_closed(&self) -> bool {
        matches!(self.mode, Mode::Closed)
    }

    /// The excluded ids, when some client is excluded.
    pub fn material(&self) -> Option<&memory::topics::exclusion::ExcludedMaterial> {
        match &self.mode {
            Mode::Excluding { material, .. } => Some(material),
            _ => None,
        }
    }

    /// Whether `text` names an excluded client (always true when the setting cannot be read).
    pub fn names_client(&self, text: &str) -> bool {
        match &self.mode {
            Mode::Open => false,
            Mode::Closed => true,
            Mode::Excluding { set, .. } => set.find_in(text),
        }
    }

    /// Whether the text [`crate::guard::word_fingerprints`] were taken from names an excluded client.
    pub fn names_client_in_fingerprints(&self, fingerprints: &[u32]) -> bool {
        match &self.mode {
            Mode::Open => false,
            Mode::Closed => true,
            Mode::Excluding { set, .. } => set.find_in_fingerprints(fingerprints),
        }
    }

    /// Whether `item` may go into a hosted prompt. Records the ones it may not.
    pub fn keeps(&self, item: &ContextItem) -> bool {
        let keep = match &self.mode {
            Mode::Open => true,
            Mode::Closed => false,
            Mode::Excluding { set, material } => {
                !(filed_under(item, material)
                    || set.find_in(&item.display_label())
                    || set.find_in(item.text())
                    || matches!(item, ContextItem::Capture(c)
                        if set.find_in(&c.window_title) || c.url.as_deref().is_some_and(|u| set.find_in(u))))
            }
        };
        if !keep {
            self.left_out.borrow_mut().insert(item.citation_ref());
        }
        keep
    }

    /// `items` without the ones [`Self::keeps`] leaves out.
    pub fn retain(&self, items: Vec<ContextItem>) -> Vec<ContextItem> {
        items.into_iter().filter(|i| self.keeps(i)).collect()
    }

    /// Whether meeting `meeting_id`, titled `title` with `attendees`, belongs to an excluded client.
    pub fn excludes_meeting(&self, meeting_id: i64, title: &str, attendees: &[String]) -> bool {
        match &self.mode {
            Mode::Open => false,
            Mode::Closed => true,
            Mode::Excluding { set, material } => {
                material.meetings.contains(&meeting_id)
                    || set.find_in(title)
                    || attendees.iter().any(|a| set.find_in(a) || attendee_is_client(material, a))
            }
        }
    }

    /// Whether meeting `meeting_id` (as stored) belongs to an excluded client.
    pub fn excludes_meeting_id(&self, meeting_id: i64) -> bool {
        match &self.mode {
            Mode::Open => false,
            Mode::Closed => true,
            Mode::Excluding { material, .. } => material.meetings.contains(&meeting_id),
        }
    }

    /// Whether topic source `(kind, ref_id)` belongs to an excluded client: the same function
    /// [`Self::keeps`] uses for its retrieved items.
    pub fn excludes_source(&self, kind: &str, ref_id: i64) -> bool {
        match &self.mode {
            Mode::Open => false,
            Mode::Closed => true,
            Mode::Excluding { material, .. } => excluded_ref(material, kind, ref_id),
        }
    }

    /// Whether a calendar event (its calendar id, title and start) is an excluded client's: filed
    /// under one, or its title names one.
    pub fn excludes_event(&self, event_id: &str, title: &str, start_unix_ms: i64) -> bool {
        match &self.mode {
            Mode::Open => false,
            Mode::Closed => true,
            Mode::Excluding { set, material } => {
                material.event_ids.contains(event_id)
                    || (!event_id.is_empty() && material.event_series.contains(&memory::topics::exclusion::event_series_id(event_id)))
                    || material.events.contains(&(title.trim().to_lowercase(), start_unix_ms))
                    || set.find_in(title)
            }
        }
    }

    /// Whether any of `emails` (addresses, or `Name <address>` strings) is a client contact's.
    pub fn addresses_client(&self, emails: &[String]) -> bool {
        match &self.mode {
            Mode::Open => false,
            Mode::Closed => true,
            Mode::Excluding { material, .. } => emails.iter().any(|e| {
                attendee_is_client(material, e) || material.attendee_emails.contains(&e.trim().to_lowercase())
            }),
        }
    }

    /// How many distinct retrieved items were left out so far.
    pub fn left_out(&self) -> usize {
        self.left_out.borrow().len()
    }
}

/// Whether attendee string `a` (an address, `Name <address>`, or a name) carries an address met
/// only in an excluded client's material.
pub fn attendee_is_client(material: &memory::topics::exclusion::ExcludedMaterial, a: &str) -> bool {
    if material.attendee_emails.is_empty() {
        return false;
    }
    memory::topics::entities::parse_addresses(a)
        .iter()
        .any(|addr| material.attendee_emails.contains(&addr.email.to_lowercase()))
}

/// The one membership answer for `(kind, id)`, shared by [`ContextFilter::keeps`] (retrieved
/// items) and [`ContextFilter::excludes_source`] (topic sources) so they cannot diverge (fix
/// round 2, N2): [`memory::topics::exclusion::ExcludedMaterial::excludes`], which recorded inputs
/// go through too.
fn excluded_ref(m: &memory::topics::exclusion::ExcludedMaterial, kind: &str, id: i64) -> bool {
    m.excludes(kind, id)
}

/// The recorded input ([`memory::topics::exclusion::input_ref`]) a retrieved item stands for: the
/// same `(kind, id)` [`ContextFilter::keeps`] checks, so a stamp is clean exactly while every
/// item it records would still be kept (fix round 4, S2).
pub fn input_ref_of(item: &ContextItem) -> String {
    let (kind, id) = item_ref(item);
    memory::topics::exclusion::input_ref(kind, id)
}

/// The `(kind, id)` [`excluded_ref`] reads for a retrieved item. A transcript segment is its
/// meeting's; a slide is its own `items` row (the material lifts every slide of an excluded
/// meeting); a thread is its thread (the material lifts every email of an excluded thread).
fn item_ref(item: &ContextItem) -> (&'static str, i64) {
    match item {
        ContextItem::Capture(c) => ("capture", c.id),
        ContextItem::Transcript(t) => ("meeting", t.meeting_id),
        ContextItem::Snapshot(h) => ("snapshot", h.id),
        ContextItem::Episode(e) => ("episode", e.id),
        ContextItem::Thread(t) => ("thread", t.id),
        ContextItem::RoutineRun(r) => ("routine_run", r.id),
    }
}

fn filed_under(item: &ContextItem, m: &memory::topics::exclusion::ExcludedMaterial) -> bool {
    let (kind, id) = item_ref(item);
    excluded_ref(m, kind, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_cloud_models_are_hosted() {
        for m in ["gpt-oss:120b-cloud", "qwen3-coder:480b-cloud", "glm-4.6:cloud", "deepseek-v3.1:671b-CLOUD"] {
            assert!(is_ollama_cloud_model(m), "{m}");
            assert!(is_hosted(Provider::Ollama, m), "{m}");
        }
        for m in ["gemma4:12b", "llama3.2", "cloudy-7b", ""] {
            assert!(!is_hosted(Provider::Ollama, m), "{m}");
        }
        assert!(!is_hosted(Provider::Local, "anything-cloud"));
        assert!(is_hosted(Provider::Anthropic, "claude"));
    }

    #[test]
    fn closed_filter_leaves_everything_out_and_none_keeps_everything() {
        let item = ContextItem::Capture(memory::CaptureItem {
            id: 1,
            device_id: "d".into(),
            ts_unix_ms: 1,
            app_bundle_id: "a".into(),
            window_title: "w".into(),
            url: None,
            text: "hello".into(),
            hash: 1,
            source: Some("ax".into()),
            local_seq: None,
        });
        assert!(ContextFilter::none().keeps(&item));
        let closed = ContextFilter::closed();
        assert!(!closed.keeps(&item));
        assert_eq!(closed.left_out(), 1);
        assert!(closed.names_client("anything"));
    }
}
