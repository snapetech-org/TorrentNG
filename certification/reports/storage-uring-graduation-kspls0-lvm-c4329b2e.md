# TorrentNG io_uring Graduation Report

- Generated: 2026-10-25T23:24:11Z
- Host: kspls0
- Commit: c4329b2
- Target: /mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage
- Blocks: 1024
- Block length: 262144

## pread

- Result: PASS
- Selected: pread
- Read MiB/s: 7.45
- Write MiB/s: 28.03
- Fixed buffers: false
- Fixed-buffer strategy: disabled
- Registered files: false

```text
    Finished `release` profile [optimized] target(s) in 0.13s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test backend_stream_roundtrip_reports_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-KRGsvk profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_backend_stream requested=pread selected=pread reason="forced by storage backend configuration" blocks=1024 block_len=262144 total_mib=256.00 write_elapsed_ms=9133 read_elapsed_ms=34354 write_mib_s=28.03 read_mib_s=7.45 fixed_buffers=false fixed_buffer_strategy=disabled registered_files=false max_batch_len=1 fixed_buffer_len=0
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 46.09s

```

## uring

- Result: PASS
- Selected: uring
- Read MiB/s: 4.68
- Write MiB/s: 34.37
- Fixed buffers: true
- Fixed-buffer strategy: frame_pool_slots
- Registered files: true

```text
    Finished `release` profile [optimized] target(s) in 0.10s
     Running tests/storage_real_device.rs (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/target/release/deps/storage_real_device-cb9e975d2ec1fefe)

running 1 test
test backend_stream_roundtrip_reports_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-BwFLZn profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
tng_storage_backend_stream requested=uring selected=uring reason="io_uring worker started; registered_files=true fixed_buffers=true" blocks=1024 block_len=262144 total_mib=256.00 write_elapsed_ms=7447 read_elapsed_ms=54678 write_mib_s=34.37 read_mib_s=4.68 fixed_buffers=true fixed_buffer_strategy=frame_pool_slots registered_files=true max_batch_len=64 fixed_buffer_len=262144
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 65.97s

```

## Graduation Gates

| Gate | Result |
| --- | --- |
| uring selected | PASS |
| fixed buffers | INFO: true |
| fixed-buffer strategy frame_pool_slots | PASS |
| registered files | INFO: true |
| read throughput ratio | INFO: not required |
| write throughput ratio | INFO: not required |

Overall status: PASS
