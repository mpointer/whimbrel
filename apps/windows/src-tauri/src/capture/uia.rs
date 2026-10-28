//! UI Automation text extraction — docs/12 §4, §4.1.
//!
//! Three strategies, deliberately kept side by side for the M3.1 measurement: the
//! go/no-go on UIA-from-Rust depends on whether the *right* strategy is fast, and a
//! naive walk (one cross-process COM round trip per property per node) would make any
//! language look unworkable on a large Chromium tree.
//!
//! - [`Strategy::TextPattern`]: find the element(s) exposing `TextPattern` and read
//!   `DocumentRange.GetText(-1)` — one call per document. Visible yield comes from
//!   `GetVisibleRanges`.
//! - [`Strategy::CachedWalk`]: one `FindAllBuildCache` over the control view that
//!   prefetches Name / Value / IsOffscreen / ControlType for every node in a single
//!   cross-process call.
//! - [`Strategy::NaiveWalk`]: `ControlViewWalker` + `Current*` per node. The baseline
//!   the other two are measured against; not for production.
//!
//! All of it must run on a COM-initialized thread; [`Uia::new`] initializes MTA.

use std::time::{Duration, Instant};

use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    TextPattern,
    CachedWalk,
    NaiveWalk,
}

impl Strategy {
    pub const ALL: [Strategy; 3] = [Strategy::TextPattern, Strategy::CachedWalk, Strategy::NaiveWalk];
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Extraction {
    pub strategy: Strategy,
    /// All text the strategy extracted, offscreen included.
    pub chars_total: usize,
    /// Text UIA reports as on-screen (visible ranges / non-offscreen nodes).
    pub chars_visible: usize,
    pub latency_ms: f64,
    /// TextPattern: documents read. Walks: nodes visited.
    pub units: usize,
    /// True if a node/document cap stopped the strategy early.
    pub truncated: bool,
    /// Whether the window's tree contains a Document element. Browsers expose each page
    /// as a Document; an Edge instance withholding web content (M3.1 U6) has none.
    pub document_found: bool,
    #[serde(skip)]
    pub text: String,
    #[serde(skip)]
    pub visible_text: String,
}

pub struct Uia {
    automation: IUIAutomation,
}

/// Cap on nodes for the walking strategies; a Gmail inbox is tens of thousands of nodes.
pub const MAX_NODES: usize = 20_000;
/// Cap on TextPattern documents read per window.
const MAX_DOCUMENTS: usize = 16;

impl Uia {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            // S_FALSE (already initialized on this thread) is fine; RPC_E_CHANGED_MODE
            // means someone initialized STA first, which UIA also tolerates.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
            Ok(Self { automation })
        }
    }

    pub fn automation(&self) -> &IUIAutomation {
        &self.automation
    }

    pub fn extract(&self, hwnd: HWND, strategy: Strategy) -> windows::core::Result<Extraction> {
        match strategy {
            Strategy::TextPattern => self.text_pattern(hwnd),
            Strategy::CachedWalk => self.cached_walk(hwnd),
            Strategy::NaiveWalk => self.naive_walk(hwnd),
        }
    }

    fn text_pattern(&self, hwnd: HWND) -> windows::core::Result<Extraction> {
        let start = Instant::now();
        unsafe {
            let root = self.automation.ElementFromHandle(hwnd)?;
            // Document elements only. Chromium also exposes TextPattern on every nested
            // editable and region (139 on one public Jira page); reading those is slow and
            // redundant, since the page Document's range already contains their text.
            // Fall back to any TextPattern element for apps whose text surface is not a
            // Document.
            let has_tp = self
                .automation
                .CreatePropertyCondition(UIA_IsTextPatternAvailablePropertyId, &VARIANT::from(true))?;
            let is_doc = self
                .automation
                .CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_DocumentControlTypeId.0))?;
            let docs_only = self.automation.CreateAndCondition(&has_tp, &is_doc)?;
            let mut found = root.FindAll(TreeScope_Subtree, &docs_only)?;
            let document_found = found.Length()? > 0;
            if !document_found {
                found = root.FindAll(TreeScope_Subtree, &has_tp)?;
            }
            let n = found.Length()? as usize;

            let mut docs: Vec<(String, String)> = Vec::new();
            for i in 0..n.min(MAX_DOCUMENTS) {
                let el = found.GetElement(i as i32)?;
                let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) else {
                    continue;
                };
                let whole = tp.DocumentRange().and_then(|r| r.GetText(-1)).map(|b| b.to_string()).unwrap_or_default();
                let mut visible = String::new();
                if let Ok(ranges) = tp.GetVisibleRanges() {
                    for j in 0..ranges.Length().unwrap_or(0) {
                        if let Ok(r) = ranges.GetElement(j) {
                            if let Ok(t) = r.GetText(-1) {
                                visible.push_str(&t.to_string());
                                visible.push('\n');
                            }
                        }
                    }
                }
                docs.push((whole, visible));
            }
            let (text, visible_text) = dedupe_nested_documents(docs);
            Ok(Extraction {
                strategy: Strategy::TextPattern,
                chars_total: count_chars(&text),
                chars_visible: count_chars(&visible_text),
                latency_ms: ms(start.elapsed()),
                units: n,
                truncated: n > MAX_DOCUMENTS,
                document_found,
                text,
                visible_text,
            })
        }
    }

    fn cached_walk(&self, hwnd: HWND) -> windows::core::Result<Extraction> {
        let start = Instant::now();
        unsafe {
            let root = self.automation.ElementFromHandle(hwnd)?;
            let cr = self.automation.CreateCacheRequest()?;
            for prop in [
                UIA_NamePropertyId,
                UIA_ValueValuePropertyId,
                UIA_IsOffscreenPropertyId,
                UIA_ControlTypePropertyId,
            ] {
                cr.AddProperty(prop)?;
            }
            cr.SetTreeFilter(&self.automation.ControlViewCondition()?)?;
            // Cached values only — no live element references to marshal back.
            cr.SetAutomationElementMode(AutomationElementMode_None)?;
            let found = root.FindAllBuildCache(TreeScope_Subtree, &self.automation.CreateTrueCondition()?, &cr)?;
            let n = found.Length()? as usize;

            let mut nodes = Vec::with_capacity(n.min(MAX_NODES));
            for i in 0..n.min(MAX_NODES) {
                let el = found.GetElement(i as i32)?;
                let name = el.CachedName().map(|b| b.to_string()).unwrap_or_default();
                let control_type = el.CachedControlType().map(|c| c.0).unwrap_or(0);
                // A hyperlink's Value is its URL — navigation data, not visible text.
                let value = if control_type == UIA_HyperlinkControlTypeId.0 {
                    String::new()
                } else {
                    el.GetCachedPropertyValue(UIA_ValueValuePropertyId)
                        .ok()
                        .and_then(|v| BSTR::try_from(&v).ok())
                        .map(|b| b.to_string())
                        .unwrap_or_default()
                };
                let offscreen = el.CachedIsOffscreen().map(|b| b.as_bool()).unwrap_or(false);
                nodes.push(NodeText { name, value, offscreen, control_type });
            }
            let (text, visible_text) = join_node_text(&nodes);
            Ok(Extraction {
                strategy: Strategy::CachedWalk,
                chars_total: count_chars(&text),
                chars_visible: count_chars(&visible_text),
                latency_ms: ms(start.elapsed()),
                units: nodes.len(),
                truncated: n > MAX_NODES,
                document_found: nodes.iter().any(|x| x.control_type == UIA_DocumentControlTypeId.0),
                text,
                visible_text,
            })
        }
    }

    fn naive_walk(&self, hwnd: HWND) -> windows::core::Result<Extraction> {
        let start = Instant::now();
        unsafe {
            let root = self.automation.ElementFromHandle(hwnd)?;
            let walker = self.automation.ControlViewWalker()?;
            let mut nodes = Vec::new();
            let mut stack = vec![root];
            let mut truncated = false;
            while let Some(el) = stack.pop() {
                if nodes.len() >= MAX_NODES {
                    truncated = true;
                    break;
                }
                let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                let control_type = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
                let value = if control_type == UIA_HyperlinkControlTypeId.0 {
                    String::new()
                } else {
                    el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                        .and_then(|vp| vp.CurrentValue())
                        .map(|b| b.to_string())
                        .unwrap_or_default()
                };
                let offscreen = el.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(false);
                nodes.push(NodeText { name, value, offscreen, control_type });

                // Children pushed in reverse so the stack pops them in document order.
                let mut kids = Vec::new();
                let mut child = walker.GetFirstChildElement(&el).ok();
                while let Some(c) = child {
                    child = walker.GetNextSiblingElement(&c).ok();
                    kids.push(c);
                }
                stack.extend(kids.into_iter().rev());
            }
            let (text, visible_text) = join_node_text(&nodes);
            Ok(Extraction {
                strategy: Strategy::NaiveWalk,
                chars_total: count_chars(&text),
                chars_visible: count_chars(&visible_text),
                latency_ms: ms(start.elapsed()),
                units: nodes.len(),
                truncated,
                document_found: nodes.iter().any(|x| x.control_type == UIA_DocumentControlTypeId.0),
                text,
                visible_text,
            })
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Non-whitespace characters, so layout whitespace (UIA emits more newlines than OCR
/// does) doesn't skew size comparisons.
pub fn count_chars(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 3)
        .map(|w| w.to_lowercase())
}

/// §4.1 "text yield" as coverage of what is on screen: the share of words OCR reads in
/// the window (3+ letters, case-folded) that also occur anywhere in `uia_text`. A
/// character ratio can't tell "UIA returned more than is visible" (offscreen document
/// text, VS Code's whole-file textarea, browser chrome) from "UIA covered what is
/// visible"; recall can. OCR misreads cap it a little below 1.0 even for perfect
/// extraction. `None` when OCR read no words.
pub fn ocr_word_recall(ocr_text: &str, uia_text: &str) -> Option<f64> {
    let on_screen: Vec<String> = words(ocr_text).collect();
    if on_screen.is_empty() {
        return None;
    }
    let extracted: std::collections::HashSet<String> = words(uia_text).collect();
    let hits = on_screen.iter().filter(|w| extracted.contains(*w)).count();
    Some(hits as f64 / on_screen.len() as f64)
}

#[derive(Debug, Clone)]
struct NodeText {
    name: String,
    value: String,
    offscreen: bool,
    control_type: i32,
}

/// Joins per-node Name/Value into text. A node's Value is kept only when it differs
/// from its Name (edits often report the same string in both), and a string identical
/// to the previous one is dropped (Chromium repeats a link's text on the link and on
/// its static-text child).
fn join_node_text(nodes: &[NodeText]) -> (String, String) {
    let mut all = String::new();
    let mut vis = String::new();
    let mut last = String::new();
    for n in nodes {
        for s in [&n.name, &n.value] {
            let t = s.trim();
            if t.is_empty() || t == last || (std::ptr::eq(s, &n.value) && t == n.name.trim()) {
                continue;
            }
            all.push_str(t);
            all.push('\n');
            if !n.offscreen {
                vis.push_str(t);
                vis.push('\n');
            }
            last = t.to_string();
        }
    }
    (all, vis)
}

/// Chromium exposes TextPattern on the root document *and* on nested editables inside
/// it, so the same text comes back more than once. Keep the longest documents first and
/// drop any document whose text is already contained in one kept.
fn dedupe_nested_documents(mut docs: Vec<(String, String)>) -> (String, String) {
    docs.sort_by_key(|(whole, _)| std::cmp::Reverse(whole.len()));
    let mut kept: Vec<(String, String)> = Vec::new();
    for (whole, visible) in docs {
        let w = whole.trim();
        if w.is_empty() && visible.trim().is_empty() {
            continue;
        }
        if kept.iter().any(|(k, _)| k.contains(w)) {
            continue;
        }
        kept.push((whole, visible));
    }
    let text = kept.iter().map(|(w, _)| w.as_str()).collect::<Vec<_>>().join("\n");
    let visible = kept.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join("\n");
    (text, visible)
}

#[allow(dead_code)]
fn _assert_interface_bound<T: Interface>() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, value: &str, offscreen: bool) -> NodeText {
        NodeText { name: name.into(), value: value.into(), offscreen, control_type: 0 }
    }

    #[test]
    fn join_drops_repeated_and_value_equal_to_name() {
        let nodes = vec![
            node("Inbox", "", false),
            node("Inbox", "", false), // link text repeated on its child
            node("Search", "Search", false), // edit with Value == Name
            node("Subject", "Quarterly numbers", true),
        ];
        let (all, vis) = join_node_text(&nodes);
        assert_eq!(all, "Inbox\nSearch\nSubject\nQuarterly numbers\n");
        assert_eq!(vis, "Inbox\nSearch\n");
    }

    #[test]
    fn nested_documents_are_counted_once() {
        let docs = vec![
            ("reply box".to_string(), "reply box".to_string()),
            ("thread body\nreply box\nfooter".to_string(), "thread body\nreply box".to_string()),
            ("".to_string(), "".to_string()),
        ];
        let (all, vis) = dedupe_nested_documents(docs);
        assert_eq!(all, "thread body\nreply box\nfooter");
        assert_eq!(vis, "thread body\nreply box");
    }

    #[test]
    fn chars_ignore_whitespace() {
        assert_eq!(count_chars(" a b\n\tc "), 3);
    }

    #[test]
    fn recall_counts_on_screen_words_found_in_uia_text() {
        // "Reduce" and "chatiness" found; "email" missing. "JIRA" found case-folded.
        let ocr = "Reduce JIRA email chatiness";
        let uia = "reduce jira CHATINESS plus lots of offscreen text";
        assert_eq!(ocr_word_recall(ocr, uia), Some(0.75));
    }

    #[test]
    fn recall_is_not_inflated_by_extra_uia_text() {
        let ocr = "alpha beta";
        let uia = "alpha beta gamma delta epsilon zeta eta theta";
        assert_eq!(ocr_word_recall(ocr, uia), Some(1.0));
    }

    #[test]
    fn recall_ignores_short_tokens_and_is_none_without_ocr_words() {
        assert_eq!(ocr_word_recall("a 12 of", "anything"), None);
        assert_eq!(ocr_word_recall("", "anything"), None);
    }
}
