//! Skins: the format, loader and repair rules shared by the Windows app and (for sync) the Mac.
//! Ports the Mac's Swift in `apps/macos/Whimbrel/Support/Skin*.swift`; `skins/testdata/` keeps the
//! two in agreement.

pub mod catalogue;
pub mod contrast;
pub mod elements;
pub mod format;
pub mod history;
pub mod icons;
pub mod images;
pub mod loader;
pub mod package;
pub mod palette;
pub mod redesign;
pub mod repair;
pub mod scrim;
pub mod store;
pub mod sync;

pub use catalogue::{catalogue, catalogue_for};
pub use format::{is_valid_id, whimbrel_default, Skin};
pub use icons::{icon_table, is_allowed_icon, IconEntry};
pub use loader::{load_bundled, parse_skin};
pub use palette::{default_palette, load_palettes, parse_palette, Palette, Token, Variant};
pub use redesign::{is_redesign_skin, visible_palettes, visible_skins, REDESIGN_PALETTE_IDS, REDESIGN_SKIN_IDS};
pub use elements::ElementChoices;
pub use repair::{all_pairs, archetype_pairs, element_pairs, redesign_pairs, repair, repair_for, Pair};
pub use scrim::{blend, extremes, scrim_opacity, Glow};
pub use store::{save_gate_repair, skin_name, UserSkinStore};
