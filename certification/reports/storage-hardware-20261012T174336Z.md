# TorrentNG Storage Hardware Matrix

- Generated: 2026-10-12T17:43:37Z
- Host: kspls0
- Commit: 687e2c8a
- Worktree state: clean
- Blocks: 8192
- Hot reads: 10000
- Syscall counts: 0

## /mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz

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

    test backend_selection_roundtrip_reports_capabilities ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-inqD80 profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_backend requested=pread selected=pread reason="forced by storage backend configuration" fixed_buffers=false fixed_buffer_strategy=disabled registered_files=false max_batch_len=1 fixed_buffer_len=0
    test backend_selection_roundtrip_reports_capabilities ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-ahkUgY profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_backend requested=uring selected=uring reason="io_uring probe succeeded; registered_files=true fixed_buffers=true" fixed_buffers=true fixed_buffer_strategy=frame_pool_slots registered_files=true max_batch_len=64 fixed_buffer_len=262144
    test peer_read_readahead_reduces_backend_reads_on_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-YcNsFV profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_readahead submitted=8192 backend_reads=256 reduction=32.00x elapsed_ms=1043
    test repeated_reads_reuse_one_open_file_handle ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-l75oBe profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_file_pool reads=10000 hits=9999 misses=1 open_files=1 capacity=64
    test recheck_range_reports_runtime_progress ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-Q6Y09H profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_recheck pieces=4096 piece_len=16384 total_mib=64.00 valid=4096 invalid=0 missing=0 first_missing=None elapsed_ms=1415 mib_s=45.22 read_ops=4096 backend_reads=4096 hash_ops=4096 hash_latency_ms=83
    test shuffled_peer_read_baseline_reports_current_scheduler_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-VNV4cX profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_shuffled_baseline blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=4673 mib_s=27.39 read_ops=8192 backend_reads=8192
    test hdd_peer_read_elevator_reduces_backend_reads_on_shuffled_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-bNriui profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_elevator blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=2202 mib_s=58.10 submitted=8192 backend_reads=1 reduction=8192.00x batches=1 coalesced=8191
    test shuffled_peer_read_baseline_reports_current_scheduler_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-AgMaoa profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_shuffled_baseline blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=8265 mib_s=15.49 read_ops=8192 backend_reads=8192
    test hdd_peer_read_elevator_reduces_backend_reads_on_shuffled_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-GzukOR profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_elevator blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=2784 mib_s=45.97 submitted=8192 backend_reads=1 reduction=8192.00x batches=1 coalesced=8191
    test shuffled_peer_read_baseline_reports_current_scheduler_throughput ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-Rhtrjy profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_shuffled_baseline blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=4413 mib_s=29.00 read_ops=8192 backend_reads=8192
    test hdd_peer_read_elevator_reduces_backend_reads_on_shuffled_adjacent_blocks ... tng_storage_bench_path=/mnt/datapool_lvm_media/.torrentng-engine-benchmark.6errmz/tng-storage-bench-MnnFp8 profile=Hdd fs=Some("ext4") cow=false device=Some(DeviceId("dm-0"))
    tng_storage_elevator blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=1392 mib_s=91.89 submitted=8192 backend_reads=1 reduction=8192.00x batches=1 coalesced=8191
    TorrentNG storage elevator wall-clock ratio: 2.97x median baseline/elevator (4673ms/2202ms across 3 trial(s))

## Gate

PASS

Overall status: PASS
