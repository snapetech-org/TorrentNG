use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rand::RngExt;
use tokio::{
    net::UdpSocket,
    sync::{mpsc, watch, Mutex as TokioMutex},
    task::JoinHandle,
    time::timeout,
};

use crate::{
    deadline::{UtpDeadlineClient, UtpDeadlineScheduler},
    error::UtpError,
    packet::{PacketType, UtpPacket},
    state::{
        InboundAction, UtpConnection, DEFAULT_INITIAL_WINDOW_BYTES, DEFAULT_MTU_PAYLOAD_BYTES,
    },
    window::MAX_WINDOW_PACKETS,
};

const MAX_RECEIVE_BUFFER_BYTES: usize = DEFAULT_INITIAL_WINDOW_BYTES as usize;
pub const MAX_UTP_ENDPOINT_CONNECTIONS: usize = 256;
const MAX_RETIRED_UTP_ENDPOINTS: usize = 2;
const ROUTE_PACKET_QUEUE_CAPACITY: usize = MAX_WINDOW_PACKETS;
/// Maximum UDP payload accepted by an IPv4/IPv6 socket. Keeping the public
/// configuration below this bound also prevents a malformed setting from
/// turning every listener or stream into an unbounded allocation.
pub const MAX_UDP_DATAGRAM_LEN: usize = 65_507;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtpTransportConfig {
    pub handshake_timeout: Duration,
    pub io_timeout: Duration,
    pub max_datagram_len: usize,
    pub max_retransmits: usize,
}

impl Default for UtpTransportConfig {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(10),
            io_timeout: Duration::from_secs(15),
            max_datagram_len: DEFAULT_MTU_PAYLOAD_BYTES + crate::HEADER_SIZE + 64,
            max_retransmits: 4,
        }
    }
}

pub struct UtpListener {
    socket: UdpSocket,
    config: UtpTransportConfig,
}

#[derive(Clone)]
pub struct UtpEndpoint {
    socket: Arc<UdpSocket>,
    config: UtpTransportConfig,
    streams: Arc<StdMutex<HashMap<UtpRouteKey, UtpRoute>>>,
    accepted_rx: Arc<TokioMutex<mpsc::Receiver<Result<UtpStream, UtpError>>>>,
    stop: watch::Sender<bool>,
    recv_task: Arc<StdMutex<Option<JoinHandle<()>>>>,
    deadlines: UtpDeadlineScheduler,
    accept_new_connections: Arc<AtomicBool>,
}

/// Engine-owned uTP socket pool.
///
/// A runtime keeps one UDP endpoint per address family and multiplexes all
/// peer streams over each socket. This avoids creating one UDP socket and
/// receive task for every outbound peer while still keeping route state
/// bounded by `MAX_UTP_ENDPOINT_CONNECTIONS`.
#[derive(Clone)]
pub struct UtpRuntime {
    endpoints: Arc<TokioMutex<UtpRuntimeEndpoints>>,
    config: UtpTransportConfig,
}

#[derive(Default)]
struct UtpRuntimeEndpoints {
    ipv4: Option<UtpEndpoint>,
    ipv6: Option<UtpEndpoint>,
    retired: Vec<UtpEndpoint>,
    shutdown: bool,
}

pub struct UtpStream {
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    conn: UtpConnection,
    config: UtpTransportConfig,
    last_remote_timestamp_us: u32,
    read_buf: Vec<u8>,
    reorder: ReorderBuffer,
    pending_send: VecDeque<PendingPacket>,
    routed_rx: Option<mpsc::Receiver<UtpPacket>>,
    deadline: Option<UtpDeadlineClient>,
    handshake_state: Option<UtpPacket>,
    route: Option<UtpRouteRegistration>,
}

struct PendingPacket {
    sequence: u16,
    packet: UtpPacket,
    retransmits: usize,
}

struct ReorderBuffer {
    next_sequence: u16,
    slots: Option<Vec<Option<(u16, Vec<u8>)>>>,
    buffered_bytes: usize,
    buffered_packets: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ReorderInsert {
    Buffered,
    Ready(Vec<u8>),
    Rejected,
}

impl ReorderBuffer {
    fn new(next_sequence: u16) -> Self {
        Self {
            next_sequence,
            slots: None,
            buffered_bytes: 0,
            buffered_packets: 0,
        }
    }

    fn buffered_bytes(&self) -> usize {
        self.buffered_bytes
    }

    fn insert(&mut self, sequence: u16, payload: Vec<u8>) -> ReorderInsert {
        let distance = sequence.wrapping_sub(self.next_sequence);
        if distance >= 0x8000 || usize::from(distance) >= MAX_WINDOW_PACKETS {
            return ReorderInsert::Rejected;
        }

        if distance != 0 {
            let slots = self
                .slots
                .get_or_insert_with(|| (0..MAX_WINDOW_PACKETS).map(|_| None).collect());
            let slot = usize::from(sequence) % slots.len();
            if slots[slot].is_some() {
                return ReorderInsert::Rejected;
            }
            let Some(buffered_bytes) = self.buffered_bytes.checked_add(payload.len()) else {
                return ReorderInsert::Rejected;
            };
            self.buffered_bytes = buffered_bytes;
            self.buffered_packets += 1;
            slots[slot] = Some((sequence, payload));
            return ReorderInsert::Buffered;
        }

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let mut ready = payload;
        while let Some(slots) = self.slots.as_mut() {
            let slot = usize::from(self.next_sequence) % slots.len();
            let Some((present_sequence, _)) = slots[slot].as_ref() else {
                break;
            };
            if *present_sequence != self.next_sequence {
                break;
            }
            let Some((_, payload)) = slots[slot].take() else {
                break;
            };
            self.buffered_bytes = self.buffered_bytes.saturating_sub(payload.len());
            self.buffered_packets = self.buffered_packets.saturating_sub(1);
            ready.extend_from_slice(&payload);
            self.next_sequence = self.next_sequence.wrapping_add(1);
        }
        if self.buffered_packets == 0 {
            self.slots = None;
        }
        ReorderInsert::Ready(ready)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UtpStats {
    pub connects: u64,
    pub accepts: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub send_timeouts: u64,
    pub recv_timeouts: u64,
    pub retransmits: u64,
    pub route_drops: u64,
    pub rtt_samples: u64,
    pub rtt_us: u64,
    pub rtt_min_us: u64,
    pub rtt_max_us: u64,
    pub rtt_var_us: u64,
    pub retransmit_timeout_us: u64,
    pub congestion_window_bytes: u64,
    pub congestion_base_delay_us: u64,
    pub congestion_current_delay_us: u64,
    pub bytes_in_flight: u64,
}

static UTP_CONNECTS: AtomicU64 = AtomicU64::new(0);
static UTP_ACCEPTS: AtomicU64 = AtomicU64::new(0);
static UTP_BYTES_SENT: AtomicU64 = AtomicU64::new(0);
static UTP_BYTES_RECEIVED: AtomicU64 = AtomicU64::new(0);
static UTP_SEND_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static UTP_RECV_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static UTP_RETRANSMITS: AtomicU64 = AtomicU64::new(0);
static UTP_ROUTE_DROPS: AtomicU64 = AtomicU64::new(0);
static UTP_RTT_SAMPLES: AtomicU64 = AtomicU64::new(0);
static UTP_RTT_US: AtomicU64 = AtomicU64::new(0);
static UTP_RTT_MIN_US: AtomicU64 = AtomicU64::new(0);
static UTP_RTT_MAX_US: AtomicU64 = AtomicU64::new(0);
static UTP_RTT_VAR_US: AtomicU64 = AtomicU64::new(0);
static UTP_RETRANSMIT_TIMEOUT_US: AtomicU64 = AtomicU64::new(0);
static UTP_CONGESTION_WINDOW_BYTES: AtomicU64 = AtomicU64::new(0);
static UTP_CONGESTION_BASE_DELAY_US: AtomicU64 = AtomicU64::new(0);
static UTP_CONGESTION_CURRENT_DELAY_US: AtomicU64 = AtomicU64::new(0);
static UTP_BYTES_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

pub fn stats_snapshot() -> UtpStats {
    UtpStats {
        connects: UTP_CONNECTS.load(Ordering::Relaxed),
        accepts: UTP_ACCEPTS.load(Ordering::Relaxed),
        bytes_sent: UTP_BYTES_SENT.load(Ordering::Relaxed),
        bytes_received: UTP_BYTES_RECEIVED.load(Ordering::Relaxed),
        send_timeouts: UTP_SEND_TIMEOUTS.load(Ordering::Relaxed),
        recv_timeouts: UTP_RECV_TIMEOUTS.load(Ordering::Relaxed),
        retransmits: UTP_RETRANSMITS.load(Ordering::Relaxed),
        route_drops: UTP_ROUTE_DROPS.load(Ordering::Relaxed),
        rtt_samples: UTP_RTT_SAMPLES.load(Ordering::Relaxed),
        rtt_us: UTP_RTT_US.load(Ordering::Relaxed),
        rtt_min_us: UTP_RTT_MIN_US.load(Ordering::Relaxed),
        rtt_max_us: UTP_RTT_MAX_US.load(Ordering::Relaxed),
        rtt_var_us: UTP_RTT_VAR_US.load(Ordering::Relaxed),
        retransmit_timeout_us: UTP_RETRANSMIT_TIMEOUT_US.load(Ordering::Relaxed),
        congestion_window_bytes: UTP_CONGESTION_WINDOW_BYTES.load(Ordering::Relaxed),
        congestion_base_delay_us: UTP_CONGESTION_BASE_DELAY_US.load(Ordering::Relaxed),
        congestion_current_delay_us: UTP_CONGESTION_CURRENT_DELAY_US.load(Ordering::Relaxed),
        bytes_in_flight: UTP_BYTES_IN_FLIGHT.load(Ordering::Relaxed),
    }
}

fn observe_connection(conn: &UtpConnection) {
    UTP_RETRANSMIT_TIMEOUT_US.store(conn.retransmit_timeout_us(), Ordering::Relaxed);
    UTP_CONGESTION_WINDOW_BYTES.store(conn.congestion_window_bytes() as u64, Ordering::Relaxed);
    UTP_BYTES_IN_FLIGHT.store(conn.bytes_in_flight() as u64, Ordering::Relaxed);
    if let Some(rtt) = conn.rtt_us() {
        let rtt = rtt as u64;
        UTP_RTT_SAMPLES.fetch_add(1, Ordering::Relaxed);
        UTP_RTT_US.store(rtt, Ordering::Relaxed);
        record_nonzero_min(&UTP_RTT_MIN_US, rtt);
        UTP_RTT_MAX_US.fetch_max(rtt, Ordering::Relaxed);
    }
    if let Some(rtt_var) = conn.rtt_var_us() {
        UTP_RTT_VAR_US.store(rtt_var as u64, Ordering::Relaxed);
    }
    if let Some(base_delay) = conn.congestion_base_delay_us() {
        UTP_CONGESTION_BASE_DELAY_US.store(base_delay as u64, Ordering::Relaxed);
    }
    if let Some(current_delay) = conn.congestion_current_delay_us() {
        UTP_CONGESTION_CURRENT_DELAY_US.store(current_delay as u64, Ordering::Relaxed);
    }
}

fn record_nonzero_min(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    loop {
        if current != 0 && current <= value {
            return;
        }
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

fn validate_config(config: UtpTransportConfig) -> Result<(), UtpError> {
    if config.max_datagram_len < crate::HEADER_SIZE {
        return Err(UtpError::InvalidConfig(
            "max_datagram_len must include the uTP header",
        ));
    }
    if config.max_datagram_len > MAX_UDP_DATAGRAM_LEN {
        return Err(UtpError::InvalidConfig(
            "max_datagram_len exceeds the maximum UDP payload",
        ));
    }
    Ok(())
}

impl std::fmt::Debug for UtpStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UtpStream")
            .field("peer", &self.peer)
            .field("state", &self.conn.state())
            .field("ids", &self.conn.ids())
            .field("routed", &self.routed_rx.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UtpRouteKey {
    peer: SocketAddr,
    recv_connection_id: u16,
}

struct UtpRoute {
    tx: mpsc::Sender<UtpPacket>,
    token: Arc<()>,
    handshake_state: Option<UtpPacket>,
}

struct UtpRouteRegistration {
    key: UtpRouteKey,
    token: Arc<()>,
    streams: Arc<StdMutex<HashMap<UtpRouteKey, UtpRoute>>>,
}

impl Drop for UtpRouteRegistration {
    fn drop(&mut self) {
        let mut streams = lock_routes(&self.streams);
        let remove = streams
            .get(&self.key)
            .map(|route| Arc::ptr_eq(&route.token, &self.token))
            .unwrap_or(false);
        if remove {
            streams.remove(&self.key);
        }
    }
}

fn lock_routes(
    streams: &StdMutex<HashMap<UtpRouteKey, UtpRoute>>,
) -> StdMutexGuard<'_, HashMap<UtpRouteKey, UtpRoute>> {
    match streams.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl UtpListener {
    pub async fn bind(addr: SocketAddr) -> Result<Self, UtpError> {
        Self::bind_with_config(addr, UtpTransportConfig::default()).await
    }

    pub async fn bind_with_config(
        addr: SocketAddr,
        config: UtpTransportConfig,
    ) -> Result<Self, UtpError> {
        validate_config(config)?;
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|err| UtpError::Io(err.to_string()))?;
        Ok(Self { socket, config })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, UtpError> {
        self.socket
            .local_addr()
            .map_err(|err| UtpError::Io(err.to_string()))
    }

    pub async fn accept(self) -> Result<UtpStream, UtpError> {
        let mut buf = vec![0u8; self.config.max_datagram_len.saturating_add(1)];
        loop {
            let (len, peer) = timeout(
                self.config.handshake_timeout,
                self.socket.recv_from(&mut buf),
            )
            .await
            .map_err(|_| UtpError::Timeout)?
            .map_err(|err| UtpError::Io(err.to_string()))?;
            if len > self.config.max_datagram_len {
                continue;
            }
            let packet = match UtpPacket::parse(&buf[..len]) {
                Ok(packet) => packet,
                Err(_error) => {
                    // Invalid UDP input is a datagram-level drop. Returning
                    // the parse error from this one-shot listener would let
                    // an unauthenticated packet permanently abort the next
                    // valid incoming handshake.
                    UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            if packet.header.packet_type != PacketType::Syn {
                continue;
            }

            self.socket
                .connect(peer)
                .await
                .map_err(|err| UtpError::Io(err.to_string()))?;
            let mut conn = UtpConnection::accept(&packet.header, random_seq_nr())?;
            let state = conn.build_state(now_us(), timestamp_diff_us(packet.header.timestamp_us));
            send_packet(&self.socket, &state).await?;
            observe_connection(&conn);
            UTP_ACCEPTS.fetch_add(1, Ordering::Relaxed);
            let reorder = ReorderBuffer::new(conn.ack_nr().wrapping_add(1));
            return Ok(UtpStream {
                socket: Arc::new(self.socket),
                peer,
                conn,
                config: self.config,
                last_remote_timestamp_us: packet.header.timestamp_us,
                read_buf: Vec::new(),
                reorder,
                pending_send: VecDeque::new(),
                routed_rx: None,
                deadline: None,
                handshake_state: Some(state),
                route: None,
            });
        }
    }
}

impl UtpEndpoint {
    pub async fn bind(addr: SocketAddr) -> Result<Self, UtpError> {
        Self::bind_with_config(addr, UtpTransportConfig::default()).await
    }

    pub async fn bind_with_config(
        addr: SocketAddr,
        config: UtpTransportConfig,
    ) -> Result<Self, UtpError> {
        validate_config(config)?;
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|err| UtpError::Io(err.to_string()))?;
        let socket = Arc::new(socket);
        let streams = Arc::new(StdMutex::new(HashMap::new()));
        let (accepted_tx, accepted_rx) = mpsc::channel(256);
        let (stop, stop_rx) = watch::channel(false);
        let deadlines = UtpDeadlineScheduler::new(stop_rx.clone());
        let accept_new_connections = Arc::new(AtomicBool::new(true));
        let recv_task = tokio::spawn(run_endpoint_recv(
            socket.clone(),
            config,
            streams.clone(),
            accepted_tx,
            Arc::clone(&accept_new_connections),
            deadlines.clone(),
            stop_rx,
        ));
        Ok(Self {
            socket,
            config,
            streams,
            accepted_rx: Arc::new(TokioMutex::new(accepted_rx)),
            stop,
            recv_task: Arc::new(StdMutex::new(Some(recv_task))),
            deadlines,
            accept_new_connections,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, UtpError> {
        self.socket
            .local_addr()
            .map_err(|err| UtpError::Io(err.to_string()))
    }

    pub async fn accept(&self) -> Result<UtpStream, UtpError> {
        timeout(self.config.handshake_timeout, async {
            self.accepted_rx
                .lock()
                .await
                .recv()
                .await
                .ok_or(UtpError::Closed)?
        })
        .await
        .map_err(|_| UtpError::Timeout)?
    }

    fn active_connections(&self) -> usize {
        lock_routes(&self.streams).len()
    }

    /// Stop admitting fresh inbound SYNs while keeping established routes
    /// alive across a peer-listener port rebind.
    pub async fn retire_listener(&self) {
        self.accept_new_connections.store(false, Ordering::Release);
        let mut accepted = self.accepted_rx.lock().await;
        while let Ok(pending) = accepted.try_recv() {
            drop(pending);
        }
    }

    /// Open an outbound stream through this endpoint's shared UDP socket.
    /// The peer address must use the same IP family as the bound endpoint.
    pub async fn connect(&self, peer: SocketAddr) -> Result<UtpStream, UtpError> {
        let local = self.local_addr()?;
        if local.is_ipv4() != peer.is_ipv4() {
            return Err(UtpError::InvalidConfig(
                "shared endpoint and peer must use the same IP family",
            ));
        }

        let mut conn = UtpConnection::connect(random_connection_id(), random_seq_nr());
        let syn = conn.build_syn(now_us());
        let key = UtpRouteKey {
            peer,
            recv_connection_id: conn.ids().recv,
        };
        let (tx, rx) = mpsc::channel(ROUTE_PACKET_QUEUE_CAPACITY);
        let token = Arc::new(());
        {
            let mut streams = lock_routes(&self.streams);
            if streams.len() >= MAX_UTP_ENDPOINT_CONNECTIONS {
                return Err(UtpError::ConnectionLimit {
                    max: MAX_UTP_ENDPOINT_CONNECTIONS,
                });
            }
            if streams.contains_key(&key) {
                return Err(UtpError::InvalidConfig(
                    "random uTP connection id collided with a live route",
                ));
            }
            streams.insert(
                key,
                UtpRoute {
                    tx,
                    token: Arc::clone(&token),
                    handshake_state: None,
                },
            );
        }
        let route = UtpRouteRegistration {
            key,
            token,
            streams: Arc::clone(&self.streams),
        };
        let mut stream = UtpStream {
            socket: Arc::clone(&self.socket),
            peer,
            conn,
            config: self.config,
            last_remote_timestamp_us: 0,
            read_buf: Vec::new(),
            reorder: ReorderBuffer::new(0),
            pending_send: VecDeque::new(),
            routed_rx: Some(rx),
            deadline: Some(self.deadlines.client()),
            handshake_state: None,
            route: Some(route),
        };

        let mut established = false;
        for _ in 0..=self.config.max_retransmits {
            stream.send_packet(&syn).await?;
            match stream.recv_packet(self.config.handshake_timeout).await {
                Ok(packet) => {
                    stream.conn.on_inbound(&packet, now_us())?;
                    observe_connection(&stream.conn);
                    stream.last_remote_timestamp_us = packet.header.timestamp_us;
                    if stream.conn.is_established() {
                        established = true;
                        break;
                    }
                }
                Err(UtpError::Timeout) => continue,
                Err(error) => return Err(error),
            }
        }
        if !established {
            return Err(UtpError::Timeout);
        }
        stream.reorder = ReorderBuffer::new(stream.conn.ack_nr().wrapping_add(1));
        UTP_CONNECTS.fetch_add(1, Ordering::Relaxed);
        Ok(stream)
    }

    /// Stop the endpoint receive loop and wait for it to release its socket.
    ///
    /// The endpoint owns a shared UDP socket, so merely dropping the task
    /// handle is insufficient: the receive task would otherwise keep the
    /// socket bound until the runtime happens to tear it down.
    pub async fn shutdown(&self) -> Result<(), UtpError> {
        let _ = self.stop.send(true);
        let recv_task = match self.recv_task.lock() {
            Ok(mut task) => task.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(recv_task) = recv_task {
            recv_task.await.map_err(|error| {
                if error.is_panic() {
                    UtpError::ReceiveTaskPanicked
                } else {
                    UtpError::ReceiveTaskCancelled
                }
            })?;
        }
        self.deadlines.wait_stopped().await?;
        Ok(())
    }
}

impl Default for UtpRuntime {
    fn default() -> Self {
        Self::with_config(UtpTransportConfig::default())
    }
}

impl UtpRuntime {
    pub fn with_config(config: UtpTransportConfig) -> Self {
        Self {
            endpoints: Arc::new(TokioMutex::new(UtpRuntimeEndpoints::default())),
            config,
        }
    }

    /// Install the listener endpoint so accepted and outgoing streams share
    /// the same socket, receive loop, and route table. Call before starting
    /// peer tasks; a family can be installed only once.
    pub async fn install_endpoint(&self, endpoint: UtpEndpoint) -> Result<(), UtpError> {
        let local = endpoint.local_addr()?;
        let mut endpoints = self.endpoints.lock().await;
        if endpoints.shutdown {
            return Err(UtpError::Closed);
        }
        let slot = if local.is_ipv4() {
            &mut endpoints.ipv4
        } else {
            &mut endpoints.ipv6
        };
        if let Some(current) = slot {
            if current.local_addr()? == local {
                return Ok(());
            }
            return Err(UtpError::InvalidConfig(
                "a different endpoint is already installed for this IP family",
            ));
        }
        *slot = Some(endpoint);
        Ok(())
    }

    /// Replace the endpoint for one family after a listener rebind.
    pub async fn replace_endpoint(&self, endpoint: UtpEndpoint) -> Result<(), UtpError> {
        let local = endpoint.local_addr()?;
        let mut endpoints = self.endpoints.lock().await;
        if endpoints.shutdown {
            return Err(UtpError::Closed);
        }
        let mut idle_retired = Vec::new();
        let mut active_retired = Vec::new();
        for retired in endpoints.retired.drain(..) {
            if retired.active_connections() == 0 {
                idle_retired.push(retired);
            } else {
                active_retired.push(retired);
            }
        }
        endpoints.retired = active_retired;
        let slot = if local.is_ipv4() {
            &mut endpoints.ipv4
        } else {
            &mut endpoints.ipv6
        };
        let previous = slot.replace(endpoint);
        drop(endpoints);
        for retired in idle_retired {
            let _ = retired.shutdown().await;
        }
        if let Some(previous) = previous.as_ref() {
            previous.retire_listener().await;
        }
        if let Some(previous) = previous {
            let mut endpoints = self.endpoints.lock().await;
            if endpoints.shutdown {
                drop(endpoints);
                previous.shutdown().await?;
                return Err(UtpError::Closed);
            }
            let evicted = (endpoints.retired.len() >= MAX_RETIRED_UTP_ENDPOINTS)
                .then(|| endpoints.retired.remove(0));
            endpoints.retired.push(previous);
            drop(endpoints);
            if let Some(evicted) = evicted {
                // Repeated listener-port changes must not retain an
                // unbounded number of old socket generations. If the small
                // retirement budget is exhausted, close the oldest one.
                let _ = evicted.shutdown().await;
            }
        }
        Ok(())
    }

    /// Connect using the shared endpoint for the peer's address family.
    /// Missing families bind one ephemeral wildcard socket on first use.
    pub async fn connect(&self, peer: SocketAddr) -> Result<UtpStream, UtpError> {
        let (endpoint, idle_retired) = {
            let mut endpoints = self.endpoints.lock().await;
            if endpoints.shutdown {
                return Err(UtpError::Closed);
            }
            let mut idle_retired = Vec::new();
            let mut active_retired = Vec::new();
            for retired in endpoints.retired.drain(..) {
                if retired.active_connections() == 0 {
                    idle_retired.push(retired);
                } else {
                    active_retired.push(retired);
                }
            }
            endpoints.retired = active_retired;
            let slot = if peer.is_ipv4() {
                &mut endpoints.ipv4
            } else {
                &mut endpoints.ipv6
            };
            if slot.is_none() {
                let bind_addr = if peer.is_ipv4() {
                    "0.0.0.0:0"
                } else {
                    "[::]:0"
                };
                let bind_addr = bind_addr
                    .parse()
                    .map_err(|_| UtpError::InvalidConfig("invalid uTP wildcard address"))?;
                let endpoint = UtpEndpoint::bind_with_config(bind_addr, self.config).await?;
                // Ephemeral sockets created only for outbound work are not
                // listeners. Refuse unsolicited SYNs instead of retaining
                // accepted streams that no actor will ever consume.
                endpoint.retire_listener().await;
                *slot = Some(endpoint);
            }
            (slot.as_ref().ok_or(UtpError::Closed)?.clone(), idle_retired)
        };
        for retired in idle_retired {
            retired.shutdown().await?;
        }
        endpoint.connect(peer).await
    }

    /// Stop every shared receive loop. The engine calls this after peer
    /// actors have stopped; outstanding streams then observe a closed route.
    pub async fn shutdown(&self) -> Result<(), UtpError> {
        let endpoints = {
            let mut endpoints = self.endpoints.lock().await;
            endpoints.shutdown = true;
            let mut owned = Vec::with_capacity(endpoints.retired.len() + 2);
            owned.extend(endpoints.ipv4.take());
            owned.extend(endpoints.ipv6.take());
            owned.append(&mut endpoints.retired);
            owned
        };
        for endpoint in endpoints {
            endpoint.shutdown().await?;
        }
        Ok(())
    }
}

impl Drop for UtpEndpoint {
    fn drop(&mut self) {
        // Explicit shutdown is used by the engine listener. Keep a best
        // effort abort fallback for callers that drop the last endpoint
        // handle without awaiting shutdown.
        if Arc::strong_count(&self.recv_task) != 1 {
            return;
        }
        let _ = self.stop.send(true);
        let mut recv_task = match self.recv_task.lock() {
            Ok(task) => task,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(task) = recv_task.take() {
            task.abort();
        }
    }
}

async fn run_endpoint_recv(
    socket: Arc<UdpSocket>,
    config: UtpTransportConfig,
    streams: Arc<StdMutex<HashMap<UtpRouteKey, UtpRoute>>>,
    accepted_tx: mpsc::Sender<Result<UtpStream, UtpError>>,
    accept_new_connections: Arc<AtomicBool>,
    deadlines: UtpDeadlineScheduler,
    mut stop: watch::Receiver<bool>,
) {
    let mut buf = vec![0u8; config.max_datagram_len.saturating_add(1)];
    loop {
        let recv_result = tokio::select! {
            stop_result = stop.changed() => {
                if stop_result.is_err() || *stop.borrow() {
                    break;
                }
                continue;
            }
            result = socket.recv_from(&mut buf) => result,
        };
        let (len, peer) = match recv_result {
            Ok(result) => result,
            Err(err) => {
                let _ = send_accepted(&accepted_tx, Err(UtpError::Io(err.to_string())), &mut stop)
                    .await;
                break;
            }
        };
        if len > config.max_datagram_len {
            UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let packet = match UtpPacket::parse(&buf[..len]) {
            Ok(packet) => packet,
            Err(_error) => {
                // Invalid UDP input is a route-level drop, not an endpoint
                // failure. Sending it through the bounded accept queue lets
                // an unauthenticated peer consume the queue and block valid
                // SYNs behind its parse errors.
                UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        let key = UtpRouteKey {
            peer,
            recv_connection_id: packet.header.connection_id,
        };
        if packet.header.packet_type != PacketType::Syn {
            let tx = {
                let streams = lock_routes(&streams);
                streams.get(&key).map(|route| route.tx.clone())
            };
            if let Some(tx) = tx {
                if tx.try_send(packet).is_err() {
                    UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
            }
            continue;
        }

        // A SYN can be retransmitted when the handshake STATE was lost. If
        // the route already exists, resend the original STATE instead of
        // replacing the live stream and orphaning its receive queue.
        if let Some(handshake_state) = {
            let streams = lock_routes(&streams);
            streams.get(&key).map(|route| route.handshake_state.clone())
        } {
            if let Some(state) = handshake_state {
                if send_packet_to(&socket, peer, &state).await.is_err() {
                    break;
                }
            } else {
                // This route belongs to an outbound handshake. A simultaneous
                // inbound SYN with the same route key must not replace it.
                UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
            }
            continue;
        }

        if !accept_new_connections.load(Ordering::Acquire) {
            UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        if lock_routes(&streams).len() >= MAX_UTP_ENDPOINT_CONNECTIONS {
            UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        let mut conn = match UtpConnection::accept(&packet.header, random_seq_nr()) {
            Ok(conn) => conn,
            Err(error) => {
                if !send_accepted(&accepted_tx, Err(error), &mut stop).await {
                    break;
                }
                continue;
            }
        };
        let state = conn.build_state(now_us(), timestamp_diff_us(packet.header.timestamp_us));
        let (tx, rx) = mpsc::channel(ROUTE_PACKET_QUEUE_CAPACITY);
        let key = UtpRouteKey {
            peer,
            recv_connection_id: conn.ids().recv,
        };
        let token = Arc::new(());
        let admitted = {
            let mut streams = lock_routes(&streams);
            if streams.len() >= MAX_UTP_ENDPOINT_CONNECTIONS {
                false
            } else {
                streams.insert(
                    key,
                    UtpRoute {
                        tx,
                        token: Arc::clone(&token),
                        handshake_state: Some(state.clone()),
                    },
                );
                true
            }
        };
        if !admitted {
            UTP_ROUTE_DROPS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let route = UtpRouteRegistration {
            key,
            token,
            streams: Arc::clone(&streams),
        };
        if let Err(error) = send_packet_to(&socket, peer, &state).await {
            drop(route);
            if !send_accepted(&accepted_tx, Err(error), &mut stop).await {
                break;
            }
            continue;
        }

        let reorder = ReorderBuffer::new(conn.ack_nr().wrapping_add(1));
        let stream = UtpStream {
            socket: socket.clone(),
            peer,
            conn,
            config,
            last_remote_timestamp_us: packet.header.timestamp_us,
            read_buf: Vec::new(),
            reorder,
            pending_send: VecDeque::new(),
            routed_rx: Some(rx),
            deadline: Some(deadlines.client()),
            handshake_state: Some(state),
            route: Some(route),
        };
        observe_connection(&stream.conn);
        UTP_ACCEPTS.fetch_add(1, Ordering::Relaxed);
        if !send_accepted(&accepted_tx, Ok(stream), &mut stop).await {
            break;
        }
    }
}

async fn send_accepted(
    accepted_tx: &mpsc::Sender<Result<UtpStream, UtpError>>,
    stream: Result<UtpStream, UtpError>,
    stop: &mut watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        result = accepted_tx.send(stream) => result.is_ok(),
        changed = stop.changed() => changed.is_ok() && !*stop.borrow(),
    }
}

impl UtpStream {
    pub async fn connect(peer: SocketAddr) -> Result<Self, UtpError> {
        Self::connect_with_config(peer, UtpTransportConfig::default()).await
    }

    pub async fn connect_with_config(
        peer: SocketAddr,
        config: UtpTransportConfig,
    ) -> Result<Self, UtpError> {
        validate_config(config)?;
        let bind_addr = if peer.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|err| UtpError::Io(err.to_string()))?;
        socket
            .connect(peer)
            .await
            .map_err(|err| UtpError::Io(err.to_string()))?;
        let mut conn = UtpConnection::connect(random_connection_id(), random_seq_nr());
        let syn = conn.build_syn(now_us());
        let mut packet = None;
        for _ in 0..=config.max_retransmits {
            send_packet(&socket, &syn).await?;
            match recv_packet(&socket, config.handshake_timeout, config.max_datagram_len).await {
                Ok(received) => {
                    packet = Some(received);
                    break;
                }
                Err(UtpError::Timeout) => continue,
                Err(err) => return Err(err),
            }
        }
        let packet = packet.ok_or(UtpError::Timeout)?;
        conn.on_inbound(&packet, now_us())?;
        observe_connection(&conn);
        if !conn.is_established() {
            return Err(UtpError::InvalidStatePacket {
                state: conn.state(),
                packet_type: packet.header.packet_type,
            });
        }
        UTP_CONNECTS.fetch_add(1, Ordering::Relaxed);
        let reorder = ReorderBuffer::new(conn.ack_nr().wrapping_add(1));
        Ok(Self {
            socket: Arc::new(socket),
            peer,
            conn,
            config,
            last_remote_timestamp_us: packet.header.timestamp_us,
            read_buf: Vec::new(),
            reorder,
            pending_send: VecDeque::new(),
            routed_rx: None,
            deadline: None,
            handshake_state: None,
            route: None,
        })
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    pub fn connection(&self) -> &UtpConnection {
        &self.conn
    }

    pub async fn send(&mut self, payload: &[u8]) -> Result<(), UtpError> {
        if payload.is_empty() {
            return Ok(());
        }
        if !self.pending_send.is_empty() {
            self.flush_pending_send().await?;
        }

        let mut offset = 0;

        loop {
            self.pending_send
                .retain(|pending| self.conn.packet_is_unacknowledged(pending.sequence));
            while offset < payload.len() && self.pending_send.len() < MAX_WINDOW_PACKETS {
                // The remote window, congestion window, and local retransmit
                // budget all constrain admission. A zero window blocks until
                // a peer STATE update arrives.
                let available =
                    usize::try_from(self.conn.available_send_window()).unwrap_or(usize::MAX);
                let chunk_len = (payload.len() - offset)
                    .min(DEFAULT_MTU_PAYLOAD_BYTES)
                    .min(available);
                if chunk_len == 0 {
                    break;
                }

                let packet = self.conn.build_data(
                    now_us(),
                    timestamp_diff_us(self.last_remote_timestamp_us),
                    payload[offset..offset + chunk_len].to_vec(),
                )?;
                let sequence = packet.header.seq_nr;
                self.pending_send.push_back(PendingPacket {
                    sequence,
                    packet,
                    retransmits: 0,
                });
                let packet = &self.pending_send.back().expect("just inserted").packet;
                self.send_packet(packet).await?;
                offset += chunk_len;
            }

            self.pending_send
                .retain(|pending| self.conn.packet_is_unacknowledged(pending.sequence));
            if offset == payload.len() && self.pending_send.is_empty() {
                return Ok(());
            }

            let wait = if self.pending_send.is_empty() {
                self.config.io_timeout
            } else {
                Duration::from_micros(self.conn.retransmit_timeout_us())
            };
            let received = match self.recv_packet(wait).await {
                Ok(received) => received,
                Err(UtpError::Timeout) => {
                    UTP_SEND_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
                    if self.pending_send.is_empty() {
                        return Err(UtpError::Timeout);
                    }
                    self.conn.on_timeout();
                    observe_connection(&self.conn);
                    let socket = Arc::clone(&self.socket);
                    let peer = self.peer;
                    let routed = self.routed_rx.is_some();
                    for pending in &mut self.pending_send {
                        if !self.conn.packet_is_unacknowledged(pending.sequence) {
                            continue;
                        }
                        if pending.retransmits >= self.config.max_retransmits {
                            return Err(UtpError::Timeout);
                        }
                        pending.retransmits += 1;
                        UTP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
                        send_packet_using(&socket, peer, routed, &pending.packet).await?;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };

            let action = self.process_inbound_packet(received).await?;
            self.pending_send
                .retain(|pending| self.conn.packet_is_unacknowledged(pending.sequence));
            match action {
                InboundAction::Close | InboundAction::Reset => return Err(UtpError::Closed),
                _ => {}
            }
        }
    }

    async fn flush_pending_send(&mut self) -> Result<(), UtpError> {
        while !self.pending_send.is_empty() {
            let received = match self
                .recv_packet(Duration::from_micros(self.conn.retransmit_timeout_us()))
                .await
            {
                Ok(received) => received,
                Err(UtpError::Timeout) => {
                    UTP_SEND_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
                    self.conn.on_timeout();
                    observe_connection(&self.conn);
                    let socket = Arc::clone(&self.socket);
                    let peer = self.peer;
                    let routed = self.routed_rx.is_some();
                    for pending in &mut self.pending_send {
                        if !self.conn.packet_is_unacknowledged(pending.sequence) {
                            continue;
                        }
                        if pending.retransmits >= self.config.max_retransmits {
                            return Err(UtpError::Timeout);
                        }
                        pending.retransmits += 1;
                        UTP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
                        send_packet_using(&socket, peer, routed, &pending.packet).await?;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            match self.process_inbound_packet(received).await? {
                InboundAction::Close | InboundAction::Reset => return Err(UtpError::Closed),
                _ => {}
            }
        }
        Ok(())
    }

    async fn process_inbound_packet(
        &mut self,
        mut packet: UtpPacket,
    ) -> Result<InboundAction, UtpError> {
        self.last_remote_timestamp_us = packet.header.timestamp_us;
        let mut action = self.conn.on_inbound(&packet, now_us())?;
        self.pending_send
            .retain(|pending| self.conn.packet_is_unacknowledged(pending.sequence));
        match action {
            InboundAction::DeliverPayload | InboundAction::BufferPayload => {
                let sequence = packet.header.seq_nr;
                let in_order = action == InboundAction::DeliverPayload;
                let actual = self
                    .read_buf
                    .len()
                    .saturating_add(self.reorder.buffered_bytes())
                    .saturating_add(packet.payload.len());
                if actual > MAX_RECEIVE_BUFFER_BYTES {
                    self.conn.reject_inbound_sequence(sequence, in_order);
                    action = InboundAction::SendState;
                } else {
                    match self
                        .reorder
                        .insert(sequence, std::mem::take(&mut packet.payload))
                    {
                        ReorderInsert::Ready(payload) => {
                            self.buffer_inbound_payload(&payload)?;
                            self.update_receive_window();
                        }
                        ReorderInsert::Buffered => self.update_receive_window(),
                        ReorderInsert::Rejected => {
                            self.conn.reject_inbound_sequence(sequence, in_order);
                            action = InboundAction::SendState;
                        }
                    }
                }
                let response = self
                    .conn
                    .build_state(now_us(), timestamp_diff_us(packet.header.timestamp_us));
                self.send_packet(&response).await?;
            }
            InboundAction::SendState => {
                self.update_receive_window();
                let response = self
                    .conn
                    .build_state(now_us(), timestamp_diff_us(packet.header.timestamp_us));
                self.send_packet(&response).await?;
            }
            InboundAction::Close => {
                self.update_receive_window();
                let response = self
                    .conn
                    .build_state(now_us(), timestamp_diff_us(packet.header.timestamp_us));
                self.send_packet(&response).await?;
                self.conn.mark_closed();
                return Ok(InboundAction::Close);
            }
            InboundAction::Reset => {
                self.conn.mark_closed();
                return Ok(InboundAction::Reset);
            }
            InboundAction::None => {}
        }
        observe_connection(&self.conn);
        Ok(action)
    }

    pub async fn write_all(&mut self, bytes: &[u8]) -> Result<(), UtpError> {
        self.send(bytes).await
    }

    pub async fn read_exact(&mut self, out: &mut [u8]) -> Result<(), UtpError> {
        let mut filled = 0;
        while filled < out.len() {
            if self.read_buf.is_empty() && !self.receive_into_buffer().await? {
                return Err(UtpError::Closed);
            }
            let n = (out.len() - filled).min(self.read_buf.len());
            out[filled..filled + n].copy_from_slice(&self.read_buf[..n]);
            self.read_buf.drain(..n);
            self.advertise_receive_window().await?;
            filled += n;
        }
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>, UtpError> {
        if !self.read_buf.is_empty() {
            return self.release_read_buffer().await;
        }
        if !self.receive_into_buffer().await? {
            return Ok(Vec::new());
        }
        self.release_read_buffer().await
    }

    async fn receive_into_buffer(&mut self) -> Result<bool, UtpError> {
        loop {
            let packet = self.recv_packet(self.config.io_timeout).await?;
            self.last_remote_timestamp_us = packet.header.timestamp_us;
            if packet.header.packet_type == PacketType::Syn
                && packet.header.connection_id == self.conn.ids().recv
            {
                if let Some(state) = self.handshake_state.as_ref() {
                    self.send_packet(state).await?;
                    continue;
                }
            }
            match self.process_inbound_packet(packet).await? {
                InboundAction::DeliverPayload => return Ok(!self.read_buf.is_empty()),
                InboundAction::Close | InboundAction::Reset => return Ok(false),
                InboundAction::None | InboundAction::SendState | InboundAction::BufferPayload => {}
            }
        }
    }

    fn buffer_inbound_payload(&mut self, payload: &[u8]) -> Result<(), UtpError> {
        let actual = self
            .read_buf
            .len()
            .saturating_add(self.reorder.buffered_bytes())
            .saturating_add(payload.len());
        if actual > MAX_RECEIVE_BUFFER_BYTES {
            self.conn.set_local_window_bytes(0);
            return Err(UtpError::ReceiveBufferFull {
                max: MAX_RECEIVE_BUFFER_BYTES,
                actual,
            });
        }
        self.read_buf.extend_from_slice(payload);
        self.update_receive_window();
        UTP_BYTES_RECEIVED.fetch_add(payload.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    fn update_receive_window(&mut self) {
        let used = self
            .read_buf
            .len()
            .saturating_add(self.reorder.buffered_bytes());
        let available = MAX_RECEIVE_BUFFER_BYTES.saturating_sub(used);
        self.conn
            .set_local_window_bytes(u32::try_from(available).unwrap_or(u32::MAX));
    }

    async fn advertise_receive_window(&mut self) -> Result<(), UtpError> {
        self.update_receive_window();
        if matches!(
            self.conn.state(),
            crate::state::ConnectionState::Closed | crate::state::ConnectionState::Reset
        ) {
            return Ok(());
        }
        let state = self
            .conn
            .build_state(now_us(), timestamp_diff_us(self.last_remote_timestamp_us));
        self.send_packet(&state).await
    }

    async fn release_read_buffer(&mut self) -> Result<Vec<u8>, UtpError> {
        let payload = std::mem::take(&mut self.read_buf);
        self.update_receive_window();
        if payload.is_empty() {
            return Ok(payload);
        }

        let mut guard = ReadBufferReleaseGuard {
            stream: self,
            payload: Some(payload),
        };
        let state = guard.stream.conn.build_state(
            now_us(),
            timestamp_diff_us(guard.stream.last_remote_timestamp_us),
        );
        guard.stream.send_packet(&state).await?;
        let payload = guard.payload.take().expect("release guard owns payload");
        drop(guard);
        Ok(payload)
    }

    pub async fn close(&mut self) -> Result<(), UtpError> {
        self.flush_pending_send().await?;
        let fin = self
            .conn
            .build_fin(now_us(), timestamp_diff_us(self.last_remote_timestamp_us));
        let mut acknowledged = false;
        for attempt in 0..=self.config.max_retransmits {
            if attempt > 0 {
                UTP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
            }
            self.send_packet(&fin).await?;
            loop {
                let received = match self
                    .recv_packet(Duration::from_micros(self.conn.retransmit_timeout_us()))
                    .await
                {
                    Ok(received) => received,
                    Err(UtpError::Timeout) => {
                        self.conn.on_timeout();
                        observe_connection(&self.conn);
                        UTP_SEND_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                    Err(err) => return Err(err),
                };
                match self.process_inbound_packet(received).await? {
                    InboundAction::Close | InboundAction::Reset => return Ok(()),
                    _ => {}
                }
                if self.conn.all_sent_packets_acked() {
                    acknowledged = true;
                    break;
                }
            }
            if acknowledged {
                break;
            }
        }
        if !acknowledged {
            return Err(UtpError::Timeout);
        }
        self.conn.mark_closed();
        Ok(())
    }

    async fn send_packet(&self, packet: &UtpPacket) -> Result<(), UtpError> {
        send_packet_using(&self.socket, self.peer, self.routed_rx.is_some(), packet).await
    }

    async fn recv_packet(&mut self, wait: Duration) -> Result<UtpPacket, UtpError> {
        if let (Some(rx), Some(deadline)) = (&mut self.routed_rx, &mut self.deadline) {
            deadline.recv_packet(rx, wait).await
        } else if let Some(rx) = &mut self.routed_rx {
            timeout(wait, rx.recv())
                .await
                .map_err(|_| UtpError::Timeout)?
                .ok_or(UtpError::Closed)
        } else {
            recv_packet(&self.socket, wait, self.config.max_datagram_len).await
        }
    }
}

struct ReadBufferReleaseGuard<'a> {
    stream: &'a mut UtpStream,
    payload: Option<Vec<u8>>,
}

impl Drop for ReadBufferReleaseGuard<'_> {
    fn drop(&mut self) {
        if let Some(payload) = self.payload.take() {
            self.stream.read_buf = payload;
            self.stream.update_receive_window();
        }
    }
}

impl Drop for UtpStream {
    fn drop(&mut self) {
        // The token prevents an old stream from deleting a newer route if the
        // same peer/connection-id pair is ever reused.
        drop(self.route.take());
    }
}

async fn send_packet(socket: &UdpSocket, packet: &UtpPacket) -> Result<(), UtpError> {
    let bytes = packet.encode()?;
    socket
        .send(&bytes)
        .await
        .map(|written| {
            UTP_BYTES_SENT.fetch_add(written as u64, Ordering::Relaxed);
        })
        .map_err(|err| UtpError::Io(err.to_string()))
}

async fn send_packet_using(
    socket: &UdpSocket,
    peer: SocketAddr,
    routed: bool,
    packet: &UtpPacket,
) -> Result<(), UtpError> {
    if routed {
        send_packet_to(socket, peer, packet).await
    } else {
        send_packet(socket, packet).await
    }
}

async fn send_packet_to(
    socket: &UdpSocket,
    peer: SocketAddr,
    packet: &UtpPacket,
) -> Result<(), UtpError> {
    let bytes = packet.encode()?;
    socket
        .send_to(&bytes, peer)
        .await
        .map(|written| {
            UTP_BYTES_SENT.fetch_add(written as u64, Ordering::Relaxed);
        })
        .map_err(|err| UtpError::Io(err.to_string()))
}

async fn recv_packet(
    socket: &UdpSocket,
    wait: Duration,
    max_datagram_len: usize,
) -> Result<UtpPacket, UtpError> {
    let mut buf = vec![0u8; max_datagram_len.saturating_add(1)];
    let len = timeout(wait, socket.recv(&mut buf))
        .await
        .map_err(|_| {
            UTP_RECV_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
            UtpError::Timeout
        })?
        .map_err(|err| UtpError::Io(err.to_string()))?;
    if len > max_datagram_len {
        return Err(UtpError::DatagramTooLarge {
            actual: len,
            max: max_datagram_len,
        });
    }
    UtpPacket::parse(&buf[..len])
}

fn random_connection_id() -> u16 {
    rand::rng().random()
}

fn random_seq_nr() -> u16 {
    rand::rng().random()
}

fn now_us() -> u32 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    elapsed.as_micros() as u32
}

fn timestamp_diff_us(remote_timestamp_us: u32) -> u32 {
    now_us().wrapping_sub(remote_timestamp_us)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> UtpTransportConfig {
        UtpTransportConfig {
            handshake_timeout: Duration::from_secs(2),
            io_timeout: Duration::from_secs(2),
            max_datagram_len: 2048,
            max_retransmits: 1,
        }
    }

    #[test]
    fn reorder_buffer_emits_contiguous_payloads_in_order_across_wraparound() {
        let mut reorder = ReorderBuffer::new(u16::MAX);

        assert!(matches!(
            reorder.insert(0, b"b".to_vec()),
            ReorderInsert::Buffered
        ));
        assert_eq!(reorder.buffered_bytes(), 1);
        assert_eq!(
            reorder.insert(u16::MAX, b"a".to_vec()),
            ReorderInsert::Ready(b"ab".to_vec())
        );
        assert_eq!(reorder.buffered_bytes(), 0);
    }

    #[test]
    fn reorder_buffer_rejects_duplicate_and_out_of_window_packets() {
        let mut reorder = ReorderBuffer::new(10);
        assert!(matches!(
            reorder.insert(12, b"future".to_vec()),
            ReorderInsert::Buffered
        ));
        assert!(matches!(
            reorder.insert(12, b"duplicate".to_vec()),
            ReorderInsert::Rejected
        ));
        assert!(matches!(
            reorder.insert(10 + MAX_WINDOW_PACKETS as u16, vec![]),
            ReorderInsert::Rejected
        ));
        assert_eq!(reorder.buffered_bytes(), 6);
    }

    #[tokio::test]
    async fn stream_releases_reordered_payloads_only_after_the_gap_arrives() {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let syn = crate::packet::UtpHeader {
            packet_type: PacketType::Syn,
            version: 1,
            extension: 0,
            connection_id: 50,
            timestamp_us: 0,
            timestamp_diff: 0,
            wnd_size: DEFAULT_INITIAL_WINDOW_BYTES,
            seq_nr: 99,
            ack_nr: 0,
        };
        let conn = UtpConnection::accept(&syn, 5).unwrap();
        let reorder = ReorderBuffer::new(conn.ack_nr().wrapping_add(1));
        let (_tx, rx) = mpsc::channel(1);
        let mut stream = UtpStream {
            socket,
            peer: "127.0.0.1:9".parse().unwrap(),
            conn,
            config: test_config(),
            last_remote_timestamp_us: 0,
            read_buf: Vec::new(),
            reorder,
            pending_send: VecDeque::new(),
            routed_rx: Some(rx),
            deadline: None,
            handshake_state: None,
            route: None,
        };

        let mut future = inbound_data_packet(stream.conn.ids().recv, 101, b"later");
        assert_eq!(
            stream.process_inbound_packet(future.clone()).await.unwrap(),
            InboundAction::BufferPayload
        );
        assert!(stream.read_buf.is_empty());
        assert_eq!(stream.reorder.buffered_bytes(), 5);

        future.header.seq_nr = 100;
        future.payload = b"first".to_vec();
        assert_eq!(
            stream.process_inbound_packet(future).await.unwrap(),
            InboundAction::DeliverPayload
        );
        assert_eq!(stream.read_buf, b"firstlater");
        assert_eq!(stream.reorder.buffered_bytes(), 0);
    }

    fn inbound_data_packet(connection_id: u16, seq_nr: u16, payload: &[u8]) -> UtpPacket {
        UtpPacket {
            header: crate::packet::UtpHeader {
                packet_type: PacketType::Data,
                version: 1,
                extension: 0,
                connection_id,
                timestamp_us: 1,
                timestamp_diff: 0,
                wnd_size: DEFAULT_INITIAL_WINDOW_BYTES,
                seq_nr,
                ack_nr: 0,
            },
            extensions: Vec::new(),
            payload: payload.to_vec(),
        }
    }

    #[tokio::test]
    async fn rejects_invalid_datagram_configuration_before_binding() {
        let mut config = test_config();
        config.max_datagram_len = crate::HEADER_SIZE - 1;
        assert!(matches!(
            UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), config).await,
            Err(UtpError::InvalidConfig(_))
        ));

        config.max_datagram_len = MAX_UDP_DATAGRAM_LEN + 1;
        assert!(matches!(
            UtpListener::bind_with_config("127.0.0.1:0".parse().unwrap(), config).await,
            Err(UtpError::InvalidConfig(_))
        ));
    }

    #[test]
    fn dropped_route_registration_cannot_remove_a_replacement_route() {
        let streams = Arc::new(StdMutex::new(HashMap::new()));
        let key = UtpRouteKey {
            peer: "127.0.0.1:1".parse().unwrap(),
            recv_connection_id: 7,
        };
        let old_token = Arc::new(());
        let (old_tx, _old_rx) = mpsc::channel(1);
        let mut old_conn = UtpConnection::connect(1, 1);
        let old_state = old_conn.build_state(0, 0);
        lock_routes(&streams).insert(
            key,
            UtpRoute {
                tx: old_tx,
                token: Arc::clone(&old_token),
                handshake_state: Some(old_state),
            },
        );
        let old_registration = UtpRouteRegistration {
            key,
            token: old_token,
            streams: Arc::clone(&streams),
        };

        let new_token = Arc::new(());
        let (new_tx, _new_rx) = mpsc::channel(1);
        let mut new_conn = UtpConnection::connect(2, 2);
        let new_state = new_conn.build_state(0, 0);
        lock_routes(&streams).insert(
            key,
            UtpRoute {
                tx: new_tx,
                token: Arc::clone(&new_token),
                handshake_state: Some(new_state),
            },
        );

        drop(old_registration);
        let streams = lock_routes(&streams);
        let route = streams.get(&key).expect("replacement route remains");
        assert!(Arc::ptr_eq(&route.token, &new_token));
    }

    #[tokio::test]
    async fn utp_stream_connects_and_exchanges_payload() {
        let before = stats_snapshot();
        let listener = UtpListener::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            let payload = stream.recv().await.unwrap();
            assert_eq!(payload, b"hello over utp");
            stream.send(b"ack").await.unwrap();
            stream.close().await.unwrap();
        });

        let mut client = UtpStream::connect_with_config(addr, test_config())
            .await
            .unwrap();
        client.send(b"hello over utp").await.unwrap();
        assert_eq!(client.recv().await.unwrap(), b"ack");
        let _ = client.recv().await.unwrap();
        server.await.unwrap();
        let after = stats_snapshot();
        assert!(after.connects > before.connects);
        assert!(after.accepts > before.accepts);
        assert!(after.bytes_sent > before.bytes_sent);
        assert!(after.bytes_received > before.bytes_received);
    }

    #[tokio::test]
    async fn utp_stream_read_exact_spans_payload_chunks() {
        let listener = UtpListener::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            stream.write_all(b"hello").await.unwrap();
            stream.write_all(b"world").await.unwrap();
        });

        let mut client = UtpStream::connect_with_config(addr, test_config())
            .await
            .unwrap();
        let mut first = [0u8; 3];
        let mut second = [0u8; 7];
        client.read_exact(&mut first).await.unwrap();
        client.read_exact(&mut second).await.unwrap();
        assert_eq!(&first, b"hel");
        assert_eq!(&second, b"loworld");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn windowed_send_emits_multiple_packets_before_the_first_ack() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let (len, peer) = socket.recv_from(&mut buf).await.unwrap();
            let syn = UtpPacket::parse(&buf[..len]).unwrap();
            let state = UtpPacket {
                header: crate::packet::UtpHeader {
                    packet_type: PacketType::State,
                    version: 1,
                    extension: 0,
                    connection_id: syn.header.connection_id.wrapping_add(1),
                    timestamp_us: 1,
                    timestamp_diff: 0,
                    wnd_size: DEFAULT_INITIAL_WINDOW_BYTES,
                    seq_nr: 100,
                    ack_nr: syn.header.seq_nr,
                },
                extensions: Vec::new(),
                payload: Vec::new(),
            };
            socket
                .send_to(&state.encode().unwrap(), peer)
                .await
                .unwrap();

            let mut received = Vec::new();
            while received.len() < 4 {
                let (len, _peer) = timeout(Duration::from_secs(2), socket.recv_from(&mut buf))
                    .await
                    .expect("sender should fill the packet window before waiting for an ACK")
                    .unwrap();
                let packet = UtpPacket::parse(&buf[..len]).unwrap();
                if packet.header.packet_type == PacketType::Data {
                    received.push(packet);
                }
            }

            let ack = UtpPacket {
                header: crate::packet::UtpHeader {
                    packet_type: PacketType::State,
                    version: 1,
                    extension: 0,
                    connection_id: syn.header.connection_id.wrapping_add(1),
                    timestamp_us: 2,
                    timestamp_diff: 0,
                    wnd_size: DEFAULT_INITIAL_WINDOW_BYTES,
                    seq_nr: 100,
                    ack_nr: received.last().unwrap().header.seq_nr,
                },
                extensions: Vec::new(),
                payload: Vec::new(),
            };
            socket.send_to(&ack.encode().unwrap(), peer).await.unwrap();
            received
        });

        let mut client = UtpStream::connect_with_config(addr, test_config())
            .await
            .unwrap();
        let payload = vec![0x5a; DEFAULT_MTU_PAYLOAD_BYTES * 4];
        client.send(&payload).await.unwrap();

        let received = server.await.unwrap();
        assert_eq!(received.len(), 4);
        assert_eq!(
            received
                .iter()
                .map(|packet| packet.payload.len())
                .sum::<usize>(),
            payload.len()
        );
    }

    #[tokio::test]
    async fn utp_endpoint_accepts_multiple_streams_on_one_socket() {
        let endpoint = UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let addr = endpoint.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut first = endpoint.accept().await.unwrap();
            let mut second = endpoint.accept().await.unwrap();
            let mut third = endpoint.accept().await.unwrap();
            assert_eq!(first.recv().await.unwrap(), b"first");
            assert_eq!(second.recv().await.unwrap(), b"second");
            assert_eq!(third.recv().await.unwrap(), b"third");
            first.send(b"ack-first").await.unwrap();
            second.send(b"ack-second").await.unwrap();
            third.send(b"ack-third").await.unwrap();
        });

        let client_endpoint =
            UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
                .await
                .unwrap();
        let client_addr = client_endpoint.local_addr().unwrap();
        let client_runtime = UtpRuntime::with_config(test_config());
        client_runtime
            .install_endpoint(client_endpoint)
            .await
            .unwrap();
        let mut first = client_runtime.connect(addr).await.unwrap();
        let replacement =
            UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
                .await
                .unwrap();
        client_runtime.replace_endpoint(replacement).await.unwrap();
        let mut second = client_runtime.connect(addr).await.unwrap();
        let mut third = client_runtime.connect(addr).await.unwrap();
        assert_eq!(
            second.socket.local_addr().unwrap(),
            third.socket.local_addr().unwrap(),
            "outbound uTP streams in one endpoint generation share the UDP socket"
        );
        assert_ne!(
            first.socket.local_addr().unwrap(),
            second.socket.local_addr().unwrap(),
            "a rebind gives new streams the replacement socket"
        );
        assert_eq!(first.socket.local_addr().unwrap(), client_addr);
        first.send(b"first").await.unwrap();
        second.send(b"second").await.unwrap();
        third.send(b"third").await.unwrap();
        assert_eq!(first.recv().await.unwrap(), b"ack-first");
        assert_eq!(second.recv().await.unwrap(), b"ack-second");
        assert_eq!(third.recv().await.unwrap(), b"ack-third");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn utp_stream_preserves_simultaneous_payloads_while_acknowledging() {
        let config = UtpTransportConfig {
            handshake_timeout: Duration::from_secs(1),
            io_timeout: Duration::from_millis(100),
            max_datagram_len: 2048,
            max_retransmits: 1,
        };
        let listener = UtpListener::bind_with_config("127.0.0.1:0".parse().unwrap(), config)
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            stream.send(b"server-to-client").await.unwrap();
            stream
        });

        let mut client = UtpStream::connect_with_config(addr, config).await.unwrap();
        client.send(b"client-to-server").await.unwrap();
        assert_eq!(client.recv().await.unwrap(), b"server-to-client");

        let mut server = tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("simultaneous uTP send timed out")
            .unwrap();
        assert_eq!(server.recv().await.unwrap(), b"client-to-server");
    }

    #[tokio::test]
    async fn utp_stream_splits_data_to_fit_a_small_remote_window() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let (len, peer) = socket.recv_from(&mut buf).await.unwrap();
            let syn = UtpPacket::parse(&buf[..len]).unwrap();
            assert_eq!(syn.header.packet_type, PacketType::Syn);

            let state = UtpPacket {
                header: crate::packet::UtpHeader {
                    packet_type: PacketType::State,
                    version: 1,
                    extension: 0,
                    connection_id: syn.header.connection_id.wrapping_add(1),
                    timestamp_us: 1,
                    timestamp_diff: 0,
                    wnd_size: 1,
                    seq_nr: 100,
                    ack_nr: syn.header.seq_nr,
                },
                extensions: Vec::new(),
                payload: Vec::new(),
            };
            socket
                .send_to(&state.encode().unwrap(), peer)
                .await
                .unwrap();

            let mut received = Vec::new();
            while received.len() < 2 {
                let (len, next_peer) = timeout(Duration::from_secs(1), socket.recv_from(&mut buf))
                    .await
                    .unwrap()
                    .unwrap();
                let packet = UtpPacket::parse(&buf[..len]).unwrap();
                if packet.header.packet_type != PacketType::Data {
                    continue;
                }
                assert_eq!(packet.payload.len(), 1);
                received.extend_from_slice(&packet.payload);
                let ack = UtpPacket {
                    header: crate::packet::UtpHeader {
                        packet_type: PacketType::State,
                        version: 1,
                        extension: 0,
                        connection_id: syn.header.connection_id.wrapping_add(1),
                        timestamp_us: 2,
                        timestamp_diff: 0,
                        wnd_size: 1,
                        seq_nr: 100,
                        ack_nr: packet.header.seq_nr,
                    },
                    extensions: Vec::new(),
                    payload: Vec::new(),
                };
                socket
                    .send_to(&ack.encode().unwrap(), next_peer)
                    .await
                    .unwrap();
            }
            received
        });

        let mut client = UtpStream::connect_with_config(addr, test_config())
            .await
            .unwrap();
        client.send(b"ab").await.unwrap();
        assert_eq!(server.await.unwrap(), b"ab");
    }

    #[tokio::test]
    async fn utp_receive_buffer_is_bounded_and_advertises_remaining_space() {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let conn = UtpConnection::connect(1, 1);
        let reorder = ReorderBuffer::new(conn.ack_nr().wrapping_add(1));
        let mut stream = UtpStream {
            socket,
            peer: "127.0.0.1:1".parse().unwrap(),
            conn,
            config: test_config(),
            last_remote_timestamp_us: 0,
            read_buf: Vec::new(),
            reorder,
            pending_send: VecDeque::new(),
            routed_rx: None,
            deadline: None,
            handshake_state: None,
            route: None,
        };

        stream
            .buffer_inbound_payload(&vec![0; MAX_RECEIVE_BUFFER_BYTES - 1])
            .unwrap();
        let state = stream.conn.build_state(1, 0);
        assert_eq!(state.header.wnd_size, 1);

        let error = stream.buffer_inbound_payload(&[1, 2]).unwrap_err();
        assert!(matches!(
            error,
            UtpError::ReceiveBufferFull { max, actual }
                if max == MAX_RECEIVE_BUFFER_BYTES && actual == MAX_RECEIVE_BUFFER_BYTES + 1
        ));
        let state = stream.conn.build_state(2, 0);
        assert_eq!(state.header.wnd_size, 0);
    }

    #[tokio::test]
    async fn utp_endpoint_shutdown_releases_socket() {
        let endpoint = UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let addr = endpoint.local_addr().unwrap();
        endpoint.shutdown().await.expect("endpoint shutdown");
        drop(endpoint);

        let rebound = UtpEndpoint::bind_with_config(addr, test_config())
            .await
            .expect("endpoint shutdown must release the UDP socket");
        rebound.shutdown().await.expect("rebound endpoint shutdown");
    }

    #[tokio::test]
    async fn utp_endpoint_shutdown_reports_receive_task_panic_without_payload() {
        let endpoint = UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let original = endpoint
            .recv_task
            .lock()
            .expect("receive-task lock")
            .take()
            .expect("receive task");
        endpoint.stop.send(true).expect("receive task is listening");
        original.await.expect("receive loop stops cleanly");

        let panicked = tokio::spawn(async { std::panic::panic_any(()) });
        *endpoint.recv_task.lock().expect("receive-task lock") = Some(panicked);

        let error = endpoint
            .shutdown()
            .await
            .expect_err("panicked receive task must be reported");
        assert!(matches!(error, UtpError::ReceiveTaskPanicked));
        assert_eq!(error.to_string(), "uTP receive task panicked");
    }

    #[tokio::test]
    async fn utp_endpoint_shutdown_recovers_a_poisoned_task_handle_lock() {
        let endpoint = UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        endpoint.stop.send(true).expect("receive task is listening");

        let task_slot = Arc::clone(&endpoint.recv_task);
        let poison_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _task_slot = task_slot.lock().expect("receive-task lock");
            std::panic::panic_any(());
        }));
        assert!(poison_result.is_err());

        endpoint
            .shutdown()
            .await
            .expect("shutdown recovers and joins the receive task");
    }

    #[tokio::test]
    async fn utp_endpoint_shutdown_closes_pending_accept() {
        let endpoint = UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let waiter = {
            let endpoint = endpoint.clone();
            tokio::spawn(async move { endpoint.accept().await })
        };

        endpoint.shutdown().await.expect("endpoint shutdown");
        assert!(matches!(waiter.await.unwrap(), Err(UtpError::Closed)));
    }

    #[tokio::test]
    async fn malformed_endpoint_datagram_does_not_poison_accept_queue() {
        let endpoint = UtpEndpoint::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let addr = endpoint.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        sender
            .send_to(&[0u8; crate::HEADER_SIZE], addr)
            .await
            .unwrap();

        let accept_waiter = {
            let endpoint = endpoint.clone();
            tokio::spawn(async move { endpoint.accept().await })
        };
        let client = UtpStream::connect_with_config(addr, test_config())
            .await
            .unwrap();
        let accepted = timeout(Duration::from_secs(2), accept_waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        drop(client);
        drop(accepted);
        endpoint.shutdown().await.expect("endpoint shutdown");
    }

    #[tokio::test]
    async fn malformed_listener_datagram_does_not_abort_next_handshake() {
        let listener = UtpListener::bind_with_config("127.0.0.1:0".parse().unwrap(), test_config())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        sender
            .send_to(&[0u8; crate::HEADER_SIZE], addr)
            .await
            .unwrap();

        let accept_waiter = tokio::spawn(async move { listener.accept().await });
        let client = UtpStream::connect_with_config(addr, test_config())
            .await
            .unwrap();
        let accepted = timeout(Duration::from_secs(2), accept_waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        drop(client);
        drop(accepted);
    }

    #[tokio::test]
    async fn recv_packet_rejects_an_oversized_datagram_instead_of_parsing_a_truncation() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        receiver
            .connect(sender.local_addr().unwrap())
            .await
            .unwrap();

        let packet = UtpPacket {
            header: crate::packet::UtpHeader {
                packet_type: PacketType::Data,
                version: 1,
                extension: 0,
                connection_id: 1,
                timestamp_us: 0,
                timestamp_diff: 0,
                wnd_size: 1,
                seq_nr: 1,
                ack_nr: 0,
            },
            extensions: Vec::new(),
            payload: vec![42],
        };
        let bytes = packet.encode().unwrap();
        sender
            .send_to(&bytes, receiver.local_addr().unwrap())
            .await
            .unwrap();

        let error = recv_packet(&receiver, Duration::from_secs(1), crate::HEADER_SIZE)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            UtpError::DatagramTooLarge { actual, max }
                if actual == crate::HEADER_SIZE + 1 && max == crate::HEADER_SIZE
        ));
    }
}
