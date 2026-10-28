//! Skins for the UI (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md, Task 6): the seven
//! built-in skins and the user's own, each with the palettes it offers, and the images a skin
//! approved. The rules live in `core/skins`; this only gathers the files and hands them to the page.
//!
//! The built-ins ship as Tauri resources: `skins/*.json`, `skins/palettes/*.json` and `themes/*.json`
//! (the plain themes are the Whimbrel skin's palettes, as on the Mac).

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use skins::palette::Palette;
use skins::store::archetype_repair;
use skins::{default_palette, load_bundled, load_palettes, Skin, UserSkinStore};
use tauri::{ipc::Response, AppHandle, Manager, State};

use crate::error::CommandError;
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

/// The palettes of `skin`, in its manifest order, found among `own` then `pool`; Default when none is.
fn offered(skin: &Skin, own: &[Palette], pool: &[Palette]) -> Vec<Value> {
    let mut found: Vec<&Palette> =
        skin.palettes.iter().filter_map(|id| own.iter().chain(pool).find(|p| &p.id == id)).collect();
    let fallback = default_palette();
    if found.is_empty() {
        found.push(pool.iter().find(|p| p.id == fallback.id).unwrap_or(&fallback));
    }
    found.into_iter().map(|p| gated(p, skin).to_json()).collect()
}

fn dto(skin: &Skin, palettes: Vec<Value>, built_in: bool) -> SkinDto {
    SkinDto { skin: serde_json::to_value(skin).expect("a skin serialises"), palettes, built_in }
}

/// Every skin: the built-ins first (Whimbrel, then by name), then the user's by name.
pub fn list_skins(dirs: &BundledDirs, user: &UserSkinStore) -> Vec<SkinDto> {
    let fallback = default_palette();
    // Plain themes first, then the bundled skins' own palettes; the first of an id wins.
    let mut pool = load_palettes(&dirs.themes, &fallback);
    for p in load_palettes(&dirs.skins.join("palettes"), &fallback) {
        if !pool.iter().any(|q| q.id == p.id) {
            pool.push(p);
        }
    }
    let mut out: Vec<SkinDto> = load_bundled(&dirs.skins).iter().map(|s| dto(s, offered(s, &[], &pool), true)).collect();
    for (skin, own) in user.list() {
        out.push(dto(&skin, offered(&skin, &own, &pool), false));
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
    fn skin_image_refuses_names_outside_the_skin_folder() {
        let root = scratch("image");
        let store = UserSkinStore::new(&root);
        for (id, name) in [("user-a", "../x.png"), ("..", "x.png"), ("user-a", "missing.png"), ("whimbrel", "x.png")] {
            assert!(read_skin_image(&store, id, name).is_err(), "{id}/{name}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
