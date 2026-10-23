use bytes::{BufMut, BytesMut};
use rt_peer_wire::{Message, PeerCodec};
use std::time::Instant;
use tokio_util::codec::Decoder;

fn governor() -> rt_metrics::ResourceGovernor {
    let cap = 512 * 1024 * 1024;
    let mut class_caps = [0; rt_metrics::MEMORY_CLASS_COUNT];
    class_caps[rt_metrics::MemoryClass::PeerBuffer as usize] = cap;
    rt_metrics::ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
        total_cap_bytes: cap,
        class_caps_bytes: class_caps,
        pressure_constrained_pct: 75,
        pressure_critical_pct: 90,
    })
}

fn process_cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    assert_eq!(result, 0);
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1_000_000.0;
    let system = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
    user + system
}

fn main() {
    let frames: usize = std::env::args().nth(1).unwrap_or_else(|| "8000000".into()).parse().unwrap();
    let mode = std::env::args().nth(2).unwrap_or_else(|| "resources".into());
    assert!(mode == "resources" || mode == "default");

    let mut input = BytesMut::with_capacity(frames * 17);
    for i in 0..frames {
        input.put_u32(13);
        input.put_u8(6);
        input.put_u32(i as u32);
        input.put_u32(((i as u32) & 127) * 16_384);
        input.put_u32(16_384);
    }

    let mut codec = if mode == "resources" {
        PeerCodec::with_resources(governor())
    } else {
        PeerCodec::default()
    };
    let mut checksum = 0u64;
    let wall_start = Instant::now();
    let cpu_start = process_cpu_seconds();
    for _ in 0..frames {
        let message = codec.decode(&mut input).unwrap().unwrap();
        let Message::Request { piece, begin, length } = message else {
            panic!("unexpected decoded message");
        };
        checksum = checksum.wrapping_add((piece as u64).rotate_left(7) ^ (begin as u64) ^ length as u64);
    }
    let cpu = process_cpu_seconds() - cpu_start;
    let wall = wall_start.elapsed().as_secs_f64();
    assert!(input.is_empty());
    assert_ne!(std::hint::black_box(checksum), 0);
    println!("mode={mode} frames={frames} cpu_s={cpu:.6} wall_s={wall:.6} msg_per_cpu_s={:.0} checksum={checksum}", frames as f64 / cpu);
}
