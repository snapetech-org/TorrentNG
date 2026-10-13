//! Async HTTP client for the TorrentNG native API (`/api/v1`).
//!
//! Auth is a Bearer token; the daemon skips CSRF for requests that carry no
//! browser headers, which is exactly what this client sends.

use std::time::Duration;

use reqwest::{header, Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::model::{
    Facets, ListResponse, LiveResponse, LiveRow, TorrentDetail, TransferInfo, ViewSpec,
};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ApiError {
    #[error("authentication failed (check the API token)")]
    Unauthorized,
    #[error("server returned {status}: {message}")]
    Http { status: u16, message: String },
    #[error("cannot reach the server: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Decode(String),
}

impl ApiError {
    /// Snapshot cursors expire server-side (410); callers restart pagination.
    pub fn is_gone(&self) -> bool {
        matches!(self, ApiError::Http { status: 410, .. })
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_decode() {
            ApiError::Decode(e.to_string())
        } else {
            ApiError::Network(e.without_url().to_string())
        }
    }
}

/// Add-torrent source.
#[derive(Debug, Clone)]
pub enum AddSource {
    Magnet(String),
    File { name: String, bytes: Vec<u8> },
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// Accept `host:port` and trailing slashes; default to plain http.
pub fn normalize_base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if url.contains("://") {
        url.to_owned()
    } else {
        format!("http://{url}")
    }
}

impl Client {
    pub fn new(base: &str, token: &str) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(30))
            .user_agent(concat!("torrentng-desktop/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            base: normalize_base(base),
            token: token.trim().to_owned(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn req(&self, method: Method, path: &str) -> RequestBuilder {
        let r = self.http.request(method, format!("{}{}", self.base, path));
        if self.token.is_empty() {
            r
        } else {
            r.bearer_auth(&self.token)
        }
    }

    async fn check(resp: Response) -> Result<Response, ApiError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ApiError::Unauthorized);
        }
        let body = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str().map(str::to_owned)))
            .unwrap_or_else(|| body.chars().take(200).collect());
        Err(ApiError::Http {
            status: status.as_u16(),
            message,
        })
    }

    async fn json<T: DeserializeOwned>(rb: RequestBuilder) -> Result<T, ApiError> {
        let resp = Self::check(rb.send().await?).await?;
        resp.json::<T>()
            .await
            .map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// Public liveness probe (no auth needed).
    pub async fn health(&self) -> Result<(), ApiError> {
        Self::check(self.req(Method::GET, "/health").send().await?)
            .await
            .map(|_| ())
    }

    /// Verifies the token against an authenticated endpoint.
    pub async fn verify(&self) -> Result<(), ApiError> {
        self.transfer_info().await.map(|_| ())
    }

    pub async fn list(
        &self,
        spec: &ViewSpec,
        offset: usize,
        limit: usize,
        snapshot: Option<u64>,
    ) -> Result<ListResponse, ApiError> {
        Self::json(
            self.req(Method::GET, "/api/v1/torrents")
                .query(&spec.query(offset, limit, snapshot)),
        )
        .await
    }

    pub async fn live(&self, hashes: &[String]) -> Result<Vec<LiveRow>, ApiError> {
        let mut out = Vec::with_capacity(hashes.len());
        // The daemon caps one request at 128 hashes.
        for chunk in hashes.chunks(128) {
            let r: LiveResponse = Self::json(
                self.req(Method::GET, "/api/v1/torrents/live")
                    .query(&[("hashes", chunk.join(","))]),
            )
            .await?;
            out.extend(r.torrents);
        }
        Ok(out)
    }

    pub async fn transfer_info(&self) -> Result<TransferInfo, ApiError> {
        Self::json(self.req(Method::GET, "/api/v1/transfer/info")).await
    }

    pub async fn facets(&self) -> Result<Facets, ApiError> {
        Self::json(self.req(Method::GET, "/api/v1/sidebar-facets")).await
    }

    pub async fn detail(&self, hash: &str) -> Result<TorrentDetail, ApiError> {
        Self::json(self.req(Method::GET, &format!("/api/v1/torrents/{hash}"))).await
    }

    /// `action` is one of `start`, `stop`, `recheck`, `reannounce`.
    pub async fn action(&self, hash: &str, action: &str) -> Result<(), ApiError> {
        Self::check(
            self.req(Method::POST, &format!("/api/v1/torrents/{hash}/{action}"))
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    pub async fn remove(&self, hash: &str, delete_files: bool) -> Result<(), ApiError> {
        Self::check(
            self.req(Method::DELETE, &format!("/api/v1/torrents/{hash}"))
                .query(&[("delete_files", delete_files)])
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    pub async fn add(
        &self,
        source: AddSource,
        save_path: &str,
        start: bool,
    ) -> Result<(), ApiError> {
        use reqwest::multipart::{Form, Part};
        let mut form = Form::new().text("start", start.to_string());
        if !save_path.trim().is_empty() {
            form = form.text("save_path", save_path.trim().to_owned());
        }
        form = match source {
            AddSource::Magnet(m) => form.text("magnet", m.trim().to_owned()),
            AddSource::File { name, bytes } => form.part(
                "torrent",
                Part::bytes(bytes)
                    .file_name(name)
                    .mime_str("application/x-bittorrent")
                    .map_err(|e| ApiError::Decode(e.to_string()))?,
            ),
        };
        Self::check(
            self.req(Method::POST, "/api/v1/torrents")
                .multipart(form)
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    /// Open the SSE delta stream, resuming from `revision` so only changes
    /// after the page we already hold are sent.
    pub async fn events(&self, revision: u64) -> Result<Response, ApiError> {
        // No overall timeout on a long-lived stream.
        let rb = self
            .req(Method::GET, "/api/v1/events")
            .query(&[
                ("last_known_revision", revision.to_string()),
                ("batch_size", "500".to_owned()),
            ])
            .header(header::ACCEPT, "text/event-stream")
            .timeout(Duration::from_secs(60 * 60 * 24));
        Self::check(rb.send().await?).await
    }
}

/// One parsed server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Incremental `text/event-stream` parser (handles chunk boundaries, CRLF,
/// comments/keep-alives and multi-line `data:`).
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: String,
    data: Vec<String>,
}

/// Upper bound for one buffered line; a misbehaving peer cannot grow memory.
const MAX_LINE: usize = 8 * 1024 * 1024;

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    out.push(SseEvent {
                        event: std::mem::take(&mut self.event),
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                }
            } else if line.starts_with(':') {
                // comment / keep-alive
            } else {
                let (field, value) = match line.split_once(':') {
                    Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                    None => (&*line, ""),
                };
                match field {
                    "event" => self.event = value.to_owned(),
                    "data" => self.data.push(value.to_owned()),
                    _ => {}
                }
            }
        }
        if self.buf.len() > MAX_LINE {
            self.buf.clear();
            self.data.clear();
            self.event.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_normalization() {
        assert_eq!(normalize_base("nas:8080/"), "http://nas:8080");
        assert_eq!(
            normalize_base(" https://x.example/app// "),
            "https://x.example/app"
        );
    }

    #[test]
    fn sse_handles_split_chunks_crlf_and_keepalive() {
        let mut p = SseParser::default();
        assert!(p
            .feed(b": keep-alive\n\nevent: torrent_delta\nid: 4\nda")
            .is_empty());
        let ev = p.feed(b"ta: {\"a\":1}\r\ndata: x\r\n\r\n");
        assert_eq!(
            ev,
            vec![SseEvent {
                event: "torrent_delta".into(),
                data: "{\"a\":1}\nx".into()
            }]
        );
        let ev = p.feed(b"data: only\n\n");
        assert_eq!(ev[0].event, "");
        assert_eq!(ev[0].data, "only");
    }

    #[test]
    fn sse_caps_runaway_line() {
        let mut p = SseParser::default();
        let big = vec![b'x'; MAX_LINE + 1];
        assert!(p.feed(&big).is_empty());
        assert_eq!(p.feed(b"data: ok\n\n")[0].data, "ok");
    }

    #[test]
    fn debug_never_prints_token() {
        let c = Client::new("http://h", "supersecret").unwrap();
        assert!(!format!("{c:?}").contains("supersecret"));
    }
}
