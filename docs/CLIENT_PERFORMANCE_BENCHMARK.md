# Client performance benchmark

This guide runs the repository's Docker-based comparison harness and explains
how to read its results. The current three-round comparison and the full
historical evidence are linked from the
[performance research index](research/README.md).

## Requirements

- Linux with cgroup v2, Docker Engine, and Docker Compose v2.
- Python dependencies from
  [scripts/transfer_rebuild/requirements.txt](../scripts/transfer_rebuild/requirements.txt).
- A local fixture with a known SHA-256 for controlled local tests.
- Enough free disk for temporary client containers, the fixture, and build
  layers. The harness uses a bounded TorrentNG BuildKit cache.

Install the pinned Python dependencies:

    python3 -m pip install -r scripts/transfer_rebuild/requirements.txt

## Choose a mode

- Default mode runs a public Debian download and a separate controlled seed
  transfer for TorrentNG, qBittorrent, Transmission, Deluge, and rTorrent.
  It makes public network requests.
- controlled-seed mode skips public metadata and swarm requests. It uses a
  pinned local fixture, an isolated task-local tracker, and one qBittorrent
  leecher.
- controlled-download mode measures each receiver alone against one fixed
  local qBittorrent seeder.
- idle mode measures no-torrent process behavior under network-isolated,
  attached/DHT-off, and attached/DHT-on profiles.

The latest small five-client screen used controlled-seed mode. It transferred
the same 64 MiB local fixture three times per client; all 15 copies passed
SHA-256. See the
[current comparison](research/current-client-comparison.md) for the result and
[exact configuration](../certification/reports/TorrentNG-current-client-comparison-20261024/run-config.md)
for seeds, versions, image pins, and reproduction details.

## Run a controlled local-seed comparison

Use a fixture that exists locally and set its correct 64-character SHA-256.
The default client list includes all five supported benchmark clients.

    TNG_CLIENT_BENCH_MODE=controlled-seed \
    TNG_CLIENT_BENCH_SEED_SOURCE=/path/to/fixture.bin \
    TNG_CLIENT_BENCH_SEED_SHA256=replace-with-fixture-sha256 \
    TNG_CLIENT_BENCH_ORDER_SEED=20261027 \
    TNG_CLIENT_BENCH_REPORT=/path/to/results.md \
    python3 scripts/client_performance_benchmark.py

The harness generates a private torrent, disables DHT and peer exchange, and
keeps the clients on an internal Docker network. The receiver cap defaults to
4 MiB/s and the seeder cap defaults to 32 MiB/s. The final fixture hash must
match for a transfer to count as successful. Set
TNG_CLIENT_BENCH_CLIENTS=torrentngd to screen only TorrentNG.

For repeatable comparison, use distinct recorded order seeds, build TorrentNG
once, reuse that exact image, and randomize client order. Run multiple rounds
on the same machine and storage. Avoid comparing results gathered under
different memory categories, rate caps, fixtures, or source revisions.

## Measures and limitations

- CPU is cgroup v2 CPU time per verified GiB. Whole-window CPU includes
  startup-to-transfer time; active-window CPU starts at first sampled payload.
  They answer different questions and should be shown together.
- Cgroup anonymous memory excludes file cache and is available across the
  client containers. File cache, total cgroup memory, process-tree RSS, and
  PSS remain separate measurements.
- Process-tree RSS/PSS are sampled periodically and can miss short peaks.
  They may also be unavailable for some containers.
- Every reported comparison should preserve fixture and leecher SHA-256
  results, source commit/fingerprint, client versions/image digests, exact
  settings, run order, per-round samples, and host-pressure data.
- A small screen shows direction on its recorded host and workload. It does
  not establish general rankings, public-swarm behavior, concurrent capacity,
  or parity with another client.

The harness accepts additional environment variables for build selection,
allocators, worker counts, rate limits, storage, memory budgets, diagnostics,
and sample intervals. Run
python3 scripts/client_performance_benchmark.py --help for the full list.

## Results and research

- [Latest five-client results](research/current-client-comparison.md)
- [Performance research index: best results, configs, trade-offs, and reviews](research/README.md)
- [Full per-round evidence and raw data](../certification/reports/TorrentNG-current-client-comparison-20261024/README.md)
- [Historical benchmark detail archive](research/client-performance-benchmark-archive.md)
- [Performance history and reverted experiments](research/performance-history.md)
