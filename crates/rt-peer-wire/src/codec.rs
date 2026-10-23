/// tokio-util `Codec` impl for BEP 3 length-prefixed peer messages.
use bytes::{Buf, Bytes, BytesMut};
use rt_metrics::{MemoryClass, MemoryLease, ResourceGovernor};
use std::sync::Arc;
use tokio_util::codec::{Decoder, Encoder};

use crate::{
    error::WireError,
    message::{Message, MAX_MESSAGE_LEN},
};

const RETAINED_BUFFER_CAPACITY: usize = 64 * 1024;

fn ensure_peer_buffer_reservation(
    resources: &ResourceGovernor,
    lease: &mut Option<MemoryLease>,
    reserved_bytes: &mut u64,
    required_bytes: u64,
) -> Result<(), WireError> {
    if required_bytes <= *reserved_bytes {
        return Ok(());
    }

    let additional = required_bytes - *reserved_bytes;
    if let Some(lease) = lease.as_mut() {
        if !lease.try_grow(additional) {
            return Err(WireError::PeerBufferAllocationDenied(required_bytes));
        }
    } else {
        *lease = Some(
            resources
                .try_acquire(MemoryClass::PeerBuffer, required_bytes)
                .ok_or(WireError::PeerBufferAllocationDenied(required_bytes))?,
        );
    }
    *reserved_bytes = required_bytes;
    Ok(())
}

fn acquire_read_buffer_lease(
    resources: &ResourceGovernor,
    capacity: usize,
) -> Result<PeerReadBufferLease, WireError> {
    let bytes = u64::try_from(capacity).map_err(|_| WireError::PeerBufferLengthOverflow)?;
    let lease = resources
        .try_acquire(MemoryClass::PeerBuffer, bytes)
        .ok_or(WireError::PeerBufferAllocationDenied(bytes))?;
    Ok(PeerReadBufferLease { lease })
}

fn ensure_read_buffer_reservation(
    resources: &ResourceGovernor,
    lease: &mut Option<Arc<PeerReadBufferLease>>,
    reserved_bytes: &mut u64,
    required_bytes: u64,
) -> Result<(), WireError> {
    if required_bytes <= *reserved_bytes {
        return Ok(());
    }

    let additional = required_bytes - *reserved_bytes;
    if let Some(lease) = lease.as_mut() {
        let Some(owner) = Arc::get_mut(lease) else {
            return Err(WireError::PeerBufferAllocationDenied(required_bytes));
        };
        if !owner.lease.try_grow(additional) {
            return Err(WireError::PeerBufferAllocationDenied(required_bytes));
        }
    } else {
        let bytes = required_bytes;
        let new_lease = resources
            .try_acquire(MemoryClass::PeerBuffer, bytes)
            .ok_or(WireError::PeerBufferAllocationDenied(bytes))?;
        *lease = Some(Arc::new(PeerReadBufferLease { lease: new_lease }));
    }
    *reserved_bytes = required_bytes;
    Ok(())
}

/// BEP 3 length-prefixed peer message codec.
///
/// The default codec is useful for protocol-only callers. Engine-owned TCP
/// connections should use [`PeerCodec::with_resources`] so a peer cannot grow
/// tokio-util's retained read buffer to the protocol maximum without passing
/// the engine's `PeerBuffer` admission gate first.
#[derive(Default)]
pub struct PeerCodec {
    resources: Option<ResourceGovernor>,
    memory: Option<Box<PeerCodecMemory>>,
    /// `BytesMut::split_to` can leave an empty tail with zero visible capacity
    /// while still retaining the original backing allocation.
    read_buffer_high_water_bytes: usize,
}

#[derive(Default)]
struct PeerCodecMemory {
    read_memory_lease: Option<Arc<PeerReadBufferLease>>,
    write_memory_lease: Option<MemoryLease>,
    read_reserved_bytes: u64,
    write_reserved_bytes: u64,
}

struct PeerReadBufferLease {
    lease: MemoryLease,
}

struct PiecePayloadOwner {
    bytes: Bytes,
    _read_buffer_lease: Arc<PeerReadBufferLease>,
}

impl AsRef<[u8]> for PiecePayloadOwner {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

impl PeerCodec {
    pub fn with_resources(resources: ResourceGovernor) -> Self {
        Self {
            resources: Some(resources),
            ..Self::default()
        }
    }

    fn reserve_capacity(&mut self, src: &mut BytesMut, required: usize) -> Result<(), WireError> {
        // Charge for the larger of the current capacity and requested frame
        // size. Any owned payload copy is charged separately and its lease
        // travels with the emitted Message instead of staying on the codec.
        let accounted_capacity = src.capacity().max(required);
        self.read_buffer_high_water_bytes =
            self.read_buffer_high_water_bytes.max(accounted_capacity);
        let resources = self.resources.clone();
        if let Some(resources) = resources.as_ref() {
            let accounted_capacity_bytes = u64::try_from(accounted_capacity)
                .map_err(|_| WireError::PeerBufferLengthOverflow)?;
            let reserved_read_bytes = self
                .memory
                .as_ref()
                .map_or(0, |memory| memory.read_reserved_bytes);
            let shared_backing = self
                .memory
                .as_ref()
                .and_then(|memory| memory.read_memory_lease.as_ref())
                .is_some_and(|lease| Arc::strong_count(lease) > 1);
            if shared_backing
                && (required > src.capacity() || accounted_capacity_bytes > reserved_read_bytes)
            {
                let lease = acquire_read_buffer_lease(resources, required)?;
                let mut replacement = BytesMut::with_capacity(required);
                replacement.extend_from_slice(src.as_ref());
                let actual_capacity = replacement.capacity();
                let actual_capacity_bytes = u64::try_from(actual_capacity)
                    .map_err(|_| WireError::PeerBufferLengthOverflow)?;
                let mut lease = lease;
                if actual_capacity_bytes > lease.lease.bytes()
                    && !lease
                        .lease
                        .try_grow(actual_capacity_bytes - lease.lease.bytes())
                {
                    return Err(WireError::PeerBufferAllocationDenied(actual_capacity_bytes));
                }
                *src = replacement;
                let memory = self
                    .memory
                    .get_or_insert_with(|| Box::new(PeerCodecMemory::default()));
                memory.read_reserved_bytes = lease.lease.bytes();
                memory.read_memory_lease = Some(Arc::new(lease));
                self.read_buffer_high_water_bytes = actual_capacity;
                return Ok(());
            }

            let memory = self
                .memory
                .get_or_insert_with(|| Box::new(PeerCodecMemory::default()));
            ensure_read_buffer_reservation(
                resources,
                &mut memory.read_memory_lease,
                &mut memory.read_reserved_bytes,
                accounted_capacity_bytes,
            )?;
        }

        if required > src.capacity() {
            src.reserve(required.saturating_sub(src.len()));
        }
        self.read_buffer_high_water_bytes = self.read_buffer_high_water_bytes.max(src.capacity());

        // Account for allocator growth beyond the requested capacity before
        // another frame can be admitted on this connection.
        if let Some(resources) = resources.as_ref() {
            let actual_capacity =
                u64::try_from(src.capacity()).map_err(|_| WireError::PeerBufferLengthOverflow)?;
            let memory = self
                .memory
                .get_or_insert_with(|| Box::new(PeerCodecMemory::default()));
            if actual_capacity > memory.read_reserved_bytes {
                ensure_read_buffer_reservation(
                    resources,
                    &mut memory.read_memory_lease,
                    &mut memory.read_reserved_bytes,
                    actual_capacity,
                )?;
            }
        }
        Ok(())
    }

    fn reserve_write_capacity(
        &mut self,
        dst: &mut BytesMut,
        required: usize,
    ) -> Result<(), WireError> {
        // The write buffer is retained by `Framed` while a slow peer drains
        // it. Charge a conservative growth allowance before BytesMut can
        // allocate. Its lease can return to the warm-buffer size after the
        // output buffer is empty.
        let accounted_capacity = dst.capacity().max(required);
        if let Some(resources) = self.resources.clone() {
            let memory = self
                .memory
                .get_or_insert_with(|| Box::new(PeerCodecMemory::default()));
            let required_bytes = u64::try_from(accounted_capacity)
                .ok()
                .and_then(|bytes| bytes.checked_mul(2))
                .ok_or(WireError::PeerBufferLengthOverflow)?;
            ensure_peer_buffer_reservation(
                &resources,
                &mut memory.write_memory_lease,
                &mut memory.write_reserved_bytes,
                required_bytes,
            )?;
        }
        if required > dst.capacity() {
            dst.reserve(required.saturating_sub(dst.len()));
        }
        Ok(())
    }

    fn trim_idle_read_buffer(&mut self, src: &mut BytesMut) {
        if !src.is_empty() || self.read_buffer_high_water_bytes <= RETAINED_BUFFER_CAPACITY {
            return;
        }
        let Some(resources) = self.resources.as_ref() else {
            *src = BytesMut::with_capacity(RETAINED_BUFFER_CAPACITY);
            self.read_buffer_high_water_bytes = src.capacity();
            return;
        };

        // Account for both allocations during the detach. If a Piece still
        // owns the old slab, its lease remains alive; otherwise replacing the
        // codec owner releases that old charge immediately.
        let lease = match acquire_read_buffer_lease(resources, RETAINED_BUFFER_CAPACITY) {
            Ok(lease) => lease,
            Err(_) => return,
        };
        let replacement = BytesMut::with_capacity(RETAINED_BUFFER_CAPACITY);
        let actual_capacity = replacement.capacity();
        let actual_bytes = match u64::try_from(actual_capacity) {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        let mut lease = lease;
        if actual_bytes > lease.lease.bytes()
            && !lease.lease.try_grow(actual_bytes - lease.lease.bytes())
        {
            return;
        }

        let memory = self
            .memory
            .get_or_insert_with(|| Box::new(PeerCodecMemory::default()));
        *src = replacement;
        memory.read_reserved_bytes = lease.lease.bytes();
        memory.read_memory_lease = Some(Arc::new(lease));
        self.read_buffer_high_water_bytes = src.capacity();
    }

    fn trim_idle_write_buffer(&mut self, dst: &mut BytesMut) {
        if !dst.is_empty() || dst.capacity() <= RETAINED_BUFFER_CAPACITY {
            return;
        }

        *dst = BytesMut::with_capacity(RETAINED_BUFFER_CAPACITY);
        let Some(required_bytes) = u64::try_from(dst.capacity())
            .ok()
            .and_then(|bytes| bytes.checked_mul(2))
        else {
            return;
        };
        let Some(memory) = self.memory.as_mut() else {
            return;
        };
        if required_bytes < memory.write_reserved_bytes {
            if let Some(lease) = memory.write_memory_lease.as_mut() {
                lease.shrink_to(required_bytes);
                memory.write_reserved_bytes = required_bytes;
            }
        }
    }
}

impl Decoder for PeerCodec {
    type Item = Message;
    type Error = WireError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.trim_idle_read_buffer(src);
        self.read_buffer_high_water_bytes = self.read_buffer_high_water_bytes.max(src.capacity());
        if src.len() < 4 {
            self.reserve_capacity(src, 4)?;
            return Ok(None);
        }
        let len = u32::from_be_bytes(src[..4].try_into().unwrap());
        if len > MAX_MESSAGE_LEN {
            return Err(WireError::MessageTooLarge(len));
        }
        let total = 4 + len as usize;
        if src.len() < total {
            self.reserve_capacity(src, total)?;
            return Ok(None);
        }
        if self.resources.is_some() && src.capacity() > 0 {
            // A first call may receive a prefilled buffer and must charge it.
            // After that, completed-frame splits only reduce this capacity;
            // any later growth is admitted by reserve_capacity before reading.
            let accounted_capacity = self.memory.as_ref().map_or(0, |memory| {
                usize::try_from(memory.read_reserved_bytes).unwrap_or(usize::MAX)
            });
            if src.capacity() > accounted_capacity {
                self.reserve_capacity(src, src.capacity())?;
            }
        }
        let copied_payload_bytes = Message::copied_payload_bytes(&src[4..total]);
        let message_memory_lease = if copied_payload_bytes == 0 {
            None
        } else if let Some(resources) = self.resources.as_ref() {
            let bytes = u64::try_from(copied_payload_bytes)
                .map_err(|_| WireError::PeerBufferLengthOverflow)?;
            Some(
                resources
                    .try_acquire(MemoryClass::PeerBuffer, bytes)
                    .ok_or(WireError::PeerBufferAllocationDenied(bytes))?,
            )
        } else {
            None
        };
        src.advance(4);
        let payload = src.split_to(len as usize).freeze();
        let mut msg = Message::parse_owned(payload)?;
        msg = match msg {
            Message::Piece { piece, begin, data } => {
                match self
                    .memory
                    .as_ref()
                    .and_then(|memory| memory.read_memory_lease.as_ref())
                {
                    Some(lease) => Message::Piece {
                        piece,
                        begin,
                        data: Bytes::from_owner(PiecePayloadOwner {
                            bytes: data,
                            _read_buffer_lease: Arc::clone(lease),
                        }),
                    },
                    None => Message::Piece { piece, begin, data },
                }
            }
            other => other,
        };
        if let Some(memory_lease) = message_memory_lease {
            msg = msg.with_memory_lease(memory_lease);
        }
        Ok(Some(msg))
    }
}

impl Encoder<Message> for PeerCodec {
    type Error = WireError;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.trim_idle_write_buffer(dst);
        let encoded_len = item
            .encoded_len()
            .ok_or(WireError::MessageTooLarge(u32::MAX))?;
        // Encode every message directly into the retained framed write
        // buffer. This removes the temporary Vec previously used for all
        // non-Piece messages and keeps Piece on the same zero-intermediate
        // path.
        self.reserve_write_capacity(dst, encoded_len)?;
        item.encode_into(dst)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use bytes::Bytes;
    use bytes::BytesMut;
    use tokio_util::codec::{Decoder, Encoder};

    fn peer_buffer_governor(cap_bytes: u64) -> ResourceGovernor {
        let mut class_caps = [0; rt_metrics::MEMORY_CLASS_COUNT];
        class_caps[MemoryClass::PeerBuffer as usize] = cap_bytes;
        ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: cap_bytes,
            class_caps_bytes: class_caps,
            pressure_constrained_pct: 75,
            pressure_critical_pct: 90,
        })
    }

    fn encode_msg(msg: Message) -> BytesMut {
        let mut buf = BytesMut::new();
        PeerCodec::default().encode(msg, &mut buf).unwrap();
        buf
    }

    #[test]
    fn codec_roundtrip_choke() {
        let mut buf = encode_msg(Message::Choke);
        let msg = PeerCodec::default().decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg, Message::Choke);
        assert!(buf.is_empty());
    }

    #[test]
    fn codec_roundtrip_have() {
        let mut buf = encode_msg(Message::Have(99));
        let msg = PeerCodec::default().decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg, Message::Have(99));
    }

    #[test]
    fn codec_partial_read_returns_none() {
        let mut buf = encode_msg(Message::Have(1));
        // Only feed 3 bytes — not enough for length prefix
        let partial = buf.split_to(3);
        let mut partial_buf = partial;
        let result = PeerCodec::default().decode(&mut partial_buf).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn codec_rejects_oversized_message() {
        let mut buf = BytesMut::new();
        buf.put_u32(MAX_MESSAGE_LEN + 1);
        buf.put_bytes(0, (MAX_MESSAGE_LEN + 1) as usize);
        assert!(PeerCodec::default().decode(&mut buf).is_err());
    }

    #[test]
    fn codec_multiple_messages_in_buffer() {
        let mut buf = BytesMut::new();
        let mut codec = PeerCodec::default();
        codec.encode(Message::Choke, &mut buf).unwrap();
        codec.encode(Message::Interested, &mut buf).unwrap();
        let m1 = codec.decode(&mut buf).unwrap().unwrap();
        let m2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(m1, Message::Choke);
        assert_eq!(m2, Message::Interested);
    }

    #[test]
    fn codec_roundtrip_piece_bytes() {
        let data = bytes::Bytes::from(vec![0x42u8; 4096]);
        let msg = Message::Piece {
            piece: 7,
            begin: 16384,
            data: data.clone(),
        };
        let mut buf = encode_msg(msg);
        let decoded = PeerCodec::default().decode(&mut buf).unwrap().unwrap();
        assert_eq!(
            decoded,
            Message::Piece {
                piece: 7,
                begin: 16384,
                data,
            }
        );
    }

    #[test]
    fn codec_piece_decode_reuses_frame_payload() {
        let message = Message::Piece {
            piece: 7,
            begin: 16_384,
            data: Bytes::from(vec![0x42u8; 4096]),
        };
        let mut frame = encode_msg(message);
        let frame_start = frame.as_ptr();

        let decoded = PeerCodec::default().decode(&mut frame).unwrap().unwrap();
        let Message::Piece { data, .. } = decoded else {
            panic!("expected Piece message");
        };

        // Four length bytes, one message id, and two u32 coordinates precede
        // the payload. Pointer identity proves the codec did not copy it.
        assert_eq!(data.as_ptr(), frame_start.wrapping_add(13));
        assert!(frame.is_empty());
    }

    #[test]
    fn piece_views_share_the_read_buffer_charge_until_the_last_view_drops() {
        let governor = peer_buffer_governor(2 * 1024 * 1024);
        let request_lease = governor
            .try_acquire(MemoryClass::PeerBuffer, 16 * 1024)
            .expect("request reservation");
        let mut codec = PeerCodec::with_resources(governor.clone());
        let mut input = BytesMut::with_capacity(512 * 1024);
        assert!(codec.decode(&mut input).unwrap().is_none());
        let old_backing = input.as_ptr();

        let mut wire = BytesMut::new();
        let mut encoder = PeerCodec::default();
        for (piece, byte) in [(1, 0x5a), (2, 0x6b)] {
            encoder
                .encode(
                    Message::Piece {
                        piece,
                        begin: 0,
                        data: Bytes::from(vec![byte; 16 * 1024]),
                    },
                    &mut wire,
                )
                .unwrap();
        }
        input.extend_from_slice(&wire);
        drop(wire);

        let Message::Piece { data: first, .. } = codec.decode(&mut input).unwrap().unwrap() else {
            panic!("expected first Piece message");
        };
        let Message::Piece { data: second, .. } = codec.decode(&mut input).unwrap().unwrap() else {
            panic!("expected second Piece message");
        };
        let first_clone = first.slice(..);
        assert_eq!(first.as_ptr(), old_backing.wrapping_add(13));
        assert_eq!(first[0], 0x5a);
        assert_eq!(second[0], 0x6b);

        assert!(codec.decode(&mut input).unwrap().is_none());
        assert_eq!(input.capacity(), RETAINED_BUFFER_CAPACITY);
        assert_eq!(
            governor.snapshot().total_used_bytes,
            (512 * 1024 + RETAINED_BUFFER_CAPACITY + 16 * 1024) as u64,
            "the old slab is charged once across both Piece views and their clone"
        );

        drop(codec);
        drop(input);
        assert_eq!(
            governor.snapshot().total_used_bytes,
            (512 * 1024 + 16 * 1024) as u64,
            "codec shutdown leaves the old slab charged while Piece owners retain it"
        );
        drop(first);
        drop(second);
        assert_eq!(
            governor.snapshot().total_used_bytes,
            (512 * 1024 + 16 * 1024) as u64,
            "a cloned Bytes slice keeps the backing lease alive"
        );
        drop(first_clone);
        assert_eq!(
            governor.snapshot().total_used_bytes,
            16 * 1024,
            "the old slab lease releases after the final Piece view drops"
        );
        drop(request_lease);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn piece_view_keeps_a_warm_read_buffer_charged_after_codec_shutdown() {
        let governor = peer_buffer_governor(256 * 1024);
        let mut codec = PeerCodec::with_resources(governor.clone());
        let mut input = BytesMut::with_capacity(RETAINED_BUFFER_CAPACITY);
        assert!(codec.decode(&mut input).unwrap().is_none());

        let mut wire = BytesMut::new();
        PeerCodec::default()
            .encode(
                Message::Piece {
                    piece: 4,
                    begin: 0,
                    data: Bytes::from(vec![0x39; 16 * 1024]),
                },
                &mut wire,
            )
            .unwrap();
        input.extend_from_slice(&wire);
        let Message::Piece { data, .. } = codec.decode(&mut input).unwrap().unwrap() else {
            panic!("expected Piece message");
        };
        assert_eq!(
            governor.snapshot().total_used_bytes,
            RETAINED_BUFFER_CAPACITY as u64
        );

        drop(codec);
        drop(input);
        assert_eq!(
            governor.snapshot().total_used_bytes,
            RETAINED_BUFFER_CAPACITY as u64,
            "the data view retains the read slab lease after its codec is dropped"
        );
        assert_eq!(data[0], 0x39);
        drop(data);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn codec_piece_frame_matches_generic_encode() {
        // The Encoder builds the frame directly in `dst` instead of going
        // through `Message::encode`'s owned Vec path. Confirm byte identity.
        let data = bytes::Bytes::from(vec![0xAAu8; 1024]);
        let msg = Message::Piece {
            piece: 1,
            begin: 0,
            data: data.clone(),
        };
        let via_codec = encode_msg(msg.clone());
        let via_generic = msg.encode();
        assert_eq!(via_codec.as_ref(), via_generic.as_slice());
    }

    #[test]
    #[ignore = "local hot-path measurement; run explicitly in release mode"]
    fn benchmark_direct_encode_against_owned_encode() {
        use std::hint::black_box;
        use std::time::Instant;

        let message = Message::Piece {
            piece: 7,
            begin: 16_384,
            data: bytes::Bytes::from(vec![0x42; 16_384]),
        };
        let iterations = 10_000;

        let started = Instant::now();
        let mut owned_bytes = 0usize;
        for _ in 0..iterations {
            let encoded = message.encode();
            owned_bytes = owned_bytes.saturating_add(black_box(encoded.len()));
        }
        let owned_elapsed = started.elapsed();

        let started = Instant::now();
        let mut direct = BytesMut::with_capacity(message.encoded_len().unwrap());
        let mut direct_bytes = 0usize;
        for _ in 0..iterations {
            direct.clear();
            message.encode_into(&mut direct).unwrap();
            direct_bytes = direct_bytes.saturating_add(black_box(direct.len()));
        }
        let direct_elapsed = started.elapsed();

        assert_eq!(owned_bytes, direct_bytes);
        println!(
            "peer-wire encode {iterations} piece frames: owned={owned_elapsed:?}, direct_reused={direct_elapsed:?}, bytes={direct_bytes}"
        );
    }

    #[test]
    fn codec_roundtrip_have_all_and_have_none() {
        let mut buf = encode_msg(Message::HaveAll);
        assert_eq!(
            PeerCodec::default().decode(&mut buf).unwrap().unwrap(),
            Message::HaveAll
        );
        let mut buf = encode_msg(Message::HaveNone);
        assert_eq!(
            PeerCodec::default().decode(&mut buf).unwrap().unwrap(),
            Message::HaveNone
        );
    }

    #[test]
    fn resource_codec_denies_declared_frame_before_growth() {
        let mut class_caps = [0; rt_metrics::MEMORY_CLASS_COUNT];
        class_caps[MemoryClass::PeerBuffer as usize] = 64;
        let governor = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 64,
            class_caps_bytes: class_caps,
            pressure_constrained_pct: 75,
            pressure_critical_pct: 90,
        });
        let mut codec = PeerCodec::with_resources(governor.clone());
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32(MAX_MESSAGE_LEN);

        let error = codec
            .decode(&mut buf)
            .expect_err("frame growth must be denied");
        assert!(matches!(error, WireError::PeerBufferAllocationDenied(_)));
        assert_eq!(buf.capacity(), 4);
        drop(codec);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
        assert_eq!(
            governor.snapshot().classes[MemoryClass::PeerBuffer as usize].denied_allocations,
            1
        );
    }

    #[test]
    fn resource_codec_charges_prefilled_complete_frame() {
        let governor = peer_buffer_governor(64 * 1024);
        let mut codec = PeerCodec::with_resources(governor.clone());
        let mut input = encode_msg(Message::Request {
            piece: 1,
            begin: 0,
            length: 16 * 1024,
        });
        let input_capacity = input.capacity() as u64;

        assert_eq!(
            codec.decode(&mut input).unwrap().unwrap(),
            Message::Request {
                piece: 1,
                begin: 0,
                length: 16 * 1024,
            }
        );
        assert_eq!(governor.snapshot().total_used_bytes, input_capacity);
        drop(codec);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn resource_codec_denies_outbound_frame_before_growth() {
        let mut class_caps = [0; rt_metrics::MEMORY_CLASS_COUNT];
        class_caps[MemoryClass::PeerBuffer as usize] = 64;
        let governor = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 64,
            class_caps_bytes: class_caps,
            pressure_constrained_pct: 75,
            pressure_critical_pct: 90,
        });
        let mut codec = PeerCodec::with_resources(governor.clone());
        let mut buf = BytesMut::with_capacity(4);
        let error = codec
            .encode(
                Message::Extended {
                    ext_id: 1,
                    payload: vec![0; 40],
                },
                &mut buf,
            )
            .expect_err("outbound frame growth must be denied");
        assert!(matches!(error, WireError::PeerBufferAllocationDenied(_)));
        assert_eq!(buf.capacity(), 4);
        drop(codec);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
        assert_eq!(
            governor.snapshot().classes[MemoryClass::PeerBuffer as usize].denied_allocations,
            1
        );
    }

    #[test]
    fn empty_read_buffer_shrinks_and_payload_charge_follows_message_clones() {
        let payload = vec![0x5a; RETAINED_BUFFER_CAPACITY * 8];
        let payload_len = payload.len();
        let mut wire = BytesMut::with_capacity(RETAINED_BUFFER_CAPACITY * 16);
        PeerCodec::default()
            .encode(
                Message::Extended {
                    ext_id: 7,
                    payload: payload.clone(),
                },
                &mut wire,
            )
            .unwrap();
        let mut input = BytesMut::from(&wire[..4]);

        let governor = peer_buffer_governor(8 * 1024 * 1024);
        let mut codec = PeerCodec::with_resources(governor.clone());
        // Framed first presents the length prefix. The codec admits the
        // declared frame before the rest of the bytes are read into the
        // retained buffer.
        assert!(codec.decode(&mut input).unwrap().is_none());
        assert!(input.capacity() > RETAINED_BUFFER_CAPACITY);
        input.extend_from_slice(&wire[4..]);

        let decoded = codec.decode(&mut input).unwrap().unwrap();
        assert!(input.is_empty());
        let used_before_trim = governor.snapshot().total_used_bytes;
        assert!(used_before_trim > RETAINED_BUFFER_CAPACITY as u64);

        assert!(codec.decode(&mut input).unwrap().is_none());

        assert!(
            input.capacity() <= RETAINED_BUFFER_CAPACITY,
            "trimmed read buffer retained {} bytes",
            input.capacity()
        );
        let used_after_trim = governor.snapshot().total_used_bytes;
        let memory = codec.memory.as_ref().expect("read lease state");
        let retained_read_bytes = memory.read_reserved_bytes;
        assert_eq!(used_after_trim, retained_read_bytes + payload_len as u64);
        assert!(used_after_trim < used_before_trim);
        assert_eq!(decoded.encoded_len(), Some(wire.len()));
        assert_eq!(decoded.encode(), wire.to_vec());

        let retained_clone = decoded.clone();
        let (decoded, payload_lease) = decoded.into_accounted_parts().unwrap();
        assert_eq!(
            governor.snapshot().total_used_bytes,
            used_after_trim + payload_len as u64,
            "unwrapping a cloned message creates a separately charged payload copy"
        );
        assert_eq!(
            decoded,
            Message::Extended { ext_id: 7, payload },
            "the parsed message owns its bytes after the read buffer is replaced"
        );
        assert_eq!(payload_lease.as_ref().unwrap().bytes(), payload_len as u64);
        drop(decoded);
        drop(payload_lease);
        assert_eq!(governor.snapshot().total_used_bytes, used_after_trim);
        drop(codec);
        assert_eq!(governor.snapshot().total_used_bytes, payload_len as u64);
        drop(retained_clone);
        assert_eq!(
            governor.snapshot().total_used_bytes,
            0,
            "the final message clone should release its payload charge after codec drop"
        );
    }

    #[test]
    fn empty_write_buffer_shrinks_on_the_next_encode() {
        let governor = peer_buffer_governor(8 * 1024 * 1024);
        let mut codec = PeerCodec::with_resources(governor.clone());
        let mut output = BytesMut::with_capacity(RETAINED_BUFFER_CAPACITY * 16);
        codec
            .encode(
                Message::Extended {
                    ext_id: 7,
                    payload: vec![0x33; RETAINED_BUFFER_CAPACITY * 8],
                },
                &mut output,
            )
            .unwrap();
        let used_before_trim = governor.snapshot().total_used_bytes;
        assert!(used_before_trim > RETAINED_BUFFER_CAPACITY as u64);

        // Framed has drained the previous output before it asks the codec to
        // encode another message.
        output.clear();
        codec.encode(Message::Choke, &mut output).unwrap();

        assert_eq!(output.len(), 5);
        assert!(output.capacity() <= RETAINED_BUFFER_CAPACITY);
        assert!(governor.snapshot().total_used_bytes < used_before_trim);
        let memory = codec.memory.as_ref().expect("write lease state");
        assert_eq!(
            governor.snapshot().total_used_bytes,
            memory.write_reserved_bytes
        );
    }
}
