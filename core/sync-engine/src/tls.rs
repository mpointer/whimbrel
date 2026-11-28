//! The HTTPS connector libsql's remote client uses to reach Turso.
//!
//! libsql (0.9 and 0.10) builds its own connector on hyper-rustls 0.25, which pins
//! rustls 0.22, and rustls 0.22 has open advisories fixed only in 0.23.46. libsql's
//! `tls` feature is turned off (see Cargo.toml) and this connector is handed to
//! `Builder::connector` instead: hyper 0.14's `HttpConnector` for TCP, then
//! tokio-rustls 0.26 (rustls 0.23) for TLS. Same shape as libsql's default: http and
//! https both allowed, HTTP/1.1 only, normal rustls verification with hostname checks.
//!
//! Roots: the OS store on desktop and iOS, as libsql's default did; bundled Mozilla
//! roots (webpki-roots) on Android, which has no store rustls-native-certs can read.
//! Unlike hyper-rustls 0.25's `with_native_roots()`, a failed OS-store read never
//! panics: it falls back to the bundled roots.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use hyper::client::connect::{Connected, Connection};
use hyper::client::HttpConnector;
use hyper::Uri;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Mozilla's trusted roots, bundled into the binary.
pub(crate) fn webpki_root_store() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// The OS trust store, or the bundled roots if it can't be read or is empty.
#[cfg(not(target_os = "android"))]
fn root_store() -> RootCertStore {
    let loaded = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    let (added, _ignored) = roots.add_parsable_certificates(loaded.certs);
    if added == 0 {
        tracing::warn!(
            errors = ?loaded.errors,
            "no OS root certificates loaded; using bundled Mozilla roots"
        );
        return webpki_root_store();
    }
    roots
}

#[cfg(target_os = "android")]
fn root_store() -> RootCertStore {
    webpki_root_store()
}

/// rustls 0.23 needs an explicit crypto provider when more than one is compiled in
/// (other crates enable aws-lc-rs too). ring is named here so the choice doesn't
/// depend on feature unification.
pub(crate) fn client_config(roots: RootCertStore) -> ClientConfig {
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports the default TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

/// The connector for `libsql::Builder::new_remote(..).connector(..)`.
pub(crate) fn https_connector() -> HttpsConnector {
    HttpsConnector::new(client_config(root_store()))
}

#[derive(Clone)]
pub(crate) struct HttpsConnector {
    http: HttpConnector,
    tls: TlsConnector,
}

impl HttpsConnector {
    pub(crate) fn new(config: ClientConfig) -> Self {
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_nodelay(true);
        Self {
            http,
            tls: TlsConnector::from(Arc::new(config)),
        }
    }
}

impl tower_service::Service<Uri> for HttpsConnector {
    type Response = MaybeTls;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<MaybeTls, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        self.http.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let mut http = self.http.clone();
        let tls = self.tls.clone();
        Box::pin(async move {
            let https = match uri.scheme_str() {
                Some("https") => true,
                Some("http") => false,
                other => return Err(format!("unsupported scheme {other:?}").into()),
            };
            let host = uri
                .host()
                .ok_or("URI has no host")?
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string();
            let tcp = http.call(uri).await?;
            if !https {
                return Ok(MaybeTls::Plain(tcp));
            }
            let name = ServerName::try_from(host)?;
            let stream = tls.connect(name, tcp).await?;
            Ok(MaybeTls::Tls(Box::new(stream)))
        })
    }
}

/// A plain TCP stream (http) or a TLS one (https).
pub(crate) enum MaybeTls {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl Connection for MaybeTls {
    fn connected(&self) -> Connected {
        match self {
            MaybeTls::Plain(s) => s.connected(),
            MaybeTls::Tls(s) => s.get_ref().0.connected(),
        }
    }
}

impl AsyncRead for MaybeTls {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTls {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_flush(cx),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_root_store_is_not_empty() {
        assert!(
            webpki_root_store().len() > 100,
            "bundled Mozilla roots missing"
        );
    }

    #[test]
    fn the_os_root_store_loads_or_falls_back() {
        assert!(root_store().len() > 0);
    }

    /// A live handshake against a real host. Ignored by default (needs network).
    #[tokio::test]
    #[ignore]
    async fn a_live_https_connect_completes_the_handshake() {
        use tower_service::Service;
        let mut c = https_connector();
        let s = c
            .call(Uri::from_static("https://example.com"))
            .await
            .expect("TLS handshake");
        assert!(matches!(s, MaybeTls::Tls(_)));
    }
}
