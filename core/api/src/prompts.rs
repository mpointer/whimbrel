//! Small pure Ask-surface logic (M3.0 part 2, Task 5), moved from apps/macos
//! `PromptLibrary.swift` (the built-in starter prompts) and
//! `QueryUrgencyClassifier.swift` (the Ollama-vs-fast routing heuristic).

/// One built-in starter question surfaced as a chip in Ask mode. Not user-editable:
/// no storage, no sync, no CRUD.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StarterPrompt {
    pub id: &'static str,
    pub title: &'static str,
    pub query: &'static str,
}

pub const STARTER_PROMPTS: [StarterPrompt; 6] = [
    StarterPrompt { id: "quietly-slipping", title: "What's quietly slipping?", query: "What's quietly slipping? Look across my recent meetings and captures for things that keep getting mentioned but never resolved." },
    StarterPrompt { id: "recurring-meetings", title: "Which recurring meetings can I kill?", query: "Which recurring meetings can I kill? Look for meetings that keep happening but don't seem to produce decisions or action items." },
    StarterPrompt { id: "todays-digest", title: "Summarize today", query: "Summarize what happened today across my meetings and captures." },
    StarterPrompt { id: "open-action-items", title: "What action items are still open?", query: "What action items from my recent meetings are still open?" },
    StarterPrompt { id: "weekly-trend", title: "Any trend this week?", query: "Is there a trend across my meetings this week — recurring topics, blockers, or themes?" },
    StarterPrompt { id: "decisions-made", title: "What decisions did I make recently?", query: "Summarize the decisions I've made in my recent meetings." },
];

/// Keywords marking a prepared-artifact / broad-synthesis question that can tolerate
/// Ollama's latency. Directionally right for the common cases, not exhaustive.
const NON_TIME_CRITICAL_KEYWORDS: [&str; 9] = [
    "summarize", "summarise", "digest", "across", "recurring", "trend", "overall", "quietly", "action item",
];

/// Whether an Ask-mode question needs a fast response (live back-and-forth) rather
/// than tolerating a slower local model. An unmatched query defaults to time-critical:
/// an ambiguous question must never be silently downgraded to a possibly-slower path.
pub fn is_time_critical(query: &str) -> bool {
    let lowered = query.to_lowercase();
    !NON_TIME_CRITICAL_KEYWORDS.iter().any(|k| lowered.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_time_critical (ported from QueryUrgencyClassifierTests.swift) ----

    #[test]
    fn each_synthesis_keyword_is_not_time_critical() {
        for q in [
            "Can you summarize my meetings this week?",
            "prepare a digest of today",
            "what's changed across my meetings",
            "which recurring meetings can I kill?",
            "any trend in my open questions?",
            "give me the overall picture",
            "what's quietly slipping?",
            "what action items are still open?",
        ] {
            assert!(!is_time_critical(q), "{q}");
        }
    }

    #[test]
    fn keyword_match_is_case_insensitive() {
        assert!(!is_time_critical("SUMMARIZE this for me"));
    }

    #[test]
    fn plain_follow_up_and_empty_queries_are_time_critical() {
        assert!(is_time_critical("what time is my next meeting?"));
        assert!(is_time_critical("what about the other one?"));
        assert!(is_time_critical(""));
    }

    #[test]
    fn starter_prompts_serialize_with_swift_field_names_and_stable_ids() {
        let json = serde_json::to_value(STARTER_PROMPTS).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 6);
        assert_eq!(json[0]["id"], "quietly-slipping");
        assert_eq!(json[0]["title"], "What's quietly slipping?");
        assert!(json[0]["query"].as_str().unwrap().starts_with("What's quietly slipping? Look across"));
        let ids: Vec<&str> = STARTER_PROMPTS.iter().map(|p| p.id).collect();
        assert_eq!(ids, ["quietly-slipping", "recurring-meetings", "todays-digest", "open-action-items", "weekly-trend", "decisions-made"]);
    }
}
