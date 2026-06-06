//! BIP-352 Silent Payments scanner client. Talks to a Frigate-compatible
//! Electrum server via the three custom methods:
//!   - `blockchain.silentpayments.subscribe`
//!   - `blockchain.silentpayments.unsubscribe`
//!   - notifications stream back under method = `blockchain.silentpayments.subscribe`
//!
//! We open a **dedicated TCP/TLS socket per scanning wallet** instead of
//! routing through the shared electrum-client used elsewhere — that client's
//! notification dispatcher only knows about `blockchain.headers.subscribe`
//! and `blockchain.scripthash.subscribe`. Everything else is silently dropped.
//!
//! Lifetime: one `SpScanner` = one connection = one scanning session. Closing
//! the socket effectively unsubscribes (Frigate holds the scan key in RAM
//! only for the session). For an explicit unsubscribe we send the method
//! before close, but it's belt-and-suspenders.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Per-message read cap for the SP-server socket. Comfortably above any legitimate
/// Electrum line (a max-size tx hex is ~2 MB); bounds a hostile server's endless line.
const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;
/// Socket read timeout. Frigate sends keepalives roughly every 5s, so reaching this
/// means the connection has gone silent (a stalled/dead link, often after the machine
/// slept) and we should reconnect.
const IDLE_READ_TIMEOUT_SECS: u64 = 300;

pub struct SpScanConfig {
    /// Same URL format as the standard Electrum config: `ssl://host:port` or
    /// `tcp://host:port`.
    pub url: String,
    pub validate_tls: bool,
    pub ca_cert_path: Option<String>,
    pub danger_accept_invalid_certs: bool,
    pub socks5_proxy: Option<String>,
}

/// Initial response to a successful subscribe call.
#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeResponse {
    pub address: String,
    pub labels: Vec<u32>,
    pub start_height: u64,
}

/// One matching transaction discovered during scanning.
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEntry {
    pub height: u64,
    pub tx_hash: String,
    pub tweak_key: String,
}

/// Frigate's unified notification payload. All three fields are optional in
/// practice — `progress` arrives during scanning, `history` arrives whenever
/// new matches are found, `subscription` echoes the current subscription
/// metadata.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpNotification {
    #[serde(default)]
    pub subscription: Option<SubscribeResponse>,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub history: Option<Vec<HistoryEntry>>,
}

/// Connected client. Drop closes the socket → server discards keys.
pub struct SpScanner {
    stream: BufReader<Stream>,
    next_id: u64,
}

enum Stream {
    Tcp(TcpStream),
    Tls(Box<native_tls::TlsStream<TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Stream {
    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Stream::Tcp(s) => s.write_all(data),
            Stream::Tls(s) => s.write_all(data),
        }
    }
}

impl SpScanner {
    /// Open the connection. Does not subscribe yet.
    pub fn connect(cfg: &SpScanConfig) -> Result<Self> {
        let (scheme, host, port) = parse_url(&cfg.url)?;
        let tcp = open_tcp(&host, port, cfg.socks5_proxy.as_deref())?;
        // Idle timeout: Frigate's scan can be quiet for stretches, but we want
        // eventual detection if the connection silently dies.
        tcp.set_read_timeout(Some(Duration::from_secs(IDLE_READ_TIMEOUT_SECS)))
            .context("set_read_timeout")?;

        let stream = match scheme {
            UrlScheme::Tcp => Stream::Tcp(tcp),
            UrlScheme::Ssl => Stream::Tls(Box::new(tls_upgrade(tcp, &host, cfg)?)),
        };
        let mut s = SpScanner {
            stream: BufReader::new(stream),
            next_id: 1,
        };
        // Electrum protocol requires `server.version` as the first message —
        // ElectrumX/Fulcrum/Frigate all reject anything else with
        // "server.version must be the first message" otherwise.
        s.handshake()?;
        Ok(s)
    }

    fn handshake(&mut self) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "server.version",
            "params": ["Corvin", "1.4"],
        });
        let mut line = serde_json::to_string(&req).context("encode server.version")?;
        line.push('\n');
        self.stream
            .get_mut()
            .write_all(line.as_bytes())
            .context("write server.version")?;
        // Drain to the matching response. Some servers send the version reply
        // back as a single line, others may interleave a banner — skip any
        // notifications that arrive before our id.
        loop {
            let val = self.read_message()?;
            if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = val.get("error") {
                    if !err.is_null() {
                        anyhow::bail!("server.version error: {err}");
                    }
                }
                return Ok(());
            }
        }
    }

    /// Send the subscribe request and wait for the (synchronous) response.
    pub fn subscribe(
        &mut self,
        scan_secret_hex: &str,
        spend_pubkey_hex: &str,
        start: Option<u64>,
    ) -> Result<SubscribeResponse> {
        let id = self.next_id;
        self.next_id += 1;

        let mut params: Vec<serde_json::Value> = vec![
            serde_json::Value::String(scan_secret_hex.to_string()),
            serde_json::Value::String(spend_pubkey_hex.to_string()),
        ];
        if let Some(h) = start {
            params.push(serde_json::Value::Number(h.into()));
        }

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "blockchain.silentpayments.subscribe",
            "params": params,
        });

        let mut line = serde_json::to_string(&req).context("encode subscribe")?;
        line.push('\n');
        self.stream
            .get_mut()
            .write_all(line.as_bytes())
            .context("write subscribe")?;

        // Read until we see our id. Notifications before the response are
        // possible but unlikely on a fresh connection — skip them if any.
        loop {
            let val = self.read_message()?;
            if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = val.get("error") {
                    if !err.is_null() {
                        anyhow::bail!("server error: {err}");
                    }
                }
                let result = val
                    .get("result")
                    .cloned()
                    .context("no result in subscribe response")?;
                return serde_json::from_value(result).context("parsing subscribe response");
            }
        }
    }

    /// Block until the next SP notification arrives (or the connection errors
    /// out). Returns `Ok(None)` on clean EOF.
    ///
    /// Skips keepalives and unrecognized shapes silently — Frigate sends
    /// periodic null-payload notifications between scan updates, and we don't
    /// want each one to trigger a reconnect.
    pub fn next_notification(&mut self) -> Result<Option<SpNotification>> {
        loop {
            let val = match self.try_read_message()? {
                Some(v) => v,
                None => return Ok(None),
            };
            // Skip anything that isn't a notification (responses with an id,
            // server-side errors etc).
            let Some(method) = val.get("method").and_then(|v| v.as_str()) else {
                continue;
            };
            if method != "blockchain.silentpayments.subscribe" {
                tracing::trace!(method, "SP scanner: ignored non-SP notification");
                continue;
            }
            // Per Electrum convention, `params` is a single-element list with
            // the payload object inside. Some servers use an object directly
            // — try both shapes.
            let payload = match val.get("params") {
                Some(serde_json::Value::Array(arr)) => {
                    arr.first().cloned().unwrap_or(serde_json::Value::Null)
                }
                Some(serde_json::Value::Object(obj)) => serde_json::Value::Object(obj.clone()),
                _ => serde_json::Value::Null,
            };
            if payload.is_null() {
                // Keepalive between scan updates — skip without erroring.
                tracing::trace!("SP scanner: keepalive notification");
                continue;
            }
            match serde_json::from_value::<SpNotification>(payload.clone()) {
                Ok(parsed) => return Ok(Some(parsed)),
                Err(e) => {
                    // Don't bail out — just log so we can adapt the parser if
                    // Frigate ships a new payload shape.
                    tracing::warn!(
                        error = %e,
                        raw = %payload,
                        "SP scanner: unrecognized notification payload — skipping",
                    );
                    continue;
                }
            }
        }
    }

    /// Fetch a transaction's raw bytes via `blockchain.transaction.get`. The
    /// SP scanner needs this whenever a `history` notification points at a
    /// match — we have to inspect the tx outputs locally to find which one
    /// is ours and what amount it carries.
    pub fn tx_get(&mut self, txid: &str) -> Result<Vec<u8>> {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "blockchain.transaction.get",
            "params": [txid, false],  // verbose=false → hex string back
        });
        let mut line = serde_json::to_string(&req).context("encode tx_get")?;
        line.push('\n');
        self.stream
            .get_mut()
            .write_all(line.as_bytes())
            .context("write tx_get")?;
        loop {
            let val = self.read_message()?;
            if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = val.get("error") {
                    if !err.is_null() {
                        anyhow::bail!("tx_get error: {err}");
                    }
                }
                let hex_str = val
                    .get("result")
                    .and_then(|v| v.as_str())
                    .context("tx_get response missing result string")?;
                return hex_decode_string(hex_str).context("decoding tx hex");
            }
        }
    }

    /// Best-effort unsubscribe. Drop alone would close the socket and let the
    /// server clean up; this is just polite.
    pub fn unsubscribe(&mut self) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "blockchain.silentpayments.unsubscribe",
            "params": [],
        });
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stream.get_mut().write_all(line.as_bytes())?;
        Ok(())
    }

    fn read_message(&mut self) -> Result<serde_json::Value> {
        let mut buf = String::new();
        // Bound the read: a hostile/compromised SP server must not be able to stream
        // an endless newline-less line and OOM the process. Any legitimate Electrum
        // line (incl. a max-size tx hex) is well under this cap.
        let n = (&mut self.stream)
            .take(MAX_LINE_BYTES)
            .read_line(&mut buf)
            .context("read_line")?;
        if n == 0 {
            anyhow::bail!("server closed the connection");
        }
        serde_json::from_str(&buf).context("parsing server message")
    }

    fn try_read_message(&mut self) -> Result<Option<serde_json::Value>> {
        let mut buf = String::new();
        let n = match (&mut self.stream).take(MAX_LINE_BYTES).read_line(&mut buf) {
            Ok(n) => n,
            // A read timeout (no data, not even a keepalive, within IDLE_READ_TIMEOUT_SECS)
            // means the connection silently died. Surface it as a recognizable, benign
            // reconnect signal rather than a raw OS error (os error 11 / WouldBlock).
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                anyhow::bail!("idle timeout: no data from the SP server in {IDLE_READ_TIMEOUT_SECS}s");
            }
            Err(e) => return Err(anyhow::Error::new(e).context("read_line")),
        };
        if n == 0 {
            return Ok(None);
        }
        let val = serde_json::from_str(&buf).context("parsing server message")?;
        Ok(Some(val))
    }
}

// ── connection helpers ────────────────────────────────────────────────────────

enum UrlScheme {
    Tcp,
    Ssl,
}

fn parse_url(url: &str) -> Result<(UrlScheme, String, u16)> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("ssl://") {
        (UrlScheme::Ssl, r)
    } else if let Some(r) = url.strip_prefix("tcp://") {
        (UrlScheme::Tcp, r)
    } else {
        anyhow::bail!("electrum URL must start with ssl:// or tcp://");
    };
    let (host, port_str) = rest
        .rsplit_once(':')
        .context("electrum URL must include a port")?;
    let port: u16 = port_str.parse().context("invalid port")?;
    Ok((scheme, host.to_string(), port))
}

fn open_tcp(host: &str, port: u16, socks5_proxy: Option<&str>) -> Result<TcpStream> {
    if let Some(proxy) = socks5_proxy {
        Ok(socks::Socks5Stream::connect(proxy, (host, port))
            .map(|s| s.into_inner())
            .with_context(|| format!("socks5 connect via {proxy} to {host}:{port}"))?)
    } else {
        TcpStream::connect((host, port)).with_context(|| format!("tcp connect to {host}:{port}"))
    }
}

fn hex_decode_string(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        anyhow::bail!("hex string has odd length");
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8> {
    Ok(match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => anyhow::bail!("invalid hex byte"),
    })
}

fn tls_upgrade(
    tcp: TcpStream,
    host: &str,
    cfg: &SpScanConfig,
) -> Result<native_tls::TlsStream<TcpStream>> {
    use native_tls::TlsConnector;
    let mut builder = TlsConnector::builder();
    builder.danger_accept_invalid_certs(cfg.danger_accept_invalid_certs);
    // Only relax hostname verification under the explicit danger flag, matching the
    // main Electrum path — not merely because chain validation was turned off.
    builder.danger_accept_invalid_hostnames(cfg.danger_accept_invalid_certs);
    if let Some(path) = &cfg.ca_cert_path {
        let bytes = std::fs::read(path).with_context(|| format!("reading CA cert '{path}'"))?;
        let cert = native_tls::Certificate::from_pem(&bytes)
            .or_else(|_| native_tls::Certificate::from_der(&bytes))
            .with_context(|| format!("parsing CA cert '{path}'"))?;
        builder.add_root_certificate(cert);
    }
    let connector = builder.build().context("building TLS connector")?;
    connector
        .connect(host, tcp)
        .with_context(|| format!("TLS handshake with {host}"))
}
