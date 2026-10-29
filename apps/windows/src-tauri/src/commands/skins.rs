//! Skins for the UI (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md, Task 6): the seven
//! built-in skins and the user's own, each with the palettes it offers, and the images a skin
//! approved. The rules live in `core/skins`; this only gathers the files and hands them to the page.
//!
//! Making skins by chat (Task 10) is here too: the turn, finding, making and approving images, and the
//! sources this device may use. The decisions live in `crate::skin_maker`.
//!
//! The built-ins ship as Tauri resources: `skins/*.json`, `skins/palettes/*.json` and `themes/*.json`
//! (the plain themes are the Whimbrel skin's palettes, as on the Mac).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use skins::format::{Archetype, Placement};
use skins::palette::Palette;
use skins::store::archetype_repair;
use skins::{default_palette, load_bundled, load_palettes, Glow, Skin, Token, UserSkinStore, Variant};
use tauri::{ipc::Response, AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::CommandError;
use crate::secrets::{accounts, APP_SERVICE};
use crate::settings::{ApiKeys, Instructions, Provider, RoutingSettings};
use crate::skin_maker::{self, SkinLine, SkinSources, SkinTurnResult};
use crate::state::AppState;

/// One skin as the page needs it: the manifest as JSON (the shape `skins/*.json` has), the palettes it
/// offers in its own order, and whether it ships with Whimbrel.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinDto {
    pub skin: Value,
    /// Only the variants each palette ships (`light`, `dark` or both), and already gated for the
    /// skin's Today layout, so the page never draws text below 4.5:1.
    pub palettes: Vec<Value>,
    pub built_in: bool,
    /// How strongly the page colour must cover each background image, by asset name, for every text
    /// colour on it to read at 4.5:1: the worst over the palettes offered, light and dark. An image the
    /// shell can't measure is missing, and the page covers it fully.
    pub image_scrims: BTreeMap<String, f64>,
}

/// Where the built-ins live: `skins/` (skins, with `palettes/` inside) and `themes/`.
#[derive(Debug, Clone)]
pub struct BundledDirs {
    pub skins: PathBuf,
    pub themes: PathBuf,
}

impl BundledDirs {
    pub fn under(resources: &Path) -> Self {
        Self { skins: resources.join("skins"), themes: resources.join("themes") }
    }
}

/// `palette` as it is drawn under `skin`'s layout: repaired when it fails that layout's pairs, with
/// the id, name, author and shipped variants it had.
fn gated(palette: &Palette, skin: &Skin) -> Palette {
    match archetype_repair(palette, &palette.id, skin.today.archetype) {
        Some((fixed, _)) => Palette {
            id: palette.id.clone(),
            name: palette.name.clone(),
            author: palette.author.clone(),
            variants: palette.variants,
            light: fixed.light,
            dark: fixed.dark,
        },
        None => palette.clone(),
    }
}

/// The palettes of `skin`, in its manifest order, found among `own` then `pool`, and gated for its layout;
/// Default when none is.
fn offered_palettes(skin: &Skin, own: &[Palette], pool: &[Palette]) -> Vec<Palette> {
    let mut found: Vec<&Palette> =
        skin.palettes.iter().filter_map(|id| own.iter().chain(pool).find(|p| &p.id == id)).collect();
    let fallback = default_palette();
    if found.is_empty() {
        found.push(pool.iter().find(|p| p.id == fallback.id).unwrap_or(&fallback));
    }
    found.into_iter().map(|p| gated(p, skin)).collect()
}

#[cfg(test)]
fn offered(skin: &Skin, own: &[Palette], pool: &[Palette]) -> Vec<Value> {
    offered_palettes(skin, own, pool).iter().map(Palette::to_json).collect()
}

fn dto(skin: &Skin, palettes: &[Palette], built_in: bool, image_scrims: BTreeMap<String, f64>) -> SkinDto {
    SkinDto {
        skin: serde_json::to_value(skin).expect("a skin serialises"),
        palettes: palettes.iter().map(Palette::to_json).collect(),
        built_in,
        image_scrims,
    }
}

/// Every colour Today draws text in; the scrim is sized for the worst of them (the Mac's `SkinImageryView.textTokens`).
const TEXT_TOKENS: [Token; 5] = [Token::TextPrimary, Token::TextSecondary, Token::AccentPrimary, Token::Error, Token::Link];

fn colour(v: &Variant, token: Token) -> u32 {
    v.get(token).unwrap_or(0)
}

/// The scrim one image needs under one palette variant (the Mac's `SkinImageryView.opacity`): every text
/// colour over the image's lightest and darkest colour, and under Glass, its glow's text on the glow too.
fn scrim_for(sample: &[u8], v: &Variant, archetype: Archetype) -> f64 {
    let page = colour(v, Token::BackgroundPage);
    let side = skins::images::SAMPLE_SIDE;
    let (lightest, darkest) = skins::extremes(sample, side, side, page);
    let text: Vec<u32> = TEXT_TOKENS.iter().map(|&t| colour(v, t)).collect();
    let glow = (archetype == Archetype::Glass).then(|| Glow {
        accent: colour(v, Token::AccentPrimary),
        text: vec![colour(v, Token::TextPrimary), colour(v, Token::TextSecondary)],
    });
    skins::scrim_opacity(&text, page, lightest, darkest, glow.as_ref())
}

/// The scrim each background image of `skin` needs: the worst over `palettes`, light and dark. Only a
/// background has text over it; a header or badge sits beside the text and is drawn as it is.
fn image_scrims(user: &UserSkinStore, skin: &Skin, palettes: &[Palette]) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for image in skin.decoration.imagery.iter().filter(|i| i.placement == Placement::Background) {
        let Some(sample) = user.image_path(&skin.id, &image.asset).ok().and_then(|path| scrim_sample(&path)) else { continue };
        let worst = palettes
            .iter()
            .flat_map(|p| [&p.light, &p.dark])
            .map(|v| scrim_for(&sample, v, skin.today.archetype))
            .fold(0.0, f64::max);
        out.insert(image.asset.clone(), worst);
    }
    out
}

/// Image samples by file, kept while the file is unchanged, so listing the skins decodes each image once.
type Samples = HashMap<(PathBuf, std::time::SystemTime, u64), Vec<u8>>;

static SAMPLES: LazyLock<Mutex<Samples>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn scrim_sample(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    let key = (path.to_path_buf(), meta.modified().ok()?, meta.len());
    if let Some(sample) = SAMPLES.lock().unwrap().get(&key) {
        return Some(sample.clone());
    }
    let sample = skins::images::scrim_sample(&std::fs::read(path).ok()?).ok()?;
    let mut samples = SAMPLES.lock().unwrap();
    // Changed or deleted files leave stale keys behind; a skin has few images, so dropping all now and then is enough.
    if samples.len() > 64 {
        samples.clear();
    }
    samples.insert(key, sample.clone());
    Some(sample)
}

/// The palettes any skin may name: the plain themes first, then the bundled skins' own; the first of an id wins.
pub fn palette_pool(dirs: &BundledDirs) -> Vec<Palette> {
    let fallback = default_palette();
    let mut pool = load_palettes(&dirs.themes, &fallback);
    for p in load_palettes(&dirs.skins.join("palettes"), &fallback) {
        if !pool.iter().any(|q| q.id == p.id) {
            pool.push(p);
        }
    }
    pool
}

/// Every skin: the built-ins first (Whimbrel, then by name), then the user's by name.
pub fn list_skins(dirs: &BundledDirs, user: &UserSkinStore) -> Vec<SkinDto> {
    let pool = palette_pool(dirs);
    // The built-ins carry no imagery.
    let mut out: Vec<SkinDto> =
        load_bundled(&dirs.skins).iter().map(|s| dto(s, &offered_palettes(s, &[], &pool), true, BTreeMap::new())).collect();
    for (skin, own) in user.list() {
        let palettes = offered_palettes(&skin, &own, &pool);
        let scrims = image_scrims(user, &skin, &palettes);
        out.push(dto(&skin, &palettes, false, scrims));
    }
    out
}

/// The bytes of an approved image in one user skin's folder. The store applies the name rule and the
/// symlink and containment checks, so a name can never reach outside the skin's folder.
pub fn read_skin_image(user: &UserSkinStore, id: &str, name: &str) -> Result<Vec<u8>, CommandError> {
    let path = user.image_path(id, name).map_err(|e| CommandError::invalid_input(e.to_string()))?;
    std::fs::read(path).map_err(|e| CommandError::invalid_input(format!("could not read the image: {e}")))
}

fn bundled(app: &AppHandle) -> Result<BundledDirs, CommandError> {
    let dir = app.path().resource_dir().map_err(|e| CommandError::not_configured(format!("no resource directory: {e}")))?;
    Ok(BundledDirs::under(&dir))
}

#[tauri::command]
pub async fn skins_list(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<SkinDto>, CommandError> {
    let dirs = bundled(&app)?;
    let user = UserSkinStore::new(state.paths.skins.clone());
    // File reads and contrast repair: off the async runtime's threads.
    tauri::async_runtime::spawn_blocking(move || list_skins(&dirs, &user))
        .await
        .map_err(|e| CommandError::store(format!("skins list failed: {e}")))
}

/// An approved image of a user skin, as raw bytes (the page makes an object URL from them).
#[tauri::command]
pub async fn skin_image(state: State<'_, AppState>, id: String, name: String) -> Result<Response, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    let bytes = tauri::async_runtime::spawn_blocking(move || read_skin_image(&user, &id, &name))
        .await
        .map_err(|e| CommandError::store(format!("image read failed: {e}")))??;
    Ok(Response::new(bytes))
}

// --- making skins (Task 10)

/// A found or made image waiting for approval (`llm::images::ImageCandidate`). A made one has a
/// `generated:` url, and its bytes wait here until it is approved or rejected.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageCandidateView {
    pub url: String,
    pub thumbnail_url: String,
    pub title: String,
    /// `"web"` or `"generated"`.
    pub source: String,
}

impl From<llm::images::ImageCandidate> for ImageCandidateView {
    fn from(c: llm::images::ImageCandidate) -> Self {
        ImageCandidateView { url: c.url, thumbnail_url: c.thumbnail_url, title: c.title, source: c.source }
    }
}

const SET_UP_LINE: &str = "Set up an AI provider in Settings to make skins.";
/// Made images not yet approved or rejected; the oldest go first past this many.
const GENERATED_KEPT: usize = 12;

/// Made images by url, oldest first.
type Made = Vec<(String, Vec<u8>)>;

static GENERATED: LazyLock<Mutex<Made>> = LazyLock::new(|| Mutex::new(Vec::new()));
/// Turns running, by the page's run id, and whether each was stopped.
static RUNS: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn remember_generated(url: &str, bytes: Vec<u8>) {
    let mut kept = GENERATED.lock().unwrap();
    kept.push((url.to_string(), bytes));
    let excess = kept.len().saturating_sub(GENERATED_KEPT);
    kept.drain(..excess);
}

fn generated(url: &str) -> Option<Vec<u8>> {
    GENERATED.lock().unwrap().iter().find(|(u, _)| u == url).map(|(_, b)| b.clone())
}

fn take_generated(url: &str) -> Option<Vec<u8>> {
    let mut kept = GENERATED.lock().unwrap();
    let at = kept.iter().position(|(u, _)| u == url)?;
    Some(kept.remove(at).1)
}

/// What this device may use now, with the settings and keys it was worked out from.
fn sources(state: &AppState) -> Result<(SkinSources, RoutingSettings, ApiKeys), CommandError> {
    let routing = state.settings.lock().unwrap().routing();
    let keys = ApiKeys::load(&*state.secrets)?;
    let search = state.secrets.get_string(APP_SERVICE, accounts::IMAGE_SEARCH_API_KEY)?.is_some();
    Ok((SkinSources::new(&routing, &keys, search), routing, keys))
}

fn caps(routing: &RoutingSettings, keys: &ApiKeys) -> llm::SpendCapConfig {
    routing.routing_config(keys, &Instructions::default()).spend_cap_config()
}

type Client = Box<dyn llm::LlmClient + Send>;

/// The preferred provider's client and model, as the Mac's skin chat uses (`SkinChatAccess`).
fn skin_client(routing: &RoutingSettings, keys: &ApiKeys) -> Result<(llm::Provider, String, Client), CommandError> {
    let model = routing.preferred_model().to_string();
    let key = |k: &Option<String>| k.clone().ok_or_else(|| CommandError::not_configured(SET_UP_LINE));
    let (provider, client): (llm::Provider, Client) = match routing.preferred_provider {
        Provider::Ollama => (llm::Provider::Ollama, Box::new(llm::OllamaClient::new(&model))),
        Provider::Anthropic => (llm::Provider::Anthropic, Box::new(llm::AnthropicClient::new(key(&keys.anthropic)?, &model))),
        Provider::Openai => (llm::Provider::OpenAI, Box::new(llm::OpenAIClient::new(key(&keys.openai)?, &model))),
        Provider::Google => (llm::Provider::Google, Box::new(llm::GoogleClient::new(key(&keys.google)?, &model))),
        Provider::Openrouter => (llm::Provider::OpenRouter, Box::new(llm::OpenAIClient::openrouter(key(&keys.openrouter)?, &model))),
    };
    Ok((provider, model, client))
}

/// Google when it is the preferred provider and has a key, else OpenAI, else Google (the Mac's choice).
fn image_maker(routing: &RoutingSettings, keys: &ApiKeys) -> Option<(llm::Provider, String)> {
    match (&keys.openai, &keys.google) {
        (_, Some(g)) if routing.preferred_provider == Provider::Google => Some((llm::Provider::Google, g.clone())),
        (Some(o), _) => Some((llm::Provider::OpenAI, o.clone())),
        (None, Some(g)) => Some((llm::Provider::Google, g.clone())),
        (None, None) => None,
    }
}

fn blocking_failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::store(format!("the skin maker stopped unexpectedly: {e}"))
}

/// Queues `skin_id` for sync after a change on this PC (Task 15), on the store actor. A failure is
/// logged there and never fails the command: the next change, or a re-push, sends it.
async fn queue_sync(state: &AppState, skin_id: Option<String>) {
    let (Some(id), Ok(store)) = (skin_id, state.store()) else { return };
    let root = state.paths.skins.clone();
    let _ = store.call(move |s| crate::skin_sync::after_local_change(s, &root, &id)).await;
}

/// Bumped when a sync changed the skins folder; every window reloads the skins when it moves (Task 15).
#[tauri::command]
pub fn skins_revision() -> u64 {
    crate::skin_sync::revision()
}

/// What the skin chat may use on this device.
#[tauri::command]
pub fn skin_sources(state: State<'_, AppState>) -> Result<SkinSources, CommandError> {
    Ok(sources(&state)?.0)
}

/// One turn on `skin_id`: the model's reply saved as one of your skins (or a question back), and the chat
/// stored. `run_id` is the page's, so `skin_turn_stop` can make a late reply change nothing.
#[tauri::command]
pub async fn skin_turn(app: AppHandle, state: State<'_, AppState>, skin_id: String, request: String, run_id: String) -> Result<SkinTurnResult, CommandError> {
    let request = request.trim().to_string();
    if request.is_empty() {
        return Err(CommandError::invalid_input("Say what you'd like the skin to look like."));
    }
    let (sources, routing, keys) = sources(&state)?;
    if !sources.chat {
        return Err(CommandError::not_configured(SET_UP_LINE));
    }
    let dirs = bundled(&app)?;
    let user = UserSkinStore::new(state.paths.skins.clone());
    let origin = skin_maker::origin(&user, &dirs.skins, &skin_id).ok_or_else(|| CommandError::invalid_input(format!("no skin {skin_id}")))?;
    let (provider, model, client) = skin_client(&routing, &keys)?;
    let caps = caps(&routing, &keys);
    let input = skin_maker::turn_input(&user, &origin, &request);
    let slow = state.slow_store()?.clone();
    RUNS.lock().unwrap().insert(run_id.clone(), false);
    // The provider call holds the slow lane, as Ask's blocking answer does, never the main store.
    let reply = slow.call(move |s| skin_maker::call_model(s, &caps, provider, &model, client.as_ref(), &input)).await;
    let stopped = RUNS.lock().unwrap().remove(&run_id).unwrap_or(false);
    let reply = reply??;
    if stopped {
        return Ok(SkinTurnResult::stopped(&origin.id));
    }
    let sent = origin.assets();
    let result = tauri::async_runtime::spawn_blocking(move || skin_maker::apply_reply(&user, &palette_pool(&dirs), &origin, &reply, &request, &sent, sources))
        .await
        .map_err(blocking_failed)?;
    queue_sync(&state, result.saved_skin_id.clone()).await;
    Ok(result)
}

/// The skin's chat (Task 11); empty for a built-in.
#[tauri::command]
pub async fn skin_chat_load(state: State<'_, AppState>, skin_id: String) -> Result<Vec<skin_maker::ChatLine>, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    tauri::async_runtime::spawn_blocking(move || skin_maker::chat(&user, &skin_id)).await.map_err(blocking_failed)
}

/// A Whimbrel line for the skin's chat, kept when the skin is yours: the page's own lines, such as the
/// spend cap stopping an image search.
#[tauri::command]
pub async fn skin_chat_post(state: State<'_, AppState>, skin_id: String, text: String) -> Result<SkinLine, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    tauri::async_runtime::spawn_blocking(move || skin_maker::post_line(&user, &skin_id, &text)).await.map_err(blocking_failed)
}

/// Undo on one of your skins: back to its previous version.
#[tauri::command]
pub async fn skin_undo(state: State<'_, AppState>, skin_id: String) -> Result<SkinLine, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    let id = skin_id.clone();
    let line = tauri::async_runtime::spawn_blocking(move || skin_maker::undo(&user, &id)).await.map_err(blocking_failed)?;
    if line.stored {
        queue_sync(&state, Some(skin_id)).await;
    }
    Ok(line)
}

// --- rename, delete, export, import (Task 12)

/// The page's line when an action fails (the Mac's wording), as a `CommandError` the page shows as is.
fn refused(line: String) -> CommandError {
    CommandError::invalid_input(line)
}

#[tauri::command]
pub async fn skin_rename(state: State<'_, AppState>, skin_id: String, name: String) -> Result<(), CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    let id = skin_id.clone();
    tauri::async_runtime::spawn_blocking(move || skin_maker::rename(&user, &id, &name)).await.map_err(blocking_failed)?.map_err(refused)?;
    queue_sync(&state, Some(skin_id)).await;
    Ok(())
}

#[tauri::command]
pub async fn skin_delete(state: State<'_, AppState>, skin_id: String) -> Result<(), CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    let id = skin_id.clone();
    tauri::async_runtime::spawn_blocking(move || skin_maker::delete(&user, &id)).await.map_err(blocking_failed)?.map_err(refused)?;
    queue_sync(&state, Some(skin_id)).await;
    Ok(())
}

fn file_dialog(app: &AppHandle, title: &str) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    let mut dialog = app.dialog().file().set_title(title).add_filter("Whimbrel Skin", &skin_maker::PACKAGE_EXTENSIONS);
    if let Some(window) = app.get_webview_window(crate::app::MAIN_WINDOW) {
        dialog = dialog.set_parent(&window);
    }
    dialog
}

/// Asks where to save one of your skins and writes it there; false when the picker is cancelled.
#[tauri::command]
pub async fn skin_export(app: AppHandle, state: State<'_, AppState>, skin_id: String) -> Result<bool, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    tauri::async_runtime::spawn_blocking(move || {
        // The id, not the name: it is always a valid file name, whatever the name holds.
        let picked = file_dialog(&app, "Export skin").set_file_name(format!("{skin_id}.whimbrelskin")).blocking_save_file();
        let Some(picked) = picked else { return Ok(false) };
        let path = picked.into_path().map_err(|e| CommandError::invalid_input(format!("not a file on this PC: {e}")))?;
        skin_maker::export_to(&user, &skin_id, &path).map(|()| true).map_err(refused)
    })
    .await
    .map_err(blocking_failed)?
}

/// Asks for a skin file and adds it as another of yours; its id, or `None` when the picker is cancelled.
#[tauri::command]
pub async fn skin_import(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    let imported = tauri::async_runtime::spawn_blocking(move || {
        let Some(picked) = file_dialog(&app, "Import skin").blocking_pick_file() else { return Ok(None) };
        let path = picked.into_path().map_err(|e| CommandError::invalid_input(format!("not a file on this PC: {e}")))?;
        skin_maker::import_from(&user, &path).map(Some).map_err(refused)
    })
    .await
    .map_err(blocking_failed)??;
    queue_sync(&state, imported.clone()).await;
    Ok(imported)
}

/// Stops a running turn: whatever it returns later changes nothing. False when it isn't running.
#[tauri::command]
pub fn skin_turn_stop(run_id: String) -> bool {
    RUNS.lock().unwrap().get_mut(&run_id).map(|stopped| *stopped = true).is_some()
}

/// A web image search (1¢, charged before the call).
#[tauri::command]
pub async fn skin_image_search(state: State<'_, AppState>, query: String) -> Result<Vec<ImageCandidateView>, CommandError> {
    let (sources, routing, keys) = sources(&state)?;
    let key = state.secrets.get_string(APP_SERVICE, accounts::IMAGE_SEARCH_API_KEY)?;
    let (true, Some(key)) = (sources.search, key) else {
        return Err(CommandError::not_configured("Add an image search key in Settings to find images."));
    };
    let caps = caps(&routing, &keys);
    state.store()?.call(move |s| skin_maker::charge(s, &caps, "skin-image-search", llm::images::IMAGE_SEARCH_CENTS)).await??;
    let found = tauri::async_runtime::spawn_blocking(move || llm::images::search_images(&key, &query)).await.map_err(blocking_failed)?;
    Ok(found.map_err(|_| CommandError::ProviderFailure)?.into_iter().map(Into::into).collect())
}

/// Makes one image (5¢, charged before the call). It waits here, unsaved, until approved.
#[tauri::command]
pub async fn skin_image_generate(state: State<'_, AppState>, prompt: String) -> Result<ImageCandidateView, CommandError> {
    let (sources, routing, keys) = sources(&state)?;
    let (true, Some((provider, key))) = (sources.generate, image_maker(&routing, &keys)) else {
        return Err(CommandError::not_configured("Add an OpenAI or Google key in Settings to make images."));
    };
    let caps = caps(&routing, &keys);
    state.store()?.call(move |s| skin_maker::charge(s, &caps, "skin-image-generate", llm::images::IMAGE_GENERATE_CENTS)).await??;
    let asked = prompt.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || llm::images::generate_image(provider, &key, &asked))
        .await
        .map_err(blocking_failed)?
        .map_err(|_| CommandError::ProviderFailure)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let url = format!("generated:{nanos:x}");
    remember_generated(&url, bytes);
    Ok(ImageCandidateView { url: url.clone(), thumbnail_url: url, title: prompt, source: "generated".into() })
}

/// A candidate's thumbnail as raw bytes: a made one from memory, a found one through the core's guarded
/// fetch (https, public addresses, image bytes only). Nothing is fetched in local-only mode.
#[tauri::command]
pub async fn skin_image_thumbnail(state: State<'_, AppState>, url: String) -> Result<Response, CommandError> {
    if let Some(bytes) = generated(&url) {
        return Ok(Response::new(bytes));
    }
    if state.settings.lock().unwrap().routing().local_only {
        return Err(CommandError::invalid_input("Local-only mode is on, so nothing is downloaded."));
    }
    let bytes = tauri::async_runtime::spawn_blocking(move || llm::images::fetch_image(&url)).await.map_err(blocking_failed)?;
    Ok(Response::new(bytes.map_err(|_| CommandError::ProviderFailure)?))
}

/// Forgets a candidate; nothing was saved, so nothing is on disk.
#[tauri::command]
pub fn skin_image_reject(url: String) {
    take_generated(&url);
}

/// Downloads an approved candidate (a made one is already here) into your skin and places it as the
/// header image. Nothing is downloaded in local-only mode.
#[tauri::command]
pub async fn skin_image_approve(state: State<'_, AppState>, skin_id: String, url: String) -> Result<SkinLine, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    let made = take_generated(&url);
    if made.is_none() && state.settings.lock().unwrap().routing().local_only {
        return Ok(SkinLine { text: "Local-only mode is on, so Whimbrel didn't download that image.".into(), stored: false });
    }
    let id = skin_id.clone();
    let line = tauri::async_runtime::spawn_blocking(move || {
        let (bytes, name) = match made {
            Some(bytes) => (Some(bytes), "generated.png".to_string()),
            None => (llm::images::fetch_image(&url).ok(), url.clone()),
        };
        skin_maker::approve(&user, &id, bytes, &name)
    })
    .await
    .map_err(blocking_failed)??;
    // Only a placed image changed the skin; a refusal ("couldn't be downloaded") changed nothing.
    if skin_maker::placed(&line) {
        queue_sync(&state, Some(skin_id)).await;
    }
    Ok(line)
}

/// Picks an image file (picking it approves it) and places it in your skin; `None` when the picker is cancelled.
#[tauri::command]
pub async fn skin_image_add_file(app: AppHandle, state: State<'_, AppState>, skin_id: String) -> Result<Option<SkinLine>, CommandError> {
    let user = UserSkinStore::new(state.paths.skins.clone());
    if user.manifest(&skin_id).is_none() {
        return Ok(Some(SkinLine { text: skin_maker::NOT_YOURS_LINE.into(), stored: false }));
    }
    let id = skin_id.clone();
    let line = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().set_title("Choose an image for this skin").add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"]);
        if let Some(window) = app.get_webview_window(crate::app::MAIN_WINDOW) {
            dialog = dialog.set_parent(&window);
        }
        let Some(picked) = dialog.blocking_pick_file() else { return Ok(None) };
        let path = picked.into_path().map_err(|e| CommandError::invalid_input(format!("not a file on this PC: {e}")))?;
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        skin_maker::add_own_image(&user, &id, size, || std::fs::read(&path)).map(Some)
    })
    .await
    .map_err(blocking_failed)??;
    if line.as_ref().is_some_and(skin_maker::placed) {
        queue_sync(&state, Some(skin_id)).await;
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn dirs() -> BundledDirs {
        BundledDirs { skins: repo().join("skins"), themes: repo().join("themes") }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-skins-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn id_of(d: &SkinDto) -> &str {
        d.skin["id"].as_str().unwrap()
    }

    #[test]
    fn skins_list_returns_seven_builtins_first() {
        let root = scratch("seven");
        let list = list_skins(&dirs(), &UserSkinStore::new(&root));
        let ids: Vec<&str> = list.iter().map(id_of).collect();
        // Whimbrel, then the others by name, as the Mac's gallery orders them.
        assert_eq!(ids, ["whimbrel", "almanac", "game-day", "high-contrast", "hypermodern", "reference", "retro-terminal"]);
        assert!(list.iter().all(|d| d.built_in));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Writes what `skins_list` answers for the bundled skins, for the browser matrix (ui/e2e), so it draws the
    /// palettes the shell really offers. Run (from ui/): `npm run matrix:skins`; `WHIMBREL_MATRIX_SKINS` overrides the file.
    #[test]
    #[ignore = "writes the browser matrix's fixture"]
    fn dump_bundled_skins_for_the_matrix() {
        let path = std::env::var("WHIMBREL_MATRIX_SKINS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/e2e/.generated/skins.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let root = scratch("matrix");
        let list = list_skins(&dirs(), &UserSkinStore::new(&root));
        std::fs::write(path, serde_json::to_string(&list).unwrap()).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whimbrel_offers_its_three_palettes_in_order() {
        let root = scratch("whimbrel");
        let list = list_skins(&dirs(), &UserSkinStore::new(&root));
        let ids: Vec<&str> = list[0].palettes.iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["default", "slate", "sepia"]);
        // Both variants, all 15 colours.
        assert_eq!(list[0].palettes[0]["light"].as_object().unwrap().len(), 15);
        assert_eq!(list[0].palettes[0]["dark"].as_object().unwrap().len(), 15);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_builtin_palette_resolves_and_passes_its_layout() {
        let root = scratch("pairs");
        for d in list_skins(&dirs(), &UserSkinStore::new(&root)) {
            let skin: Skin = serde_json::from_value(d.skin.clone()).unwrap();
            assert!(!d.palettes.is_empty(), "{} has no palettes", skin.id);
            for p in &d.palettes {
                let parsed = skins::parse_palette(&serde_json::to_vec(p).unwrap(), &default_palette()).unwrap();
                assert!(archetype_repair(&parsed, &parsed.id, skin.today.archetype).is_none(), "{} / {}", skin.id, parsed.id);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_skin_naming_no_known_palette_offers_default() {
        let mut skin = skins::whimbrel_default();
        skin.palettes = vec!["nope".into()];
        let offered = offered(&skin, &[], &[default_palette()]);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0]["id"], "default");
    }

    #[test]
    fn a_failing_palette_is_repaired_for_the_layout_and_keeps_its_identity() {
        let mut skin = skins::whimbrel_default();
        skin.palettes = vec!["pale".into()];
        let mut pale = default_palette();
        pale.id = "pale".into();
        pale.name = "Pale".into();
        pale.author = "Someone".into();
        let text = pale.light.get(skins::Token::TextPrimary).unwrap();
        let page = pale.light.get(skins::Token::BackgroundPage).unwrap();
        assert_ne!(text, page);
        pale.light.set(skins::Token::TextPrimary, page); // unreadable on purpose
        let out = offered(&skin, &[pale.clone()], &[]);
        assert_eq!((out[0]["id"].as_str(), out[0]["name"].as_str(), out[0]["author"].as_str()), (Some("pale"), Some("Pale"), Some("Someone")));
        let fixed = skins::parse_palette(&serde_json::to_vec(&out[0]).unwrap(), &default_palette()).unwrap();
        assert!(archetype_repair(&fixed, "pale", skin.today.archetype).is_none());
        assert_ne!(fixed.light.get(skins::Token::TextPrimary), Some(page));
    }

    #[test]
    fn user_skins_follow_the_builtins_with_their_own_palettes() {
        let root = scratch("user");
        let store = UserSkinStore::new(&root);
        let id = store.new_skin_id("mine");
        let mut skin = skins::whimbrel_default();
        skin.id = id.clone();
        skin.name = "Mine".into();
        skin.palettes = vec!["slate".into()];
        store.save(&skin, &[], &[]).unwrap();
        let list = list_skins(&dirs(), &store);
        assert_eq!(list.len(), 8);
        let mine = list.last().unwrap();
        assert_eq!((id_of(mine), mine.built_in), (id.as_str(), false));
        // "slate" is a plain theme, found in the shared pool.
        assert_eq!(mine.palettes[0]["id"], "slate");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skin_image_returns_an_approved_images_bytes() {
        let root = scratch("bytes");
        let store = UserSkinStore::new(&root);
        let mut skin = skins::whimbrel_default();
        skin.id = "user-pic".into();
        skin.name = "Pic".into();
        store.save(&skin, &[], &[("logo.png".into(), vec![1, 2, 3])]).unwrap();
        assert_eq!(read_skin_image(&store, "user-pic", "logo.png").unwrap(), vec![1, 2, 3]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_background_image_gets_the_worst_scrim_of_its_palettes() {
        let root = scratch("scrim");
        let store = UserSkinStore::new(&root);
        let mut skin = skins::whimbrel_default();
        skin.id = "user-pic".into();
        skin.name = "Pic".into();
        skin.palettes = vec!["default".into()];
        let white = {
            let img = image::RgbImage::from_pixel(40, 30, image::Rgb([255, 255, 255]));
            let mut out = Vec::new();
            image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        for (asset, placement) in [("bg.png", Placement::Background), ("top.png", Placement::Header), ("gone.png", Placement::Background)] {
            skin.decoration.imagery.push(skins::format::Image { asset: asset.into(), placement, text_free: false });
        }
        store.save(&skin, &[], &[("bg.png".into(), white.clone()), ("top.png".into(), white)]).unwrap();
        let list = list_skins(&dirs(), &store);
        let mine = list.last().unwrap();
        // Only the background is measured, and a missing file is left for the page to cover fully.
        assert_eq!(mine.image_scrims.keys().collect::<Vec<_>>(), ["bg.png"]);
        // White under Default's dark variant (light text) needs a strong scrim; under its light variant, little.
        let palette = &offered_palettes(&skin, &[], &palette_pool(&dirs()))[0];
        let sample = scrim_sample(&store.image_path("user-pic", "bg.png").unwrap()).unwrap();
        let (light, dark) = (scrim_for(&sample, &palette.light, skin.today.archetype), scrim_for(&sample, &palette.dark, skin.today.archetype));
        assert!(dark > light, "{light} {dark}");
        assert_eq!(mine.image_scrims["bg.png"], dark.max(light));
        assert!(list.iter().filter(|d| d.built_in).all(|d| d.image_scrims.is_empty()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skin_image_refuses_names_outside_the_skin_folder() {
        let root = scratch("image");
        let store = UserSkinStore::new(&root);
        for (id, name) in [("user-a", "../x.png"), ("..", "x.png"), ("user-a", "missing.png"), ("whimbrel", "x.png")] {
            assert!(read_skin_image(&store, id, name).is_err(), "{id}/{name}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
