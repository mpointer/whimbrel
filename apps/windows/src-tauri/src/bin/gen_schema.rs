//! Writes the command DTOs' JSON Schema bundle to `ui/src/generated/schema.json`.
//! Run from `apps/windows/src-tauri`: `cargo run --bin gen-schema`.

fn main() {
    use whimbrel_windows::commands::schema::{schema_bundle_text, SCHEMA_PATH};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_PATH);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create ui/src/generated");
    std::fs::write(&path, schema_bundle_text()).expect("write schema.json");
    println!("wrote {}", path.display());
}
