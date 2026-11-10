use anyhow::{anyhow, bail, Context, Result};
use bytes::{BufMut, BytesMut};
use quick_xml::{events::Event, name::QName, Reader};
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Value};
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant;

use crate::config::RtorrentConfig;

const MAX_SCGI_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const BACKGROUND_RPC_CIRCUIT_DELAY: std::time::Duration = std::time::Duration::from_secs(15);
pub(crate) const MAX_PRESSURE_SCALED_RPC_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(3600);
pub(crate) const MAX_PRESSURE_SCALED_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(90);
const MAX_BACKGROUND_RPC_CIRCUIT_DELAY: std::time::Duration = std::time::Duration::from_secs(120);
const BACKGROUND_RPC_MIN_SPACING: std::time::Duration = std::time::Duration::from_secs(2);
const MAX_BACKGROUND_RPC_MIN_SPACING: std::time::Duration = std::time::Duration::from_secs(30);
const RPC_PRESSURE_RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const MODERATE_IO_PRESSURE_PERCENT: f64 = 5.0;
const HIGH_IO_PRESSURE_PERCENT: f64 = 15.0;
const SEVERE_IO_PRESSURE_PERCENT: f64 = 30.0;
const EXTREME_IO_PRESSURE_PERCENT: f64 = 60.0;
const CRITICAL_IO_PRESSURE_PERCENT: f64 = 80.0;

/// Read the maximum host I/O PSI average across `some`/`full` and the
/// 10/60/300-second windows. Longer windows keep background work damped across
/// short pressure dips; missing or malformed PSI falls back to normal pacing.
pub(crate) fn read_io_pressure_percent() -> Option<f64> {
    let pressure = std::fs::read_to_string("/proc/pressure/io").ok()?;
    let mut maximum = None;

    for line in pressure.lines() {
        let mut fields = line.split_whitespace();
        let Some(kind) = fields.next() else {
            continue;
        };
        if kind != "some" && kind != "full" {
            continue;
        }

        for field in fields {
            let Some((key, raw_value)) = field.split_once('=') else {
                continue;
            };
            if !matches!(key, "avg10" | "avg60" | "avg300") {
                continue;
            }
            if let Ok(percent) = raw_value.parse::<f64>() {
                if percent.is_finite() && percent >= 0.0 {
                    maximum = Some(maximum.map_or(percent, |current: f64| current.max(percent)));
                }
            }
        }
    }

    maximum
}

/// Scale advisory background work with sustained I/O pressure. This never
/// disables requests or waits for pressure to clear.
pub(crate) fn io_pressure_multiplier(io_pressure_percent: Option<f64>) -> u32 {
    match io_pressure_percent {
        Some(percent) if percent >= CRITICAL_IO_PRESSURE_PERCENT => 64,
        Some(percent) if percent >= EXTREME_IO_PRESSURE_PERCENT => 32,
        Some(percent) if percent >= SEVERE_IO_PRESSURE_PERCENT => 16,
        Some(percent) if percent >= HIGH_IO_PRESSURE_PERCENT => 8,
        Some(percent) if percent >= MODERATE_IO_PRESSURE_PERCENT => 4,
        _ => 1,
    }
}

/// Keep polling progress under sustained disk pressure. RPC deadlines use a
/// much larger scale so slow calls can finish, while this separate curve caps
/// the delay between bounded background pages and samples.
fn io_pressure_pacing_multiplier(io_pressure_percent: Option<f64>) -> u32 {
    match io_pressure_percent {
        Some(percent) if percent >= CRITICAL_IO_PRESSURE_PERCENT => 12,
        Some(percent) if percent >= EXTREME_IO_PRESSURE_PERCENT => 8,
        Some(percent) if percent >= SEVERE_IO_PRESSURE_PERCENT => 6,
        Some(percent) if percent >= HIGH_IO_PRESSURE_PERCENT => 4,
        Some(percent) if percent >= MODERATE_IO_PRESSURE_PERCENT => 2,
        _ => 1,
    }
}

pub(crate) fn pressure_scaled_poll_interval(
    base: std::time::Duration,
    io_pressure_percent: Option<f64>,
) -> std::time::Duration {
    let ceiling = base.max(MAX_PRESSURE_SCALED_POLL_INTERVAL);
    base.saturating_mul(io_pressure_pacing_multiplier(io_pressure_percent))
        .min(ceiling)
}

fn pressure_scaled_rpc_timeout(
    base: std::time::Duration,
    io_pressure_percent: Option<f64>,
) -> std::time::Duration {
    let ceiling = base.max(MAX_PRESSURE_SCALED_RPC_TIMEOUT);
    base.saturating_mul(io_pressure_multiplier(io_pressure_percent))
        .min(ceiling)
}

/// Keep an in-flight SCGI request alive when pressure rises after it starts.
/// The pinned future is never cancelled and replayed, so this remains safe for
/// mutating RPC methods as well as reads. Pressure sampling only extends the
/// original deadline, up to the same bounded timeout used for initial pacing.
async fn with_pressure_scaled_deadline<T, F>(
    initial_timeout: std::time::Duration,
    future: F,
) -> std::result::Result<Result<T>, ()>
where
    F: Future<Output = Result<T>>,
{
    let started = Instant::now();
    let mut deadline = started + initial_timeout;
    let mut next_pressure_sample = started + RPC_PRESSURE_RECHECK_INTERVAL;
    tokio::pin!(future);

    loop {
        tokio::select! {
            biased;
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep_until(deadline) => {
                let extended_timeout = pressure_scaled_rpc_timeout(
                    initial_timeout,
                    read_io_pressure_percent(),
                );
                let extended_deadline = started + extended_timeout;
                if extended_deadline > deadline {
                    deadline = extended_deadline;
                } else {
                    return Err(());
                }
            }
            _ = tokio::time::sleep_until(next_pressure_sample) => {
                let extended_timeout = pressure_scaled_rpc_timeout(
                    initial_timeout,
                    read_io_pressure_percent(),
                );
                deadline = deadline.max(started + extended_timeout);
                next_pressure_sample = Instant::now() + RPC_PRESSURE_RECHECK_INTERVAL;
            }
        }
    }
}

fn background_rpc_circuit_delay(io_pressure_percent: Option<f64>) -> std::time::Duration {
    BACKGROUND_RPC_CIRCUIT_DELAY
        .saturating_mul(io_pressure_pacing_multiplier(io_pressure_percent))
        .min(MAX_BACKGROUND_RPC_CIRCUIT_DELAY)
}

/// Space successful background reads under sustained I/O pressure. A single
/// semaphore prevents parallel SCGI calls, but without pacing a burst of fast
/// responses can still keep rTorrent's storage-facing work continuously busy.
fn background_rpc_minimum_spacing(io_pressure_percent: Option<f64>) -> std::time::Duration {
    let multiplier = io_pressure_pacing_multiplier(io_pressure_percent);
    if multiplier == 1 {
        return std::time::Duration::ZERO;
    }

    BACKGROUND_RPC_MIN_SPACING
        .saturating_mul(multiplier)
        .min(MAX_BACKGROUND_RPC_MIN_SPACING)
}

fn background_rpc_busy_retry_delay(io_pressure_percent: Option<f64>) -> std::time::Duration {
    background_rpc_minimum_spacing(io_pressure_percent).max(std::time::Duration::from_secs(1))
}

#[derive(Debug)]
pub(crate) enum RpcBackpressure {
    CircuitOpen(std::time::Duration),
    BackgroundLaneBusy(std::time::Duration),
    RpcGateBusy(std::time::Duration),
}

impl fmt::Display for RpcBackpressure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CircuitOpen(retry_after) => write!(
                f,
                "rTorrent RPC circuit breaker is open for another {} ms",
                retry_after.as_millis()
            ),
            Self::BackgroundLaneBusy(retry_after) => write!(
                f,
                "rTorrent background RPC lane is busy; retry after {} ms",
                retry_after.as_millis()
            ),
            Self::RpcGateBusy(retry_after) => write!(
                f,
                "rTorrent RPC gate is busy; background call deferred for {} ms",
                retry_after.as_millis()
            ),
        }
    }
}

impl std::error::Error for RpcBackpressure {}

pub(crate) fn is_rpc_unavailable_or_backpressured(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RpcBackpressure>().is_some() || is_transient_transport_error(error)
}

pub(crate) fn rpc_backpressure_retry_after(error: &anyhow::Error) -> Option<std::time::Duration> {
    match error.downcast_ref::<RpcBackpressure>()? {
        RpcBackpressure::CircuitOpen(retry_after)
        | RpcBackpressure::BackgroundLaneBusy(retry_after)
        | RpcBackpressure::RpcGateBusy(retry_after) => Some(*retry_after),
    }
}

// XML-RPC responses can be large for a raw metainfo load, but their decoded
// shape must stay bounded independently of the wire size.  These limits also
// protect the JSON-RPC compatibility path, which is converted into XmlValue
// before callers inspect it.
const MAX_XMLRPC_REQUEST_BYTES: usize = 128 * 1024 * 1024;
const MAX_XMLRPC_COLLECTION_ITEMS: usize = 16_384;
const MAX_XMLRPC_VALUE_DEPTH: usize = 64;
const MAX_XMLRPC_NODES: usize = 65_536;
const MAX_XMLRPC_DEPTH: usize = 64;
const MAX_XMLRPC_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_JSONRPC_NODES: usize = 65_536;
const MAX_JSONRPC_DEPTH: usize = 64;
const MAX_JSONRPC_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_JSONRPC_COLLECTION_ITEMS: usize = 16_384;

#[derive(Debug, Clone)]
pub enum Transport {
    Unix(String),
    Tcp(String),
}

#[derive(Clone)]
pub struct Client {
    transport: Transport,
    timeout: std::time::Duration,
    identity_timeout: std::time::Duration,
    rpc_gate: Arc<Semaphore>,
    low_priority_gate: Arc<Mutex<()>>,
    low_priority_next_allowed_at: Arc<Mutex<Option<Instant>>>,
    low_priority_pause_until: Arc<Mutex<Option<Instant>>>,
    low_priority_failures: Arc<Mutex<u8>>,
    tracker_peer_id: Option<String>,
}

impl Client {
    pub fn new(cfg: &RtorrentConfig) -> Result<Self> {
        let transport = match (&cfg.scgi_socket, &cfg.scgi_addr) {
            (Some(path), None) => Transport::Unix(path.clone()),
            (None, Some(addr)) => Transport::Tcp(addr.clone()),
            _ => bail!("exactly one of scgi_socket or scgi_addr must be set"),
        };
        Ok(Self {
            transport,
            timeout: std::time::Duration::from_secs(cfg.timeout_secs),
            identity_timeout: std::time::Duration::from_secs(cfg.identity_timeout_secs),
            rpc_gate: Arc::new(Semaphore::new(1)),
            low_priority_gate: Arc::new(Mutex::new(())),
            low_priority_next_allowed_at: Arc::new(Mutex::new(None)),
            low_priority_pause_until: Arc::new(Mutex::new(None)),
            low_priority_failures: Arc::new(Mutex::new(0)),
            tracker_peer_id: Some(cfg.peer_id.clone()),
        })
    }

    /// Construct a Unix-socket client without a full config — for tests.
    pub fn new_unix(socket_path: &str, timeout_secs: u64) -> Self {
        Self {
            transport: Transport::Unix(socket_path.to_owned()),
            timeout: std::time::Duration::from_secs(timeout_secs),
            identity_timeout: std::time::Duration::from_secs(timeout_secs),
            rpc_gate: Arc::new(Semaphore::new(1)),
            low_priority_gate: Arc::new(Mutex::new(())),
            low_priority_next_allowed_at: Arc::new(Mutex::new(None)),
            low_priority_pause_until: Arc::new(Mutex::new(None)),
            low_priority_failures: Arc::new(Mutex::new(0)),
            tracker_peer_id: None,
        }
    }

    /// Return the resolved per-install tracker identity for commands that
    /// create or resume a download. Test-only clients intentionally have no
    /// identity because they do not own a configured rTorrent instance.
    pub(crate) fn tracker_peer_id(&self) -> Option<&str> {
        self.tracker_peer_id.as_deref()
    }

    /// Execute a single XMLRPC method and return the parsed result.
    pub async fn call(&self, method: &str, args: &[XmlValue]) -> Result<XmlValue> {
        self.call_with_priority(method, args, RpcPriority::User)
            .await
    }

    /// Execute a single XML-RPC method without trying rTorrent's JSON-RPC
    /// adapter first. Some rTorrent mutators return success through JSON-RPC
    /// without applying the state transition.
    pub async fn call_xmlrpc(&self, method: &str, args: &[XmlValue]) -> Result<XmlValue> {
        let _permit = self
            .rpc_gate
            .acquire()
            .await
            .context("rTorrent RPC gate closed")?;
        let request_timeout = pressure_scaled_rpc_timeout(self.timeout, read_io_pressure_percent());
        self.call_xml_with_timeout(method, args, request_timeout)
            .await
    }

    /// Execute a startup/control XML-RPC call with an operation-specific
    /// timeout. Large session rewrites must not force ordinary list, stats, or
    /// user-control calls to wait for the same multi-minute budget.
    pub(crate) async fn call_identity_xmlrpc(
        &self,
        method: &str,
        args: &[XmlValue],
    ) -> Result<XmlValue> {
        let _permit = self
            .rpc_gate
            .acquire()
            .await
            .context("rTorrent RPC gate closed")?;
        let request_timeout =
            pressure_scaled_rpc_timeout(self.identity_timeout, read_io_pressure_percent());
        self.call_xml_with_timeout(method, args, request_timeout)
            .await
    }

    pub async fn call_sync(&self, method: &str, args: &[XmlValue]) -> Result<XmlValue> {
        self.call_with_priority(method, args, RpcPriority::Background)
            .await
    }

    /// Execute many independent, single-argument XMLRPC calls in one round
    /// trip via the standard `system.multicall` meta-method, instead of one
    /// connection + request/response per call. rTorrent processes them in
    /// array order within a single request; each entry reports its own
    /// success/fault independently of the others. Confirmed supported by
    /// this project's patched rTorrent 0.16.11 build (see docs/TRACKER-IDENTITY.md
    /// for the other rTorrent patches this codebase carries).
    ///
    /// `calls` is `(method, single_arg)` pairs, e.g. `("d.stop", hash)`.
    /// Returns one `Result` per input call, in the same order.
    pub async fn call_multicall(
        &self,
        method: &str,
        args: &[String],
    ) -> Result<Vec<Result<XmlValue, String>>> {
        if args.is_empty() {
            return Ok(Vec::new());
        }
        let entries: Vec<XmlValue> = args
            .iter()
            .map(|arg| {
                XmlValue::Struct(vec![
                    ("methodName".to_owned(), XmlValue::String(method.to_owned())),
                    (
                        "params".to_owned(),
                        XmlValue::Array(vec![XmlValue::String(arg.clone())]),
                    ),
                ])
            })
            .collect();

        let _permit = self
            .rpc_gate
            .acquire()
            .await
            .context("rTorrent RPC gate closed")?;
        let request_timeout = pressure_scaled_rpc_timeout(self.timeout, read_io_pressure_percent());
        let response = self
            .call_xml_with_timeout(
                "system.multicall",
                &[XmlValue::Array(entries)],
                request_timeout,
            )
            .await
            .with_context(|| format!("system.multicall({method})"))?;

        parse_multicall_response(response, args.len(), method)
    }

    async fn call_with_priority(
        &self,
        method: &str,
        args: &[XmlValue],
        priority: RpcPriority,
    ) -> Result<XmlValue> {
        // Keep low-priority calls out of unbounded mutex and semaphore
        // queues. Under contention, defer them with a retry hint instead of
        // letting many pollers line up and run back-to-back after slow I/O.
        let _background_gate = if priority == RpcPriority::Background {
            let retry_after = background_rpc_busy_retry_delay(read_io_pressure_percent());
            Some(self.low_priority_gate.try_lock().map_err(|_| {
                anyhow::Error::new(RpcBackpressure::BackgroundLaneBusy(retry_after))
            })?)
        } else {
            None
        };

        if priority == RpcPriority::Background {
            self.check_background_pause().await?;
            self.pace_background_rpc().await;
        }

        let _permit = if priority == RpcPriority::Background {
            let retry_after = background_rpc_busy_retry_delay(read_io_pressure_percent());
            self.rpc_gate
                .try_acquire()
                .map_err(|_| anyhow::Error::new(RpcBackpressure::RpcGateBusy(retry_after)))?
        } else {
            self.rpc_gate
                .acquire()
                .await
                .context("rTorrent RPC gate closed")?
        };

        // A request can time out and open the circuit while this one waits on
        // the single SCGI permit. Re-checking here prevents queued work from
        // bypassing that backpressure signal.
        if priority == RpcPriority::Background {
            self.check_background_pause().await?;
        }

        let io_pressure_percent = read_io_pressure_percent();
        let request_timeout = pressure_scaled_rpc_timeout(self.timeout, io_pressure_percent);
        let call_started = Instant::now();

        let result = match self.call_json(method, args, request_timeout).await {
            Ok(value) => Ok(value),
            Err(json_err) => {
                if is_jsonrpc_unavailable(&json_err) {
                    self.call_xml_with_timeout(method, args, request_timeout)
                        .await
                } else {
                    Err(json_err)
                }
            }
        };
        if priority == RpcPriority::Background {
            let settle_pressure = read_io_pressure_percent()
                .into_iter()
                .chain(io_pressure_percent)
                .reduce(f64::max);
            self.schedule_background_rpc_after(settle_pressure, call_started.elapsed())
                .await;

            match &result {
                Ok(_) => {
                    *self.low_priority_failures.lock().await = 0;
                    *self.low_priority_pause_until.lock().await = None;
                }
                Err(error) if is_transient_transport_error(error) => {
                    let mut failures = self.low_priority_failures.lock().await;
                    let failure_count = (*failures).saturating_add(1);
                    *failures = failure_count;
                    let exponent = failure_count.saturating_sub(1).min(8);
                    let multiplier = 1_u32 << exponent;
                    // A slow failed call can observe more pressure while it
                    // is in flight; use the worst sample across the call for
                    // the retry cooldown rather than the stale start value.
                    let delay = background_rpc_circuit_delay(settle_pressure)
                        .saturating_mul(multiplier)
                        .min(MAX_BACKGROUND_RPC_CIRCUIT_DELAY);
                    *self.low_priority_pause_until.lock().await = Some(Instant::now() + delay);
                }
                Err(_) => {}
            }
        }
        result
    }

    async fn pace_background_rpc(&self) {
        let mut next_allowed_at = self.low_priority_next_allowed_at.lock().await;
        if let Some(next) = *next_allowed_at {
            if next > Instant::now() {
                tokio::time::sleep_until(next).await;
            }
        }
        *next_allowed_at = None;
    }

    async fn schedule_background_rpc_after(
        &self,
        io_pressure_percent: Option<f64>,
        call_duration: std::time::Duration,
    ) {
        let pressure_spacing = background_rpc_minimum_spacing(io_pressure_percent);
        // Long-running RPCs are evidence that the backend is already slow.
        // Give it a full bounded settle period after completion instead of
        // measuring the gap from call start, which can collapse to zero under
        // load or leave the next request stacked immediately behind it.
        let latency_spacing = call_duration.min(MAX_BACKGROUND_RPC_MIN_SPACING);
        let spacing = pressure_spacing.max(latency_spacing);
        let next = (!spacing.is_zero()).then(|| Instant::now() + spacing);
        *self.low_priority_next_allowed_at.lock().await = next;
    }

    async fn check_background_pause(&self) -> Result<()> {
        let mut pause = self.low_priority_pause_until.lock().await;
        if let Some(until) = *pause {
            let now = Instant::now();
            if until > now {
                return Err(anyhow::Error::new(RpcBackpressure::CircuitOpen(
                    until.duration_since(now),
                )));
            }
            *pause = None;
        }
        Ok(())
    }

    async fn call_json(
        &self,
        method: &str,
        args: &[XmlValue],
        timeout: std::time::Duration,
    ) -> Result<XmlValue> {
        if args.iter().any(contains_base64) {
            bail!("JSON-RPC unavailable for XML-RPC base64 payload");
        }
        if method.len() > MAX_XMLRPC_TEXT_BYTES {
            bail!("JSON-RPC method name exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
        }
        if args.len() > MAX_XMLRPC_COLLECTION_ITEMS {
            bail!("JSON-RPC parameter count exceeds {MAX_XMLRPC_COLLECTION_ITEMS} items");
        }
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": args.iter().map(xml_to_json).collect::<Vec<_>>(),
        })
        .to_string();
        if body.len() > MAX_XMLRPC_REQUEST_BYTES {
            bail!("JSON-RPC request exceeds {MAX_XMLRPC_REQUEST_BYTES} byte limit");
        }
        let response = self
            .scgi_roundtrip_with_timeout("application/json", body.as_bytes(), timeout)
            .await
            .with_context(|| format!("JSON-RPC call {method}"))?;
        parse_jsonrpc_response(&response)
    }

    async fn call_xml_with_timeout(
        &self,
        method: &str,
        args: &[XmlValue],
        timeout: std::time::Duration,
    ) -> Result<XmlValue> {
        let body = build_xmlrpc_request(method, args)?;
        let response = self
            .scgi_roundtrip_with_timeout("text/xml", body.as_bytes(), timeout)
            .await
            .with_context(|| format!("XMLRPC call {method}"))?;
        parse_xmlrpc_response(&response)
    }

    async fn scgi_roundtrip_with_timeout(
        &self,
        content_type: &str,
        body: &[u8],
        timeout: std::time::Duration,
    ) -> Result<Vec<u8>> {
        let content_length = body.len();
        let headers = format!(
            "CONTENT_LENGTH\0{content_length}\0SCGI\01\0REQUEST_METHOD\0POST\0\
             REQUEST_URI\0/RPC2\0CONTENT_TYPE\0{content_type}\0"
        );
        let netstring = format!("{}:{},", headers.len(), headers);

        let mut packet = BytesMut::with_capacity(netstring.len() + body.len());
        packet.put(netstring.as_bytes());
        packet.put(body);

        let response = with_pressure_scaled_deadline(timeout, async {
            match &self.transport {
                Transport::Unix(path) => {
                    #[cfg(not(unix))]
                    bail!("Unix SCGI sockets are unsupported on this platform: {path}");

                    #[cfg(unix)]
                    {
                        let mut stream = UnixStream::connect(path)
                            .await
                            .with_context(|| format!("connect to SCGI socket {path}"))?;
                        stream.write_all(&packet).await?;
                        let mut buf = Vec::new();
                        let mut limited = stream.take((MAX_SCGI_RESPONSE_BYTES + 1) as u64);
                        limited.read_to_end(&mut buf).await?;
                        if buf.len() > MAX_SCGI_RESPONSE_BYTES {
                            bail!("SCGI response exceeds {MAX_SCGI_RESPONSE_BYTES} byte limit");
                        }
                        Ok::<_, anyhow::Error>(buf)
                    }
                }
                Transport::Tcp(addr) => {
                    let mut stream = TcpStream::connect(addr)
                        .await
                        .with_context(|| format!("connect to SCGI addr {addr}"))?;
                    stream.write_all(&packet).await?;
                    let mut buf = Vec::new();
                    let mut limited = stream.take((MAX_SCGI_RESPONSE_BYTES + 1) as u64);
                    limited.read_to_end(&mut buf).await?;
                    if buf.len() > MAX_SCGI_RESPONSE_BYTES {
                        bail!("SCGI response exceeds {MAX_SCGI_RESPONSE_BYTES} byte limit");
                    }
                    Ok(buf)
                }
            }
        })
        .await
        .map_err(|()| anyhow!("SCGI call timed out"))??;

        // Strip HTTP headers (everything up to \r\n\r\n)
        let body_start = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(0);

        let body = &response[body_start..];
        if body.iter().all(|byte| byte.is_ascii_whitespace()) {
            return Err(unexpected_scgi_eof(
                "SCGI response body was empty or whitespace-only",
            ));
        }

        Ok(body.to_vec())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RpcPriority {
    User,
    Background,
}

// --- XMLRPC types ---

#[derive(Debug, Clone)]
pub enum XmlValue {
    String(String),
    Base64(String),
    Int(i64),
    Bool(bool),
    Array(Vec<XmlValue>),
    Struct(Vec<(String, XmlValue)>),
    Nil,
}

impl XmlValue {
    pub fn as_str(&self) -> Option<&str> {
        if let XmlValue::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            XmlValue::Int(n) => Some(*n),
            XmlValue::String(s) => s.parse().ok(),
            XmlValue::Base64(s) => s.parse().ok(),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            XmlValue::Bool(b) => Some(*b),
            XmlValue::Int(n) => Some(*n != 0),
            _ => None,
        }
    }
    pub fn into_array(self) -> Vec<XmlValue> {
        if let XmlValue::Array(v) = self {
            v
        } else {
            vec![]
        }
    }

    pub fn try_into_array(self) -> Result<Vec<XmlValue>> {
        match self {
            XmlValue::Array(values) => Ok(values),
            other => bail!("expected XML-RPC array, got {}", describe_xml_value(&other)),
        }
    }
}

impl From<&str> for XmlValue {
    fn from(s: &str) -> Self {
        XmlValue::String(s.to_owned())
    }
}
impl From<String> for XmlValue {
    fn from(s: String) -> Self {
        XmlValue::String(s)
    }
}
impl From<i64> for XmlValue {
    fn from(n: i64) -> Self {
        XmlValue::Int(n)
    }
}
impl From<bool> for XmlValue {
    fn from(b: bool) -> Self {
        XmlValue::Bool(b)
    }
}

fn xml_to_json(value: &XmlValue) -> Value {
    match value {
        XmlValue::String(s) | XmlValue::Base64(s) => Value::String(s.clone()),
        XmlValue::Int(n) => json!(n),
        XmlValue::Bool(b) => json!(b),
        XmlValue::Array(items) => Value::Array(items.iter().map(xml_to_json).collect()),
        XmlValue::Struct(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), xml_to_json(v)))
                .collect(),
        ),
        XmlValue::Nil => Value::Null,
    }
}

fn json_to_xml(value: Value) -> Result<XmlValue> {
    json_to_xml_at_depth(value, 0)
}

fn json_to_xml_at_depth(value: Value, depth: usize) -> Result<XmlValue> {
    if depth > MAX_XMLRPC_VALUE_DEPTH {
        bail!("JSON-RPC value nesting exceeds {MAX_XMLRPC_VALUE_DEPTH} levels");
    }
    Ok(match value {
        Value::Null => XmlValue::Nil,
        Value::Bool(b) => XmlValue::Bool(b),
        Value::Number(n) => XmlValue::Int(
            n.as_i64()
                .or_else(|| n.as_u64().and_then(|value| i64::try_from(value).ok()))
                .ok_or_else(|| anyhow!("JSON-RPC number does not fit in signed 64 bits"))?,
        ),
        Value::String(s) => {
            if s.len() > MAX_XMLRPC_TEXT_BYTES {
                bail!("JSON-RPC text exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
            }
            XmlValue::String(s)
        }
        Value::Array(items) => {
            if items.len() > MAX_XMLRPC_COLLECTION_ITEMS {
                bail!("JSON-RPC array exceeds {MAX_XMLRPC_COLLECTION_ITEMS} items");
            }
            XmlValue::Array(
                items
                    .into_iter()
                    .map(|value| json_to_xml_at_depth(value, depth + 1))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        Value::Object(map) => {
            if map.len() > MAX_XMLRPC_COLLECTION_ITEMS {
                bail!("JSON-RPC object exceeds {MAX_XMLRPC_COLLECTION_ITEMS} members");
            }
            XmlValue::Struct(
                map.into_iter()
                    .map(|(key, value)| {
                        if key.len() > MAX_XMLRPC_TEXT_BYTES {
                            bail!("JSON-RPC object key exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
                        }
                        Ok((key, json_to_xml_at_depth(value, depth + 1)?))
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
        }
    })
}

fn parse_jsonrpc_response(body: &[u8]) -> Result<XmlValue> {
    validate_jsonrpc_bytes(body)?;
    let value: Value =
        serde_json::from_slice(body).context("JSON-RPC response is not valid JSON")?;
    let response = value
        .as_object()
        .ok_or_else(|| anyhow!("JSON-RPC response is not an object"))?;
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        if message.len() > MAX_XMLRPC_TEXT_BYTES {
            bail!("JSON-RPC error message exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
        }
        bail!("JSON-RPC error: {}", message);
    }
    let result = response
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow!("JSON-RPC response has no result"))?;
    json_to_xml(result)
}

/// Validate JSON-RPC structure before `serde_json::from_slice` materializes a
/// `Value` tree. The SCGI byte ceiling alone still permits a response with
/// millions of small values to allocate a much larger in-memory tree.
fn validate_jsonrpc_bytes(body: &[u8]) -> Result<()> {
    let mut state = JsonShapeState { nodes: 0 };
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    deserializer
        .deserialize_any(JsonShapeVisitor {
            depth: 0,
            state: &mut state,
        })
        .context("JSON-RPC response shape is invalid")?;
    deserializer
        .end()
        .context("JSON-RPC response contains trailing data")?;
    Ok(())
}

struct JsonShapeState {
    nodes: usize,
}

impl JsonShapeState {
    fn visit_node<E: de::Error>(&mut self, depth: usize) -> Result<(), E> {
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| E::custom("JSON-RPC response node count overflowed"))?;
        if self.nodes > MAX_JSONRPC_NODES {
            return Err(E::custom(format!(
                "JSON-RPC response exceeds {MAX_JSONRPC_NODES} value limit"
            )));
        }
        if depth > MAX_JSONRPC_DEPTH {
            return Err(E::custom(format!(
                "JSON-RPC response exceeds {MAX_JSONRPC_DEPTH} nesting depth"
            )));
        }
        Ok(())
    }
}

struct JsonShapeSeed<'a> {
    depth: usize,
    state: &'a mut JsonShapeState,
}

impl<'de, 'a> DeserializeSeed<'de> for JsonShapeSeed<'a> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonShapeVisitor {
            depth: self.depth,
            state: self.state,
        })
    }
}

struct RejectJsonValueSeed;

impl<'de> DeserializeSeed<'de> for RejectJsonValueSeed {
    type Value = ();

    fn deserialize<D>(self, _deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Err(de::Error::custom(
            "JSON-RPC collection exceeds its item limit",
        ))
    }
}

struct JsonShapeVisitor<'a> {
    depth: usize,
    state: &'a mut JsonShapeState,
}

impl<'de, 'a> Visitor<'de> for JsonShapeVisitor<'a> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.state.visit_node(self.depth)
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.state.visit_node(self.depth)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.state.visit_node(self.depth)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.state.visit_node(self.depth)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.state.visit_node(self.depth)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_JSONRPC_TEXT_BYTES {
            return Err(E::custom(format!(
                "JSON-RPC string exceeds {MAX_JSONRPC_TEXT_BYTES} bytes"
            )));
        }
        self.state.visit_node(self.depth)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_JSONRPC_TEXT_BYTES {
            return Err(E::custom(format!(
                "JSON-RPC string exceeds {MAX_JSONRPC_TEXT_BYTES} bytes"
            )));
        }
        self.state.visit_node(self.depth)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_JSONRPC_TEXT_BYTES {
            return Err(E::custom(format!(
                "JSON-RPC string exceeds {MAX_JSONRPC_TEXT_BYTES} bytes"
            )));
        }
        self.state.visit_node(self.depth)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.state.visit_node(self.depth)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonShapeVisitor {
            depth: self.depth,
            state: self.state,
        })
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        self.state.visit_node(self.depth)?;
        let mut items = 0;
        loop {
            if items >= MAX_JSONRPC_COLLECTION_ITEMS {
                let _ = sequence.next_element_seed(RejectJsonValueSeed)?;
                break;
            }
            match sequence.next_element_seed(JsonShapeSeed {
                depth: self.depth + 1,
                state: self.state,
            })? {
                Some(()) => items += 1,
                None => break,
            }
        }
        Ok(())
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        self.state.visit_node(self.depth)?;
        let mut fields = 0;
        loop {
            if fields >= MAX_JSONRPC_COLLECTION_ITEMS {
                if map.next_key::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(format!(
                        "JSON-RPC object contains more than {MAX_JSONRPC_COLLECTION_ITEMS} members"
                    )));
                }
                break;
            }
            match map.next_key_seed(JsonKeySeed)? {
                Some(_) => {
                    map.next_value_seed(JsonShapeSeed {
                        depth: self.depth + 1,
                        state: self.state,
                    })?;
                    fields += 1;
                }
                None => break,
            }
        }
        Ok(())
    }
}

struct JsonKeySeed;

impl<'de> DeserializeSeed<'de> for JsonKeySeed {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(JsonKeyVisitor)
    }
}

struct JsonKeyVisitor;

impl<'de> Visitor<'de> for JsonKeyVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded JSON object key")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_JSONRPC_TEXT_BYTES {
            return Err(E::custom(format!(
                "JSON-RPC object key exceeds {MAX_JSONRPC_TEXT_BYTES} bytes"
            )));
        }
        Ok(())
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }
}

fn is_jsonrpc_unavailable(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("JSON-RPC not supported")
        || text.contains("JSON-RPC unavailable for XML-RPC base64 payload")
        || text.contains("method not found: system.listMethods")
        || text.contains("method not found: method.list_keys")
}

fn contains_base64(value: &XmlValue) -> bool {
    match value {
        XmlValue::Base64(_) => true,
        XmlValue::Array(items) => items.iter().any(contains_base64),
        XmlValue::Struct(fields) => fields.iter().any(|(_, value)| contains_base64(value)),
        _ => false,
    }
}

fn is_timeout_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("timed out") || text.contains("deadline has elapsed")
}

fn is_transient_transport_error(error: &anyhow::Error) -> bool {
    if is_timeout_error(error) {
        return true;
    }

    error.chain().any(|cause| {
        cause
            .downcast_ref::<serde_json::Error>()
            .is_some_and(|json_error| json_error.is_eof())
            || cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io_error| {
                    matches!(
                        io_error.kind(),
                        std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::NotConnected
                            | std::io::ErrorKind::NotFound
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::WouldBlock
                    )
                })
    })
}

fn unexpected_scgi_eof(message: &'static str) -> anyhow::Error {
    anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        message,
    ))
}

// --- XMLRPC builder ---

fn build_xmlrpc_request(method: &str, args: &[XmlValue]) -> Result<String> {
    if method.len() > MAX_XMLRPC_TEXT_BYTES {
        bail!("XML-RPC method name exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
    }
    if args.len() > MAX_XMLRPC_COLLECTION_ITEMS {
        bail!("XML-RPC parameter count exceeds {MAX_XMLRPC_COLLECTION_ITEMS} items");
    }
    let mut out = String::from("<?xml version=\"1.0\"?>\n<methodCall>\n");
    out.push_str(&format!(
        "  <methodName>{}</methodName>\n  <params>\n",
        xml_escape(method)
    ));
    for arg in args {
        out.push_str("    <param><value>");
        write_xml_value(&mut out, arg, 0)?;
        out.push_str("</value></param>\n");
    }
    out.push_str("  </params>\n</methodCall>");
    if out.len() > MAX_XMLRPC_REQUEST_BYTES {
        bail!("XML-RPC request exceeds {MAX_XMLRPC_REQUEST_BYTES} byte limit");
    }
    Ok(out)
}

fn write_xml_value(out: &mut String, v: &XmlValue, depth: usize) -> Result<()> {
    if depth > MAX_XMLRPC_VALUE_DEPTH {
        bail!("XML-RPC value nesting exceeds {MAX_XMLRPC_VALUE_DEPTH} levels");
    }
    match v {
        XmlValue::String(s) => {
            if s.len() > MAX_XMLRPC_TEXT_BYTES {
                bail!("XML-RPC text exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
            }
            out.push_str("<string>");
            out.push_str(&xml_escape(s));
            out.push_str("</string>");
        }
        XmlValue::Base64(s) => {
            if s.len() > MAX_XMLRPC_REQUEST_BYTES {
                bail!("XML-RPC base64 value exceeds {MAX_XMLRPC_REQUEST_BYTES} bytes");
            }
            out.push_str("<base64>");
            out.push_str(s);
            out.push_str("</base64>");
        }
        XmlValue::Int(n) => {
            out.push_str(&format!("<i8>{n}</i8>"));
        }
        XmlValue::Bool(b) => {
            out.push_str(&format!("<boolean>{}</boolean>", if *b { 1 } else { 0 }));
        }
        XmlValue::Array(items) => {
            if items.len() > MAX_XMLRPC_COLLECTION_ITEMS {
                bail!("XML-RPC array exceeds {MAX_XMLRPC_COLLECTION_ITEMS} items");
            }
            out.push_str("<array><data>");
            for item in items {
                out.push_str("<value>");
                write_xml_value(out, item, depth + 1)?;
                out.push_str("</value>");
            }
            out.push_str("</data></array>");
        }
        XmlValue::Struct(fields) => {
            if fields.len() > MAX_XMLRPC_COLLECTION_ITEMS {
                bail!("XML-RPC struct exceeds {MAX_XMLRPC_COLLECTION_ITEMS} members");
            }
            out.push_str("<struct>");
            for (k, v) in fields {
                if k.len() > MAX_XMLRPC_TEXT_BYTES {
                    bail!("XML-RPC struct key exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
                }
                out.push_str(&format!("<member><name>{}</name><value>", xml_escape(k)));
                write_xml_value(out, v, depth + 1)?;
                out.push_str("</value></member>");
            }
            out.push_str("</struct>");
        }
        XmlValue::Nil => {
            out.push_str("<nil/>");
        }
    }
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn describe_xml_value(value: &XmlValue) -> String {
    match value {
        XmlValue::String(value) | XmlValue::Base64(value) => value.clone(),
        XmlValue::Int(value) => value.to_string(),
        XmlValue::Bool(value) => value.to_string(),
        XmlValue::Nil => "nil".to_owned(),
        XmlValue::Array(items) => items
            .iter()
            .map(describe_xml_value)
            .collect::<Vec<_>>()
            .join(", "),
        XmlValue::Struct(fields) => fields
            .iter()
            .map(|(key, value)| format!("{key}={}", describe_xml_value(value)))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

// --- XMLRPC parser ---

/// Turn a parsed `system.multicall` response array into one Result per
/// input call. Per the XMLRPC multicall convention: a successful entry is
/// a one-element array wrapping the real return value; a failed entry is a
/// `{faultCode, faultString}` struct. Confirmed against this project's
/// patched rTorrent 0.16.11 build via a raw SCGI probe (both shapes).
fn parse_multicall_response(
    response: XmlValue,
    expected_count: usize,
    method: &str,
) -> Result<Vec<Result<XmlValue, String>>> {
    let items = response.try_into_array()?;
    if items.len() > MAX_XMLRPC_COLLECTION_ITEMS {
        bail!(
            "system.multicall({method}) returned more than {MAX_XMLRPC_COLLECTION_ITEMS} results"
        );
    }
    if items.len() != expected_count {
        bail!(
            "system.multicall({method}) returned {} results for {} calls",
            items.len(),
            expected_count
        );
    }
    Ok(items
        .into_iter()
        .map(|item| match item {
            // Per-call fault: {faultCode, faultString}.
            XmlValue::Struct(fields) => Err(fields
                .into_iter()
                .find(|(k, _)| k == "faultString")
                .and_then(|(_, v)| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown multicall fault".to_owned())),
            // Per-call success: single-element array wrapping the value.
            XmlValue::Array(mut inner) if inner.len() == 1 => Ok(inner.pop().unwrap()),
            XmlValue::Array(_) => {
                Err("multicall success entry did not contain one value".to_owned())
            }
            other => Ok(other),
        })
        .collect())
}

fn parse_xmlrpc_response(xml: &[u8]) -> Result<XmlValue> {
    let xml_str = std::str::from_utf8(xml).context("XMLRPC response not UTF-8")?;
    validate_xmlrpc_shape(xml_str)?;
    let mut reader = Reader::from_str(xml_str);
    reader.config_mut().trim_text(true);

    // Seek to <methodResponse>
    loop {
        match reader.read_event()? {
            Event::Start(e) if e.name().into_inner() == "methodResponse" => break,
            Event::Eof => {
                return Err(unexpected_scgi_eof(
                    "XMLRPC response missing <methodResponse>",
                ))
            }
            _ => {}
        }
    }

    // Check for <fault> vs <params>
    loop {
        match reader.read_event()? {
            Event::Start(e) => match e.name().into_inner() {
                "fault" => {
                    let fault = parse_value(&mut reader)?;
                    bail!(
                        "XMLRPC fault returned by rTorrent: {}",
                        describe_xml_value(&fault)
                    )
                }
                "params" => break,
                _ => {}
            },
            Event::Eof => return Err(unexpected_scgi_eof("unexpected EOF in XMLRPC response")),
            _ => {}
        }
    }

    parse_value(&mut reader)
}

/// Validate the structural complexity before the recursive XML-RPC projection
/// runs. The SCGI byte ceiling alone still permits a deeply nested document or
/// millions of empty elements to exhaust parser stack/CPU resources.
fn validate_xmlrpc_shape(xml: &str) -> Result<()> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut nodes = 0usize;
    loop {
        match reader.read_event()? {
            Event::Start(_) => {
                nodes = nodes.saturating_add(1);
                if nodes > MAX_XMLRPC_NODES {
                    bail!("XML-RPC response exceeds {MAX_XMLRPC_NODES} element limit");
                }
                depth = depth.saturating_add(1);
                if depth > MAX_XMLRPC_DEPTH {
                    bail!("XML-RPC response exceeds {MAX_XMLRPC_DEPTH} nesting depth");
                }
            }
            Event::Empty(_) => {
                nodes = nodes.saturating_add(1);
                if nodes > MAX_XMLRPC_NODES {
                    bail!("XML-RPC response exceeds {MAX_XMLRPC_NODES} element limit");
                }
            }
            Event::Text(text) => {
                if text.as_ref().len() > MAX_XMLRPC_TEXT_BYTES {
                    bail!("XML-RPC text exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
                }
            }
            Event::CData(text) => {
                if text.as_ref().len() > MAX_XMLRPC_TEXT_BYTES {
                    bail!("XML-RPC text exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| anyhow!("XML-RPC response has an unmatched closing element"))?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 {
        return Err(unexpected_scgi_eof(
            "XML-RPC response has an unterminated element",
        ));
    }
    Ok(())
}

fn parse_value(reader: &mut Reader<&[u8]>) -> Result<XmlValue> {
    // Seek to <value>
    loop {
        match reader.read_event()? {
            Event::Start(e) if e.name().into_inner() == "value" => break,
            Event::End(_) => return Err(anyhow!("XML-RPC response missing <value>")),
            Event::Eof => return Err(unexpected_scgi_eof("XML-RPC response ended before <value>")),
            _ => {}
        }
    }
    parse_value_content(reader, 0)
}

fn parse_value_content(reader: &mut Reader<&[u8]>, depth: usize) -> Result<XmlValue> {
    if depth > MAX_XMLRPC_VALUE_DEPTH {
        bail!("XML-RPC value nesting exceeds {MAX_XMLRPC_VALUE_DEPTH} levels");
    }
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                let val = match e.name().into_inner() {
                    "string" => {
                        let text = read_text_string(reader, e.name())?;
                        XmlValue::String(text)
                    }
                    "int" | "i4" | "i8" => {
                        let text = read_text_string(reader, e.name())?;
                        XmlValue::Int(text.trim().parse().with_context(|| {
                            format!("invalid XML-RPC integer value {:?}", text.trim())
                        })?)
                    }
                    "boolean" => {
                        let text = read_text_string(reader, e.name())?;
                        XmlValue::Bool(match text.trim() {
                            "0" => false,
                            "1" => true,
                            value => bail!("invalid XML-RPC boolean value {value:?}"),
                        })
                    }
                    "array" => parse_array(reader, depth + 1)?,
                    "struct" => parse_struct(reader, depth + 1)?,
                    "nil" => {
                        reader.read_text(e.name())?;
                        XmlValue::Nil
                    }
                    _ => {
                        let text = read_text_string(reader, e.name())?;
                        XmlValue::String(text.trim().to_owned())
                    }
                };
                return Ok(val);
            }
            Event::Text(t) => {
                let text = t.into_inner();
                if text.len() > MAX_XMLRPC_TEXT_BYTES {
                    bail!("XML-RPC text exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
                }
                let s = text.trim().to_owned();
                if !s.is_empty() {
                    return Ok(XmlValue::String(s));
                }
            }
            Event::End(_) => return Ok(XmlValue::Nil),
            Event::Eof => return Err(anyhow!("unexpected EOF in value")),
            _ => {}
        }
    }
}

fn parse_array(reader: &mut Reader<&[u8]>, depth: usize) -> Result<XmlValue> {
    if depth > MAX_XMLRPC_VALUE_DEPTH {
        bail!("XML-RPC value nesting exceeds {MAX_XMLRPC_VALUE_DEPTH} levels");
    }
    let mut items = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) if e.name().into_inner() == "value" => {
                if items.len() >= MAX_XMLRPC_COLLECTION_ITEMS {
                    bail!("XML-RPC array exceeds {MAX_XMLRPC_COLLECTION_ITEMS} items");
                }
                items.push(parse_value_content(reader, depth)?);
            }
            Event::End(e) if e.name().into_inner() == "array" => break,
            Event::Eof => return Err(anyhow!("unexpected EOF in XML-RPC array")),
            _ => {}
        }
    }
    Ok(XmlValue::Array(items))
}

fn parse_struct(reader: &mut Reader<&[u8]>, depth: usize) -> Result<XmlValue> {
    if depth > MAX_XMLRPC_VALUE_DEPTH {
        bail!("XML-RPC value nesting exceeds {MAX_XMLRPC_VALUE_DEPTH} levels");
    }
    let mut fields = Vec::new();
    let mut current_name = String::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) => match e.name().into_inner() {
                "name" => {
                    if fields.len() >= MAX_XMLRPC_COLLECTION_ITEMS {
                        bail!(
                            "XML-RPC struct contains more than {MAX_XMLRPC_COLLECTION_ITEMS} members"
                        );
                    }
                    current_name = read_text_string(reader, e.name())?;
                }
                "value" => {
                    if fields.len() >= MAX_XMLRPC_COLLECTION_ITEMS {
                        bail!("XML-RPC struct exceeds {MAX_XMLRPC_COLLECTION_ITEMS} members");
                    }
                    let val = parse_value_content(reader, depth)?;
                    fields.push((std::mem::take(&mut current_name), val));
                }
                _ => {}
            },
            Event::End(e) if e.name().into_inner() == "struct" => break,
            Event::Eof => return Err(anyhow!("unexpected EOF in XML-RPC struct")),
            _ => {}
        }
    }
    Ok(XmlValue::Struct(fields))
}

fn read_text_string(reader: &mut Reader<&[u8]>, end: QName<'_>) -> Result<String> {
    let text = reader.read_text(end)?.into_inner().into_owned();
    if text.len() > MAX_XMLRPC_TEXT_BYTES {
        bail!("XML-RPC text exceeds {MAX_XMLRPC_TEXT_BYTES} byte limit");
    }
    Ok(text)
}

#[cfg(test)]
mod multicall_tests {
    use super::*;

    // Exact shapes observed from a real raw SCGI probe against this
    // project's patched rTorrent 0.16.11 build:
    //   success: <value><array><data><value><i8>0</i8></value></data></array></value>
    //   fault:   <value><struct><member><name>faultCode</name>...
    //                    <member><name>faultString</name>
    //                       <value><string>invalid parameters: invalid target</string>...

    #[test]
    fn all_success_unwraps_each_single_element_array() {
        let response = XmlValue::Array(vec![
            XmlValue::Array(vec![XmlValue::Int(0)]),
            XmlValue::Array(vec![XmlValue::Int(0)]),
        ]);
        let results = parse_multicall_response(response, 2, "d.stop").unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
        assert_eq!(results[0].as_ref().unwrap().as_i64(), Some(0));
    }

    #[test]
    fn per_call_fault_reports_that_hash_without_failing_the_others() {
        let fault = XmlValue::Struct(vec![
            ("faultCode".to_owned(), XmlValue::Int(-500)),
            (
                "faultString".to_owned(),
                XmlValue::String("invalid parameters: invalid target".to_owned()),
            ),
        ]);
        let response = XmlValue::Array(vec![fault, XmlValue::Array(vec![XmlValue::Int(0)])]);
        let results = parse_multicall_response(response, 2, "d.stop").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].as_ref().unwrap_err(),
            "invalid parameters: invalid target"
        );
        assert!(
            results[1].is_ok(),
            "one fault must not affect the other result"
        );
    }

    #[test]
    fn result_count_mismatch_is_an_error_not_a_panic() {
        // Guards the zip() in stop_many()/recheck_many() against ever being
        // fed a shorter results list than the hashes it's paired with.
        let response = XmlValue::Array(vec![XmlValue::Array(vec![XmlValue::Int(0)])]);
        let err = parse_multicall_response(response, 5, "d.stop").unwrap_err();
        assert!(err.to_string().contains("returned 1 results for 5 calls"));
    }

    #[test]
    fn empty_input_short_circuits_without_a_request() {
        let response = XmlValue::Array(vec![]);
        let results = parse_multicall_response(response, 0, "d.stop").unwrap();
        assert!(results.is_empty());
    }

    fn xmlrpc_response(value: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><methodResponse><params><param><value>{value}</value></param></params></methodResponse>"
        )
    }

    #[test]
    fn xml_parser_rejects_oversized_arrays() {
        let item = "<value><int>0</int></value>";
        let items = item.repeat(MAX_XMLRPC_COLLECTION_ITEMS + 1);
        let xml = xmlrpc_response(&format!("<array><data>{items}</data></array>"));

        let error = parse_xmlrpc_response(xml.as_bytes()).unwrap_err();
        assert!(error
            .to_string()
            .contains("XML-RPC array exceeds 16384 items"));
    }

    #[test]
    fn xml_parser_rejects_excessive_nesting() {
        let levels = MAX_XMLRPC_VALUE_DEPTH + 1;
        let mut value = String::from("<array><data><value>");
        for _ in 1..levels {
            value.push_str("<array><data><value>");
        }
        value.push_str("<int>0</int>");
        for _ in 0..levels {
            value.push_str("</value></data></array>");
        }
        let xml = xmlrpc_response(&value);

        let error = parse_xmlrpc_response(xml.as_bytes()).unwrap_err();
        assert!(error
            .to_string()
            .contains("XML-RPC response exceeds 64 nesting depth"));
    }

    #[test]
    fn json_rpc_conversion_rejects_oversized_collections_and_nesting() {
        let items = Value::Array(
            (0..=MAX_XMLRPC_COLLECTION_ITEMS)
                .map(|_| Value::from(0))
                .collect(),
        );
        let error = json_to_xml(items).unwrap_err();
        assert!(error
            .to_string()
            .contains("JSON-RPC array exceeds 16384 items"));

        let error = json_to_xml(Value::String("x".repeat(MAX_XMLRPC_TEXT_BYTES + 1))).unwrap_err();
        assert!(error
            .to_string()
            .contains("JSON-RPC text exceeds 16777216 byte limit"));

        let mut nested = Value::from(0);
        for _ in 0..=MAX_XMLRPC_VALUE_DEPTH {
            nested = Value::Array(vec![nested]);
        }
        let error = json_to_xml(nested).unwrap_err();
        assert!(error
            .to_string()
            .contains("JSON-RPC value nesting exceeds 64 levels"));
    }

    #[test]
    fn xml_builder_rejects_oversized_collections() {
        let args = [XmlValue::Array(vec![
            XmlValue::Int(0);
            MAX_XMLRPC_COLLECTION_ITEMS + 1
        ])];
        let error = build_xmlrpc_request("d.multicall2", &args).unwrap_err();
        assert!(error
            .to_string()
            .contains("XML-RPC array exceeds 16384 items"));
    }
}

#[cfg(all(test, unix))]
mod circuit_breaker_tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn background_timeout_opens_circuit_before_returning() {
        let directory = tempfile::Builder::new()
            .prefix("tng-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket_path = directory.path().join("rpc.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });
        let client = Client::new_unix(socket_path.to_str().unwrap(), 1);

        let timeout = client
            .call_with_priority("d.name", &[], RpcPriority::Background)
            .await
            .expect_err("silent SCGI server should time out");
        assert!(is_timeout_error(&timeout));

        let next = client
            .call_with_priority("d.name", &[], RpcPriority::Background)
            .await
            .expect_err("circuit should be open before timeout returns");
        assert_eq!(next.to_string(), "rTorrent RPC circuit breaker is open");

        server.abort();
        let _ = server.await;
    }
}
