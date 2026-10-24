//! Bounded, read-only swarm observations. Never sends announce_peer, joins a
//! private swarm through DHT, or exposes tracker credentials to the browser.
use super::swarm_switch::SwarmPreview;
use super::*;
use crate::egress_policy::OutboundTargetKind;
use rt_bencode::{encode, BValue, Decoder};
use rt_dht::{DhtResponse, KrpcMessage, NodeId};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

static PROBE_SLOTS: Semaphore = Semaphore::const_new(2);
const MAX_NODES: usize = 64;
const MAX_PEERS: usize = 256;
const MAX_SAMPLES: usize = 12;

pub(crate) struct ProbeContext {
    pub bootstrap: Vec<String>,
    pub trackers: Vec<rt_db::TorrentTrackerRow>,
    pub policy: OutboundEgressPolicy,
    pub network: GlobalNetworkBudget,
    pub registry: Arc<RwLock<SessionRegistry>>,
}

impl std::fmt::Debug for ProbeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeContext").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicObservation {
    pub status: String,
    pub observed_at: i64,
    pub responding_nodes: usize,
    /// Unique endpoints returned, not a census or evidence they are reachable.
    pub discovered_peers: Option<usize>,
    pub seed_estimate: Option<u64>,
    pub peer_estimate: Option<u64>,
    pub sampled_peers: usize,
    pub reachable_peers: usize,
    pub observed_seeds: usize,
    pub bounded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrivateObservation {
    pub tracker: String,
    pub status: String,
    pub observed_at: Option<i64>,
    pub seeds: Option<i64>,
    pub leechers: Option<i64>,
    pub fresh: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwarmComparison {
    pub candidate: SwarmPreview,
    pub public: PublicObservation,
    pub private: Vec<PrivateObservation>,
}

pub(super) async fn compare(
    candidate: SwarmPreview,
    context: ProbeContext,
    resources: ResourceGovernor,
) -> CmdResult<SwarmComparison> {
    let _permit = PROBE_SLOTS
        .try_acquire()
        .map_err(|_| "Swarm comparison is busy; retry shortly")?;
    let _memory = resources
        .try_acquire(MemoryClass::Metadata, 2 * 1024 * 1024)
        .ok_or("Swarm comparison memory budget is busy; retry shortly")?;
    let original_bytes =
        hex::decode(&candidate.original_hash).map_err(|_| "Invalid original hash")?;
    let original: [u8; 20] = original_bytes[..20]
        .try_into()
        .map_err(|_| "Invalid original hash")?;
    let public_bytes = hex::decode(&candidate.public_hash).map_err(|_| "Invalid public hash")?;
    let public: [u8; 20] = public_bytes[..20]
        .try_into()
        .map_err(|_| "Invalid public hash")?;
    let (public, private) = tokio::join!(
        public_observation(public, &candidate, &context),
        private_observations(original, &context, &resources)
    );
    Ok(SwarmComparison {
        candidate,
        public,
        private,
    })
}

async fn socket_for(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    UdpSocket::bind(if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await
}

struct NodeReply {
    response: DhtResponse,
    seeds: Option<[u8; 256]>,
    peers: Option<[u8; 256]>,
}

async fn query_node(
    addr: SocketAddr,
    hash: [u8; 20],
    id: NodeId,
    policy: OutboundEgressPolicy,
) -> Option<NodeReply> {
    policy.validate_socket_addr(addr).ok()?;
    timeout(Duration::from_secs(2), async {
        let socket = socket_for(addr).await.ok()?;
        socket.connect(addr).await.ok()?; // kernel also filters source address
        let nonce = *NodeId::random().as_bytes();
        let transaction = &nonce[..8];
        let request = encode(&BValue::Dict(vec![
            (
                b"a",
                BValue::Dict(vec![
                    (b"id", BValue::Bytes(id.as_bytes())),
                    (b"info_hash", BValue::Bytes(&hash)),
                    (b"scrape", BValue::Int(1)),
                    (
                        b"want",
                        BValue::List(vec![BValue::Bytes(b"n4"), BValue::Bytes(b"n6")]),
                    ),
                ]),
            ),
            (b"q", BValue::Bytes(b"get_peers")),
            (b"t", BValue::Bytes(transaction)),
            (b"y", BValue::Bytes(b"q")),
        ]));
        socket.send(&request).await.ok()?;
        let mut bytes = [0u8; 2049];
        for _ in 0..4 {
            let len = socket.recv(&mut bytes).await.ok()?;
            if len > 2048 {
                continue;
            }
            let KrpcMessage::Response {
                transaction_id,
                response,
            } = KrpcMessage::parse(&bytes[..len]).ok()?
            else {
                continue;
            };
            if transaction_id != transaction {
                continue;
            }
            let value = Decoder::new(&bytes[..len])
                .with_strict_dict_keys(false)
                .decode()
                .ok()?;
            let result = value.get(b"r")?;
            let bloom =
                |key: &[u8]| -> Option<[u8; 256]> { result.get(key)?.as_bytes()?.try_into().ok() };
            return Some(NodeReply {
                response,
                seeds: bloom(b"BFsd"),
                peers: bloom(b"BFpe"),
            });
        }
        None
    })
    .await
    .ok()
    .flatten()
}

fn bloom_estimate(bloom: &[u8; 256]) -> Option<u64> {
    let zeros: u32 = bloom.iter().map(|b| b.count_zeros()).sum();
    if zeros == 2048 {
        return Some(0);
    }
    if zeros == 0 {
        return None;
    }
    let count = ((f64::from(zeros) / 2048.0).ln() / (2.0 * (1.0_f64 - 1.0 / 2048.0).ln())).round();
    (count.is_finite() && count <= 6000.0).then_some(count as u64)
}

async fn public_observation(
    hash: [u8; 20],
    candidate: &SwarmPreview,
    context: &ProbeContext,
) -> PublicObservation {
    let mut result = PublicObservation {
        status: "unavailable".into(),
        observed_at: unix_now_i64(),
        responding_nodes: 0,
        discovered_peers: None,
        seed_estimate: None,
        peer_estimate: None,
        sampled_peers: 0,
        reachable_peers: 0,
        observed_seeds: 0,
        bounded: false,
    };
    let mut pending = VecDeque::new();
    let mut visited = HashSet::new();
    let mut peers = HashSet::new();
    let mut seed_bloom = [0u8; 256];
    let mut peer_bloom = [0u8; 256];
    let mut has_seeds = false;
    let mut has_peers = false;
    let id = NodeId::random();
    let lookup = async {
        for host in context.bootstrap.iter().take(8) {
            let Ok(_dns) = crate::outbound_resolver::try_acquire_dns_lookup() else {
                continue;
            };
            if let Ok(Ok(addresses)) = timeout(
                Duration::from_secs(1),
                tokio::net::lookup_host(host.as_str()),
            )
            .await
            {
                for addr in addresses.take(8) {
                    if context.policy.validate_socket_addr(addr).is_ok() {
                        pending.push_back(addr);
                    }
                }
            }
        }
        while visited.len() < MAX_NODES && !pending.is_empty() && peers.len() < MAX_PEERS {
            let mut batch = Vec::new();
            while batch.len() < 3 && visited.len() < MAX_NODES {
                let Some(addr) = pending.pop_front() else {
                    break;
                };
                if visited.insert(addr) {
                    batch.push(addr);
                }
            }
            let replies = stream::iter(batch)
                .map(|addr| query_node(addr, hash, id, context.policy))
                .buffer_unordered(3)
                .collect::<Vec<_>>()
                .await;
            for reply in replies.into_iter().flatten() {
                result.responding_nodes += 1;
                if let Some(bloom) = reply.seeds {
                    has_seeds = true;
                    for (a, b) in seed_bloom.iter_mut().zip(bloom) {
                        *a |= b;
                    }
                }
                if let Some(bloom) = reply.peers {
                    has_peers = true;
                    for (a, b) in peer_bloom.iter_mut().zip(bloom) {
                        *a |= b;
                    }
                }
                for peer in reply.response.values {
                    if peers.len() < MAX_PEERS
                        && peer.port() != 0
                        && context.policy.validate_peer_addr(peer).is_ok()
                        && !context.registry.read().await.is_peer_banned(peer)
                    {
                        peers.insert(peer);
                    }
                }
                // Prioritize returned nodes by XOR distance, with both families
                // sharing one bounded frontier and one request budget.
                let mut nodes = reply
                    .response
                    .nodes
                    .into_iter()
                    .map(|n| (n.id, SocketAddr::V4(n.addr)))
                    .chain(
                        reply
                            .response
                            .nodes6
                            .into_iter()
                            .map(|n| (n.id, SocketAddr::V6(n.addr))),
                    )
                    .collect::<Vec<_>>();
                nodes.sort_by_key(|(id, _)| {
                    let mut d = *id.as_bytes();
                    for (a, b) in d.iter_mut().zip(hash) {
                        *a ^= b;
                    }
                    d
                });
                for (_, addr) in nodes {
                    if pending.len() < 256
                        && !visited.contains(&addr)
                        && context.policy.validate_socket_addr(addr).is_ok()
                    {
                        pending.push_back(addr);
                    }
                }
            }
        }
    };
    let timed_out = timeout(Duration::from_secs(8), lookup).await.is_err();
    result.bounded = timed_out || visited.len() == MAX_NODES || peers.len() == MAX_PEERS;
    if result.responding_nodes > 0 {
        result.status = "observed".into();
        result.discovered_peers = Some(peers.len());
        if has_seeds {
            result.seed_estimate = bloom_estimate(&seed_bloom);
        }
        if has_peers {
            result.peer_estimate = bloom_estimate(&peer_bloom);
        }
    }
    let sample = peers.into_iter().take(MAX_SAMPLES).collect::<Vec<_>>();
    result.sampled_peers = sample.len();
    let observations = stream::iter(sample)
        .map(|peer| sample_peer(peer, hash, candidate, context))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
    for (reachable, seed) in observations {
        result.reachable_peers += usize::from(reachable);
        result.observed_seeds += usize::from(seed);
    }
    result.observed_at = unix_now_i64();
    result
}

async fn sample_peer(
    addr: SocketAddr,
    hash: [u8; 20],
    candidate: &SwarmPreview,
    context: &ProbeContext,
) -> (bool, bool) {
    let Ok(_slot) = context.network.try_acquire_peer() else {
        return (false, false);
    };
    if context.policy.validate_peer_addr(addr).is_err()
        || context.registry.read().await.is_peer_banned(addr)
    {
        return (false, false);
    }
    let mut reachable = false;
    let seed = timeout(Duration::from_secs(2), async {
        let mut socket = TcpStream::connect(addr).await.ok()?;
        let mut handshake = rt_peer_wire::Handshake::new(hash, *NodeId::random().as_bytes());
        handshake.reserved =
            rt_peer_wire::ExtensionFlags::with_extension_protocol_and_fast_and_v2();
        socket.write_all(&handshake.encode()).await.ok()?;
        let mut bytes = [0u8; 68];
        socket.read_exact(&mut bytes).await.ok()?;
        let remote = rt_peer_wire::Handshake::parse(&bytes).ok()?;
        if remote.info_hash != hash {
            return None;
        }
        reachable = true;
        if candidate.public_hash.len() == 64 && !remote.reserved.supports_v2() {
            return None;
        }
        for _ in 0..8 {
            let len = socket.read_u32().await.ok()? as usize;
            if len == 0 {
                continue;
            }
            // A probe never requests, accepts or stores payload blocks.
            if len > 256 * 1024 {
                return None;
            }
            let kind = socket.read_u8().await.ok()?;
            if kind == 14 && len == 1 && remote.reserved.supports_fast_extension() {
                return Some(true);
            }
            if kind == 5 {
                let count = candidate.piece_count;
                if count == 0 || len != count.div_ceil(8) + 1 {
                    return None;
                }
                let mut field = vec![0; len - 1];
                socket.read_exact(&mut field).await.ok()?;
                return Some((0..count).all(|bit| field[bit / 8] & (0x80 >> (bit % 8)) != 0));
            }
            if kind == 7 {
                return None;
            }
            let mut discard = vec![0; len - 1];
            socket.read_exact(&mut discard).await.ok()?;
        }
        None
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    (reachable, seed)
}

async fn private_observations(
    hash: [u8; 20],
    context: &ProbeContext,
    resources: &ResourceGovernor,
) -> Vec<PrivateObservation> {
    // Report each tracker separately: totals cannot safely be summed.
    stream::iter(context.trackers.iter().take(8).cloned().collect::<Vec<_>>())
        .map(|tracker| async move {
            let fresh = timeout(
                Duration::from_secs(5),
                scrape(hash, &tracker.url, context.policy, resources),
            )
            .await
            .ok()
            .flatten();
            let (seeds, leechers, observed_at, status) = if let Some(ref stats) = fresh {
                (
                    Some(i64::from(stats.complete)),
                    Some(i64::from(stats.incomplete)),
                    Some(unix_now_i64()),
                    "scraped",
                )
            } else {
                (
                    tracker.seeders,
                    tracker.leechers,
                    tracker.last_success_at,
                    "cached_or_unavailable",
                )
            };
            PrivateObservation {
                tracker: crate::tracker_runtime::url_log_target(&tracker.url),
                seeds,
                leechers,
                observed_at,
                status: status.into(),
                fresh: fresh.is_some(),
            }
        })
        .buffered(4)
        .collect()
        .await
}

async fn scrape(
    hash: [u8; 20],
    address: &str,
    policy: OutboundEgressPolicy,
    resources: &ResourceGovernor,
) -> Option<rt_tracker::ScrapeStats> {
    let url = url::Url::parse(address).ok()?;
    if url.scheme() == "udp" {
        // UDP trackers with path/query authentication need BEP 41. Do not strip
        // credentials and silently scrape a different unauthenticated endpoint.
        if url.query().is_some() || !matches!(url.path(), "" | "/" | "/announce") {
            return None;
        }
        let addr = policy
            .resolve_and_validate(OutboundTargetKind::Tracker, &url, Duration::from_secs(2))
            .await
            .ok()?
            .into_iter()
            .next()?;
        let socket = socket_for(addr).await.ok()?;
        socket.connect(addr).await.ok()?;
        let request = rt_tracker::udp::UdpConnectRequest::new();
        socket.send(&request.encode()).await.ok()?;
        let mut buf = [0u8; 2048];
        let len = socket.recv(&mut buf).await.ok()?;
        let connection = rt_tracker::udp::UdpConnectResponse::parse(&buf[..len]).ok()?;
        if connection.transaction_id != request.transaction_id {
            return None;
        }
        let nonce = rt_tracker::udp::UdpConnectRequest::new().transaction_id;
        let mut request = Vec::with_capacity(36);
        request.extend(connection.connection_id.to_be_bytes());
        request.extend(2u32.to_be_bytes());
        request.extend(nonce.to_be_bytes());
        request.extend(hash);
        socket.send(&request).await.ok()?;
        let len = socket.recv(&mut buf).await.ok()?;
        if len != 20 || buf[..4] != 2u32.to_be_bytes() || buf[4..8] != nonce.to_be_bytes() {
            return None;
        }
        return Some(rt_tracker::ScrapeStats {
            complete: u32::from_be_bytes(buf[8..12].try_into().ok()?),
            downloaded: u32::from_be_bytes(buf[12..16].try_into().ok()?),
            incomplete: u32::from_be_bytes(buf[16..20].try_into().ok()?),
        });
    }
    let scrape_url =
        rt_tracker::to_http_scrape_url(address, rt_tracker::InfoHash::V1(hash)).ok()?;
    let client = policy
        .http_client(
            OutboundTargetKind::Tracker,
            &url,
            Duration::from_secs(4),
            &crate::peer_id::user_agent(),
        )
        .await
        .ok()?;
    let response = client.get(scrape_url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body =
        crate::tracker_runtime::bounded_response_body_with_memory(response, 1024 * 1024, resources)
            .await
            .ok()?;
    rt_tracker::ScrapeStats::parse(&body.bytes, &hash).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bloom_estimates_empty_and_saturated_without_inventing_counts() {
        assert_eq!(bloom_estimate(&[0; 256]), Some(0));
        assert_eq!(bloom_estimate(&[255; 256]), None);
        let mut single = [0; 256];
        single[0] = 3;
        assert_eq!(bloom_estimate(&single), Some(1));
    }
    fn context(bootstrap: Vec<String>) -> ProbeContext {
        let mut config = Config::default();
        config.tracker.allow_loopback_egress = true;
        ProbeContext {
            bootstrap,
            trackers: vec![],
            policy: OutboundEgressPolicy::from_config(&config.tracker),
            network: GlobalNetworkBudget::new(8, None, None),
            registry: Arc::new(RwLock::new(SessionRegistry::new())),
        }
    }

    fn candidate() -> SwarmPreview {
        SwarmPreview {
            original_hash: hex::encode([2; 20]),
            public_hash: hex::encode([3; 20]),
            public_v2_hash: None,
            piece_length: 16384,
            piece_count: 1,
            files: 1,
            total_bytes: 4,
            variant: super::super::swarm_switch::SwarmVariant::RemovePrivate,
        }
    }

    #[tokio::test]
    async fn dht_probe_authenticates_transaction_and_never_announces_private_identity() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut buf = [0; 2048];
            let (len, from) = socket.recv_from(&mut buf).await.unwrap();
            let query = Decoder::new(&buf[..len]).decode().unwrap();
            assert_eq!(
                query.get(b"q").unwrap().as_bytes(),
                Some(b"get_peers".as_slice())
            );
            assert_eq!(
                query
                    .get(b"a")
                    .unwrap()
                    .get(b"info_hash")
                    .unwrap()
                    .as_bytes(),
                Some([3; 20].as_slice())
            );
            let transaction = query.get(b"t").unwrap().as_bytes().unwrap().to_vec();
            for tx in [b"wrong-id".as_slice(), transaction.as_slice()] {
                let response = encode(&BValue::Dict(vec![
                    (
                        b"r",
                        BValue::Dict(vec![
                            (b"BFpe", BValue::Bytes(&[0; 256])),
                            (b"BFsd", BValue::Bytes(&[0; 256])),
                            (b"id", BValue::Bytes(&[4; 20])),
                            (b"nodes", BValue::Bytes(b"")),
                        ]),
                    ),
                    (b"t", BValue::Bytes(tx)),
                    (b"y", BValue::Bytes(b"r")),
                ]));
                socket.send_to(&response, from).await.unwrap();
            }
            // A read-only lookup never follows the response with announce_peer.
            assert!(
                timeout(Duration::from_millis(100), socket.recv_from(&mut buf))
                    .await
                    .is_err()
            );
        });
        let context = context(vec![address.to_string()]);
        let result = public_observation([3; 20], &candidate(), &context).await;
        assert_eq!(result.responding_nodes, 1);
        assert_eq!(result.discovered_peers, Some(0));
        assert_eq!(result.seed_estimate, Some(0));
        assert_eq!(result.peer_estimate, Some(0));
        server.await.unwrap();
        let unavailable = public_observation([3; 20], &candidate(), &self::context(vec![])).await;
        assert_eq!(unavailable.discovered_peers, None);
        assert_eq!(unavailable.seed_estimate, None);
    }

    #[tokio::test]
    async fn peer_sample_verifies_hash_and_bitfield_without_requesting_payload() {
        for correct in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 68];
                socket.read_exact(&mut bytes).await.unwrap();
                let client = rt_peer_wire::Handshake::parse(&bytes).unwrap();
                assert_eq!(client.info_hash, [3; 20]);
                let handshake =
                    rt_peer_wire::Handshake::new(if correct { [3; 20] } else { [2; 20] }, [5; 20]);
                socket.write_all(&handshake.encode()).await.unwrap();
                if correct {
                    socket.write_all(&[0, 0, 0, 2, 5, 0x80]).await.unwrap();
                }
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            });
            let result = sample_peer(address, [3; 20], &candidate(), &context(vec![])).await;
            assert_eq!(result, (correct, correct));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn private_http_scrape_preserves_auth_query_and_reports_real_zero() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let len = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..len]);
            assert!(request.starts_with("GET /secret/scrape?"));
            assert!(request.contains("passkey=test"));
            let body = encode(&BValue::Dict(vec![(
                b"files",
                BValue::Dict(vec![(
                    &[2; 20],
                    BValue::Dict(vec![
                        (b"complete", BValue::Int(0)),
                        (b"downloaded", BValue::Int(10)),
                        (b"incomplete", BValue::Int(7)),
                    ]),
                )]),
            )]));
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&body).await.unwrap();
        });
        let resources = ResourceGovernor::new(ResourceGovernorConfig::default());
        let stats = scrape(
            [2; 20],
            &format!("http://{address}/secret/announce?passkey=test"),
            context(vec![]).policy,
            &resources,
        )
        .await
        .unwrap();
        assert_eq!((stats.complete, stats.incomplete), (0, 7));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn private_udp_scrape_checks_transactions_and_never_announces() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut buf = [0; 2048];
            let (len, from) = socket.recv_from(&mut buf).await.unwrap();
            assert_eq!(len, 16);
            assert_eq!(&buf[8..12], &0u32.to_be_bytes());
            let mut response = Vec::new();
            response.extend(0u32.to_be_bytes());
            response.extend(&buf[12..16]);
            response.extend(42u64.to_be_bytes());
            socket.send_to(&response, from).await.unwrap();
            let (len, from) = socket.recv_from(&mut buf).await.unwrap();
            assert_eq!(len, 36);
            assert_eq!(&buf[..8], &42u64.to_be_bytes());
            assert_eq!(&buf[8..12], &2u32.to_be_bytes());
            assert_eq!(&buf[16..36], &[2; 20]);
            let mut response = Vec::new();
            response.extend(2u32.to_be_bytes());
            response.extend(&buf[12..16]);
            response.extend(4u32.to_be_bytes());
            response.extend(10u32.to_be_bytes());
            response.extend(7u32.to_be_bytes());
            socket.send_to(&response, from).await.unwrap();
        });
        let resources = ResourceGovernor::new(ResourceGovernorConfig::default());
        let stats = scrape(
            [2; 20],
            &format!("udp://{address}/announce"),
            context(vec![]).policy,
            &resources,
        )
        .await
        .unwrap();
        assert_eq!((stats.complete, stats.incomplete), (4, 7));
        server.await.unwrap();
        assert!(scrape(
            [2; 20],
            &format!("udp://{address}/announce?passkey=secret"),
            context(vec![]).policy,
            &resources
        )
        .await
        .is_none());
    }
}
