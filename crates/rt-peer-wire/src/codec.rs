/// tokio-util `Codec` impl for BEP 3 length-prefixed peer messages.
use bytes::{Buf, BytesMut};
use rt_metrics::{MemoryClass, MemoryLease, ResourceGovernor};
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
    read_memory_lease: Option<MemoryLease>,
    write_memory_lease: Option<MemoryLease>,
    /// Keep one high-water allowance for a parsed message that the caller may
    /// still own after the framed read buffer has been trimmed.
    read_copy_high_water_bytes: u64,
    read_reserved_bytes: u64,
    write_reserved_bytes: u64,
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
        // size, plus a same-sized allowance for Message::parse's owned
        // Bitfield/Extended payload copy. The copy allowance stays at its
        // high-water mark because the emitted Message may outlive this codec
        // call; only the actual empty read buffer is trimmed later.
        let accounted_capacity = src.capacity().max(required);
        self.read_buffer_high_water_bytes =
            self.read_buffer_high_water_bytes.max(accounted_capacity);
        let resources = self.resources.clone();
        let mut capacity_bytes = 0;
        if let Some(resources) = resources.as_ref() {
            let memory = self
                .memory
                .get_or_insert_with(|| Box::new(PeerCodecMemory::default()));
            capacity_bytes = u64::try_from(accounted_capacity)
                .ok()
                .ok_or(WireError::PeerBufferLengthOverflow)?;
            let copy_high_water = memory.read_copy_high_water_bytes.max(capacity_bytes);
            let required_bytes = copy_high_water
                .checked_add(capacity_bytes)
                .ok_or(WireError::PeerBufferLengthOverflow)?;
            ensure_peer_buffer_reservation(
                resources,
                &mut memory.read_memory_lease,
                &mut memory.read_reserved_bytes,
                required_bytes,
            )?;
            memory.read_copy_high_water_bytes = copy_high_water;
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
            if actual_capacity > capacity_bytes {
                let memory = self
                    .memory
                    .get_or_insert_with(|| Box::new(PeerCodecMemory::default()));
                let copy_high_water = memory.read_copy_high_water_bytes.max(actual_capacity);
                let required_bytes = copy_high_water
                    .checked_add(actual_capacity)
                    .ok_or(WireError::PeerBufferLengthOverflow)?;
                ensure_peer_buffer_reservation(
                    resources,
                    &mut memory.read_memory_lease,
                    &mut memory.read_reserved_bytes,
                    required_bytes,
                )?;
                memory.read_copy_high_water_bytes = copy_high_water;
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

        *src = BytesMut::with_capacity(RETAINED_BUFFER_CAPACITY);
        self.read_buffer_high_water_bytes = src.capacity();
        let Some(buffer_bytes) = u64::try_from(src.capacity()).ok() else {
            return;
        };
        let Some(memory) = self.memory.as_mut() else {
            return;
        };
        let Some(required_bytes) = memory.read_copy_high_water_bytes.checked_add(buffer_bytes)
        else {
            return;
        };
        if required_bytes < memory.read_reserved_bytes {
            if let Some(lease) = memory.read_memory_lease.as_mut() {
                lease.shrink_to(required_bytes);
                memory.read_reserved_bytes = required_bytes;
            }
        }
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
        src.advance(4);
        let payload = src.split_to(len as usize).freeze();
        let msg = Message::parse_owned(payload)?;
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
    fn empty_read_buffer_shrinks_but_keeps_live_message_copy_credit() {
        let payload = vec![0x5a; RETAINED_BUFFER_CAPACITY * 8];
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
        assert!(used_after_trim < used_before_trim);
        let memory = codec.memory.as_ref().expect("read lease state");
        assert_eq!(used_after_trim, memory.read_reserved_bytes);
        assert!(used_after_trim >= memory.read_copy_high_water_bytes);
        assert_eq!(
            decoded,
            Message::Extended { ext_id: 7, payload },
            "the parsed message owns its bytes after the read buffer is replaced"
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
