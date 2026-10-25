# TorrentNG Storage Hardware Matrix

- Generated: 2026-10-25T23:23:20Z
- Host: kspls0
- Commit: c4329b2
- Worktree state: clean
- Blocks: 4096
- Hot reads: 10000
- Syscall counts: 0

## /mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage

| Field | Value |
| --- | --- |
| mount source | /dev/mapper/datapool_lvm-media |
| filesystem | ext4 |
| root block | /dev/dm-0 |
| physical device type | disk |
| rotational | 1 |
| inferred profile | HDD |
| hardware qualification | PASS |
| qualification detail | block device and filesystem identified |
| HDD 5x gate | not enforced |

- Result: PASS

Summary:

    test backend_selection_roundtrip_reports_capabilities ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-S49MQP profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_backend requested=pread selected=pread reason="forced by storage backend configuration" fixed_buffers=false fixed_buffer_strategy=disabled registered_files=false max_batch_len=1 fixed_buffer_len=0
    test backend_selection_roundtrip_reports_capabilities ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-vX0x6F profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_backend requested=uring selected=uring reason="io_uring worker started; registered_files=true fixed_buffers=true" fixed_buffers=true fixed_buffer_strategy=frame_pool_slots registered_files=true max_batch_len=64 fixed_buffer_len=262144
    test peer_read_readahead_reduces_backend_reads_on_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-ghJNf2 profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_readahead submitted=4096 backend_reads=133 reduction=30.80x elapsed_ms=7654
    test repeated_reads_reuse_one_open_file_handle ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-UAPtsn profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_file_pool reads=10000 hits=9999 misses=1 opened=1 capacity=64
    test recheck_range_reports_runtime_progress ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-85RTmx profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_recheck pieces=4096 piece_len=16384 total_mib=64.00 valid=4096 invalid=0 missing=0 first_missing=None elapsed_ms=7544 mib_s=8.48 read_ops=4096 backend_reads=4096 hash_ops=4096 hash_latency_ms=140
    test shuffled_peer_read_baseline_reports_current_scheduler_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-6seDyn profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_shuffled_baseline blocks=4096 block_len=16384 total_mib=64.00 elapsed_ms=3016 mib_s=21.21 read_ops=4096 backend_reads=4096
    test hdd_peer_read_elevator_reduces_backend_reads_on_shuffled_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage/tng-storage-bench-sDdtBQ profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_elevator blocks=4096 block_len=16384 total_mib=64.00 elapsed_ms=2511 mib_s=25.48 submitted=4096 backend_reads=1 reduction=4096.00x batches=1 coalesced=4095
    TorrentNG storage elevator wall-clock ratio: 1.20x median baseline/elevator (3016ms/2511ms across 1 trial(s))

## Gate

PASS

Overall status: PASS
