//! Skins Release 2: one conversational skin turn. Builds the prompt from the user's
//! request, the skin's chat, the current manifest and the Swift-built catalogue of
//! allowed values; reads the model's JSON reply tolerantly (fences, prose, extra
//! objects) and caps its sizes. Validation of the manifest and palettes is Swift's job.

use serde::{Deserialize, Serialize};

use crate::{LlmClient, LlmError, TokenUsage};

const HISTORY_TURNS: usize = 20;
const REPLY_MAX_CHARS: usize = 2000;
const NOTES_MAX: usize = 8;
const NOTE_MAX_CHARS: usize = 80;
const IMAGE_QUERIES_MAX: usize = 3;
const IMAGE_PROMPTS_MAX: usize = 2;
const PALETTES_MAX: usize = 4;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinTurnInput {
    pub request: String,
    #[serde(default)]
    pub history: Vec<ChatTurn>,
    #[serde(default)]
    pub current_manifest: Option<serde_json::Value>,
    pub catalogue: serde_json::Value,
}

/// One earlier message in this skin's chat. `role` is `"user"` or `"whimbrel"`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatTurn {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinIntent {
    New,
    Refine,
    Clarify,
    Decline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinNote {
    pub step: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinTurnReply {
    pub intent: SkinIntent,
    #[serde(default)]
    pub reply: String,
    #[serde(default)]
    pub notes: Vec<SkinNote>,
    #[serde(default)]
    pub skin: Option<serde_json::Value>,
    #[serde(default)]
    pub palettes: Vec<serde_json::Value>,
    #[serde(default)]
    pub image_queries: Vec<String>,
    #[serde(default)]
    pub image_prompts: Vec<String>,
}

#[derive(Debug)]
pub enum SkinParseError {
    /// No complete JSON object anywhere in the text.
    NoJson,
    /// The first JSON object is not a skin reply (e.g. no valid `intent`).
    BadShape(String),
}

impl std::fmt::Display for SkinParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkinParseError::NoJson => write!(f, "no JSON object in the skin reply"),
            SkinParseError::BadShape(msg) => write!(f, "skin reply has the wrong shape: {msg}"),
        }
    }
}

const INSTRUCTIONS: &str = r#"You are Whimbrel's skin maker. A skin changes how the Whimbrel app looks: palettes, typography, shape, icons, decoration, voice (the app's own short wording), the Today layout archetype and motion. You talk with the user in a chat and design or change one skin per turn.

Rules:
- Use only values listed in the catalogue below: fonts, enum values, icon roles and names, shape ranges, archetypes. Never invent a key or value. No code, no HTML, no markup anywhere.
- Skin wording only from voice keys, within their limits: every piece of text in the skin goes in a voice key the catalogue lists, no longer than that key's limit.
- Never ask for, guess or use a personal name. Greetings address the user only through the literal {name} slot, for example "Game day, {name}."
- intent "new" when the user wants a different skin; "refine" when they want to change the current skin (return the whole updated manifest, keeping everything they did not ask to change); "clarify" when you need one short question answered first; "decline" when the request is not about skins.
- For "clarify" and "decline", leave skin null and every list empty.
- reply is plain conversational text for the chat (what you did or what you need), at most a few sentences.
- notes are short status lines (each under 80 characters) about what you did, one per part of the skin, for example {"step": "palette", "text": "Navy page, gold accents"}. At most 8.
- palettes: the full palette objects this skin uses (id, name, and "dark" and/or "light" maps of colour tokens to #RRGGBB). Keep text readable on its background. The skin's "palettes" list names their ids.
- imageQueries: for a new skin, up to 3 short web image search terms that fit it. For a refine, only when the user's latest message mentions images. Otherwise [].
- imagePrompts: emit image prompts only when the user's latest message asks for an image to be made (up to 2). Otherwise [].

Reply with only one JSON object, no prose and no fences, in this shape:
{"intent": "new" | "refine" | "clarify" | "decline", "reply": "...", "notes": [{"step": "...", "text": "..."}], "skin": {...manifest...} | null, "palettes": [{...}], "imageQueries": ["..."], "imagePrompts": ["..."]}"#;

const RETRY_NUDGE: &str =
    "\n\nYour previous reply could not be read. Reply with only the JSON object, nothing else.";

pub fn build_skin_prompt(input: &SkinTurnInput) -> String {
    let catalogue = serde_json::to_string(&input.catalogue).unwrap_or_default();
    let manifest = match &input.current_manifest {
        Some(m) => serde_json::to_string(m).unwrap_or_default(),
        None => "none (no skin yet)".to_string(),
    };
    let skip = input.history.len().saturating_sub(HISTORY_TURNS);
    let history: String = input.history[skip..]
        .iter()
        .map(|t| format!("{}: {}\n", t.role, t.text))
        .collect();
    let history = if history.is_empty() { "(none)\n".to_string() } else { history };
    format!(
        "{INSTRUCTIONS}\n\nCatalogue of allowed values:\n{catalogue}\n\nCurrent skin manifest:\n{manifest}\n\nEarlier chat for this skin:\n{history}\nUser's latest message:\n{}\n",
        input.request
    )
}

/// The first complete top-level JSON object anywhere in `text`, read as a skin reply,
/// with its sizes capped.
pub fn parse_skin_reply(text: &str) -> Result<SkinTurnReply, SkinParseError> {
    let value = text
        .match_indices('{')
        .find_map(|(i, _)| {
            let mut de = serde_json::Deserializer::from_str(&text[i..]);
            serde_json::Value::deserialize(&mut de).ok()
        })
        .ok_or(SkinParseError::NoJson)?;
    let mut reply: SkinTurnReply =
        serde_json::from_value(value).map_err(|e| SkinParseError::BadShape(e.to_string()))?;
    reply.reply = truncate(&reply.reply, REPLY_MAX_CHARS);
    reply.notes.truncate(NOTES_MAX);
    for note in &mut reply.notes {
        note.text = truncate(&note.text, NOTE_MAX_CHARS);
    }
    reply.image_queries.truncate(IMAGE_QUERIES_MAX);
    reply.image_prompts.truncate(IMAGE_PROMPTS_MAX);
    reply.palettes.truncate(PALETTES_MAX);
    Ok(reply)
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// One model call, plus one retry with a "JSON only" nudge if the reply can't be read.
/// The returned usage covers every call made.
pub fn run_skin_turn(
    client: &dyn LlmClient,
    input: &SkinTurnInput,
) -> Result<(SkinTurnReply, TokenUsage), LlmError> {
    let prompt = build_skin_prompt(input);
    let first = client.complete(&prompt)?;
    if let Ok(reply) = parse_skin_reply(&first.text) {
        return Ok((reply, first.token_usage));
    }
    let second = client.complete(&format!("{prompt}{RETRY_NUDGE}"))?;
    let usage = add_usage(first.token_usage, second.token_usage);
    parse_skin_reply(&second.text)
        .map(|reply| (reply, usage))
        .map_err(|e| LlmError::Provider(e.to_string()))
}

fn add_usage(a: TokenUsage, b: TokenUsage) -> TokenUsage {
    let add = |x: Option<u32>, y: Option<u32>| match (x, y) {
        (Some(x), Some(y)) => Some(x + y),
        (x, None) | (None, x) => x,
    };
    TokenUsage {
        input_tokens: add(a.input_tokens, b.input_tokens),
        output_tokens: add(a.output_tokens, b.output_tokens),
    }
}
