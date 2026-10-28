//! Sync configuration: the Worker URL and the Google OAuth client (plan Decision 6).
//!
//! `build.rs` copies the gitignored `Secrets.toml` into the build output, and this
//! module embeds and parses it. The format is the three-line subset of TOML that
//! `Secrets.toml.example` shows. Anything missing leaves sync "not configured": the rest
//! of the app works, and Settings → Sync says which value is absent.

const EMBEDDED: &str = include_str!(concat!(env!("OUT_DIR"), "/secrets.toml"));

pub const KEY_WORKER_URL: &str = "sync_worker_base_url";
pub const KEY_CLIENT_ID: &str = "google_oauth_client_id";
pub const KEY_CLIENT_SECRET: &str = "google_oauth_client_secret";

#[derive(Clone, PartialEq, Eq)]
pub struct SyncConfig {
    pub worker_base_url: String,
    pub google_client_id: String,
    pub google_client_secret: String,
}

impl std::fmt::Debug for SyncConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncConfig")
            .field("worker_base_url", &self.worker_base_url)
            .field("google_client_id", &self.google_client_id)
            .finish_non_exhaustive()
    }
}

impl SyncConfig {
    /// The configuration this build was compiled with.
    pub fn embedded() -> Result<Self, String> {
        parse(EMBEDDED)
    }

    /// Google sign-in for sync: an ID token is all the Worker needs.
    pub fn google_provider(&self) -> api::oauth::ProviderConfig {
        api::oauth::ProviderConfig {
            provider: "google".into(),
            client_id: self.google_client_id.clone(),
            client_secret: Some(self.google_client_secret.clone()),
            scope: "openid email".into(),
            auth_endpoint: None,
            token_endpoint: None,
        }
    }
}

/// `key = "value"` lines; `#` comments and blank lines are skipped. Errors name every
/// missing or empty key so Settings can say exactly what to add.
pub fn parse(text: &str) -> Result<SyncConfig, String> {
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
            values.insert(key.trim().to_string(), value.replace("\\\"", "\"").replace("\\\\", "\\"));
        }
    }
    let get = |key: &str| values.get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let (url, id, secret) = (get(KEY_WORKER_URL), get(KEY_CLIENT_ID), get(KEY_CLIENT_SECRET));
    let missing: Vec<&str> = [(KEY_WORKER_URL, &url), (KEY_CLIENT_ID, &id), (KEY_CLIENT_SECRET, &secret)]
        .into_iter()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Sync is not configured: apps/windows/src-tauri/Secrets.toml is missing {} (see Secrets.toml.example), then rebuild.",
            missing.join(", ")
        ));
    }
    Ok(SyncConfig { worker_base_url: url.unwrap(), google_client_id: id.unwrap(), google_client_secret: secret.unwrap() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_example_format() {
        let cfg = parse(
            "# comment\n\nsync_worker_base_url = \"https://w.example.dev\"\ngoogle_oauth_client_id = \"id.apps.googleusercontent.com\"\ngoogle_oauth_client_secret = \"GOCSPX-abc\"\n",
        )
        .unwrap();
        assert_eq!(cfg.worker_base_url, "https://w.example.dev");
        assert_eq!(cfg.google_client_id, "id.apps.googleusercontent.com");
        assert_eq!(cfg.google_client_secret, "GOCSPX-abc");
    }

    #[test]
    fn an_empty_or_missing_file_names_every_missing_key() {
        let err = parse("").unwrap_err();
        for key in [KEY_WORKER_URL, KEY_CLIENT_ID, KEY_CLIENT_SECRET] {
            assert!(err.contains(key), "{err}");
        }
        let err = parse("sync_worker_base_url = \"https://w\"\ngoogle_oauth_client_id = \"\"").unwrap_err();
        assert!(err.contains(KEY_CLIENT_ID) && err.contains(KEY_CLIENT_SECRET) && !err.contains(KEY_WORKER_URL), "{err}");
    }

    #[test]
    fn the_google_provider_asks_only_for_an_id_token() {
        let cfg = parse("sync_worker_base_url = \"u\"\ngoogle_oauth_client_id = \"i\"\ngoogle_oauth_client_secret = \"s\"").unwrap();
        let p = cfg.google_provider();
        assert_eq!((p.provider.as_str(), p.scope.as_str(), p.client_secret.as_deref()), ("google", "openid email", Some("s")));
    }

    /// Opt-in check that this build picked up a complete `Secrets.toml`:
    /// `cargo test --release --lib -- --ignored this_build_embeds`.
    #[test]
    #[ignore = "needs a real, gitignored Secrets.toml"]
    fn this_build_embeds_a_complete_config() {
        let cfg = SyncConfig::embedded().expect("Secrets.toml complete");
        assert!(cfg.worker_base_url.starts_with("https://"), "{cfg:?}");
        assert!(cfg.google_client_id.ends_with(".apps.googleusercontent.com"), "{cfg:?}");
    }

    #[test]
    fn debug_output_omits_the_client_secret() {
        let cfg = parse("sync_worker_base_url = \"u\"\ngoogle_oauth_client_id = \"i\"\ngoogle_oauth_client_secret = \"GOCSPX-secret\"").unwrap();
        assert!(!format!("{cfg:?}").contains("GOCSPX-secret"));
    }
}
