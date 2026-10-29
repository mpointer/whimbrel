//! Making a skin by talking to Whimbrel (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md,
//! Task 10): what one turn does with the model's reply, where approved images go, and each skin's
//! chat. Ports the decisions of the Mac's `SkinChatSession.swift` (fork, intent, the palette gate and
//! the save); `commands::skins` makes the calls, and the page stays thin.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use llm::skin_maker::{SkinIntent, SkinTurnInput, SkinTurnReply};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use skins::images::{downscale, MAX_SIDE};
use skins::palette::Variants;
use skins::repair::{pairs_pass, repair_variant};
use skins::store::is_user_id;
use skins::{
    all_pairs, default_palette, is_valid_id, parse_palette, parse_skin, save_gate_repair, skin_name, whimbrel_default, Palette, Token,
    UserSkinStore, Variant,
};

use crate::error::CommandError;
use crate::settings::{ApiKeys, Provider, RoutingSettings};

pub const SPEND_CAP_LINE: &str = "Today's AI spending limit is reached.";
const CLARIFY_LINE: &str = "Could you say a bit more about the skin you want?";
const UNUSABLE_LINE: &str = "Whimbrel couldn't make a usable skin from that, so nothing changed. Try again or say it another way.";
const DONE_LINE: &str = "Here's your skin.";
const KEPT_COLOURS_LINE: &str = "The new colours didn't pass Whimbrel's checks, so this skin keeps its current colours.";
const NO_PALETTE_LINE: &str = "I couldn't make a usable palette, so this skin uses Whimbrel's colours for now.";
/// The placement an approved image takes: one image per placement, the newest shown.
const HEADER: &str = "header";

/// Held by each save of a turn or an image, which read a skin, change it and write a version. An image
/// may be approved while a turn runs; on the Mac the main actor keeps the two apart.
static SAVES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn saving() -> std::sync::MutexGuard<'static, ()> {
    SAVES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the skin chat may use on this device. Chat follows Ask's rule on the preferred provider
/// (`TodayComposer.askAvailable` on the Mac, so local-only mode blocks a hosted one); search and
/// image generation are hosted, so local-only mode turns them off too. `provider` is the preferred
/// provider, which the chat sends to (the page names it in its status line).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkinSources {
    pub provider: Provider,
    pub chat: bool,
    /// A web image search: needs the image search key.
    pub search: bool,
    /// Making an image: needs an OpenAI or Google key.
    pub generate: bool,
}

impl SkinSources {
    pub fn new(routing: &RoutingSettings, keys: &ApiKeys, has_search_key: bool) -> Self {
        let has_key = match routing.preferred_provider {
            Provider::Ollama => true,
            Provider::Anthropic => keys.anthropic.is_some(),
            Provider::Openai => keys.openai.is_some(),
            Provider::Google => keys.google.is_some(),
            // OpenRouter has no default model; without one there is nothing to call.
            Provider::Openrouter => keys.openrouter.is_some() && !routing.openrouter_model.is_empty(),
        };
        SkinSources {
            provider: routing.preferred_provider,
            chat: routing.preferred_provider == Provider::Ollama || (!routing.local_only && has_key),
            search: has_search_key && !routing.local_only,
            generate: (keys.openai.is_some() || keys.google.is_some()) && !routing.local_only,
        }
    }
}

// --- the chat

/// Seconds from 1970 to 2001-01-01 UTC, the reference date of a Swift `Date`.
const SWIFT_EPOCH_OFFSET: f64 = 978_307_200.0;

/// One message in a skin's chat, stored as `chat.json` in the skin's folder in the Mac's shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChatLine {
    /// `"user"` or `"whimbrel"`.
    pub role: String,
    pub text: String,
    /// Seconds since 2001-01-01 UTC, as Swift's `JSONEncoder` writes a `Date`.
    pub date: f64,
}

impl ChatLine {
    pub fn now(role: &str, text: &str) -> Self {
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        ChatLine { role: role.to_string(), text: text.to_string(), date: unix - SWIFT_EPOCH_OFFSET }
    }
}

/// The skin's chat; empty for a built-in, a skin never saved, or an unreadable file.
pub fn chat(user: &UserSkinStore, skin_id: &str) -> Vec<ChatLine> {
    user.folder(skin_id)
        .ok()
        .and_then(|dir| std::fs::read(dir.join("chat.json")).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Adds `lines` to the skin's chat. Refuses a skin that was never saved, so no folder ever holds only a chat.
pub fn append_chat(user: &UserSkinStore, skin_id: &str, lines: &[ChatLine]) -> Result<(), CommandError> {
    if user.manifest(skin_id).is_none() {
        return Err(CommandError::invalid_input(format!("{skin_id} isn't one of your skins")));
    }
    let dir = user.folder(skin_id).map_err(store_error)?;
    let mut all = chat(user, skin_id);
    all.extend_from_slice(lines);
    let bytes = serde_json::to_vec(&all).map_err(|e| CommandError::store(e.to_string()))?;
    let temp = dir.join("chat.json.tmp");
    std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, dir.join("chat.json"))).map_err(|e| CommandError::store(e.to_string()))
}

fn store_error(e: skins::store::StoreError) -> CommandError {
    CommandError::store(e.to_string())
}

// --- one turn

/// The skin a turn starts from.
#[derive(Debug, Clone)]
pub struct Origin {
    pub id: String,
    pub name: String,
    pub palette_ids: Vec<String>,
    /// What the model is shown: a user skin's `skin.json`, a built-in's bundled file.
    pub manifest: Option<Value>,
    /// One of yours, saved, so a refine changes it in place.
    pub saved: bool,
}

impl Origin {
    /// The images the manifest places, so one approved while the turn runs is kept.
    pub fn assets(&self) -> HashSet<String> {
        self.manifest.as_ref().and_then(Value::as_object).map(imagery_of).unwrap_or_default().iter().filter_map(asset).collect()
    }
}

/// `id` as a turn's origin: one of your saved skins, or a built-in in `bundled_skins`.
pub fn origin(user: &UserSkinStore, bundled_skins: &Path, id: &str) -> Option<Origin> {
    let own = if is_user_id(id) { user.manifest(id) } else { None };
    let bytes = match &own {
        Some(bytes) => bytes.clone(),
        // A valid id is a plain file name.
        None if is_valid_id(id) => std::fs::read(bundled_skins.join(format!("{id}.json"))).ok()?,
        None => return None,
    };
    let (skin, _) = parse_skin(&bytes, &whimbrel_default())?;
    Some(Origin { id: skin.id, name: skin.name, palette_ids: skin.palettes, manifest: serde_json::from_slice(&bytes).ok(), saved: own.is_some() })
}

/// The model's input for a turn on `origin`: its chat, its manifest and the catalogue of allowed values.
pub fn turn_input(user: &UserSkinStore, origin: &Origin, request: &str) -> SkinTurnInput {
    SkinTurnInput {
        request: request.to_string(),
        history: chat(user, &origin.id).into_iter().map(|t| llm::skin_maker::ChatTurn { role: t.role, text: t.text }).collect(),
        current_manifest: origin.manifest.clone(),
        catalogue: skins::catalogue(),
    }
}

/// What a turn did, for the page.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkinTurnResult {
    /// `"new"`, `"refine"`, `"clarify"` or `"decline"`; `None` when the turn was stopped, so nothing changed.
    pub intent: Option<String>,
    /// The model's status lines, one per part of the skin.
    pub notes: Vec<String>,
    /// Whimbrel's chat lines for the turn, in order.
    pub lines: Vec<String>,
    /// Whether the request and `lines` were stored in `chat_skin_id`'s chat.
    pub stored: bool,
    /// The skin whose chat the turn belongs to: the one saved, or the one asked about.
    pub chat_skin_id: String,
    /// The skin saved, which the page switches to; `None` when nothing was saved.
    pub saved_skin_id: Option<String>,
    /// Web searches to run for images, already limited to what this device may use and the request asked for.
    pub image_queries: Vec<String>,
    /// Images to make, likewise.
    pub image_prompts: Vec<String>,
}

impl SkinTurnResult {
    pub fn stopped(origin_id: &str) -> Self {
        SkinTurnResult {
            intent: None,
            notes: vec![],
            lines: vec![],
            stored: false,
            chat_skin_id: origin_id.to_string(),
            saved_skin_id: None,
            image_queries: vec![],
            image_prompts: vec![],
        }
    }
}

fn intent_name(intent: SkinIntent) -> &'static str {
    match intent {
        SkinIntent::New => "new",
        SkinIntent::Refine => "refine",
        SkinIntent::Clarify => "clarify",
        SkinIntent::Decline => "decline",
    }
}

/// One turn's model call: the cap on route `"skin"` first, the call (with its one retry), then the usage
/// row the next cap check sums. The FFI's `skin_turn`, on this app's store.
pub fn call_model(
    store: &memory::Store,
    caps: &llm::SpendCapConfig,
    provider: llm::Provider,
    model: &str,
    client: &dyn llm::LlmClient,
    input: &SkinTurnInput,
) -> Result<SkinTurnReply, CommandError> {
    llm::check_spend_cap(store, caps, None, Some("skin"), now_ms()).map_err(|e| CommandError::from(api::ApiError::SpendCap(e)))?;
    let (reply, usage) = llm::skin_maker::run_skin_turn(client, input).map_err(|e| {
        tracing::warn!(error = %e, "skin turn failed");
        CommandError::ProviderFailure
    })?;
    api::util::log_usage_for(store, provider, model, "skin", usage);
    Ok(reply)
}

/// Charges a flat-priced image call (`"skin-image-search"`, `"skin-image-generate"`) before it is made,
/// failing closed when the charge can't be recorded.
pub fn charge(store: &memory::Store, caps: &llm::SpendCapConfig, route: &str, cents: i64) -> Result<(), CommandError> {
    llm::charge_skin_call(store, caps, route, cents, now_ms()).map_err(|e| match e {
        llm::ChargeError::SpendCap(cap) => api::ApiError::SpendCap(cap).into(),
        llm::ChargeError::Ledger(_) => CommandError::ProviderFailure,
    })
}

/// Saves the reply (a new skin, a fork of a built-in, or a new version of yours) and stores the chat.
/// `pool` holds the palettes every skin may name (the plain themes and the built-ins' own); `sent_assets`
/// the images in the manifest the model was given.
pub fn apply_reply(
    user: &UserSkinStore,
    pool: &[Palette],
    origin: &Origin,
    reply: &SkinTurnReply,
    request: &str,
    sent_assets: &HashSet<String>,
    sources: SkinSources,
) -> SkinTurnResult {
    let _saving = saving();
    let mut result = SkinTurnResult::stopped(&origin.id);
    result.intent = Some(intent_name(reply.intent).to_string());
    result.notes = reply.notes.iter().map(|n| n.text.clone()).collect();
    let asked = ChatLine::now("user", request);
    if !matches!(reply.intent, SkinIntent::New | SkinIntent::Refine) {
        // A question back or a decline: chat only.
        let line = if reply.reply.is_empty() { CLARIFY_LINE.to_string() } else { reply.reply.clone() };
        result.stored = origin.saved && append_chat(user, &origin.id, &[asked, ChatLine::now("whimbrel", &line)]).is_ok();
        result.lines = vec![line];
        return result;
    }
    let Some((target, extra)) = save_reply(user, pool, origin, reply, sent_assets) else {
        result.lines = vec![UNUSABLE_LINE.to_string()];
        return result;
    };
    let mut lines = vec![if reply.reply.is_empty() { DONE_LINE.to_string() } else { reply.reply.clone() }];
    lines.extend(extra);
    let turns: Vec<ChatLine> = std::iter::once(asked).chain(lines.iter().map(|l| ChatLine::now("whimbrel", l))).collect();
    result.stored = append_chat(user, &target, &turns).is_ok();
    result.lines = lines;
    // Search for a new skin (or when the request mentions images); make one only when it asks (R4).
    let mentions = mentions_images(request);
    if sources.search && (reply.intent == SkinIntent::New || mentions) {
        result.image_queries = reply.image_queries.clone();
    }
    if sources.generate && mentions {
        result.image_prompts = reply.image_prompts.clone();
    }
    result.chat_skin_id = target.clone();
    result.saved_skin_id = Some(target);
    result
}

/// The image words of R4, at a word start ("photos" and "backgrounds" count), as the Mac's
/// `\b(image|picture|photo|banner|logo|art|background)`.
pub fn mentions_images(text: &str) -> bool {
    const WORDS: [&str; 7] = ["image", "picture", "photo", "banner", "logo", "art", "background"];
    let lower = text.to_lowercase();
    lower.char_indices().any(|(i, _)| {
        let at_word_start = lower[..i].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        at_word_start && WORDS.iter().any(|w| lower[i..].starts_with(w))
    })
}

/// The reply's manifest saved as a user skin: palettes repaired and given `<id>-p<n>` ids (a refine keeps
/// the ids of its own palettes the reply returns or still lists, so the selected colours stay put),
/// imagery limited to approved files (plus any approved while the turn ran). Returns its id and the
/// chat lines explaining any repair or fallback; `None` when no usable manifest survives.
fn save_reply(user: &UserSkinStore, pool: &[Palette], origin: &Origin, reply: &SkinTurnReply, sent_assets: &HashSet<String>) -> Option<(String, Vec<String>)> {
    let mut manifest = reply.skin.as_ref()?.as_object()?.clone();
    let refine = reply.intent == SkinIntent::Refine;
    // Capped first, so a name of only spaces counts as missing.
    let replied_name = skin_name(manifest.get("name").and_then(Value::as_str).unwrap_or(""), "");
    let (target, name) = if refine && origin.saved {
        (origin.id.clone(), if replied_name.is_empty() { origin.name.clone() } else { replied_name })
    } else if refine {
        (user.new_skin_id(&origin.id), skin_name(&origin.name, " (yours)"))
    } else {
        let name = if replied_name.is_empty() { "My skin".to_string() } else { replied_name };
        (user.new_skin_id(&name), name)
    };
    manifest.insert("id".into(), target.clone().into());
    manifest.insert("name".into(), name.into());
    let refining_own = refine && origin.saved;

    let mut lines = Vec::new();
    let fallback = default_palette();
    let existing: Vec<Palette> =
        if refining_own { user.palette_files(&target).iter().filter_map(|(_, bytes)| parse_palette(bytes, &fallback)).collect() } else { vec![] };
    let mut palettes: Vec<Palette> = Vec::new();
    let mut renamed: HashMap<String, String> = HashMap::new();
    let mut next = 1;
    for raw in &reply.palettes {
        let original = raw.get("id").and_then(Value::as_str).map(str::to_string);
        let id = match &original {
            Some(o) if existing.iter().any(|p| &p.id == o) && !palettes.iter().any(|p| &p.id == o) => o.clone(),
            _ => {
                while taken(&existing, &palettes, &format!("{target}-p{next}")) {
                    next += 1;
                }
                format!("{target}-p{next}")
            }
        };
        let Some((palette, changed)) = repaired_palette(raw, &id) else { continue };
        if !changed.is_empty() {
            lines.push(adjusted(&changed, &palette.name));
        }
        palettes.push(palette);
        if let Some(original) = original {
            renamed.entry(original).or_insert(id);
        }
    }
    let all_rejected = !reply.palettes.is_empty() && palettes.is_empty();
    let mut ids: Vec<String> = Vec::new();
    for reference in manifest.get("palettes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let id = if let Some(new) = renamed.get(reference) {
            Some(new.clone())
        } else if let Some(kept) = existing.iter().find(|p| p.id == reference) {
            if !palettes.iter().any(|p| p.id == kept.id) {
                palettes.push(kept.clone());
            }
            Some(kept.id.clone())
        } else {
            None
        };
        if let Some(id) = id.filter(|id| !ids.contains(id)) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        ids = palettes.iter().map(|p| p.id.clone()).collect();
    }
    if palettes.is_empty() && refine {
        // No usable new colours: keep the skin's own (a fork keeps the built-in's, which ship anyway).
        palettes = existing.clone();
        ids = if origin.saved { existing.iter().map(|p| p.id.clone()).collect() } else { origin.palette_ids.clone() };
        if all_rejected && !ids.is_empty() {
            lines.push(KEPT_COLOURS_LINE.to_string());
        }
    }
    if ids.is_empty() {
        ids = vec![fallback.id.clone()];
        lines.push(NO_PALETTE_LINE.to_string());
    }
    manifest.insert("palettes".into(), ids.into());

    // Only images already approved into this skin's folder may be placed; one approved while this turn
    // ran (so the model never saw it) is carried over from the saved manifest.
    let mut imagery = imagery_of(&manifest);
    if refining_own {
        let replied: HashSet<String> = imagery.iter().filter_map(asset).collect();
        let saved = user.manifest(&target).and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        for entry in saved.as_ref().and_then(Value::as_object).map(imagery_of).unwrap_or_default() {
            if asset(&entry).is_some_and(|a| !sent_assets.contains(&a) && !replied.contains(&a)) {
                imagery = placing(entry, imagery);
            }
        }
    }
    imagery.retain(|entry| asset(entry).is_some_and(|a| user.image_path(&target, &a).is_ok()));
    if manifest.contains_key("decoration") || !imagery.is_empty() {
        let mut decoration = manifest.get("decoration").and_then(Value::as_object).cloned().unwrap_or_default();
        decoration.insert("imagery".into(), Value::Array(imagery));
        manifest.insert("decoration".into(), Value::Object(decoration));
    }

    let (skin, _) = parse_skin(&serde_json::to_vec(&Value::Object(manifest)).ok()?, &whimbrel_default())?;
    // Palettes not repaired above (a fork's built-ins, Whimbrel's own, your earlier ones) must still suit
    // this skin's layout: one that doesn't is replaced by a repaired copy of its own.
    let gated = save_gate_repair(&skin, &palettes, &|id| pool.iter().find(|p| p.id == id).cloned());
    lines.extend(gated.notes);
    let files: Vec<(String, Vec<u8>)> = gated.palettes.iter().map(|p| (p.id.clone(), pretty(&p.to_json()))).collect();
    if let Err(e) = user.save_raw(&target, &pretty(&serde_json::to_value(&gated.skin).ok()?), &files) {
        tracing::error!(error = %e, "skin save failed");
        return None;
    }
    Some((target, lines))
}

fn taken(existing: &[Palette], palettes: &[Palette], id: &str) -> bool {
    existing.iter().chain(palettes).any(|p| p.id == id)
}

/// The palette with id `id`, each variant it ships (gaps filled from Default) repaired to pass every
/// layout's pairs, since any skin may take it. `None` when a variant still fails, so the visible fallback
/// runs, or when it would not load.
fn repaired_palette(raw: &Value, id: &str) -> Option<(Palette, Vec<Token>)> {
    let object = raw.as_object()?;
    let base = default_palette();
    let pairs = all_pairs();
    let mut changed: Vec<Token> = Vec::new();
    let mut variant = |key: &str, base: &Variant| -> Option<Option<Variant>> {
        let Some(tokens) = object.get(key).and_then(Value::as_object) else { return Some(None) };
        let mut filled = base.clone();
        for token in Token::ALL {
            if let Some(colour) = tokens.get(token.name()).and_then(Value::as_str).and_then(hex) {
                filled.set(token, colour);
            }
        }
        let (fixed, moved) = repair_variant(&filled, &pairs);
        if !pairs_pass(&fixed, &pairs) {
            return None;
        }
        for token in moved {
            if !changed.contains(&token) {
                changed.push(token);
            }
        }
        Some(Some(fixed))
    };
    let light = variant("light", &base.light)?;
    let dark = variant("dark", &base.dark)?;
    let variants = match (&light, &dark) {
        (Some(_), Some(_)) => Variants::Both,
        (Some(_), None) => Variants::LightOnly,
        (None, Some(_)) => Variants::DarkOnly,
        (None, None) => return None,
    };
    let palette = Palette {
        id: id.to_string(),
        name: object.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Colours").to_string(),
        author: object.get("author").and_then(Value::as_str).unwrap_or("").to_string(),
        light: light.clone().or_else(|| dark.clone())?,
        dark: dark.or(light)?,
        variants,
    };
    // As it will be read back.
    let loaded = parse_palette(&serde_json::to_vec(&palette.to_json()).ok()?, &base)?;
    Some((loaded, changed))
}

/// `#rrggbb` (either case) to a colour.
fn hex(s: &str) -> Option<u32> {
    let digits = s.strip_prefix('#')?;
    (digits.len() == 6).then(|| u32::from_str_radix(digits, 16).ok()).flatten()
}

/// The chat line for a palette whose `changed` tokens were moved for contrast.
fn adjusted(changed: &[Token], name: &str) -> String {
    let words: Vec<String> = changed.iter().map(|t| words(*t)).collect();
    format!("I adjusted {} in \u{201C}{name}\u{201D} so text stays readable.", words.join(", "))
}

/// `textSecondary` as "text secondary".
fn words(token: Token) -> String {
    token.name().chars().fold(String::new(), |mut out, c| {
        if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
        out
    })
}

fn imagery_of(manifest: &Map<String, Value>) -> Vec<Value> {
    manifest.get("decoration").and_then(|d| d.get("imagery")).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// An imagery entry's file name, when it is a plain one.
fn asset(entry: &Value) -> Option<String> {
    entry.get("asset").and_then(Value::as_str).filter(|a| !a.is_empty() && *a != "." && *a != ".." && !a.contains(['/', '\\'])).map(str::to_string)
}

fn placement(entry: &Value) -> Option<&str> {
    entry.get("placement").and_then(Value::as_str)
}

/// `imagery` with `entry` added, replacing any entry for the same placement: one image is drawn per
/// placement, so the newest approval is the one shown. The replaced file stays on disk for undo.
fn placing(entry: Value, imagery: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = imagery.into_iter().filter(|e| placement(e) != placement(&entry)).collect();
    out.push(entry);
    out
}

fn pretty(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("a JSON value serialises")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// --- images

/// A chat line from an image action, and whether it was stored in the skin's chat.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkinLine {
    pub text: String,
    pub stored: bool,
}

fn post(user: &UserSkinStore, skin_id: &str, text: &str) -> SkinLine {
    SkinLine { text: text.to_string(), stored: append_chat(user, skin_id, &[ChatLine::now("whimbrel", text)]).is_ok() }
}

fn unsaved(text: &str) -> SkinLine {
    SkinLine { text: text.to_string(), stored: false }
}

/// A Whimbrel line for the skin's chat, stored when the skin is one of yours (a built-in keeps no chat).
pub fn post_line(user: &UserSkinStore, skin_id: &str, text: &str) -> SkinLine {
    post(user, skin_id, text)
}

/// Undo on `skin_id`: back to its previous version. A built-in never changes, so it has none.
pub fn undo(user: &UserSkinStore, skin_id: &str) -> SkinLine {
    if user.manifest(skin_id).is_none() {
        return unsaved("Built-in skins never change, so there's nothing to undo.");
    }
    let _saving = saving();
    match user.undo(skin_id) {
        Ok(true) => post(user, skin_id, "Undone: this skin is back to its previous version."),
        Ok(false) => unsaved("There's no earlier version to go back to."),
        Err(e) => {
            tracing::error!(error = %e, "skin undo failed");
            unsaved("Whimbrel couldn't undo that.")
        }
    }
}

pub const ADDED_LINE: &str = "Added the image to the header.";
pub const REPLACED_LINE: &str = "Replaced the header image with this one.";

/// Whether an image action's line says the image was placed, so the skin changed (and syncs).
pub fn placed(line: &SkinLine) -> bool {
    line.text == ADDED_LINE || line.text == REPLACED_LINE
}

/// The line for a skin that isn't one of yours yet, so it has nowhere to keep an image.
pub const NOT_YOURS_LINE: &str = "Ask for a change first, so Whimbrel makes your own copy of this skin, then add the image.";

/// Stores `bytes` (downscaled to 1600 px) in the skin's folder and places it as the header image,
/// replacing any earlier one, in a new version.
pub fn place_image(user: &UserSkinStore, skin_id: &str, bytes: &[u8], preferred_name: &str) -> Result<SkinLine, CommandError> {
    let _saving = saving();
    let unreadable = || CommandError::store(format!("the manifest of {skin_id} can't be read"));
    let saved = user.manifest(skin_id).ok_or_else(unreadable)?;
    let Ok(Value::Object(mut manifest)) = serde_json::from_slice::<Value>(&saved) else { return Err(unreadable()) };
    let small = downscale(bytes, MAX_SIDE).map_err(|e| CommandError::invalid_input(format!("not an image Whimbrel can use: {e}")))?;
    let name = user.add_image(skin_id, &small, preferred_name).map_err(store_error)?;
    let imagery = imagery_of(&manifest);
    let replaced = imagery.iter().any(|e| placement(e) == Some(HEADER));
    let entry = serde_json::json!({ "asset": name, "placement": HEADER, "textFree": false });
    let mut decoration = manifest.get("decoration").and_then(Value::as_object).cloned().unwrap_or_default();
    decoration.insert("imagery".into(), Value::Array(placing(entry, imagery)));
    manifest.insert("decoration".into(), Value::Object(decoration));
    user.save_raw(skin_id, &pretty(&Value::Object(manifest)), &user.palette_files(skin_id)).map_err(store_error)?;
    Ok(post(user, skin_id, if replaced { REPLACED_LINE } else { ADDED_LINE }))
}

/// A found or made image the user approved: `bytes` is `None` when it couldn't be downloaded.
pub fn approve(user: &UserSkinStore, skin_id: &str, bytes: Option<Vec<u8>>, preferred_name: &str) -> Result<SkinLine, CommandError> {
    if user.manifest(skin_id).is_none() {
        return Ok(unsaved(NOT_YOURS_LINE));
    }
    match bytes {
        Some(bytes) if skins::package::is_usable_image(&bytes) => match place_image(user, skin_id, &bytes, preferred_name) {
            Err(CommandError::InvalidInput { .. }) => Ok(post(user, skin_id, "That image couldn't be downloaded, so it was left out.")),
            other => other,
        },
        _ => Ok(post(user, skin_id, "That image couldn't be downloaded, so it was left out.")),
    }
}

/// A file the user picked, which approves it, placed in their skin `skin_id`. `size` is the file's
/// length, so a large file is refused before it is read.
pub fn add_own_image(user: &UserSkinStore, skin_id: &str, size: u64, read: impl FnOnce() -> std::io::Result<Vec<u8>>) -> Result<SkinLine, CommandError> {
    if user.manifest(skin_id).is_none() {
        return Ok(unsaved(NOT_YOURS_LINE));
    }
    if size > skins::package::IMAGE_BYTE_LIMIT as u64 {
        return Ok(post(user, skin_id, "That image is over 10 MB, so Whimbrel left it out."));
    }
    let Ok(bytes) = read() else { return Ok(post(user, skin_id, "Whimbrel couldn't read that file.")) };
    let not_an_image = || post(user, skin_id, "That file isn't an image Whimbrel can use.");
    if !skins::package::is_usable_image(&bytes) {
        return Ok(not_an_image());
    }
    // A neutral name by type: the file's own name (say, a person's) would reach the provider in the manifest.
    match place_image(user, skin_id, &bytes, &format!("own.{}", extension(&bytes))) {
        Err(CommandError::InvalidInput { .. }) => Ok(not_an_image()),
        other => other,
    }
}

/// `"png"`, `"jpg"`, `"gif"` or `"webp"` by the magic bytes `is_usable_image` checked.
fn extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(b"GIF") {
        "gif"
    } else if bytes.starts_with(b"RIFF") {
        "webp"
    } else {
        "png"
    }
}

// --- rename, delete, export, import (Task 12)

pub const RENAME_FAILED_LINE: &str = "Couldn't rename this skin.";
pub const DELETE_FAILED_LINE: &str = "Couldn't delete this skin.";
pub const EXPORT_FAILED_LINE: &str = "Couldn't export this skin.";
pub const TOO_BIG_TO_EXPORT_LINE: &str = "This skin is too big to export (more than 64 MB or 200 files), so it couldn't be imported again.";
pub const IMPORT_FAILED_LINE: &str = "Whimbrel couldn't import that file. It needs to be a skin exported from Whimbrel.";
/// The file types the pickers offer: the Mac's, and its name from before the rename.
pub const PACKAGE_EXTENSIONS: [&str; 2] = ["whimbrelskin", "kestrelskin"]; // exported before the rename

/// Renames one of your skins, as a new version (so Undo brings the old name back).
pub fn rename(user: &UserSkinStore, skin_id: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() || user.manifest(skin_id).is_none() {
        return Err(RENAME_FAILED_LINE.to_string());
    }
    let _saving = saving();
    user.rename(skin_id, name).map_err(|e| {
        tracing::error!(error = %e, "skin rename failed");
        RENAME_FAILED_LINE.to_string()
    })
}

/// Moves one of your skins to `.deleted/`, with its chat and history.
pub fn delete(user: &UserSkinStore, skin_id: &str) -> Result<(), String> {
    let _saving = saving();
    user.delete(skin_id).map_err(|e| {
        tracing::error!(error = %e, "skin delete failed");
        DELETE_FAILED_LINE.to_string()
    })
}

/// Writes one of your skins to `path` as a package. Refuses one too large to import again.
pub fn export_to(user: &UserSkinStore, skin_id: &str, path: &Path) -> Result<(), String> {
    let bytes = skins::package::export(user, skin_id).map_err(|e| match e {
        skins::package::PackageError::TooLargeToExport => TOO_BIG_TO_EXPORT_LINE.to_string(),
        e => {
            tracing::error!(error = %e, "skin export failed");
            EXPORT_FAILED_LINE.to_string()
        }
    })?;
    std::fs::write(path, bytes).map_err(|e| {
        tracing::error!(error = %e, "skin export write failed");
        EXPORT_FAILED_LINE.to_string()
    })
}

/// Adds the package at `path` as another of your skins and returns its id. Its images are then kept to
/// 1600 px, as an approved one is. Nothing is selected.
pub fn import_from(user: &UserSkinStore, path: &Path) -> Result<String, String> {
    let failed = |e: &dyn std::fmt::Display| {
        tracing::warn!(error = %e, "skin import failed");
        IMPORT_FAILED_LINE.to_string()
    };
    // A file past the package limit is refused before it is read.
    let size = std::fs::metadata(path).map_err(|e| failed(&e))?.len();
    if size > skins::package::MAX_BYTES as u64 {
        return Err(failed(&"the file is over the package limit"));
    }
    let bytes = std::fs::read(path).map_err(|e| failed(&e))?;
    let _saving = saving();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let id = skins::package::import(user, &bytes, deadline).map_err(|e| failed(&e))?;
    downscale_images(user, &id);
    Ok(id)
}

/// Rewrites each image in the skin's folder that is over 1600 px; one that won't decode is left as it is.
fn downscale_images(user: &UserSkinStore, skin_id: &str) {
    let Ok(dir) = user.folder(skin_id).map(|d| d.join("images")) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(bytes) = std::fs::read(&path) else { continue };
        match downscale(&bytes, MAX_SIDE) {
            Ok(small) if small != bytes => {
                if let Err(e) = std::fs::write(&path, small) {
                    tracing::warn!(error = %e, "could not downscale an imported image");
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "an imported image did not decode"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn pool() -> Vec<Palette> {
        crate::commands::skins::palette_pool(&crate::commands::skins::BundledDirs { skins: repo().join("skins"), themes: repo().join("themes") })
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("whimbrel-skin-maker-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn store(&self) -> UserSkinStore {
            UserSkinStore::new(self.0.join("skins"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const ALL: SkinSources = SkinSources { provider: Provider::Anthropic, chat: true, search: true, generate: true };

    fn reply(json: Value) -> SkinTurnReply {
        llm::skin_maker::parse_skin_reply(&json.to_string()).unwrap()
    }

    fn built_in(user: &UserSkinStore, id: &str) -> Origin {
        origin(user, &repo().join("skins"), id).unwrap()
    }

    fn navy() -> Value {
        serde_json::json!({
            "id": "navy", "name": "Navy",
            "dark": { "backgroundPage": "#0b1a33", "textPrimary": "#f5f0e1", "accent": "#e0b040" }
        })
    }

    fn stadium(intent: &str) -> SkinTurnReply {
        reply(serde_json::json!({
            "intent": intent,
            "reply": "Here is a stadium skin.",
            "notes": [{ "step": "palette", "text": "Navy page, gold accents" }],
            "skin": { "name": "Stadium", "palettes": ["navy"], "today": { "archetype": "field" } },
            "palettes": [navy()],
            "imageQueries": ["stadium lights"],
            "imagePrompts": ["a stadium at night"]
        }))
    }

    /// The saved skin as the gallery would load it.
    fn loaded(user: &UserSkinStore, id: &str) -> (skins::Skin, Vec<Palette>) {
        user.list().into_iter().find(|(s, _)| s.id == id).unwrap()
    }

    #[test]
    fn turn_new_saves_user_skin() {
        let scratch = Scratch::new("new");
        let user = scratch.store();
        let origin = built_in(&user, "whimbrel");
        let result = apply_reply(&user, &pool(), &origin, &stadium("new"), "a stadium skin", &HashSet::new(), ALL);
        let id = result.saved_skin_id.clone().unwrap();
        assert_eq!(id, "user-stadium-1");
        assert_eq!(result.chat_skin_id, id);
        assert_eq!(result.lines[0], "Here is a stadium skin.");
        assert_eq!(result.notes, ["Navy page, gold accents"]);
        assert!(result.stored);
        // A new skin searches whatever the request says, but makes images only when asked.
        assert_eq!(result.image_queries, ["stadium lights"]);
        assert!(result.image_prompts.is_empty());
        let (skin, palettes) = loaded(&user, &id);
        assert_eq!(skin.name, "Stadium");
        assert_eq!(skin.palettes, ["user-stadium-1-p1"]);
        assert_eq!(palettes[0].name, "Navy");
        // Every palette saved passes every layout's pairs.
        assert!(pairs_pass(&palettes[0].dark, &all_pairs()));
        // The sparse palette was filled and repaired, and the chat says so.
        assert!(result.lines[1].starts_with("I adjusted "), "{:?}", result.lines);
        let chat = chat(&user, &id);
        assert_eq!(chat.len(), 1 + result.lines.len());
        assert_eq!((chat[0].role.as_str(), chat[0].text.as_str()), ("user", "a stadium skin"));
        assert!(chat[1..].iter().all(|t| t.role == "whimbrel"));
    }

    #[test]
    fn turn_refine_on_builtin_forks_yours() {
        let scratch = Scratch::new("fork");
        let user = scratch.store();
        let origin = built_in(&user, "game-day");
        let refine = reply(serde_json::json!({
            "intent": "refine", "reply": "Brighter.",
            "skin": { "name": "Game Day", "palettes": ["game-day"], "today": { "archetype": "field" } },
            "palettes": []
        }));
        let result = apply_reply(&user, &pool(), &origin, &refine, "make it brighter", &HashSet::new(), ALL);
        let id = result.saved_skin_id.unwrap();
        assert_eq!(id, "user-game-day-1");
        let (skin, _) = loaded(&user, &id);
        assert_eq!(skin.name, "Game Day (yours)");
        // No new colours: the fork keeps the built-in's.
        assert_eq!(skin.palettes, origin.palette_ids);
        // The built-in itself never changes.
        assert!(user.manifest("game-day").is_none());
        // A refine searches only when the request mentions images.
        assert!(result.image_queries.is_empty());
    }

    #[test]
    fn turn_new_on_user_skin_makes_second() {
        let scratch = Scratch::new("second");
        let user = scratch.store();
        let first = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL);
        let mine = origin(&user, &repo().join("skins"), first.saved_skin_id.as_deref().unwrap()).unwrap();
        assert!(mine.saved);
        let second = apply_reply(&user, &pool(), &mine, &stadium("new"), "another one", &HashSet::new(), ALL);
        assert_eq!(second.saved_skin_id.as_deref(), Some("user-stadium-2"));
        assert_eq!(user.skin_ids(), ["user-stadium-1", "user-stadium-2"]);
        // A refine of your own changes it in place.
        let third = apply_reply(&user, &pool(), &mine, &stadium("refine"), "gold, please", &HashSet::new(), ALL);
        assert_eq!(third.saved_skin_id.as_deref(), Some("user-stadium-1"));
        assert_eq!(user.skin_ids().len(), 2);
        assert_eq!(chat(&user, "user-stadium-1").len(), 2 + first.lines.len() + third.lines.len());
        // The reply's palette came under an id the skin doesn't have, so it gets the next free one.
        assert_eq!(loaded(&user, "user-stadium-1").0.palettes, ["user-stadium-1-p2"]);
        // Returned under its own id, it keeps it, so the selected colours stay put.
        let mut same = navy();
        same["id"] = "user-stadium-1-p2".into();
        let keep = reply(serde_json::json!({
            "intent": "refine", "reply": "",
            "skin": { "name": "Stadium", "palettes": ["user-stadium-1-p2"], "today": { "archetype": "field" } },
            "palettes": [same]
        }));
        let mine = origin(&user, &repo().join("skins"), "user-stadium-1").unwrap();
        apply_reply(&user, &pool(), &mine, &keep, "a bit warmer", &HashSet::new(), ALL);
        assert_eq!(loaded(&user, "user-stadium-1").0.palettes, ["user-stadium-1-p2"]);
    }

    #[test]
    fn turn_garbage_changes_nothing() {
        let scratch = Scratch::new("garbage");
        let user = scratch.store();
        let origin = built_in(&user, "whimbrel");
        // A manifest the loader can't use, and a palette nothing can rescue.
        let bad = reply(serde_json::json!({ "intent": "new", "reply": "Done!", "skin": "not an object", "palettes": [] }));
        let result = apply_reply(&user, &pool(), &origin, &bad, "a skin", &HashSet::new(), ALL);
        assert_eq!(result.saved_skin_id, None);
        assert_eq!(result.lines, [UNUSABLE_LINE]);
        assert!(!result.stored);
        assert!(user.skin_ids().is_empty());
        // A reply that is not JSON at all fails the turn (after one retry), before anything is saved.
        struct Garbage;
        impl llm::LlmClient for Garbage {
            fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
                Ok(llm::Completion { text: "I'd love to help!".into(), token_usage: llm::TokenUsage { input_tokens: None, output_tokens: None } })
            }
        }
        let store = memory::Store::open(&scratch.0.join("db.sqlite"), Path::new("")).unwrap();
        let input = turn_input(&user, &origin, "a skin");
        let err = call_model(&store, &llm::SpendCapConfig::default(), llm::Provider::Ollama, "m", &Garbage, &input).unwrap_err();
        assert_eq!(err, CommandError::ProviderFailure);
        assert!(user.skin_ids().is_empty());
    }

    #[test]
    fn a_question_back_is_chat_only_and_kept_only_for_your_skins() {
        let scratch = Scratch::new("clarify");
        let user = scratch.store();
        let question = reply(serde_json::json!({ "intent": "clarify", "reply": "Which team?" }));
        let on_builtin = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &question, "sports", &HashSet::new(), ALL);
        assert_eq!((on_builtin.lines.as_slice(), on_builtin.stored, on_builtin.saved_skin_id.as_deref()), (&["Which team?".to_string()][..], false, None));
        let mine = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL);
        let mine = origin(&user, &repo().join("skins"), mine.saved_skin_id.as_deref().unwrap()).unwrap();
        let on_mine = apply_reply(&user, &pool(), &mine, &question, "sports", &HashSet::new(), ALL);
        assert!(on_mine.stored);
        assert_eq!(chat(&user, &mine.id).last().unwrap().text, "Which team?");
    }

    #[test]
    fn cap_reached_returns_cap_error() {
        let scratch = Scratch::new("cap");
        let store = memory::Store::open(&scratch.0.join("db.sqlite"), Path::new("")).unwrap();
        let caps = llm::SpendCapConfig { user_daily_cents: Some(3), global_daily_cents: None };
        // One generated image (5¢) spends past a 3¢ cap.
        charge(&store, &caps, "skin-image-generate", 5).unwrap();
        struct Unreachable;
        impl llm::LlmClient for Unreachable {
            fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
                panic!("the cap is checked before any provider call")
            }
        }
        let user = scratch.store();
        let input = turn_input(&user, &built_in(&user, "whimbrel"), "a skin");
        let err = call_model(&store, &caps, llm::Provider::Anthropic, "m", &Unreachable, &input).unwrap_err();
        assert!(matches!(err, CommandError::SpendCap { .. }), "{err:?}");
        assert!(matches!(charge(&store, &caps, "skin-image-search", 1), Err(CommandError::SpendCap { .. })));
    }

    #[test]
    fn no_brave_key_hides_search() {
        let routing = RoutingSettings { preferred_provider: Provider::Anthropic, ..RoutingSettings::default() };
        let keys = ApiKeys { anthropic: Some("k".into()), openai: None, google: Some("g".into()), ..ApiKeys::default() };
        let anthropic = |chat, search, generate| SkinSources { provider: Provider::Anthropic, chat, search, generate };
        assert_eq!(SkinSources::new(&routing, &keys, false), anthropic(true, false, true));
        assert_eq!(SkinSources::new(&routing, &keys, true), anthropic(true, true, true));
        // Local-only mode: hosted chat, search and image making all stop; Ollama still chats.
        let local = RoutingSettings { local_only: true, ..routing.clone() };
        assert_eq!(SkinSources::new(&local, &keys, true), anthropic(false, false, false));
        let ollama = RoutingSettings { preferred_provider: Provider::Ollama, local_only: true, ..routing };
        assert!(SkinSources::new(&ollama, &ApiKeys::default(), false).chat);
        // A search the model asked for is dropped when this device can't search.
        let scratch = Scratch::new("nosearch");
        let user = scratch.store();
        let none = SkinSources { provider: Provider::Anthropic, chat: true, search: false, generate: false };
        let result = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium photo skin", &HashSet::new(), none);
        assert!(result.image_queries.is_empty() && result.image_prompts.is_empty());
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([200, 30, 30]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn header_asset(user: &UserSkinStore, id: &str) -> String {
        let (skin, _) = loaded(user, id);
        let header: Vec<_> = skin.decoration.imagery.iter().filter(|i| i.placement == skins::format::Placement::Header).collect();
        assert_eq!(header.len(), 1);
        header[0].asset.clone()
    }

    #[test]
    fn approve_downscales_to_1600() {
        let scratch = Scratch::new("approve");
        let user = scratch.store();
        let id = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL).saved_skin_id.unwrap();
        let line = approve(&user, &id, Some(png(2400, 1200)), "https://example.com/lights.png?w=2400").unwrap();
        assert_eq!(line, SkinLine { text: "Added the image to the header.".into(), stored: true });
        let asset = header_asset(&user, &id);
        assert_eq!(asset, "lights.png");
        let bytes = std::fs::read(user.image_path(&id, &asset).unwrap()).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().dimensions(), (1600, 800));
        // A second approval replaces the header image, and the first file stays for undo.
        let again = approve(&user, &id, Some(png(10, 10)), "generated.png").unwrap();
        assert_eq!(again.text, "Replaced the header image with this one.");
        assert_eq!(header_asset(&user, &id), "generated.png");
        assert!(user.image_path(&id, "lights.png").is_ok());
        // An image the model never saw survives the next refine.
        let mine = origin(&user, &repo().join("skins"), &id).unwrap();
        let sent = HashSet::from(["lights.png".to_string()]);
        apply_reply(&user, &pool(), &mine, &stadium("refine"), "gold", &sent, ALL);
        assert_eq!(header_asset(&user, &id), "generated.png");
        // A download that failed, or a built-in, places nothing.
        assert_eq!(approve(&user, &id, None, "x.png").unwrap().text, "That image couldn't be downloaded, so it was left out.");
        assert_eq!(approve(&user, "whimbrel", Some(png(4, 4)), "x.png").unwrap(), SkinLine { text: NOT_YOURS_LINE.into(), stored: false });
    }

    use image::GenericImageView;

    #[test]
    fn add_file_refuses_non_image() {
        let scratch = Scratch::new("own");
        let user = scratch.store();
        let id = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL).saved_skin_id.unwrap();
        let text = b"%PDF-1.7 not a picture".to_vec();
        let line = add_own_image(&user, &id, text.len() as u64, || Ok(text.clone())).unwrap();
        assert_eq!(line.text, "That file isn't an image Whimbrel can use.");
        // PNG magic, but it doesn't decode.
        let broken = [&[0x89, 0x50, 0x4E, 0x47][..], b"junk"].concat();
        assert_eq!(add_own_image(&user, &id, 8, || Ok(broken.clone())).unwrap().text, "That file isn't an image Whimbrel can use.");
        let big = add_own_image(&user, &id, 11 * 1024 * 1024, || panic!("a large file is never read")).unwrap();
        assert_eq!(big.text, "That image is over 10 MB, so Whimbrel left it out.");
        assert!(user.list()[0].0.decoration.imagery.is_empty());
        // A real image gets a neutral name, never the file's own.
        let picture = png(20, 10);
        assert_eq!(add_own_image(&user, &id, picture.len() as u64, || Ok(picture.clone())).unwrap().text, "Added the image to the header.");
        assert_eq!(header_asset(&user, &id), "own.png");
    }

    #[test]
    fn a_palette_too_weak_to_repair_falls_back_with_a_line() {
        let scratch = Scratch::new("fallback");
        let user = scratch.store();
        // No variants at all: nothing to repair, so the skin takes Whimbrel's colours and says so.
        let empty = reply(serde_json::json!({
            "intent": "new", "reply": "",
            "skin": { "name": "Plain", "palettes": ["x"] },
            "palettes": [{ "id": "x", "name": "X" }]
        }));
        let result = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &empty, "plain", &HashSet::new(), ALL);
        assert_eq!(result.lines, [DONE_LINE, NO_PALETTE_LINE]);
        assert_eq!(loaded(&user, "user-plain-1").0.palettes, ["default"]);
    }

    #[test]
    fn undo_goes_back_one_version_and_says_so() {
        let scratch = Scratch::new("undo");
        let user = scratch.store();
        let id = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL).saved_skin_id.unwrap();
        assert_eq!(undo(&user, &id), SkinLine { text: "There's no earlier version to go back to.".into(), stored: false });
        let mine = origin(&user, &repo().join("skins"), &id).unwrap();
        let renamed = reply(serde_json::json!({ "intent": "refine", "reply": "", "skin": { "name": "Arena", "palettes": [] } }));
        apply_reply(&user, &pool(), &mine, &renamed, "call it Arena", &HashSet::new(), ALL);
        assert_eq!(loaded(&user, &id).0.name, "Arena");
        let line = undo(&user, &id);
        assert_eq!(line, SkinLine { text: "Undone: this skin is back to its previous version.".into(), stored: true });
        assert_eq!(loaded(&user, &id).0.name, "Stadium");
        assert_eq!(chat(&user, &id).last().unwrap().text, line.text);
        assert_eq!(undo(&user, "game-day").text, "Built-in skins never change, so there's nothing to undo.");
        // A line posted to a built-in is shown, never stored.
        assert!(!post_line(&user, "game-day", SPEND_CAP_LINE).stored);
        assert!(post_line(&user, &id, SPEND_CAP_LINE).stored);
    }

    #[test]
    fn rename_and_delete_your_skins_only() {
        let scratch = Scratch::new("rename");
        let user = scratch.store();
        let id = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL).saved_skin_id.unwrap();
        rename(&user, &id, "  Arena  ").unwrap();
        assert_eq!(loaded(&user, &id).0.name, "Arena");
        // A rename is a version, so Undo brings the name back.
        undo(&user, &id);
        assert_eq!(loaded(&user, &id).0.name, "Stadium");
        assert_eq!(rename(&user, &id, "   "), Err(RENAME_FAILED_LINE.into()));
        assert_eq!(rename(&user, "game-day", "Mine"), Err(RENAME_FAILED_LINE.into()));
        delete(&user, &id).unwrap();
        assert!(user.skin_ids().is_empty());
        assert_eq!(user.deleted_ids(), std::slice::from_ref(&id));
        assert_eq!(delete(&user, &id), Err(DELETE_FAILED_LINE.into()));
    }

    #[test]
    fn import_mac_fixture_via_command() {
        // No Mac-made package is checked in yet (core/skins/tests/package.rs waits for one), so this
        // round-trips one in the Mac's format: a skin with its own palette and an image over 1600 px.
        let scratch = Scratch::new("import");
        let user = scratch.store();
        let id = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL).saved_skin_id.unwrap();
        // Written straight to the folder, as an image approved before downscaling existed would be.
        user.add_image(&id, &png(2000, 1000), "wide.png").unwrap();
        let mut manifest: Value = serde_json::from_slice(&user.manifest(&id).unwrap()).unwrap();
        manifest["decoration"]["imagery"] = serde_json::json!([{ "asset": "wide.png", "placement": "header", "textFree": false }]);
        user.save_raw(&id, &pretty(&manifest), &user.palette_files(&id)).unwrap();
        let file = scratch.0.join("stadium.whimbrelskin");
        export_to(&user, &id, &file).unwrap();
        let imported = import_from(&user, &file).unwrap();
        // A clash with one of yours gets the next id and a " 2" name.
        assert_eq!(imported, "user-stadium-1-2");
        let (skin, palettes) = loaded(&user, &imported);
        assert_eq!(skin.name, "Stadium 2");
        assert_eq!(palettes.len(), 1);
        assert!(palettes[0].id.starts_with(&imported));
        let wide = std::fs::read(user.image_path(&imported, "wide.png").unwrap()).unwrap();
        assert_eq!(image::load_from_memory(&wide).unwrap().dimensions(), (1600, 800));
        // Anything else is refused with the Mac's line, and nothing is added.
        let junk = scratch.0.join("junk.whimbrelskin");
        std::fs::write(&junk, b"not a zip").unwrap();
        assert_eq!(import_from(&user, &junk), Err(IMPORT_FAILED_LINE.into()));
        assert_eq!(import_from(&user, &scratch.0.join("missing.whimbrelskin")), Err(IMPORT_FAILED_LINE.into()));
        assert_eq!(user.skin_ids().len(), 2);
        assert_eq!(export_to(&user, "game-day", &scratch.0.join("x.whimbrelskin")), Err(EXPORT_FAILED_LINE.into()));
    }

    #[test]
    fn mentions_images_at_word_starts() {
        for yes in ["add a photo", "Backgrounds please", "a LOGO", "some art"] {
            assert!(mentions_images(yes), "{yes}");
        }
        for no in ["make it darker", "smart colours", "the epicture"] {
            assert!(!mentions_images(no), "{no}");
        }
    }

    #[test]
    fn chat_reads_the_macs_file() {
        let scratch = Scratch::new("chat");
        let user = scratch.store();
        let id = apply_reply(&user, &pool(), &built_in(&user, "whimbrel"), &stadium("new"), "a stadium skin", &HashSet::new(), ALL).saved_skin_id.unwrap();
        // As Swift's JSONEncoder writes `[SkinChatTurn]`.
        let mac = r#"[{"role":"user","text":"hi","date":783000000.5},{"role":"whimbrel","text":"Hello.","date":783000001}]"#;
        std::fs::write(user.folder(&id).unwrap().join("chat.json"), mac).unwrap();
        let read = chat(&user, &id);
        assert_eq!(read[1], ChatLine { role: "whimbrel".into(), text: "Hello.".into(), date: 783000001.0 });
        append_chat(&user, &id, &[ChatLine::now("user", "more")]).unwrap();
        let now = chat(&user, &id).pop().unwrap();
        // Around now in Swift's reckoning (2026 is ~ 8.1e8 s after 2001).
        assert!(now.date > 8.0e8 && now.date < 9.0e8, "{}", now.date);
        assert!(append_chat(&user, "user-never-saved-1", &[now]).is_err());
    }
}
