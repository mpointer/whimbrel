//! OAuth 2.0 Authorization Code + PKCE over a loopback redirect (RFC 8252), moved from
//! apps/macos `OAuthLoopbackFlow.swift` / `GoogleOAuthHelper.swift` /
//! `MicrosoftOAuthHelper.swift` (M3.0 part 2, Task 10). No `Store` anywhere. The shell
//! opens the consent URL in its browser of choice, stores the returned tokens in its
//! secret store, and supplies the client id/secret; this module owns PKCE, state, the
//! loopback listener, the code exchange, refresh, and the pure scope/JWT helpers.
//!
//! Security properties pinned by the tests below: the listener binds 127.0.0.1 only,
//! a redirect whose `state` does not match is rejected (the wait ends in
//! [`OAuthError::StateMismatch`]), and only a matching redirect yields a code.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use sha2::Digest;

/// What the shell passes to [`begin`] / [`refresh_stored`]: the provider, its client
/// credentials and the scope. Endpoint overrides are a test-only hook.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderConfig {
    /// `"google"` or `"microsoft"`.
    pub provider: String,
    pub client_id: String,
    /// Google's installed-app clients still require the secret at exchange/refresh
    /// (confirmed live); Microsoft's public clients send none.
    #[serde(default)]
    pub client_secret: Option<String>,
    pub scope: String,
    #[serde(default)]
    pub auth_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OAuthConfig {
    auth_endpoint: String,
    token_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
    scope: String,
    /// The hostname advertised in `redirect_uri`. The socket always binds the literal
    /// 127.0.0.1; Microsoft's public-client wildcard-port matching needs "localhost".
    redirect_host: &'static str,
    extra_auth_params: Vec<(&'static str, &'static str)>,
    /// Microsoft wants `scope` on token requests; Google does not.
    scope_in_token_requests: bool,
}

impl OAuthConfig {
    fn from_provider(p: &ProviderConfig) -> Result<Self, OAuthError> {
        let (auth, token, host, extra, scope_in_token) = match p.provider.as_str() {
            "google" => (
                "https://accounts.google.com/o/oauth2/v2/auth",
                "https://oauth2.googleapis.com/token",
                "127.0.0.1",
                vec![("access_type", "offline"), ("prompt", "consent")],
                false,
            ),
            "microsoft" => (
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                "localhost",
                vec![("prompt", "select_account")],
                true,
            ),
            other => return Err(OAuthError::UnknownProvider(other.to_string())),
        };
        Ok(OAuthConfig {
            auth_endpoint: p.auth_endpoint.clone().unwrap_or_else(|| auth.to_string()),
            token_endpoint: p.token_endpoint.clone().unwrap_or_else(|| token.to_string()),
            client_id: p.client_id.clone(),
            client_secret: p.client_secret.clone(),
            scope: p.scope.clone(),
            redirect_host: host,
            extra_auth_params: extra,
            scope_in_token_requests: scope_in_token,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum OAuthError {
    UnknownProvider(String),
    ListenerFailed,
    StateMismatch,
    TimedOut,
    AuthorizationDenied { reason: String },
    TokenExchangeFailed,
    TokenRefreshFailed,
    /// The provider rejected the refresh token itself (HTTP 400 `invalid_grant`:
    /// revoked or expired). Only a new sign-in fixes it, unlike a network blip.
    RefreshTokenRevoked,
    NoRefreshTokenReturned,
    Cancelled,
    UnknownSession,
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// A provider token response (exchange or refresh).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    pub expires_in: i64,
    /// The scopes the provider ACTUALLY granted (Google returns this; not guaranteed).
    #[serde(default)]
    pub scope: Option<String>,
}

/// The blob the shell keeps in its secret store. Field names are the ones the macOS
/// Keychain items already carry, so existing entries decode unchanged.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: Option<String>,
    pub expires_at_unix_ms: i64,
    /// The granted (not requested) scope, when known. `None` for entries written
    /// before it was recorded; a write action resolves that by forcing one refresh.
    #[serde(default)]
    pub granted_scope: Option<String>,
}

/// Treat a token as due for refresh once it's within a minute of actual expiry.
pub const TOKEN_EXPIRY_LEEWAY_MS: i64 = 60_000;

impl StoredTokens {
    pub fn from_exchange(response: TokenResponse, now_unix_ms: i64) -> Result<Self, OAuthError> {
        let refresh_token = response.refresh_token.ok_or(OAuthError::NoRefreshTokenReturned)?;
        Ok(StoredTokens {
            access_token: response.access_token,
            refresh_token,
            id_token: response.id_token,
            expires_at_unix_ms: now_unix_ms + response.expires_in * 1000,
            granted_scope: response.scope,
        })
    }

    /// Providers don't always rotate the refresh token, id token or echo the scope on
    /// a refresh: keep what we had unless told otherwise.
    pub fn applying_refresh(&self, response: TokenResponse, now_unix_ms: i64) -> Self {
        StoredTokens {
            access_token: response.access_token,
            refresh_token: response.refresh_token.unwrap_or_else(|| self.refresh_token.clone()),
            id_token: response.id_token.or_else(|| self.id_token.clone()),
            expires_at_unix_ms: now_unix_ms + response.expires_in * 1000,
            granted_scope: response.scope.or_else(|| self.granted_scope.clone()),
        }
    }

    pub fn is_fresh(&self, now_unix_ms: i64) -> bool {
        now_unix_ms < self.expires_at_unix_ms - TOKEN_EXPIRY_LEEWAY_MS
    }
}

// ---- PKCE ----

fn random_url_safe(byte_count: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; byte_count];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn generate_code_verifier() -> String {
    random_url_safe(32)
}

pub fn generate_state() -> String {
    random_url_safe(16)
}

/// RFC 7636 S256: base64url(SHA-256(verifier)), unpadded.
pub fn code_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

// ---- Session registry ----

struct Session {
    config: OAuthConfig,
    listener: Mutex<Option<TcpListener>>,
    code_verifier: String,
    state: String,
    redirect_uri: String,
    cancel: AtomicBool,
}

static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn sessions() -> &'static Mutex<HashMap<String, Arc<Session>>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What [`begin`] hands the shell: open `auth_url` in a browser, then call [`wait`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BegunSession {
    pub session_id: String,
    pub auth_url: String,
    pub redirect_uri: String,
}

/// Binds an ephemeral port on the loopback interface only and builds the consent URL
/// (PKCE S256 challenge, fresh `state`, the provider's extra parameters).
pub fn begin(provider: &ProviderConfig) -> Result<BegunSession, OAuthError> {
    let config = OAuthConfig::from_provider(provider)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| OAuthError::ListenerFailed)?;
    let port = listener.local_addr().map_err(|_| OAuthError::ListenerFailed)?.port();
    listener.set_nonblocking(true).map_err(|_| OAuthError::ListenerFailed)?;
    let redirect_uri = format!("http://{}:{port}/", config.redirect_host);
    let code_verifier = generate_code_verifier();
    let state = generate_state();

    let mut params: Vec<(&str, String)> = vec![
        ("client_id", config.client_id.clone()),
        ("redirect_uri", redirect_uri.clone()),
        ("response_type", "code".into()),
        ("scope", config.scope.clone()),
        ("code_challenge", code_challenge(&code_verifier)),
        ("code_challenge_method", "S256".into()),
    ];
    params.extend(config.extra_auth_params.iter().map(|(k, v)| (*k, v.to_string())));
    params.push(("state", state.clone()));
    let auth_url = url::Url::parse_with_params(&config.auth_endpoint, params.iter().map(|(k, v)| (*k, v.as_str())))
        .map_err(|_| OAuthError::ListenerFailed)?
        .to_string();

    let session_id = format!("oauth-{}-{}", std::process::id(), SESSION_COUNTER.fetch_add(1, Ordering::Relaxed));
    let session = Arc::new(Session {
        config,
        listener: Mutex::new(Some(listener)),
        code_verifier,
        state,
        redirect_uri: redirect_uri.clone(),
        cancel: AtomicBool::new(false),
    });
    sessions().lock().unwrap().insert(session_id.clone(), session);
    Ok(BegunSession { session_id, auth_url, redirect_uri })
}

/// Requests that a pending [`wait`] return [`OAuthError::Cancelled`] (the shell's auth
/// window was closed). Returns whether the session was known.
pub fn cancel(session_id: &str) -> bool {
    match sessions().lock().unwrap().get(session_id) {
        Some(s) => {
            s.cancel.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// **Blocking.** Waits up to `timeout_ms` for the provider's redirect, ignoring
/// incidental connections (browser preconnects, `/favicon.ico`), rejects a mismatched
/// `state`, serves the "you can close this window" page, exchanges the code for tokens
/// and returns them with `now_unix_ms` stamped into `expires_at_unix_ms`. The session
/// is consumed whatever the outcome. Never call this on the store queue or the UI
/// thread.
pub fn wait(session_id: &str, timeout_ms: u64, now_unix_ms: i64) -> Result<StoredTokens, OAuthError> {
    // The session stays registered while we block so a concurrent `cancel` (the shell's
    // auth window closed) can still find it; it is removed on every exit path below.
    let session = sessions().lock().unwrap().get(session_id).cloned().ok_or(OAuthError::UnknownSession)?;
    let listener = session.listener.lock().unwrap().take().ok_or(OAuthError::UnknownSession)?;
    let result = wait_for_code(&listener, &session, Duration::from_millis(timeout_ms));
    drop(listener);
    sessions().lock().unwrap().remove(session_id);
    let code = result?;
    let response = exchange_code(&session.config, &code, &session.redirect_uri, &session.code_verifier)?;
    StoredTokens::from_exchange(response, now_unix_ms)
}

fn wait_for_code(listener: &TcpListener, session: &Session, timeout: Duration) -> Result<String, OAuthError> {
    let deadline = Instant::now() + timeout;
    loop {
        if session.cancel.load(Ordering::Relaxed) {
            return Err(OAuthError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(OAuthError::TimedOut);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(outcome) = handle_connection(stream, &session.state) {
                    return outcome;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return Err(OAuthError::ListenerFailed),
        }
    }
}

/// `None` means "not this attempt's redirect, keep waiting".
fn handle_connection(mut stream: TcpStream, expected_state: &str) -> Option<Result<String, OAuthError>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).ok()?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request.lines().next().and_then(|l| l.split(' ').nth(1));
    let Some(components) = path.and_then(|p| url::Url::parse(&format!("http://127.0.0.1{p}")).ok()) else {
        respond(&mut stream, "400 Bad Request", "Bad request");
        return None;
    };
    let query: HashMap<String, String> = components.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let received_state = query.get("state").map(String::as_str);
    let Some(code) = query.get("code") else {
        return match query.get("error") {
            Some(error) if received_state == Some(expected_state) => {
                respond(&mut stream, "400 Bad Request", &format!("Authorization failed: {error}"));
                Some(Err(OAuthError::AuthorizationDenied { reason: error.clone() }))
            }
            _ => {
                respond(&mut stream, "400 Bad Request", "Bad request");
                None
            }
        };
    };
    if received_state != Some(expected_state) {
        respond(&mut stream, "400 Bad Request", "State mismatch");
        return Some(Err(OAuthError::StateMismatch));
    }
    respond(&mut stream, "200 OK", "Signed in. You can close this window.");
    Some(Ok(code.clone()))
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let html = format!("<html><body>{body}</body></html>");
    let _ = stream.write_all(
        format!("HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{html}", html.len()).as_bytes(),
    );
    let _ = stream.flush();
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build()
}

fn exchange_code(config: &OAuthConfig, code: &str, redirect_uri: &str, code_verifier: &str) -> Result<TokenResponse, OAuthError> {
    let mut form: Vec<(&str, &str)> = vec![
        ("code", code),
        ("client_id", &config.client_id),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
        ("code_verifier", code_verifier),
    ];
    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret));
    }
    if config.scope_in_token_requests {
        form.push(("scope", &config.scope));
    }
    agent()
        .post(&config.token_endpoint)
        .send_form(&form)
        .map_err(|_| OAuthError::TokenExchangeFailed)?
        .into_json()
        .map_err(|_| OAuthError::TokenExchangeFailed)
}

/// The refresh grant. Google keeps requiring `client_secret` here too.
pub fn refresh(provider: &ProviderConfig, refresh_token: &str) -> Result<TokenResponse, OAuthError> {
    let config = OAuthConfig::from_provider(provider)?;
    let mut form: Vec<(&str, &str)> = vec![
        ("refresh_token", refresh_token),
        ("client_id", &config.client_id),
        ("grant_type", "refresh_token"),
    ];
    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret));
    }
    if config.scope_in_token_requests {
        form.push(("scope", &config.scope));
    }
    agent()
        .post(&config.token_endpoint)
        .send_form(&form)
        .map_err(|e| match e {
            ureq::Error::Status(400, resp) => match resp.into_json::<serde_json::Value>() {
                Ok(body) if body["error"] == "invalid_grant" => OAuthError::RefreshTokenRevoked,
                _ => OAuthError::TokenRefreshFailed,
            },
            _ => OAuthError::TokenRefreshFailed,
        })?
        .into_json()
        .map_err(|_| OAuthError::TokenRefreshFailed)
}

/// Refreshes `stored` and merges the response into it (see
/// [`StoredTokens::applying_refresh`]); the shell writes the result back to its secret
/// store.
pub fn refresh_stored(provider: &ProviderConfig, stored: &StoredTokens, now_unix_ms: i64) -> Result<StoredTokens, OAuthError> {
    let response = refresh(provider, &stored.refresh_token)?;
    Ok(stored.applying_refresh(response, now_unix_ms))
}

// ---- Pure helpers ----

/// Whether a granted scope string covers everything `required` asks for, comparing
/// whitespace-separated components. Extra grants are fine (a containment test, not
/// equality). Google's scopes are hierarchical, so a known broader parent also
/// satisfies a narrower requirement. `None` or empty never satisfies anything.
pub fn granted_scope_satisfies(granted: Option<&str>, required: &str) -> bool {
    let Some(granted) = granted else { return false };
    let granted: std::collections::HashSet<&str> = granted.split_whitespace().collect();
    required
        .split_whitespace()
        .all(|req| granted.contains(req) || broader_scopes(req).iter().any(|b| granted.contains(b)))
}

/// Scopes that fully subsume `scope` under Google's own hierarchy.
fn broader_scopes(scope: &str) -> &'static [&'static str] {
    match scope {
        "https://www.googleapis.com/auth/calendar.events" | "https://www.googleapis.com/auth/calendar.readonly" => {
            &["https://www.googleapis.com/auth/calendar"]
        }
        // gmail.compose is Google's documented superset of gmail.send.
        "https://www.googleapis.com/auth/gmail.send" => &["https://mail.google.com/", "https://www.googleapis.com/auth/gmail.compose"],
        _ => &[],
    }
}

/// Decodes an unverified JWT's payload claims. No signature verification: the token
/// comes straight from the provider's token endpoint over TLS and is read only for
/// account identity, never as an authorization decision.
pub fn jwt_claims(jwt: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let segments: Vec<&str> = jwt.split('.').collect();
    if segments.len() != 3 {
        return None;
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segments[1].trim_end_matches('='))
        .ok()?;
    match serde_json::from_slice::<serde_json::Value>(&payload).ok()? {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}

pub fn jwt_email_claim(jwt: &str) -> Option<String> {
    jwt_claims(jwt)?.get("email")?.as_str().map(str::to_string)
}

/// Microsoft: `email` first, falling back to `preferred_username` (work/school
/// accounts frequently omit `email`).
pub fn microsoft_account_identifier(jwt: &str) -> Option<String> {
    let claims = jwt_claims(jwt)?;
    claims
        .get("email")
        .and_then(|v| v.as_str())
        .or_else(|| claims.get("preferred_username").and_then(|v| v.as_str()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn google(token_endpoint: Option<String>) -> ProviderConfig {
        ProviderConfig {
            provider: "google".into(),
            client_id: "cid".into(),
            client_secret: Some("sec".into()),
            scope: "openid email".into(),
            auth_endpoint: None,
            token_endpoint,
        }
    }

    fn microsoft(token_endpoint: Option<String>) -> ProviderConfig {
        ProviderConfig {
            provider: "microsoft".into(),
            client_id: "mcid".into(),
            client_secret: None,
            scope: "openid email offline_access".into(),
            auth_endpoint: None,
            token_endpoint,
        }
    }

    /// One-shot token server: records the POST body, replies with `body`.
    fn token_server(status: u16, body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let body = body.to_string();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            // Read the whole request (headers + Content-Length body) before replying, so
            // the client never sees a reset mid-write.
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw);
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let len: usize = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: ").or_else(|| l.strip_prefix("content-length: ")))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if raw.len() >= head_end + 4 + len {
                        break;
                    }
                }
            }
            let req = String::from_utf8_lossy(&raw).into_owned();
            let _ = tx.send(req);
            let _ = stream.write_all(format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
        });
        (url, rx)
    }

    /// Simulates the browser hitting the loopback redirect; returns the HTTP status
    /// line and body the listener served.
    fn browser_redirect(redirect_uri: &str, query: &str) -> (String, String) {
        let u = url::Url::parse(redirect_uri).unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", u.port().unwrap())).unwrap();
        s.write_all(format!("GET /?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        let status = out.lines().next().unwrap_or("").to_string();
        let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    fn state_of(auth_url: &str) -> String {
        url::Url::parse(auth_url).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned()
    }

    #[test]
    fn pkce_challenge_matches_the_rfc_7636_vector() {
        assert_eq!(code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let v = generate_code_verifier();
        assert!(v.len() >= 43 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(generate_state(), generate_state());
    }

    #[test]
    fn begin_binds_loopback_only_and_builds_the_consent_url() {
        let session = begin(&google(None)).unwrap();
        let u = url::Url::parse(&session.auth_url).unwrap();
        assert_eq!(u.host_str(), Some("accounts.google.com"));
        let q: HashMap<String, String> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        assert_eq!(q["client_id"], "cid");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "consent");
        assert_eq!(q["scope"], "openid email");
        assert_eq!(q["redirect_uri"], session.redirect_uri);
        assert!(session.redirect_uri.starts_with("http://127.0.0.1:"));
        let port: u16 = url::Url::parse(&session.redirect_uri).unwrap().port().unwrap();
        // Bound to the loopback interface only: not reachable via any other address.
        let s = sessions().lock().unwrap().get(&session.session_id).unwrap().clone();
        let addr = s.listener.lock().unwrap().as_ref().unwrap().local_addr().unwrap();
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), port);
        cancel(&session.session_id);
        assert!(matches!(wait(&session.session_id, 5_000, 0), Err(OAuthError::Cancelled)));

        let ms = begin(&microsoft(None)).unwrap();
        assert!(ms.redirect_uri.starts_with("http://localhost:"));
        let q: HashMap<String, String> = url::Url::parse(&ms.auth_url).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        assert_eq!(q["prompt"], "select_account");
        assert!(!q.contains_key("access_type"));
        cancel(&ms.session_id);
        let _ = wait(&ms.session_id, 1_000, 0);
    }

    #[test]
    fn mismatched_state_is_rejected_and_ends_the_wait() {
        let session = begin(&google(None)).unwrap();
        let redirect = session.redirect_uri.clone();
        let id = session.session_id.clone();
        let waiter = std::thread::spawn(move || wait(&id, 5_000, 0));
        std::thread::sleep(Duration::from_millis(50));
        let (status, body) = browser_redirect(&redirect, "code=abc&state=WRONG");
        assert!(status.contains("400"), "{status}");
        assert!(body.contains("State mismatch"));
        assert_eq!(waiter.join().unwrap(), Err(OAuthError::StateMismatch));
    }

    #[test]
    fn matching_redirect_returns_the_code_and_exchanges_it_with_pkce() {
        let (token_url, rx) = token_server(200, r#"{"access_token":"at","refresh_token":"rt","id_token":"idt","expires_in":3600,"scope":"openid email"}"#);
        let session = begin(&google(Some(token_url))).unwrap();
        let state = state_of(&session.auth_url);
        let redirect = session.redirect_uri.clone();
        let id = session.session_id.clone();
        let waiter = std::thread::spawn(move || wait(&id, 5_000, 1_000));
        std::thread::sleep(Duration::from_millis(50));
        // Incidental connections never end the wait.
        let (status, _) = browser_redirect(&redirect, "junk=1");
        assert!(status.contains("400"));
        let (status, body) = browser_redirect(&redirect, &format!("code=the-code&state={state}"));
        assert!(status.contains("200"), "{status}");
        assert!(body.contains("You can close this window"));
        let tokens = waiter.join().unwrap().unwrap();
        assert_eq!(tokens, StoredTokens { access_token: "at".into(), refresh_token: "rt".into(), id_token: Some("idt".into()), expires_at_unix_ms: 1_000 + 3_600_000, granted_scope: Some("openid email".into()) });
        let req = rx.recv().unwrap();
        let form = req.split("\r\n\r\n").nth(1).unwrap();
        for part in ["code=the-code", "client_id=cid", "client_secret=sec", "grant_type=authorization_code", "code_verifier="] {
            assert!(form.contains(part), "{form}");
        }
        assert!(form.contains(&format!("redirect_uri={}", url::form_urlencoded::byte_serialize(redirect.as_bytes()).collect::<String>())), "{form}");
        assert!(!form.contains("scope="), "google sends no scope on token requests");
        assert!(sessions().lock().unwrap().get(&session.session_id).is_none(), "session consumed");
    }

    #[test]
    fn microsoft_exchange_sends_scope_and_no_secret() {
        let (token_url, rx) = token_server(200, r#"{"access_token":"at","refresh_token":"rt","expires_in":60}"#);
        let session = begin(&microsoft(Some(token_url))).unwrap();
        let state = state_of(&session.auth_url);
        let redirect = session.redirect_uri.clone();
        let id = session.session_id.clone();
        let waiter = std::thread::spawn(move || wait(&id, 5_000, 0));
        std::thread::sleep(Duration::from_millis(50));
        browser_redirect(&redirect, &format!("code=c&state={state}"));
        let tokens = waiter.join().unwrap().unwrap();
        assert_eq!(tokens.id_token, None);
        let form = rx.recv().unwrap().split("\r\n\r\n").nth(1).unwrap().to_string();
        assert!(form.contains("scope=openid+email+offline_access") || form.contains("scope=openid%20email%20offline_access"), "{form}");
        assert!(!form.contains("client_secret"));
    }

    #[test]
    fn denied_authorization_with_matching_state_ends_the_wait() {
        let session = begin(&google(None)).unwrap();
        let state = state_of(&session.auth_url);
        let redirect = session.redirect_uri.clone();
        let id = session.session_id.clone();
        let waiter = std::thread::spawn(move || wait(&id, 5_000, 0));
        std::thread::sleep(Duration::from_millis(50));
        let (status, body) = browser_redirect(&redirect, &format!("error=access_denied&state={state}"));
        assert!(status.contains("400") && body.contains("Authorization failed: access_denied"));
        assert_eq!(waiter.join().unwrap(), Err(OAuthError::AuthorizationDenied { reason: "access_denied".into() }));
    }

    #[test]
    fn wait_times_out_and_unknown_sessions_are_rejected() {
        let session = begin(&google(None)).unwrap();
        let started = Instant::now();
        assert_eq!(wait(&session.session_id, 200, 0), Err(OAuthError::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(wait(&session.session_id, 200, 0), Err(OAuthError::UnknownSession));
        assert!(!cancel("nope"));
    }

    #[test]
    fn cancel_from_another_thread_interrupts_an_in_flight_wait() {
        let session = begin(&google(None)).unwrap();
        let id = session.session_id.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            assert!(cancel(&id), "session must still be registered while wait blocks");
        });
        let started = Instant::now();
        let err = wait(&session.session_id, 10_000, 0).unwrap_err();
        assert_eq!(err, OAuthError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(1), "cancel must interrupt promptly, took {:?}", started.elapsed());
        canceller.join().unwrap();
        assert!(sessions().lock().unwrap().get(&session.session_id).is_none(), "session consumed");
    }

    #[test]
    fn exchange_failure_and_missing_refresh_token_are_errors() {
        let (token_url, _rx) = token_server(400, r#"{"error":"invalid_grant"}"#);
        let session = begin(&google(Some(token_url))).unwrap();
        let state = state_of(&session.auth_url);
        let redirect = session.redirect_uri.clone();
        let id = session.session_id.clone();
        let waiter = std::thread::spawn(move || wait(&id, 5_000, 0));
        std::thread::sleep(Duration::from_millis(50));
        browser_redirect(&redirect, &format!("code=c&state={state}"));
        assert_eq!(waiter.join().unwrap(), Err(OAuthError::TokenExchangeFailed));

        let (token_url, _rx) = token_server(200, r#"{"access_token":"at","expires_in":60}"#);
        let session = begin(&google(Some(token_url))).unwrap();
        let state = state_of(&session.auth_url);
        let redirect = session.redirect_uri.clone();
        let id = session.session_id.clone();
        let waiter = std::thread::spawn(move || wait(&id, 5_000, 0));
        std::thread::sleep(Duration::from_millis(50));
        browser_redirect(&redirect, &format!("code=c&state={state}"));
        assert_eq!(waiter.join().unwrap(), Err(OAuthError::NoRefreshTokenReturned));
    }

    #[test]
    fn refresh_stored_merges_and_keeps_what_the_provider_omits() {
        let (token_url, rx) = token_server(200, r#"{"access_token":"new-at","expires_in":100}"#);
        let stored = StoredTokens { access_token: "old".into(), refresh_token: "rt".into(), id_token: Some("idt".into()), expires_at_unix_ms: 0, granted_scope: Some("s".into()) };
        assert!(!stored.is_fresh(0));
        let updated = refresh_stored(&google(Some(token_url)), &stored, 5_000).unwrap();
        assert_eq!(updated, StoredTokens { access_token: "new-at".into(), refresh_token: "rt".into(), id_token: Some("idt".into()), expires_at_unix_ms: 105_000, granted_scope: Some("s".into()) });
        assert!(updated.is_fresh(5_000));
        assert!(!updated.is_fresh(105_000 - TOKEN_EXPIRY_LEEWAY_MS));
        let form = rx.recv().unwrap().split("\r\n\r\n").nth(1).unwrap().to_string();
        assert!(form.contains("refresh_token=rt") && form.contains("grant_type=refresh_token") && form.contains("client_secret=sec"), "{form}");

        let (token_url, _rx) = token_server(401, "{}");
        assert_eq!(refresh_stored(&google(Some(token_url)), &stored, 0), Err(OAuthError::TokenRefreshFailed));
        assert!(matches!(begin(&ProviderConfig { provider: "yahoo".into(), ..google(None) }), Err(OAuthError::UnknownProvider(_))));
    }

    #[test]
    fn refresh_invalid_grant_is_revoked_not_a_transient_failure() {
        let stored = StoredTokens { access_token: "old".into(), refresh_token: "rt".into(), id_token: None, expires_at_unix_ms: 0, granted_scope: None };
        let (token_url, _rx) = token_server(400, r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#);
        assert_eq!(refresh_stored(&google(Some(token_url)), &stored, 0), Err(OAuthError::RefreshTokenRevoked));
        assert_eq!(serde_json::to_value(OAuthError::RefreshTokenRevoked).unwrap()["error"], "refresh_token_revoked", "the FFI code Swift maps");
        let (token_url, _rx) = token_server(400, r#"{"error":"invalid_request"}"#);
        assert_eq!(refresh_stored(&google(Some(token_url)), &stored, 0), Err(OAuthError::TokenRefreshFailed));
    }

    #[test]
    fn stored_tokens_keychain_json_round_trips_with_swift_field_names() {
        let json = r#"{"accessToken":"a","refreshToken":"r","idToken":null,"expiresAtUnixMs":5}"#;
        let t: StoredTokens = serde_json::from_str(json).unwrap();
        assert_eq!(t.granted_scope, None, "entries written before grantedScope existed decode");
        let out = serde_json::to_value(&t).unwrap();
        assert_eq!(out["accessToken"], "a");
        assert_eq!(out["expiresAtUnixMs"], 5);
        assert!(out.get("grantedScope").is_some());
    }

    // ---- ported from GoogleOAuthScopeTests.swift ----

    const CAL_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";
    const CAL_RO: &str = "https://www.googleapis.com/auth/calendar.readonly";
    const CAL_FULL: &str = "https://www.googleapis.com/auth/calendar";
    const GMAIL_SEND: &str = "https://www.googleapis.com/auth/gmail.send";

    #[test]
    fn scope_containment_rules() {
        assert!(granted_scope_satisfies(Some(GMAIL_SEND), GMAIL_SEND));
        assert!(granted_scope_satisfies(Some(&format!("{CAL_RO} {CAL_EVENTS} openid https://www.googleapis.com/auth/userinfo.email")), CAL_EVENTS));
        assert!(!granted_scope_satisfies(Some(CAL_RO), CAL_EVENTS), "stale readonly-only grant is insufficient for writes");
        assert!(granted_scope_satisfies(Some(CAL_FULL), CAL_EVENTS));
        assert!(granted_scope_satisfies(Some(CAL_FULL), CAL_RO));
        assert!(granted_scope_satisfies(Some("https://mail.google.com/"), GMAIL_SEND));
        assert!(granted_scope_satisfies(Some("https://www.googleapis.com/auth/gmail.compose"), GMAIL_SEND));
        assert!(granted_scope_satisfies(Some("openid email https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose"), GMAIL_SEND));
        assert!(!granted_scope_satisfies(None, GMAIL_SEND));
        assert!(!granted_scope_satisfies(Some(""), GMAIL_SEND));
        assert!(!granted_scope_satisfies(Some(GMAIL_SEND), CAL_EVENTS));
        assert!(!granted_scope_satisfies(Some(CAL_EVENTS), &format!("{CAL_EVENTS} {GMAIL_SEND}")));
        assert!(granted_scope_satisfies(Some(&format!("{CAL_EVENTS} {GMAIL_SEND}")), &format!("{CAL_EVENTS} {GMAIL_SEND}")));
        assert!(granted_scope_satisfies(Some(&format!("  {CAL_EVENTS}\n {GMAIL_SEND}  ")), CAL_EVENTS));
    }

    // ---- ported from the JWT cases in both helper test files ----

    fn make_jwt(payload: &str) -> String {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.as_bytes());
        format!("{}.{}.signature", b64(r#"{"alg":"RS256"}"#), b64(payload))
    }

    #[test]
    fn jwt_claim_helpers() {
        assert_eq!(jwt_email_claim(&make_jwt(r#"{"email":"alice@genesys.com","sub":"123"}"#)).as_deref(), Some("alice@genesys.com"));
        assert_eq!(jwt_email_claim(&make_jwt(r#"{"sub":"123"}"#)), None);
        assert_eq!(jwt_email_claim("not-a-real-jwt"), None);
        assert_eq!(microsoft_account_identifier(&make_jwt(r#"{"email":"alice@outlook.com","preferred_username":"alice@contoso.com"}"#)).as_deref(), Some("alice@outlook.com"));
        assert_eq!(microsoft_account_identifier(&make_jwt(r#"{"preferred_username":"alice@contoso.com","sub":"123"}"#)).as_deref(), Some("alice@contoso.com"));
        assert_eq!(microsoft_account_identifier(&make_jwt(r#"{"sub":"123"}"#)), None);
        assert_eq!(microsoft_account_identifier("not-a-real-jwt"), None);
        // Padding-bearing base64url payloads (some issuers pad) still decode.
        let padded = format!("{}.{}=.sig", "eyJhbGciOiJSUzI1NiJ9", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"email":"p@x.com"}"#));
        assert_eq!(jwt_email_claim(&padded).as_deref(), Some("p@x.com"));
    }
}
