//! Skins: the format, loader and repair rules shared by the Windows app and (for sync) the Mac.
//! Ports the Mac's Swift in `apps/macos/Whimbrel/Support/Skin*.swift`; `skins/testdata/` keeps the
//! two in agreement.

pub mod catalogue;
pub mod contrast;
pub mod format;
pub mod history;
pub mod icons;
pub mod loader;
pub mod package;
pub mod palette;
pub mod repair;
pub mod scrim;
pub mod store;

pub use catalogue::catalogue;
pub use format::{is_valid_id, whimbrel_default, Skin};
pub use icons::{icon_table, is_allowed_icon, IconEntry};
pub use loader::{load_bundled, parse_skin};
pub use palette::{default_palette, load_palettes, parse_palette, Palette, Token, Variant};
pub use repair::{all_pairs, archetype_pairs, repair, Pair};
pub use scrim::{blend, extremes, scrim_opacity, Glow};
pub use store::{save_gate_repair, skin_name, UserSkinStore};
