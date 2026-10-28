use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use llm::images::{
    capped, fetch_image, fetch_image_insecure_for_tests, image_generate_request,
    image_search_request, is_public_addr, parse_google_image, parse_image_search,
    parse_openai_image, search_images_with, IMAGE_SEARCH_CENTS,
};
use llm::{charge_skin_call, ChargeError, LlmError, Provider, SpendCapConfig, SpendCapScope};
use memory::Store;

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/images/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn parses_brave_image_results() {
    let candidates = parse_image_search(&fixture("brave_images.json")).unwrap();
    assert_eq!(candidates.len(), 6);
    let first = &candidates[0];
    // The image itself (properties.url), not the page that hosts it (url).
    assert_eq!(first.url, "https://example1.com/images/stadium1.jpg");
    assert_eq!(
        first.thumbnail_url,
        "https://imgs.search.brave.com/thumb1.jpg"
    );
    assert_eq!(first.title, "Stadium at night 1");
    assert!(candidates.iter().all(|c| c.source == "web"));
    assert!(parse_image_search("not json").is_err());
}

#[test]
fn search_request_is_strict_and_capped() {
    let (url, headers) = image_search_request("stadium & lights", 6);
    assert!(url.starts_with("https://api.search.brave.com/res/v1/images/search?"));
    assert!(url.contains("safesearch=strict"), "{url}");
    assert!(url.contains("count=6"), "{url}");
    assert!(url.contains("q=stadium%20%26%20lights"), "{url}");
    assert!(!headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("X-Subscription-Token")));

    // The key travels in a header, never the URL.
    let sent = Cell::new(false);
    search_images_with("secret-key", "stadium", |url, headers| {
        sent.set(true);
        assert!(!url.contains("secret-key"));
        assert!(headers.contains(&("X-Subscription-Token".into(), "secret-key".into())));
        Ok(fixture("brave_images.json"))
    })
    .unwrap();
    assert!(sent.get());

    // Queries are capped at 200 chars (on char boundaries) before any paid call.
    let (url, _) = image_search_request(&"a".repeat(500), 6);
    assert!(url.contains(&format!("q={}&", "a".repeat(200))), "{url}");
    assert!(!url.contains(&"a".repeat(201)));
    let emoji = "🎨".repeat(300);
    assert_eq!(capped(&emoji).chars().count(), 200);
    assert_eq!(capped("short"), "short");
}

#[test]
fn parses_openai_and_google_image_payloads() {
    let openai = parse_openai_image(&fixture("openai_image.json")).unwrap();
    assert!(openai.starts_with(&PNG_MAGIC));
    let google = parse_google_image(&fixture("google_image.json")).unwrap();
    assert!(google.starts_with(&PNG_MAGIC));
    assert!(parse_openai_image(r#"{"data":[]}"#).is_err());
    assert!(parse_google_image(r#"{"predictions":[{"bytesBase64Encoded":"aGVsbG8="}]}"#).is_err());

    let (url, body) = image_generate_request(Provider::OpenAI, &"p".repeat(300)).unwrap();
    assert_eq!(url, "https://api.openai.com/v1/images/generations");
    assert_eq!(body["model"], "gpt-image-1");
    assert_eq!(body["size"], "1024x1024");
    assert_eq!(body["prompt"].as_str().unwrap().len(), 200);
    let (url, body) = image_generate_request(Provider::Google, "a banner").unwrap();
    assert!(
        url.ends_with("/models/imagen-4.0-generate-001:predict"),
        "{url}"
    );
    assert_eq!(body["instances"][0]["prompt"], "a banner");
    assert!(image_generate_request(Provider::Ollama, "x").is_none());
}

/// Ruling I5: quality is explicit, so `auto` can never pick "high" and the 5-cent charge is an upper bound.
#[test]
fn openai_generation_asks_for_medium_quality_square() {
    let (_, body) = image_generate_request(Provider::OpenAI, "a banner").unwrap();
    assert_eq!(body["quality"], "medium");
    assert_eq!(body["size"], "1024x1024");
}

/// One request per connection; routes by path. Runs until the test process exits.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            let path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            let png: Vec<u8> = [&PNG_MAGIC[..], b"rest-of-png"].concat();
            let (status, extra, body): (&str, String, Vec<u8>) = match path.as_str() {
                "/html" => (
                    "200 OK",
                    "Content-Type: text/html\r\n".into(),
                    b"<html></html>".to_vec(),
                ),
                "/big" | "/big-no-length" => {
                    let mut body = png.clone();
                    body.resize(11 * 1024 * 1024, 0);
                    ("200 OK", "Content-Type: image/png\r\n".into(), body)
                }
                "/fake-png" => (
                    "200 OK",
                    "Content-Type: image/png\r\n".into(),
                    b"<html></html>".to_vec(),
                ),
                "/png" => ("200 OK", "Content-Type: image/png\r\n".into(), png),
                "/loop" => ("302 Found", "Location: /loop\r\n".into(), Vec::new()),
                p if p.starts_with("/hop") => {
                    let n: u32 = p[4..].parse().unwrap();
                    let next = if n == 1 {
                        "/png".to_string()
                    } else {
                        format!("/hop{}", n - 1)
                    };
                    ("302 Found", format!("Location: {next}\r\n"), Vec::new())
                }
                _ => ("404 Not Found", String::new(), Vec::new()),
            };
            // "/big-no-length" sends no Content-Length (the body ends when the
            // connection closes), so only the client's capped read can catch it.
            let length = if path == "/big-no-length" {
                String::new()
            } else {
                format!("Content-Length: {}\r\n", body.len())
            };
            let head = format!("HTTP/1.1 {status}\r\n{extra}{length}Connection: close\r\n\r\n");
            // The client hangs up early on oversize bodies; a broken pipe is expected.
            let _ = stream
                .write_all(head.as_bytes())
                .and_then(|_| stream.write_all(&body));
        }
    });
    base
}

#[test]
fn fetch_image_rejects_non_image_and_oversize() {
    let base = serve();
    let fetch = |path: &str| fetch_image_insecure_for_tests(&format!("{base}{path}"));
    assert!(fetch("/html").is_err(), "text/html");
    assert!(fetch("/big").is_err(), "11 MB");
    let err = fetch("/big-no-length").unwrap_err().to_string();
    assert!(
        err.contains("over 10 MB"),
        "11 MB, no Content-Length: {err}"
    );
    assert!(fetch("/loop").is_err(), "redirect loop");
    assert!(
        fetch("/fake-png").is_err(),
        "image/png header over non-image bytes"
    );
    assert!(fetch("/hop4").is_err(), "4 redirects");
    assert!(
        fetch("/hop3").unwrap().starts_with(&PNG_MAGIC),
        "3 redirects are allowed"
    );
    assert!(fetch("/png").unwrap().starts_with(&PNG_MAGIC));
    // The real entry point refuses anything but https, before connecting.
    assert!(llm::images::fetch_image(&format!("{base}/png")).is_err());
}

#[test]
fn image_calls_respect_spend_cap() {
    let store = Store::open_in_memory().unwrap();
    let config = SpendCapConfig {
        user_daily_cents: Some(2),
        global_daily_cents: None,
    };
    let now = 1_760_000_000_000;
    let network_calls = Cell::new(0);
    let search = || -> Result<Result<usize, LlmError>, ChargeError> {
        charge_skin_call(
            &store,
            &config,
            "skin-image-search",
            IMAGE_SEARCH_CENTS,
            now,
        )?;
        Ok(search_images_with("k", "stadium", |_, _| {
            network_calls.set(network_calls.get() + 1);
            Ok(fixture("brave_images.json"))
        })
        .map(|c| c.len()))
    };
    // Two searches fit under a 2¢ cap, each recorded at 1¢.
    assert_eq!(search().unwrap().unwrap(), 6);
    assert_eq!(search().unwrap().unwrap(), 6);
    assert_eq!(store.count_usage_log_rows().unwrap(), 2);
    assert_eq!(store.sum_spend_cents_anonymous(0).unwrap(), 2);
    // At the cap: SpendCapError, nothing recorded, the network never touched.
    let Err(ChargeError::SpendCap(err)) = search() else {
        panic!("expected a spend-cap refusal");
    };
    assert_eq!(err.scope, SpendCapScope::User);
    assert_eq!((err.spent_cents, err.cap_cents), (2, 2));
    assert_eq!(network_calls.get(), 2);
    assert_eq!(store.count_usage_log_rows().unwrap(), 2);
}

#[test]
fn is_public_addr_rejects_local_private_and_special_ranges() {
    let refused = [
        "0.0.0.0",
        "0.1.2.3",
        "127.0.0.1",
        "127.255.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "100.127.255.255",
        "224.0.0.1",
        "239.255.255.255",
        "240.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "febf::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "::ffff:169.254.169.254",
        "::ffff:192.168.0.1",
        "::127.0.0.1",
        // Added IPv4 ranges, both edges.
        "192.0.0.0",
        "192.0.0.255",
        "192.0.2.0",
        "192.0.2.255",
        "198.18.0.0",
        "198.19.255.255",
        "198.51.100.0",
        "198.51.100.255",
        "203.0.113.0",
        "203.0.113.255",
        "240.0.0.0",
        "255.255.255.254",
        // IPv6 allowlist: outside 2000::/3, or special-purpose inside it.
        "1fff:ffff::1",
        "4000::1",
        "fec0::1",
        "feff::1",
        "2001::",
        "2001:0:ffff::1",
        "2001:1ff:ffff::1",
        "2001:db8::",
        "2001:db8:ffff:ffff::1",
        "2002::",
        "2002:ffff::1",
        "64:ff9b:1::",
        "64:ff9b:1:ffff::1",
        // NAT64 embedding a refused IPv4.
        "64:ff9b::127.0.0.1",
        "64:ff9b::a9fe:a9fe",
        "64:ff9b::10.0.0.1",
    ];
    for ip in refused {
        assert!(!is_public_addr(ip.parse().unwrap()), "{ip} must be refused");
    }
    let allowed = [
        "8.8.8.8",
        "1.1.1.1",
        "172.15.255.255",
        "172.32.0.1",
        "100.63.255.255",
        "100.128.0.1",
        "169.253.0.1",
        "192.167.1.1",
        "2606:4700::1111",
        "::ffff:8.8.8.8",
        // Edges just outside the added ranges.
        "191.255.255.255",
        "192.0.1.0",
        "192.0.3.0",
        "198.17.255.255",
        "198.20.0.0",
        "198.51.99.255",
        "198.51.101.0",
        "203.0.112.255",
        "203.0.114.0",
        "223.255.255.255",
        "2000::1",
        "3fff:ffff::1",
        "2001:200::1",
        "2001:db7:ffff::1",
        "2001:db9::1",
        "2003::1",
        "64:ff9b::8.8.8.8",
    ];
    for ip in allowed {
        assert!(is_public_addr(ip.parse().unwrap()), "{ip} must be allowed");
    }
}

#[test]
fn fetch_image_refuses_non_public_hosts_and_ports_before_connecting() {
    for url in [
        "https://localhost/x.png",
        "https://127.0.0.1/x.png",
        "https://[::1]/x.png",
        "https://[::ffff:127.0.0.1]/x.png",
        "https://169.254.169.254/latest/meta-data",
        "https://10.0.0.1/x.png",
        // Public address, but not port 443.
        "https://8.8.8.8:8443/x.png",
    ] {
        let err = fetch_image(url).unwrap_err().to_string();
        assert!(err.contains("not a public https address"), "{url}: {err}");
    }
}

#[test]
fn search_results_without_https_image_and_thumbnail_are_dropped() {
    let body = r#"{"results":[
        {"title":"ok","thumbnail":{"src":"https://t/1.jpg"},"properties":{"url":"https://i/1.jpg"}},
        {"title":"http image","thumbnail":{"src":"https://t/2.jpg"},"properties":{"url":"http://i/2.jpg"}},
        {"title":"http thumb","thumbnail":{"src":"http://t/3.jpg"},"properties":{"url":"https://i/3.jpg"}},
        {"title":"no thumb","properties":{"url":"https://i/4.jpg"}}
    ]}"#;
    let titles: Vec<String> = parse_image_search(body)
        .unwrap()
        .into_iter()
        .map(|c| c.title)
        .collect();
    assert_eq!(titles, ["ok"]);
}
