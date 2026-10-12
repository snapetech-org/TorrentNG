# TorrentNG Backend Engine Resource Benchmark

- Started: 2026-10-12T17:43:37Z
- Host: kspls0
- Source revision: 181f2453e62a4a02cab2f3a22f97e6fbc610d005
- Source tree: 12bccc8037cef1aba657400954d60bfbce79f431
- Binary: /home/keith/src/TorrentNG/target/release/torrentngd (torrentngd 0.1.0)
- Binary SHA-256: 023814807f0d58b29f3cb2e3f4d1c9c5071dd188fba51249d8aad8d16abf7840
- Kernel: Linux 7.2.4-arch1-2 x86_64 GNU/Linux
- Host CPU threads: 32
- Benchmark process CPU affinity: 30-31 (2 CPUs)
- RAM: 62.7 GiB
- Host load at start: 170.43, 158.33, 130.80
- Available RAM at start: 14.3 GiB
- Scratch filesystem: /dev/nvme0n1p2 ext4
- Storage target filesystem: /dev/mapper/datapool_lvm-media ext4
- Storage target devices: datapool_lvm-media 1,|-sda              1,|-sdb              1,|-sdc              1,|-sdd              1,|-sde              1,|-sdf              1,|-sdg              1,|-sdh              1,|-sdi              1,|-sdj              1,|-sdl              1,`-sdm              1
- Temporary API bind: 127.0.0.1:28182
- Temporary peer/DHT port: 44445
- Max peers: 32
- Download/upload caps: 2097152/1048576 bytes/sec
- Process scheduling: effective nice 19 (outer nice +15 plus child nice +10), idle I/O priority
- Public source: https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- ISO filename: debian-13.7.0-amd64-netinst.iso
- Completion timeout: 1200s

## Storage benchmark

The existing real-device matrix compares selected storage backends, adjacent-read readahead, file-handle reuse, recheck/hash work, and HDD elevator scheduling. Its temporary files are scoped to a unique directory on the selected mount.

- Storage report: /home/keith/src/TorrentNG/certification/reports/storage-hardware-20261012T174336Z.md

System-wide device sample before the benchmark (includes unrelated workloads):
Linux 7.2.4-arch1-2 (kspls0) 	10/12/26 	_x86_64_	(32 CPU)

Device            r/s     rkB/s   rrqm/s  %rrqm r_await rareq-sz     w/s     wkB/s   wrqm/s  %wrqm w_await wareq-sz     d/s     dkB/s   drqm/s  %drqm d_await dareq-sz     f/s f_await  aqu-sz  %util
dm-0           606.80  28695.52     0.00   0.00    0.49    47.29  199.37   4084.43     0.00   0.00    5.22    20.49    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    1.34  69.10
nvme0n1        770.98  14762.71    95.77  11.05    0.23    19.15  401.21   7974.25   463.91  53.62    1.39    19.88    0.72    924.14     0.00   0.00    0.43  1288.55   31.39    0.52    0.75  14.36
sda             26.85   1179.57     2.55   8.68    2.04    43.94   36.48    102.36     0.12   0.33    1.77     2.81    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.98    0.14   5.19
sdb            117.60   2965.47     1.07   0.90    3.53    25.22   34.79    793.36     2.44   6.55    3.72    22.81    0.00      0.00     0.00   0.00    0.00     0.00   18.86    2.97    0.60  16.43
sdc            133.05  10283.67    19.15  12.58    2.76    77.29   20.96    850.09     0.39   1.83    4.26    40.57    0.00      0.00     0.00   0.00    0.00     0.00   18.86    2.73    0.51  17.59
sdd             34.69   1716.28     0.55   1.57    2.11    49.48   23.05     95.15     0.24   1.01    1.11     4.13    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.87    0.12   5.54
sde             16.45   1247.55     0.62   3.62    4.43    75.85   22.75    154.44     0.55   2.35    2.71     6.79    0.00      0.00     0.00   0.00    0.00     0.00   18.87    2.62    0.18   8.70
sdf             29.59   1142.97     0.69   2.26    2.68    38.63   21.04    558.97     0.18   0.84    2.68    26.57    0.00      0.00     0.00   0.00    0.00     0.00   18.86    2.25    0.18   8.08
sdg             33.80   1396.33     2.96   8.04    1.45    41.31   19.23    121.22     0.12   0.63    0.85     6.30    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.64    0.08   4.06
sdh             18.52    953.02     0.74   3.86    1.64    51.45   19.81    153.94     0.13   0.68    1.04     7.77    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.78    0.07   3.61
sdi             12.72    883.78     0.51   3.89    2.96    69.50   19.56    143.37     0.13   0.67    1.64     7.33    0.00      0.00     0.00   0.00    0.00     0.00   18.87    1.34    0.09   4.52
sdj             23.22   1341.01     2.55   9.88    2.04    57.76   19.42     67.95     0.08   0.39    0.78     3.50    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.65    0.07   3.88
sdk              0.00      0.02     0.00   0.00    0.41    65.08    0.00      0.00     0.00   0.00    0.00     0.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.00
sdl             25.60   1585.86     1.31   4.88    2.16    61.96   19.68    315.80     0.11   0.56    1.31    16.05    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.88    0.10   4.56
sdm             97.88   4000.14     4.40   4.31    2.60    40.87   45.73    727.78   101.13  68.86    4.28    15.91    0.00      0.00     0.00   0.00    0.00     0.00   30.23    3.23    0.55  32.94
zram0           14.21     56.86     0.00   0.00    0.02     4.00   13.54     54.15     0.00   0.00    0.03     4.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.07


Device            r/s     rkB/s   rrqm/s  %rrqm r_await rareq-sz     w/s     wkB/s   wrqm/s  %wrqm w_await wareq-sz     d/s     dkB/s   drqm/s  %drqm d_await dareq-sz     f/s f_await  aqu-sz  %util
dm-0           938.00  11428.00     0.00   0.00    1.66    12.18  342.00   2240.00     0.00   0.00    3.63     6.55    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    2.80  81.40
nvme0n1       3741.00  33424.00    29.00   0.77    0.64     8.93 4068.00 273224.00  3842.00  48.57   40.06    67.16    0.00      0.00     0.00   0.00    0.00     0.00   36.00    3.72  165.46  89.30
sda              0.00      0.00     0.00   0.00    0.00     0.00  176.00    896.00     0.00   0.00    1.53     5.09    0.00      0.00     0.00   0.00    0.00     0.00   17.00    3.41    0.33   7.40
sdb            105.00   1680.00     0.00   0.00    4.32    16.00   36.00    340.00     4.00  10.00    0.67     9.44    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.88    0.49  34.00
sdc            136.00    916.00     0.00   0.00    0.07     6.74   19.00    404.00     0.00   0.00    1.05    21.26    0.00      0.00     0.00   0.00    0.00     0.00   17.00    1.06    0.05   3.00
sdd             44.00    176.00     0.00   0.00    0.59     4.00   36.00     76.00     0.00   0.00    0.67     2.11    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.29    0.06   4.10
sde              5.00    828.00     0.00   0.00    0.60   165.60   21.00     48.00     1.00   4.55    0.67     2.29    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.71    0.03   1.50
sdf            125.00    500.00     0.00   0.00    0.29     4.00   21.00     16.00     0.00   0.00    0.43     0.76    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.41    0.05   4.80
sdg             65.00    516.00     0.00   0.00    0.15     7.94   17.00      0.00     0.00   0.00    0.24     0.00    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.24    0.02   1.20
sdh              1.00      4.00     0.00   0.00    0.00     4.00   18.00      4.00     0.00   0.00    0.39     0.22    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.41    0.01   0.80
sdi              1.00     20.00     0.00   0.00    1.00    20.00   17.00      0.00     0.00   0.00    0.41     0.00    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.35    0.01   0.60
sdj              1.00     96.00     0.00   0.00    0.00    96.00   17.00      0.00     0.00   0.00    0.29     0.00    0.00      0.00     0.00   0.00    0.00     0.00   17.00    0.29    0.01   0.60
sdk              0.00      0.00     0.00   0.00    0.00     0.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.00
sdl              0.00      0.00     0.00   0.00    0.00     0.00   20.00     20.00     0.00   0.00    1.60     1.00    0.00      0.00     0.00   0.00    0.00     0.00   17.00    1.76    0.06   3.10
sdm            446.00   6684.00     8.00   1.76    1.98    14.99   41.00    436.00    85.00  67.46    1.98    10.63    0.00      0.00     0.00   0.00    0.00     0.00   27.00    1.19    1.00  67.80
zram0           21.00     84.00     0.00   0.00    0.05     4.00    1.00      4.00     0.00   0.00    0.00     4.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.10


- Matrix result: PASS
- Matrix report exists: yes
- Elevator/readahead/backend lines:
text output follows
30:    tng_storage_backend requested=pread selected=pread reason="forced by storage backend configuration" fixed_buffers=false fixed_buffer_strategy=disabled registered_files=false max_batch_len=1 fixed_buffer_len=0
32:    tng_storage_backend requested=uring selected=uring reason="io_uring probe succeeded; registered_files=true fixed_buffers=true" fixed_buffers=true fixed_buffer_strategy=frame_pool_slots registered_files=true max_batch_len=64 fixed_buffer_len=262144
34:    tng_storage_readahead submitted=8192 backend_reads=256 reduction=32.00x elapsed_ms=1043
36:    tng_storage_file_pool reads=10000 hits=9999 misses=1 open_files=1 capacity=64
38:    tng_storage_recheck pieces=4096 piece_len=16384 total_mib=64.00 valid=4096 invalid=0 missing=0 first_missing=None elapsed_ms=1415 mib_s=45.22 read_ops=4096 backend_reads=4096 hash_ops=4096 hash_latency_ms=83
40:    tng_storage_shuffled_baseline blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=4673 mib_s=27.39 read_ops=8192 backend_reads=8192
42:    tng_storage_elevator blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=2202 mib_s=58.10 submitted=8192 backend_reads=1 reduction=8192.00x batches=1 coalesced=8191
44:    tng_storage_shuffled_baseline blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=8265 mib_s=15.49 read_ops=8192 backend_reads=8192
46:    tng_storage_elevator blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=2784 mib_s=45.97 submitted=8192 backend_reads=1 reduction=8192.00x batches=1 coalesced=8191
48:    tng_storage_shuffled_baseline blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=4413 mib_s=29.00 read_ops=8192 backend_reads=8192
50:    tng_storage_elevator blocks=8192 block_len=16384 total_mib=128.00 elapsed_ms=1392 mib_s=91.89 submitted=8192 backend_reads=1 reduction=8192.00x batches=1 coalesced=8191
51:    TorrentNG storage elevator wall-clock ratio: 2.97x median baseline/elevator (4673ms/2202ms across 3 trial(s))

## Public ISO transfer

- Status: PASS
- Torrent: https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- Filename: debian-13.7.0-amd64-netinst.iso
- Last API sample: {"sample_utc":"2026-10-12T17:52:41Z","elapsed_seconds":422,"progress":1.0,"downloaded":836665344,"dlspeed":2250158,"upspeed":0,"num_seeds":27,"num_leechs":5,"state":"uploading"}
- Bytes reported downloaded: 836665344
- Elapsed transfer seconds: 422
- Mean API download rate: 1.89 MiB/s
- Download cap: 2097152 bytes/sec
- Poll samples: 84
- Official checksum status: PASS
- SHA-256 expected: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- SHA-256 actual: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355

## Memory-governor and frame-pool metrics

These counters describe managed memory classes and storage frames; process RSS is reported separately below.

/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_cap_bytes 536870912
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_used_bytes 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_pressure_state 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="storage_frame"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="storage_frame"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="storage_frame"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="piece_assembly"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="piece_assembly"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="piece_assembly"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="peer_buffer"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="peer_buffer"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="peer_buffer"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="webseed_body"} 67108864
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="webseed_body"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="webseed_body"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="metadata"} 33554432
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="metadata"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="metadata"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="tracker_peers"} 33554432
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="tracker_peers"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="tracker_peers"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="dht_table"} 33554432
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="dht_table"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="dht_table"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="queued_disk"} 67108864
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="queued_disk"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="queued_disk"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="api_snapshot"} 16777216
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="api_snapshot"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="api_snapshot"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_cap_bytes{class="piece_index"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_used_bytes{class="piece_index"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_memory_class_denied_allocations_total{class="piece_index"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_storage_frame_bytes_in_use 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-before.txt:torrentng_storage_frame_bytes_cap 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_cap_bytes 536870912
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_used_bytes 8088140
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_pressure_state 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="storage_frame"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="storage_frame"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="storage_frame"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="piece_assembly"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="piece_assembly"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="piece_assembly"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="peer_buffer"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="peer_buffer"} 7753664
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="peer_buffer"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="webseed_body"} 67108864
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="webseed_body"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="webseed_body"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="metadata"} 33554432
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="metadata"} 160428
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="metadata"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="tracker_peers"} 33554432
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="tracker_peers"} 32768
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="tracker_peers"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="dht_table"} 33554432
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="dht_table"} 2016
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="dht_table"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="queued_disk"} 67108864
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="queued_disk"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="queued_disk"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="api_snapshot"} 16777216
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="api_snapshot"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="api_snapshot"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_cap_bytes{class="piece_index"} 134217728
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_used_bytes{class="piece_index"} 139264
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_memory_class_denied_allocations_total{class="piece_index"} 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_storage_frame_bytes_in_use 0
/var/tmp/torrentng-engine-benchmark.gNBV12/metrics-after.txt:torrentng_storage_frame_bytes_cap 134217728
Peak sampled governor bytes/cap: 35126290/536870912; frame bytes/cap: 0/134217728; samples=84

## Process resource samples

Samples came from /proc for the isolated torrentngd PID; CPU is percent of one CPU core and RSS is resident MiB. Process I/O counters are kernel-accounted block bytes, not network bytes.

| Phase | Samples | Mean CPU (% of one core) | Peak CPU (% of one core) | Peak RSS (MiB) |
|---|---:|---:|---:|---:|
| idle | 7 | 0.07 | 0.50 | 25.25 |
| transfer | 215 | 7.22 | 13.47 | 71.91 |
| after-transfer | 3 | 0.33 | 1.00 | 29.92 |

- Process CPU time: 31.06 seconds
- Process block read bytes: 14409728
- Process block write bytes: 1606553600
- Raw samples: 225; interval: 2 seconds

## Host context after run

- Host load at end: 214.09, 192.77, 162.29
- Available RAM at end: 12.8 GiB
- iostat sample (system-wide; includes unrelated workloads):
System-wide device data follows:
Linux 7.2.4-arch1-2 (kspls0) 	10/12/26 	_x86_64_	(32 CPU)

Device            r/s     rkB/s   rrqm/s  %rrqm r_await rareq-sz     w/s     wkB/s   wrqm/s  %wrqm w_await wareq-sz     d/s     dkB/s   drqm/s  %drqm d_await dareq-sz     f/s f_await  aqu-sz  %util
dm-0           607.03  28691.97     0.00   0.00    0.49    47.27  199.37   4084.20     0.00   0.00    5.22    20.49    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    1.34  69.10
nvme0n1        771.65  14773.60    95.82  11.05    0.23    19.15  401.42   7982.71   464.33  53.63    1.40    19.89    0.72    923.91     0.00   0.00    0.43  1288.55   31.40    0.52    0.75  14.37
sda             26.84   1179.28     2.55   8.68    2.04    43.94   36.48    102.35     0.12   0.33    1.77     2.81    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.98    0.14   5.19
sdb            117.65   2965.88     1.07   0.90    3.53    25.21   34.79    793.65     2.44   6.55    3.72    22.81    0.00      0.00     0.00   0.00    0.00     0.00   18.86    2.97    0.60  16.43
sdc            133.03  10281.21    19.15  12.58    2.76    77.28   20.96    849.95     0.39   1.83    4.26    40.56    0.00      0.00     0.00   0.00    0.00     0.00   18.86    2.73    0.51  17.58
sdd             34.70   1716.07     0.55   1.57    2.11    49.46   23.05     95.13     0.24   1.01    1.11     4.13    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.87    0.12   5.54
sde             16.45   1247.28     0.62   3.62    4.43    75.84   22.75    154.41     0.55   2.35    2.71     6.79    0.00      0.00     0.00   0.00    0.00     0.00   18.87    2.62    0.18   8.70
sdf             29.67   1143.17     0.69   2.26    2.68    38.54   21.04    558.84     0.18   0.84    2.68    26.56    0.00      0.00     0.00   0.00    0.00     0.00   18.87    2.25    0.18   8.09
sdg             33.86   1396.52     2.96   8.03    1.44    41.24   19.23    121.22     0.12   0.63    0.85     6.30    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.64    0.08   4.06
sdh             18.52    952.80     0.74   3.86    1.64    51.45   19.81    153.90     0.13   0.68    1.04     7.77    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.78    0.07   3.61
sdi             12.71    883.59     0.51   3.89    2.96    69.50   19.56    143.34     0.13   0.67    1.64     7.33    0.00      0.00     0.00   0.00    0.00     0.00   18.87    1.34    0.09   4.52
sdj             23.21   1340.72     2.54   9.88    2.04    57.76   19.42     67.93     0.08   0.39    0.78     3.50    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.65    0.07   3.88
sdk              0.00      0.02     0.00   0.00    0.41    65.08    0.00      0.00     0.00   0.00    0.00     0.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.00
sdl             25.60   1585.56     1.31   4.88    2.16    61.95   19.68    315.76     0.11   0.56    1.31    16.05    0.00      0.00     0.00   0.00    0.00     0.00   18.87    0.88    0.10   4.56
sdm             97.96   4000.03     4.40   4.30    2.60    40.84   45.73    727.72   101.13  68.86    4.28    15.91    0.00      0.00     0.00   0.00    0.00     0.00   30.23    3.23    0.55  32.94
zram0           14.22     56.88     0.00   0.00    0.02     4.00   13.54     54.18     0.00   0.00    0.03     4.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.07


Device            r/s     rkB/s   rrqm/s  %rrqm r_await rareq-sz     w/s     wkB/s   wrqm/s  %wrqm w_await wareq-sz     d/s     dkB/s   drqm/s  %drqm d_await dareq-sz     f/s f_await  aqu-sz  %util
dm-0           660.00   5912.00     0.00   0.00    3.49     8.96  420.00   1532.00     0.00   0.00    3.24     3.65    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    3.67  97.50
nvme0n1       2054.00  78656.00     7.00   0.34    0.26    38.29 1768.00  83548.00   370.00  17.31    8.95    47.26    0.00      0.00     0.00   0.00    0.00     0.00   60.00    0.65   16.40  55.00
sda              0.00      0.00     0.00   0.00    0.00     0.00   59.00      0.00     0.00   0.00    0.51     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.49    0.06   2.80
sdb            279.00   2576.00     3.00   1.06    6.56     9.23  123.00    336.00     5.00   3.91    3.37     2.73    0.00      0.00     0.00   0.00    0.00     0.00   58.00    6.31    2.61  94.20
sdc             11.00    248.00     0.00   0.00    1.09    22.55   59.00      0.00     0.00   0.00    0.76     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.76    0.10   5.50
sdd             50.00    232.00     0.00   0.00    1.12     4.64   74.00     60.00     0.00   0.00    0.73     0.81    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.78    0.16   8.90
sde              5.00     20.00     0.00   0.00    2.60     4.00   71.00     96.00     5.00   6.58    1.01     1.35    0.00      0.00     0.00   0.00    0.00     0.00   59.00    1.15    0.15   8.00
sdf             10.00    100.00     0.00   0.00    2.50    10.00   59.00      0.00     0.00   0.00    0.69     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.69    0.11   5.50
sdg             57.00    228.00     0.00   0.00    0.33     4.00   59.00      0.00     0.00   0.00    0.41     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.41    0.07   4.00
sdh             56.00   1096.00     9.00  13.85    0.21    19.57   59.00      0.00     0.00   0.00    0.29     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.29    0.04   2.20
sdi              0.00      0.00     0.00   0.00    0.00     0.00   59.00      0.00     0.00   0.00    0.54     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.53    0.06   3.60
sdj              4.00     16.00     0.00   0.00    0.25     4.00   59.00      0.00     0.00   0.00    0.37     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.37    0.04   2.00
sdk              0.00      0.00     0.00   0.00    0.00     0.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.00
sdl              0.00      0.00     0.00   0.00    0.00     0.00   59.00      0.00     0.00   0.00    0.93     0.00    0.00      0.00     0.00   0.00    0.00     0.00   59.00    0.92    0.11   5.50
sdm            176.00   1396.00     0.00   0.00    1.60     7.93  119.00   1040.00   189.00  61.36    2.19     8.74    0.00      0.00     0.00   0.00    0.00     0.00   89.00    1.52    0.68  39.80
zram0            6.00     24.00     0.00   0.00    0.17     4.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00      0.00     0.00   0.00    0.00     0.00    0.00    0.00    0.00   0.10


- Storage result: PASS
- Torrent completion result: PASS
- Official ISO checksum result: PASS
- The daemon ran with its API bound to loopback and exited before cleanup.

## Cleanup

- Torrent/data/config/session scratch removed: yes
- Storage benchmark scratch removed: yes
- Storage result: PASS; ISO result: PASS; checksum result: PASS
- Exit status: 0
