//! Whimbrel AI on Windows (plan `docs/superpowers/plans/2026-11-10-whimbrel-ai-credits.md`, Windows
//! hand-off): the sync account's Google ID token and this PC's device key, cached for
//! `RoutingSettings::routing_config`, and the timing rules that keep them fresh. The Mac's
//! `SyncState.whimbrelCredentials`.
//!
//! The cache is process-wide because `ApiKeys::load`, which every AI call site already makes, is
//! where routing reads it: background jobs (routines, titles, summaries) get the credentials with
//! no new plumbing. The sync controller fills it while sync is ready and empties it otherwise.
//! The token is never logged and never `Debug`-printed.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The priced model ids, in the Mac's order (`RoutingSettings.whimbrelModels`). Whimbrel's server
/// answers any other id with 400 `unknown_model`. Keep in step with `AI_PRICES` in
/// `server/src/aiProxy.ts`.
pub const MODELS: [&str; 10] = [
    "claude-haiku-4-5",
    "claude-haiku-4-5-20251001",
    "claude-sonnet-4-6",
    "claude-sonnet-5",
    "claude-opus-4-6",
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-opus-5",
    "claude-fable-5",
    "claude-fable-5-1",
];

/// The credentials are re-read at least this often while sync is ready.
pub const MAX_AGE: Duration = Duration::from_secs(30 * 60);
/// An Ask reuses the cached token only with more than this left.
pub const ASK_MIN_VALIDITY: Duration = Duration::from_secs(5 * 60);
/// The background refresher replaces the token once less than this is left, so background calls,
/// which read the cache as it is, never send a dead one.
pub const BACKGROUND_MIN_VALIDITY: Duration = Duration::from_secs(10 * 60);

/// What `api::RoutingConfig` carries for Whimbrel AI.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub id_token: String,
    pub device_key: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials { .. }")
    }
}

struct Cached {
    credentials: Credentials,
    fetched_at: Instant,
    /// The ID token's own `exp`, in unix ms; `None` when it has none, which counts as about to expire.
    expires_at_ms: Option<i64>,
}

/// The sync controller's credentials. The app installs its one cache with [`install`] so
/// `ApiKeys::load` can read it; tests use their own.
#[derive(Default)]
pub struct Cache(Mutex<Option<Cached>>);

impl Cache {
    pub fn get(&self) -> Option<Credentials> {
        self.0.lock().unwrap().as_ref().map(|c| c.credentials.clone())
    }

    /// Stores freshly read credentials.
    pub fn store(&self, credentials: Credentials, now: Instant) {
        let expires_at_ms = expiry_ms(&credentials.id_token);
        *self.0.lock().unwrap() = Some(Cached { credentials, fetched_at: now, expires_at_ms });
    }

    /// Forgets them: sync left ready.
    pub fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }

    pub fn needs_background_refresh(&self, now: Instant, now_ms: i64) -> bool {
        let cache = self.0.lock().unwrap();
        background_refresh_due(cache.as_ref().map(|c| c.fetched_at), cache.as_ref().and_then(|c| c.expires_at_ms), now, now_ms)
    }

    pub fn needs_ask_refresh(&self, now_ms: i64) -> bool {
        let cache = self.0.lock().unwrap();
        ask_refresh_due(cache.is_some(), cache.as_ref().and_then(|c| c.expires_at_ms), now_ms)
    }
}

static INSTALLED: OnceLock<Arc<Cache>> = OnceLock::new();

/// Makes `cache` the one [`cached`] reads. The first install wins.
pub fn install(cache: Arc<Cache>) {
    let _ = INSTALLED.set(cache);
}

/// The installed cache's credentials: set while sync is ready, else None.
pub fn cached() -> Option<Credentials> {
    INSTALLED.get().and_then(|c| c.get())
}

/// A JWT's `exp` claim in unix ms: an unverified read, used only to know when to replace it.
pub fn expiry_ms(jwt: &str) -> Option<i64> {
    let exp = api::oauth::jwt_claims(jwt)?.get("exp")?.as_f64()?;
    Some((exp * 1000.0) as i64)
}

/// Whether `id_token` has more than `min_validity` left at `now_ms`.
pub fn token_good_for(id_token: &str, min_validity: Duration, now_ms: i64) -> bool {
    expiry_ms(id_token).is_some_and(|exp| exp - now_ms > min_validity.as_millis() as i64)
}

/// Time left on the token; 0 when `exp` is unknown.
fn ms_left(expires_at_ms: Option<i64>, now_ms: i64) -> i64 {
    expires_at_ms.map_or(0, |exp| exp - now_ms)
}

/// Whether the upkeep tick should re-read the credentials: none cached, older than `MAX_AGE`, or
/// less than `BACKGROUND_MIN_VALIDITY` left on the token.
pub fn background_refresh_due(fetched_at: Option<Instant>, expires_at_ms: Option<i64>, now: Instant, now_ms: i64) -> bool {
    let Some(at) = fetched_at else { return true };
    now.duration_since(at) >= MAX_AGE || ms_left(expires_at_ms, now_ms) < BACKGROUND_MIN_VALIDITY.as_millis() as i64
}

/// Whether an Ask bound for Whimbrel AI must refresh first: none cached, or `ASK_MIN_VALIDITY` or
/// less left.
pub fn ask_refresh_due(cached: bool, expires_at_ms: Option<i64>, now_ms: i64) -> bool {
    !cached || ms_left(expires_at_ms, now_ms) <= ASK_MIN_VALIDITY.as_millis() as i64
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::Engine;

    /// An unsigned JWT whose payload carries `exp` (seconds).
    pub(crate) fn jwt(exp_secs: i64) -> String {
        let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!("{}.{}.sig", enc(r#"{"alg":"none"}"#), enc(&format!(r#"{{"exp":{exp_secs}}}"#)))
    }

    const NOW_MS: i64 = 1_800_000_000_000;

    #[test]
    fn the_expiry_comes_from_the_tokens_own_exp() {
        assert_eq!(expiry_ms(&jwt(1_800_000_600)), Some(1_800_000_600_000));
        assert_eq!(expiry_ms("not-a-jwt"), None);
        // Six minutes left is good for an Ask (five) but not for the background (ten).
        let t = jwt(NOW_MS / 1000 + 6 * 60);
        assert!(token_good_for(&t, ASK_MIN_VALIDITY, NOW_MS));
        assert!(!token_good_for(&t, BACKGROUND_MIN_VALIDITY, NOW_MS));
        assert!(!token_good_for("not-a-jwt", ASK_MIN_VALIDITY, NOW_MS));
    }

    #[test]
    fn the_background_refreshes_when_empty_old_or_nearly_expired() {
        let now = Instant::now();
        let hour = Some(NOW_MS + 60 * 60 * 1000);
        assert!(background_refresh_due(None, None, now, NOW_MS));
        assert!(!background_refresh_due(Some(now), hour, now, NOW_MS));
        assert!(background_refresh_due(Some(now), hour, now + MAX_AGE, NOW_MS));
        assert!(background_refresh_due(Some(now), Some(NOW_MS + 9 * 60 * 1000), now, NOW_MS));
        // An unknown `exp` counts as about to expire.
        assert!(background_refresh_due(Some(now), None, now, NOW_MS));
    }

    #[test]
    fn an_ask_refreshes_with_five_minutes_or_less_left() {
        assert!(ask_refresh_due(false, Some(NOW_MS + 60 * 60 * 1000), NOW_MS));
        assert!(!ask_refresh_due(true, Some(NOW_MS + 6 * 60 * 1000), NOW_MS));
        assert!(ask_refresh_due(true, Some(NOW_MS + 5 * 60 * 1000), NOW_MS));
        assert!(ask_refresh_due(true, None, NOW_MS));
    }

    #[test]
    fn credentials_never_print_the_token() {
        let c = Credentials { id_token: "SECRET".into(), device_key: "pk".into() };
        assert!(!format!("{c:?}").contains("SECRET"));
    }

    #[test]
    fn the_models_match_the_servers_price_table() {
        let prices = include_str!("../../../../server/src/aiProxy.ts");
        let table = &prices[prices.find("AI_PRICES").unwrap()..];
        let table = &table[..table.find("};").unwrap()];
        let priced: Vec<&str> = table.lines().filter_map(|l| l.trim().strip_prefix('"')?.split('"').next()).collect();
        assert_eq!(priced, MODELS.to_vec());
    }
}
