# Benchmarks

Compatible-client service benchmarks that can run without a live rTorrent
client live in `sidecar/tests/benchmarks.rs`. The source path is retained for
repository compatibility.

Run the synthetic API checks explicitly:

```sh
cd sidecar
cargo test --test benchmarks -- --ignored --nocapture
```

Use a bounded local fixture while diagnosing a regression:

Set `TNG_BENCH_TORRENTS` to a bounded fixture size while diagnosing a
regression, then run the same command above.

Generate a markdown report:

```sh
./scripts/benchmark_report.sh
```

The report runner uses `cargo test --release` because optimized behavior is
useful for diagnosis. The generated report is informational and is not a
release gate or torrent-count capacity certificate.

Current covered regression surfaces:

- qBit `/api/qb/v2/torrents/info` serialization and pagination
- qBit `/api/qb/v2/sync/maindata` delta under normal churn

Storage hardware checks live in `rt-storage` and can be run against a specific
mount:

```sh
scripts/storage_real_device_benchmark.sh /path/on/storage
```

Recent hardware results are tracked in
`benchmarks/STORAGE_REAL_DEVICE_RESULTS.md`.

## Whole-client CPU and memory comparison

The whole-client harness lives in
`scripts/client_performance_benchmark.py`; detailed runs and their raw samples
are retained under `certification/reports/`. It compares verified downloads
and controlled seed transfers using cgroup CPU, categorized memory, I/O, and
payload hashes.

The rTorrent comparator now defaults to a source build of the current pinned
upstream release, rTorrent/libtorrent v0.16.25. The tags resolve to the exact
commits recorded in `certification/benchmarks/rtorrent-current/Dockerfile` and
the benchmark report. The older 0.9.8 image remains available only with
`TNG_CLIENT_BENCH_RTORRENT_PROFILE=legacy`; old results remain labeled as
historical and are not relabeled as current upstream results. Update the
source tag, commit, and image label together when adopting a newer upstream
release.

Small fixture runs screen changes and verify that the harness and clients work;
they are not qualification evidence. The benchmark knowledge page records the
matched-trial requirements for making comparative resource claims.

The latest local controlled-seed screen is documented in
[`TorrentNG-current-rtorrent-screen-20261022.md`](../certification/reports/TorrentNG-current-rtorrent-screen-20261022.md),
with raw samples and the memory profile beside it. It used a generated private
torrent, task-local OpenTracker, and one qBittorrent leecher on an internal
Docker network; it contacted no public tracker or swarm. The peer was visible
by 3.0 seconds for both clients. TorrentNG showed payload by 3.0 seconds;
rTorrent v0.16.25 showed payload at 66.4 seconds. Once payload started, the
sample-derived active window was about 3.53 MiB/s and 1.28 CPU-s/GiB for
TorrentNG, versus 3.85 MiB/s and 0.50 CPU-s/GiB for rTorrent. This is one
screening pair, not a ranking: the roughly 63-second rTorrent post-peer wait is
unexplained, and the full-transfer averages were 3.43 versus 1.31 MiB/s. Future
efficiency screens should continue using local controlled swarms and report
peer arrival, first payload, active-window cost, and end-to-end completion
separately. Public swarms remain a separate interoperability check.
The same screen measured anonymous-memory peaks of 14.5 MiB for TorrentNG and
3.4 MiB for rTorrent; rTorrent's cgroup current peak was 134.4 MiB, largely
file cache. These are different memory categories, not a total-RAM win.

The modern rTorrent source points to a useful seed-path experiment. At the
v0.16.25 tags, libtorrent retains a chunk handle when consecutive requests use
the same chunk, writes plaintext piece ranges directly from mapped file chunks
to its socket path, and uses a buffer for encrypted uploads. Its file chunks
can span mapped file ranges. This is not `sendfile`; the source avoids an
application-owned plaintext payload copy while the kernel still manages file
cache. Compare a bounded, verified mapped-range lease against TorrentNG's
generic storage-read path first for plaintext TCP, retaining the current
fallback and testing partial sends, cancellation, ordering, and cold files.
Keep `sendfile` as a separate candidate so each approach has attributable
results. Primary source: [rTorrent v0.16.25 release](https://github.com/rakshasa/rtorrent/releases/tag/v0.16.25),
[libtorrent upload path](https://github.com/rakshasa/libtorrent/blob/v0.16.25/src/protocol/peer_connection_base.cc),
and [file-backed chunk creation](https://github.com/rakshasa/libtorrent/blob/v0.16.25/src/torrent/data/file_list.cc).
