//! The redesign switch's one effect on the bundled lists, now none (docs/ui-redesign/screens/switch-removal.md, A2):
//! the built-in Thread skin and its palette ship in `skins/` like any other and every list shows them. The
//! functions and the `redesign` argument stay so no caller changes; the clean-up (PR B) removes them with the switch.

use crate::format::Skin;
use crate::palette::Palette;

/// Bundled skins that were only shown behind the redesign flag (kept for the clean-up; nothing hides them now).
pub const REDESIGN_SKIN_IDS: &[&str] = &["thread"];
/// Bundled palettes that were only shown behind the redesign flag (kept for the clean-up).
pub const REDESIGN_PALETTE_IDS: &[&str] = &["thread"];

/// Whether `id` names a skin that used to be shown only with the redesign flag on.
pub fn is_redesign_skin(id: &str) -> bool {
    REDESIGN_SKIN_IDS.contains(&id)
}

/// `skins` as a list shows them: all of them. `redesign` is no longer read.
pub fn visible_skins(skins: Vec<Skin>, _redesign: bool) -> Vec<Skin> {
    skins
}

/// `palettes` as a list shows them: all of them. `redesign` is no longer read.
pub fn visible_palettes(palettes: Vec<Palette>, _redesign: bool) -> Vec<Palette> {
    palettes
}
