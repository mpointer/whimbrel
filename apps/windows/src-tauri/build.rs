use std::path::PathBuf;

fn main() {
    // Sync configuration (plan Decision 6): the gitignored Secrets.toml, or nothing, is
    // copied into OUT_DIR, where src/syncing/config.rs embeds and parses it. A missing
    // file builds fine and leaves sync "not configured".
    println!("cargo:rerun-if-changed=Secrets.toml");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("secrets.toml");
    let text = std::fs::read_to_string("Secrets.toml").unwrap_or_default();
    std::fs::write(out, text).expect("write OUT_DIR/secrets.toml");

    tauri_build::build()
}
