//! The hidden redesign flag's one effect on the bundled lists (docs/ui-redesign/skins.md): the built-in
//! Thread skin and its palette ship in `skins/` like any other, but no list shows them unless the flag
//! is on. Loading, repair and storage never look at the flag. The flag itself is read by each app
//! (Mac: `com.mpointer.whimbrel.redesign.enabled` in defaults; Windows: `redesign.enabled` in settings.json),
//! off unless set to true by hand.

use crate::format::Skin;
use crate::palette::Palette;

/// Bundled skins that exist only behind the redesign flag.
pub const REDESIGN_SKIN_IDS: &[&str] = &["thread"];
/// Bundled palettes that exist only behind the redesign flag.
pub const REDESIGN_PALETTE_IDS: &[&str] = &["thread"];

/// Whether `id` names a skin that only the redesign flag shows.
pub fn is_redesign_skin(id: &str) -> bool {
    REDESIGN_SKIN_IDS.contains(&id)
}

/// `skins` as a list shows them: without the redesign skins unless `redesign` is on.
pub fn visible_skins(skins: Vec<Skin>, redesign: bool) -> Vec<Skin> {
    if redesign {
        return skins;
    }
    skins.into_iter().filter(|s| !is_redesign_skin(&s.id)).collect()
}

/// `palettes` as a list shows them: without the redesign palettes unless `redesign` is on.
pub fn visible_palettes(palettes: Vec<Palette>, redesign: bool) -> Vec<Palette> {
    if redesign {
        return palettes;
    }
    palettes.into_iter().filter(|p| !REDESIGN_PALETTE_IDS.contains(&p.id.as_str())).collect()
}
