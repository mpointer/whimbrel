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

pub const STARTER_PROMPTS: [StarterPrompt; 6] = {
    use crate::l10n::keys::*;
    [
        StarterPrompt { id: "quietly-slipping", title: STARTER_SLIPPING_TITLE, query: STARTER_SLIPPING_QUERY },
        StarterPrompt { id: "recurring-meetings", title: STARTER_RECURRING_TITLE, query: STARTER_RECURRING_QUERY },
        StarterPrompt { id: "todays-digest", title: STARTER_TODAY_TITLE, query: STARTER_TODAY_QUERY },
        StarterPrompt { id: "open-action-items", title: STARTER_ACTIONS_TITLE, query: STARTER_ACTIONS_QUERY },
        StarterPrompt { id: "weekly-trend", title: STARTER_TREND_TITLE, query: STARTER_TREND_QUERY },
        StarterPrompt { id: "decisions-made", title: STARTER_DECISIONS_TITLE, query: STARTER_DECISIONS_QUERY },
    ]
};

/// `STARTER_PROMPTS` with `title` and `query` in `lang`; ids never change.
pub fn starter_prompts(lang: &str) -> Vec<StarterPrompt> {
    use crate::l10n::text_static;
    STARTER_PROMPTS
        .iter()
        .map(|p| StarterPrompt { id: p.id, title: text_static(lang, p.title), query: text_static(lang, p.query) })
        .collect()
}

/// Keywords marking a prepared-artifact / broad-synthesis question that can tolerate
/// Ollama's latency. Directionally right for the common cases, not exhaustive.
/// Union of all rows is matched (the question's language decides).
/// Left out on purpose: "resume"/"résumé"/"global"/"general" alone, which are English words.
const KEYWORDS_NON_TIME_CRITICAL: &[(&str, &[&str])] = &[
    ("en", &["summarize", "summarise", "digest", "across", "recurring", "trend", "overall", "quietly", "action item"]),
    (
        "es",
        &[
            "resumen", "resumir", "resúmeme", "compendio", "a través de", "recurrente", "recurrentes", "tendencia", "tendencias", "en general",
            "en silencio", "tarea pendiente", "tareas pendientes", "punto de acción", "puntos de acción",
            "elemento de acción", "elementos de acción",
        ],
    ),
    (
        "fr",
        &[
            "résume", "résumer", "résumez", "synthèse", "à travers", "récurrent", "récurrente", "récurrents", "récurrentes", "tendance", "tendances", "dans l'ensemble",
            "globalement", "en général", "discrètement", "en silence", "point d'action", "points d'action",
            "élément d'action", "éléments d'action",
        ],
    ),
    (
        "de",
        &[
            "zusammenfassung", "zusammenfassen", "zusammen", "fasse", "überblick", "übergreifend", "wiederkehrend", "wiederkehrende", "wiederkehrenden",
            "trend", "trends", "insgesamt", "im allgemeinen", "aktionspunkt", "aktionspunkte", "offene aufgaben",
        ],
    ),
    (
        "pt",
        &[
            "resumo", "resumir", "resuma", "síntese", "em todas", "recorrente", "recorrentes", "tendência", "tendências", "no geral", "em geral",
            "discretamente", "em silêncio", "item de ação", "itens de ação",
        ],
    ),
];

/// Whether an Ask-mode question needs a fast response (live back-and-forth) rather
/// than tolerating a slower local model. An unmatched query defaults to time-critical:
/// an ambiguous question must never be silently downgraded to a possibly-slower path.
pub fn is_time_critical(query: &str) -> bool {
    !memory::keywords::any_keyword(query, KEYWORDS_NON_TIME_CRITICAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_time_critical (ported from QueryUrgencyClassifierTests.swift) ----

    #[test]
    fn is_time_critical_spanish() {
        for q in [
            "resúmeme mis reuniones",
            "prepara un resumen de hoy",
            "qué cambió a través de mis reuniones",
            "¿qué reuniones recurrentes puedo cancelar?",
            "¿alguna tendencia en mis preguntas?",
            "dame el panorama en general",
            "¿qué se está retrasando en silencio?",
            "¿qué tareas pendientes siguen abiertas?",
            "¿qué puntos de acción siguen abiertos?",
        ] {
            assert!(!is_time_critical(q), "{q}");
        }
        assert!(is_time_critical("¿a qué hora es mi próxima reunión?"));
    }

    #[test]
    fn is_time_critical_french() {
        for q in [
            "résume mes réunions",
            "une synthèse de la semaine",
            "quelles réunions récurrentes annuler ?",
            "une tendance ?",
            "quels points d\u{2019}action restent ouverts ?",
            "dans l'ensemble ?",
        ] {
            assert!(!is_time_critical(q), "{q}");
        }
        assert!(is_time_critical("à quelle heure est ma prochaine réunion ?"));
    }

    #[test]
    fn is_time_critical_foreign_keywords_do_not_over_match_english() {
        for q in ["resume the meeting", "update my résumé", "global warming", "general update", "what did Ana tell me"] {
            assert!(is_time_critical(q), "{q}");
        }
    }

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
    fn german_and_portuguese_synthesis_questions_are_not_time_critical() {
        for q in [
            "Fasse meine Woche zusammen: eine Zusammenfassung bitte",
            "Gibt es einen Trend in meinen Terminen?",
            "welche wiederkehrenden Besprechungen kann ich streichen?",
            "faça um resumo da minha semana",
            "quais reuniões recorrentes posso cancelar?",
            "há alguma tendência nas minhas perguntas?",
        ] {
            assert!(!is_time_critical(q), "{q}");
        }
        assert!(is_time_critical("Wann ist mein nächster Termin?"));
        assert!(is_time_critical("quando é minha próxima reunião?"));
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
    fn starter_prompts_ids_stable() {
        let ids: Vec<&str> = starter_prompts("es").iter().map(|p| p.id).collect();
        assert_eq!(ids, ["quietly-slipping", "recurring-meetings", "todays-digest", "open-action-items", "weekly-trend", "decisions-made"]);
        assert_eq!(starter_prompts(""), STARTER_PROMPTS.to_vec(), "English renders exactly the source");
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
