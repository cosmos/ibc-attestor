//! Local HTTP(S) JSON-RPC servers for testing the adapters' TLS configuration.

use std::{sync::Arc, time::Duration};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use rustls::{
    ServerConfig,
    crypto::ring,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use tokio_rustls::TlsAcceptor;

/// A certificate authority that issues test server certificates.
pub struct TestCa {
    cert: rcgen::Certificate,
    key: KeyPair,
}

impl TestCa {
    /// Creates a CA; `name` must be unique so issuers can be told apart.
    pub fn new(name: &str) -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key).unwrap();
        Self { cert, key }
    }

    /// The CA certificate, to be trusted as a root.
    pub fn der(&self) -> &CertificateDer<'static> {
        self.cert.der()
    }

    /// Issues a server certificate for `server_name` (a DNS name or IP address).
    pub fn issue(&self, server_name: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec![server_name.to_string()])
            .unwrap()
            .signed_by(&key, &self.cert, &self.key)
            .unwrap();
        (
            cert.der().clone(),
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
    }

    /// Starts an HTTPS server on `127.0.0.1` with a certificate for
    /// `server_name` issued by this CA, answering every JSON-RPC request with
    /// `result`. Returns the server's port.
    pub async fn serve(&self, server_name: &str, result: Value) -> u16 {
        let (cert, key) = self.issue(server_name);
        let mut config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
        // Like most servers, prefer HTTP/2: a client that offers `h2` without
        // speaking it fails, so tests catch a wrong ALPN setting.
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let result = result.clone();
                tokio::spawn(async move {
                    // The handshake fails for clients that reject the certificate.
                    if let Ok(stream) = acceptor.accept(stream).await {
                        respond(stream, &Reply::Result(result)).await;
                    }
                });
            }
        });
        port
    }
}

/// How a test server answers each request.
#[derive(Clone)]
pub enum Reply {
    /// A JSON-RPC response with this result.
    Result(Value),
    /// A `301` redirect to this URL.
    Redirect(String),
}

/// Starts a plain HTTP server on `127.0.0.1` answering every request with
/// `reply`. Returns the server's port.
pub async fn serve_http(reply: Reply) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let reply = reply.clone();
            tokio::spawn(async move {
                respond(stream, &reply).await;
            });
        }
    });
    port
}

/// A listener nothing should connect to, and an HTTPS URL pointing at it.
pub async fn redirect_target() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "https://localhost:{}/",
        listener.local_addr().unwrap().port()
    );
    (listener, url)
}

/// Asserts that no client connected to `listener`.
pub async fn assert_not_connected(listener: &TcpListener) {
    let connection = timeout(Duration::from_millis(100), listener.accept()).await;
    assert!(connection.is_err(), "client followed the redirect");
}

/// Answers one HTTP/1.1 request with `reply`.
async fn respond(mut stream: impl AsyncRead + AsyncWrite + Unpin, reply: &Reply) -> Option<()> {
    let mut request = Vec::new();
    let mut buf = [0; 4096];
    let body_start = loop {
        let n = stream.read(&mut buf).await.ok().filter(|&n| n > 0)?;
        request.extend_from_slice(&buf[..n]);
        if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..body_start]).to_ascii_lowercase();
    // Requests without a body (and so without `content-length`) have length 0.
    let content_length: usize = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .map_or(Some(0), |len| len.trim().parse().ok())?;
    while request.len() < body_start + content_length {
        let n = stream.read(&mut buf).await.ok().filter(|&n| n > 0)?;
        request.extend_from_slice(&buf[..n]);
    }

    let (status, extra_headers, body) = match reply {
        Reply::Result(result) => {
            let request: Value =
                serde_json::from_slice(&request[body_start..body_start + content_length]).ok()?;
            let body = json!({ "jsonrpc": "2.0", "id": request["id"], "result": result });
            (
                "200 OK",
                "content-type: application/json\r\n".to_string(),
                body.to_string(),
            )
        }
        Reply::Redirect(location) => (
            "301 Moved Permanently",
            format!("location: {location}\r\n"),
            String::new(),
        ),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\n{extra_headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.ok()?;
    stream.shutdown().await.ok()
}
