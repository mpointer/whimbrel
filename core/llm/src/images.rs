//! Skins Release 2: finding candidate images for a skin — Brave image search, image
//! generation (OpenAI gpt-image-1, Google Imagen), and a guarded fetch of a chosen
//! image. Nothing here places an image in a skin; the shell only offers what these
//! return and the user approves. Spend is charged by the caller through
//! [`crate::charge_skin_call`] before any of these touch the network.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::{ureq_error_to_llm, LlmError, Provider};

pub const IMAGE_SEARCH_CENTS: i64 = 1;
pub const IMAGE_GENERATE_CENTS: i64 = 5;

/// Model-written search queries and prompts are cut to this many characters before any
/// paid call, so a runaway reply can't send an essay.
pub const MAX_IMAGE_TEXT_CHARS: usize = 200;

const SEARCH_COUNT: u8 = 6;
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Generation can take most of a minute; search and fetch finish well inside this.
const READ_TIMEOUT: Duration = Duration::from_secs(120);
/// Overall deadline for one image fetch, redirects and body included.
const FETCH_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCandidate {
    pub url: String,
    pub thumbnail_url: String,
    pub title: String,
    /// `"web"` or `"generated"`.
    pub source: String,
}

/// `text` cut to [`MAX_IMAGE_TEXT_CHARS`] characters (never mid-character).
pub fn capped(text: &str) -> &str {
    match text.char_indices().nth(MAX_IMAGE_TEXT_CHARS) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn agent_builder(https_only: bool) -> ureq::AgentBuilder {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .https_only(https_only)
        // ureq errors once its redirect history reaches `n - 1`, so 4 follows 3.
        .redirects(4)
}

fn agent(https_only: bool) -> ureq::Agent {
    agent_builder(https_only).build()
}

/// IPv4 ranges a fetch may never connect to: "this network", private, CGNAT, loopback,
/// link-local (incl. cloud metadata 169.254.169.254), IETF protocol assignments,
/// documentation, benchmarking, multicast, and 240/4 (reserved, incl. broadcast).
const V4_BLOCKED: [([u8; 4], u32); 14] = [
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 0, 2, 0], 24),
    ([192, 168, 0, 0], 16),
    ([198, 18, 0, 0], 15),
    ([198, 51, 100, 0], 24),
    ([203, 0, 113, 0], 24),
    ([224, 0, 0, 0], 4),
    ([240, 0, 0, 0], 4),
];

/// Special-purpose ranges inside 2000::/3 that are refused: 2001::/23 (IETF protocol
/// assignments, incl. Teredo 2001::/32), 2001:db8::/32 (documentation), 2002::/16 (6to4).
const V6_BLOCKED: [(u128, u32); 3] = [
    (0x2001_0000 << 96, 23),
    (0x2001_0db8 << 96, 32),
    (0x2002 << 112, 16),
];

/// Whether `ip` is a public unicast address a fetch may connect to. IPv4: anything
/// outside [`V4_BLOCKED`]. IPv6 is an allowlist: only global unicast 2000::/3 outside
/// [`V6_BLOCKED`]; IPv4-mapped (`::ffff:a.b.c.d`) and NAT64 (`64:ff9b::/96`) addresses
/// get the IPv4 rules on their embedded address. Everything else (::, ::1, fc00::/7,
/// fe80::/10, fec0::/10, ff00::/8, 64:ff9b:1::/48, `::a.b.c.d`, …) is refused.
pub fn is_public_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let x = u32::from(v4);
            !V4_BLOCKED
                .iter()
                .any(|&(base, len)| x >> (32 - len) == u32::from_be_bytes(base) >> (32 - len))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_addr(IpAddr::V4(v4));
            }
            let x = u128::from(v6);
            if x >> 32 == 0x0064_ff9b << 64 {
                return is_public_addr(IpAddr::V4(Ipv4Addr::from(x as u32)));
            }
            x >> 125 == 1
                && !V6_BLOCKED
                    .iter()
                    .any(|&(base, len)| x >> (128 - len) == base >> (128 - len))
        }
    }
}

/// The resolver every production fetch connects through, so the check covers the first
/// request and every redirect hop, and the connection goes to the address that was
/// checked (no second lookup for DNS rebinding to win). Port 443 only.
fn resolve_public(netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
    let addrs: Vec<SocketAddr> = netloc
        .to_socket_addrs()?
        .filter(|a| a.port() == 443 && is_public_addr(a.ip()))
        .collect();
    if addrs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{netloc} is not a public https address"),
        ));
    }
    Ok(addrs)
}

/// The Brave image search URL (strict safesearch) and its headers, minus the
/// `X-Subscription-Token` key header, which [`search_images_with`] adds.
pub fn image_search_request(query: &str, count: u8) -> (String, Vec<(String, String)>) {
    let url = format!(
        "https://api.search.brave.com/res/v1/images/search?q={}&count={count}&safesearch=strict",
        percent_encode(capped(query))
    );
    (url, vec![("Accept".into(), "application/json".into())])
}

#[derive(Deserialize)]
struct BraveResponse {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    #[serde(default)]
    title: String,
    thumbnail: Option<BraveSrc>,
    properties: Option<BraveProperties>,
}

#[derive(Deserialize)]
struct BraveSrc {
    src: Option<String>,
}

#[derive(Deserialize)]
struct BraveProperties {
    url: Option<String>,
}

/// Candidates from a Brave image search body. A result's own `url` is the page hosting
/// the image; the image itself is `properties.url`. Results without an https image URL
/// and an https thumbnail are skipped.
pub fn parse_image_search(body: &str) -> Result<Vec<ImageCandidate>, LlmError> {
    let parsed: BraveResponse = serde_json::from_str(body)
        .map_err(|e| LlmError::Provider(format!("failed to parse Brave image results: {e}")))?;
    Ok(parsed
        .results
        .into_iter()
        .filter_map(|r| {
            let url = r.properties?.url?;
            let thumbnail_url = r.thumbnail?.src?;
            if !url.starts_with("https://") || !thumbnail_url.starts_with("https://") {
                return None;
            }
            Some(ImageCandidate {
                url,
                thumbnail_url,
                title: r.title,
                source: "web".into(),
            })
        })
        .collect())
}

/// [`search_images`] over an injected GET (`url`, headers including the key) → body.
pub fn search_images_with(
    key: &str,
    query: &str,
    get: impl FnOnce(&str, &[(String, String)]) -> Result<String, LlmError>,
) -> Result<Vec<ImageCandidate>, LlmError> {
    let (url, mut headers) = image_search_request(query, SEARCH_COUNT);
    headers.push(("X-Subscription-Token".into(), key.into()));
    parse_image_search(&get(&url, &headers)?)
}

pub fn search_images(key: &str, query: &str) -> Result<Vec<ImageCandidate>, LlmError> {
    search_images_with(key, query, |url, headers| {
        let mut request = agent(true).get(url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        request
            .call()
            .map_err(ureq_error_to_llm)?
            .into_string()
            .map_err(|e| LlmError::Network(e.to_string()))
    })
}

/// The generation endpoint and JSON body for `provider` (prompt capped), or `None` for a
/// provider that can't generate images.
pub fn image_generate_request(
    provider: Provider,
    prompt: &str,
) -> Option<(String, serde_json::Value)> {
    let prompt = capped(prompt);
    match provider {
        // gpt-image-1 always answers base64 (`b64_json`) and rejects `response_format`.
        // Quality is explicit ("auto" may pick "high"), so IMAGE_GENERATE_CENTS is an upper bound.
        Provider::OpenAI => Some((
            "https://api.openai.com/v1/images/generations".into(),
            serde_json::json!({"model": "gpt-image-1", "prompt": prompt, "size": "1024x1024", "quality": "medium", "n": 1}),
        )),
        Provider::Google => Some((
            "https://generativelanguage.googleapis.com/v1beta/models/imagen-4.0-generate-001:predict".into(),
            serde_json::json!({"instances": [{"prompt": prompt}], "parameters": {"sampleCount": 1, "aspectRatio": "16:9"}}),
        )),
        Provider::Ollama | Provider::Anthropic => None,
    }
}

fn decode_image_b64(b64: Option<&str>, who: &str) -> Result<Vec<u8>, LlmError> {
    let b64 = b64.ok_or_else(|| LlmError::Provider(format!("{who} response had no image")))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| LlmError::Provider(format!("{who} image is not base64: {e}")))?;
    check_image_bytes(&bytes)?;
    Ok(bytes)
}

pub fn parse_openai_image(body: &str) -> Result<Vec<u8>, LlmError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| LlmError::Provider(format!("failed to parse OpenAI image response: {e}")))?;
    decode_image_b64(v["data"][0]["b64_json"].as_str(), "OpenAI")
}

pub fn parse_google_image(body: &str) -> Result<Vec<u8>, LlmError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| LlmError::Provider(format!("failed to parse Google image response: {e}")))?;
    decode_image_b64(v["predictions"][0]["bytesBase64Encoded"].as_str(), "Google")
}

pub fn generate_image(provider: Provider, key: &str, prompt: &str) -> Result<Vec<u8>, LlmError> {
    let (url, body) = image_generate_request(provider, prompt)
        .ok_or(LlmError::NotImplemented("image generation"))?;
    let request = agent(true).post(&url);
    // Keys go in headers, never the URL.
    let request = match provider {
        Provider::OpenAI => request.set("Authorization", &format!("Bearer {key}")),
        _ => request.set("x-goog-api-key", key),
    };
    let text = request
        .send_json(body)
        .map_err(ureq_error_to_llm)?
        .into_string()
        .map_err(|e| LlmError::Network(e.to_string()))?;
    match provider {
        Provider::OpenAI => parse_openai_image(&text),
        _ => parse_google_image(&text),
    }
}

/// Accepts PNG, JPEG, GIF or WebP by magic number.
fn check_image_bytes(bytes: &[u8]) -> Result<(), LlmError> {
    let ok = bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP");
    if ok {
        Ok(())
    } else {
        Err(LlmError::Provider(
            "not a PNG, JPEG, GIF or WebP image".into(),
        ))
    }
}

/// Downloads an image the user picked: https only to public addresses on port 443
/// (every hop, see [`resolve_public`]), at most 3 redirects, `Content-Type: image/*`, at
/// most 10 MB, and bytes that really are PNG/JPEG/GIF/WebP.
pub fn fetch_image(url: &str) -> Result<Vec<u8>, LlmError> {
    fetch(url, false)
}

/// [`fetch_image`] with the https and public-address guards OFF, so the integration
/// tests can use a plain 127.0.0.1 server. Never call this outside tests.
#[doc(hidden)]
pub fn fetch_image_insecure_for_tests(url: &str) -> Result<Vec<u8>, LlmError> {
    fetch(url, true)
}

fn fetch(url: &str, insecure: bool) -> Result<Vec<u8>, LlmError> {
    let builder = agent_builder(!insecure).timeout(FETCH_DEADLINE);
    let builder = if insecure {
        builder
    } else {
        builder.resolver(resolve_public)
    };
    let response = builder.build().get(url).call().map_err(ureq_error_to_llm)?;
    if response.status() != 200 {
        return Err(LlmError::Provider(format!(
            "image fetch answered {}",
            response.status()
        )));
    }
    if !response
        .content_type()
        .to_ascii_lowercase()
        .starts_with("image/")
    {
        return Err(LlmError::Provider(format!(
            "not an image: {}",
            response.content_type()
        )));
    }
    let declared: Option<u64> = response
        .header("Content-Length")
        .and_then(|v| v.parse().ok());
    if declared.is_some_and(|n| n > MAX_IMAGE_BYTES) {
        return Err(LlmError::Provider("image is over 10 MB".into()));
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| LlmError::Network(e.to_string()))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(LlmError::Provider("image is over 10 MB".into()));
    }
    check_image_bytes(&bytes)?;
    Ok(bytes)
}
