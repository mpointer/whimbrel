use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use llm::skin_maker::{
    build_skin_prompt, parse_skin_reply, run_skin_turn, ChatTurn, SkinIntent, SkinParseError,
    SkinTurnInput,
};
use llm::{Completion, LlmClient, LlmError, TokenUsage};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/skin_replies/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Returns the scripted replies in order and counts calls.
struct ScriptedClient {
    replies: RefCell<VecDeque<String>>,
    calls: Cell<u32>,
}

impl ScriptedClient {
    fn new(replies: &[String]) -> Self {
        Self {
            replies: RefCell::new(replies.iter().cloned().collect()),
            calls: Cell::new(0),
        }
    }
}

impl LlmClient for ScriptedClient {
    fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
        self.calls.set(self.calls.get() + 1);
        let text = self.replies.borrow_mut().pop_front().unwrap_or_default();
        Ok(Completion {
            text,
            token_usage: TokenUsage {
                input_tokens: Some(10),
                output_tokens: Some(5),
            },
        })
    }
}

fn input(history_len: usize) -> SkinTurnInput {
    SkinTurnInput {
        request: "make it feel like a 1970s stadium".into(),
        history: (0..history_len)
            .map(|i| ChatTurn {
                role: if i % 2 == 0 { "user" } else { "whimbrel" }.into(),
                text: format!("history-turn-{i:02}"),
            })
            .collect(),
        current_manifest: Some(serde_json::json!({"id": "user-stadium", "name": "Stadium"})),
        catalogue: serde_json::json!({
            "fonts": ["Avenir Next"],
            "voice": {"greetingMorning": {"limit": 40}}
        }),
        output_language: String::new(),
        can_search_images: None,
        can_make_images: None,
    }
}

#[test]
fn image_limits_appear_only_when_the_device_lacks_them() {
    let unsaid = build_skin_prompt(&input(0));
    assert!(!unsaid.contains("can't search the web"));
    assert!(!unsaid.contains("can't make images"));

    let able = SkinTurnInput { can_search_images: Some(true), can_make_images: Some(true), ..input(0) };
    assert_eq!(build_skin_prompt(&able), unsaid);

    let unable = SkinTurnInput { can_search_images: Some(false), can_make_images: Some(false), ..input(0) };
    let prompt = build_skin_prompt(&unable);
    assert!(prompt.contains("can't search the web for images: leave imageQueries empty"));
    assert!(prompt.contains("can't make images: leave imagePrompts empty"));
    // The rules sit with the others, before the catalogue.
    assert!(prompt.find("can't search").unwrap() < prompt.find("Catalogue of allowed values").unwrap());
}

#[test]
fn image_limits_read_from_json_and_default_to_unsaid() {
    let old: SkinTurnInput = serde_json::from_value(serde_json::json!({"request": "x", "catalogue": {}})).unwrap();
    assert_eq!((old.can_search_images, old.can_make_images), (None, None));
    let new: SkinTurnInput =
        serde_json::from_value(serde_json::json!({"request": "x", "catalogue": {}, "canSearchImages": false})).unwrap();
    assert_eq!(new.can_search_images, Some(false));
}

#[test]
fn parses_fenced_and_prose_wrapped_json() {
    let fenced = parse_skin_reply(&fixture("fenced.txt")).unwrap();
    assert_eq!(fenced.intent, SkinIntent::New);
    assert_eq!(fenced.reply, "Here is a stadium skin.");
    assert_eq!(fenced.notes[0].step, "palette");
    assert_eq!(fenced.image_queries, vec!["stadium at night"]);
    assert!(fenced.skin.is_some());

    let prose = parse_skin_reply(&fixture("prose_then_json.txt")).unwrap();
    assert_eq!(prose.intent, SkinIntent::Clarify);
    assert_eq!(prose.reply, "Which era did you have in mind?");
    assert!(prose.skin.is_none());
    assert!(prose.notes.is_empty());

    let two = parse_skin_reply(&fixture("two_objects.txt")).unwrap();
    assert_eq!(two.intent, SkinIntent::Refine);
    assert_eq!(two.reply, "I darkened the gold a little.");
}

#[test]
fn rejects_non_json() {
    assert!(matches!(
        parse_skin_reply(&fixture("no_json.txt")),
        Err(SkinParseError::NoJson)
    ));
}

#[test]
fn caps_reply_notes_and_lists() {
    let notes: Vec<_> = (0..12)
        .map(|i| serde_json::json!({"step": "palette", "text": format!("{i}{}", "n".repeat(200))}))
        .collect();
    let text = serde_json::json!({
        "intent": "new",
        "reply": "é".repeat(5000),
        "notes": notes,
        "imageQueries": ["a", "b", "c", "d", "e"],
        "imagePrompts": ["p1", "p2", "p3"],
    })
    .to_string();
    let reply = parse_skin_reply(&text).unwrap();
    assert_eq!(reply.reply.chars().count(), 2000);
    assert_eq!(reply.notes.len(), 8);
    assert!(reply.notes.iter().all(|n| n.text.chars().count() <= 80));
    assert_eq!(reply.image_queries, vec!["a", "b", "c"]);
    assert_eq!(reply.image_prompts.len(), 2);
}

#[test]
fn caps_palettes_at_four() {
    let palettes: Vec<_> = (0..9)
        .map(|i| serde_json::json!({"id": format!("p{i}"), "name": "x", "light": {}}))
        .collect();
    let text = serde_json::json!({"intent": "new", "palettes": palettes}).to_string();
    let reply = parse_skin_reply(&text).unwrap();
    assert_eq!(reply.palettes.len(), 4);
    assert_eq!(reply.palettes[0]["id"], "p0");
}

#[test]
fn retries_once_then_fails() {
    let failing = ScriptedClient::new(&[fixture("no_json.txt"), fixture("no_json.txt")]);
    assert!(run_skin_turn(&failing, &input(0)).is_err());
    assert_eq!(failing.calls.get(), 2);

    let recovering = ScriptedClient::new(&[fixture("no_json.txt"), fixture("fenced.txt")]);
    let (reply, usage) = run_skin_turn(&recovering, &input(0)).unwrap();
    assert_eq!(recovering.calls.get(), 2);
    assert_eq!(reply.intent, SkinIntent::New);
    // Both calls cost tokens; neither is dropped.
    assert_eq!(usage.input_tokens, Some(20));
    assert_eq!(usage.output_tokens, Some(10));
}

#[test]
fn prompt_contains_catalogue_history_and_no_name_slot_fill() {
    let input = input(25);
    let prompt = build_skin_prompt(&input);
    assert!(prompt.contains(&serde_json::to_string(&input.catalogue).unwrap()));
    assert!(prompt.contains(&serde_json::to_string(input.current_manifest.as_ref().unwrap()).unwrap()));
    assert!(prompt.contains("history-turn-24"));
    assert!(prompt.contains("history-turn-05"));
    assert!(!prompt.contains("history-turn-04"));
    assert!(prompt.contains("{name}"));
    assert!(prompt.contains(&input.request));
}

#[test]
fn skin_maker_es_us_adds_language_line_first() {
    let mut i = input(0);
    i.output_language = "es-US".into();
    let p = build_skin_prompt(&i);
    let at = p.find(&llm::profile::language_line("es-US")).expect("language line present");
    assert!(at < p.find("You are Whimbrel's skin maker").unwrap());
    assert!(p.contains("Keep JSON field names"));
}

#[test]
fn skin_maker_empty_lang_has_no_language_line() {
    let p = build_skin_prompt(&input(0));
    assert!(!p.contains("Language:"));
    assert!(p.starts_with("You are Whimbrel's skin maker"));
}

#[test]
fn skin_maker_input_without_output_language_still_parses() {
    let i: SkinTurnInput = serde_json::from_str(r#"{"request":"r","catalogue":{}}"#).unwrap();
    assert_eq!(i.output_language, "");
    let i: SkinTurnInput = serde_json::from_str(r#"{"request":"r","catalogue":{},"outputLanguage":"fr-CA"}"#).unwrap();
    assert_eq!(i.output_language, "fr-CA");
}
