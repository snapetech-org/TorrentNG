# TorrentNG Storage Release Certification

- Generated: 2026-10-25T23:23:20Z
- Host: kspls0
- Source commit: c4329b2e2e74aa0703ccceaf74165417aae88d82
- Source branch: detached
- Source worktree state: clean
- Targets: /mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage

| Gate | Status | Detail |
| --- | --- | --- |
| source provenance | PASS | clean source commit c4329b2e2e74aa0703ccceaf74165417aae88d82 |
| storage hardware matrix | PASS | physical-device qualification passed; smoke-only targets remain labeled |

## storage hardware matrix

```text
== TorrentNG storage hardware matrix: /mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage (HDD) ==
TorrentNG storage benchmark dir: /mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage
Backing source: /dev/mapper/datapool_lvm-media
Rotational flag: 1
Syscall summary: 0

==> backend_selection_roundtrip_reports_capabilities backend=pread
   Compiling rt-hash v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-hash)
   Compiling rt-path v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-path)
   Compiling rt-metrics v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-metrics)
   Compiling rt-piece-map v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-piece-map)
   Compiling rt-storage v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-storage)
    Finished `release` profile [optimized] target(s) in 21.17s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test backend_selection_roundtrip_reports_capabilities ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-S49MQP profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_backend requested=pread selected=pread reason="forced by storage backend configuration" fixed_buffers=false fixed_buffer_strategy=disabled registered_files=false max_batch_len=1 fixed_buffer_len=0
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.08s


==> backend_selection_roundtrip_reports_capabilities backend=uring
    Finished `release` profile [optimized] target(s) in 0.23s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test backend_selection_roundtrip_reports_capabilities ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-vX0x6F profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_backend requested=uring selected=uring reason="io_uring worker started; registered_files=true fixed_buffers=true" fixed_buffers=true fixed_buffer_strategy=frame_pool_slots registered_files=true max_batch_len=64 fixed_buffer_len=262144
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.01s


==> peer_read_readahead_reduces_backend_reads_on_adjacent_blocks
    Finished `release` profile [optimized] target(s) in 0.14s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test peer_read_readahead_reduces_backend_reads_on_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-ghJNf2 profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_readahead submitted=4096 backend_reads=133 reduction=30.80x elapsed_ms=7654
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 9.09s


==> repeated_reads_reuse_one_open_file_handle
    Finished `release` profile [optimized] target(s) in 0.23s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test repeated_reads_reuse_one_open_file_handle ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-UAPtsn profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_file_pool reads=10000 hits=9999 misses=1 opened=1 capacity=64
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.89s


==> recheck_range_reports_runtime_progress
    Finished `release` profile [optimized] target(s) in 0.14s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test recheck_range_reports_runtime_progress ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-85RTmx profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_recheck pieces=4096 piece_len=16384 total_mib=64.00 valid=4096 invalid=0 missing=0 first_missing=None elapsed_ms=7544 mib_s=8.48 read_ops=4096 backend_reads=4096 hash_ops=4096 hash_latency_ms=140
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 8.80s


==> shuffled_peer_read_baseline_reports_current_scheduler_throughput
    Finished `release` profile [optimized] target(s) in 0.16s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test shuffled_peer_read_baseline_reports_current_scheduler_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-6seDyn profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_shuffled_baseline blocks=4096 block_len=16384 total_mib=64.00 elapsed_ms=3016 mib_s=21.21 read_ops=4096 backend_reads=4096
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 4.87s


==> hdd_peer_read_elevator_reduces_backend_reads_on_shuffled_adjacent_blocks
    Finished `release` profile [optimized] target(s) in 0.15s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test hdd_peer_read_elevator_reduces_backend_reads_on_shuffled_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-sDdtBQ profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_elevator blocks=4096 block_len=16384 total_mib=64.00 elapsed_ms=2511 mib_s=25.48 submitted=4096 backend_reads=1 reduction=4096.00x batches=1 coalesced=4095
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 4.30s

TorrentNG storage elevator trial 1/1: 1.20x baseline/elevator (3016ms/2511ms)

TorrentNG storage elevator wall-clock ratio: 1.20x median baseline/elevator (3016ms/2511ms across 1 trial(s))
storage hardware report: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/storage-hardware-final.md
```
| io_uring graduation | PASS | storage uring graduation report: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/storage-uring-final.md |

## io_uring graduation

```text
== TorrentNG backend stream: pread ==
    Finished `release` profile [optimized] target(s) in 0.13s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test backend_stream_roundtrip_reports_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-KRGsvk profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_backend_stream requested=pread selected=pread reason="forced by storage backend configuration" blocks=1024 block_len=262144 total_mib=256.00 write_elapsed_ms=9133 read_elapsed_ms=34354 write_mib_s=28.03 read_mib_s=7.45 fixed_buffers=false fixed_buffer_strategy=disabled registered_files=false max_batch_len=1 fixed_buffer_len=0
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 46.09s

== TorrentNG backend stream: uring ==
    Finished `release` profile [optimized] target(s) in 0.10s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test backend_stream_roundtrip_reports_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-BwFLZn profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_backend_stream requested=uring selected=uring reason="io_uring worker started; registered_files=true fixed_buffers=true" blocks=1024 block_len=262144 total_mib=256.00 write_elapsed_ms=7447 read_elapsed_ms=54678 write_mib_s=34.37 read_mib_s=4.68 fixed_buffers=true fixed_buffer_strategy=frame_pool_slots registered_files=true max_batch_len=64 fixed_buffer_len=262144
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 65.97s

storage uring graduation report: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/storage-uring-final.md
```
| real-root move/import | PASS | storage move/import report: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/storage-move-import-final.md |

## real-root move/import

```text
storage move/import report: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/storage-move-import-final.md
```
| storage certification index | PASS | storage certification index: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/storage-certification-index-final.md |

## storage certification index

```text
storage certification index: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/storage-certification-index-final.md
```

## Boundaries

- This script runs destructive-safe fixtures only; it creates and removes its own test files under the selected roots.
- Physical PV affinity remains evidence-only because ordinary LV path writes do not select a specific PV.
- `TNG_STORAGE_SKIP_URING=1` and `TNG_STORAGE_SKIP_MOVE_IMPORT=1` fail release reports unless `TNG_STORAGE_ALLOW_RELEASE_SKIP=1` is also set for an explicit dry run.

Overall status: PASS
