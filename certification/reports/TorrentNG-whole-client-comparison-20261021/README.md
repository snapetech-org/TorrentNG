# Whole-client torrent client comparison — 2026-10-21

> **Version caveat:** the rTorrent measurements below used rTorrent **0.9.8** from the pinned `jesec/rtorrent` image. They are historical results, not a comparison against current upstream rTorrent. As of 2026-10-22, the latest tagged stable rTorrent release and its matching libtorrent source tag are both **v0.16.25**. Future runs of the benchmark harness now default to those exact source revisions; use `TNG_CLIENT_BENCH_RTORRENT_PROFILE=legacy` only to reproduce the old image.

The first current-upstream controlled-seed screen is recorded in
[`TorrentNG-current-rtorrent-screen-20261022.md`](../TorrentNG-current-rtorrent-screen-20261022.md).
It used a local generated private torrent and internal one-seeder/one-leecher
swarm, not public swarm traffic. Both clients had a peer by the 3.020-second
sample, while rTorrent did not show payload until 66.424 seconds. Its sample-
derived active window used about 0.50 CPU-s/GiB versus TorrentNG's 1.28, but
TorrentNG started transferring much earlier. This is a single screening pair;
the post-peer wait is unexplained and no general ranking is established. The
new harness reports first-peer, first-payload, active-window, and full-transfer
metrics separately so future results preserve both sides of that tradeoff.

## Seed workload results

Three complete controlled-seed rounds transferred the same 792,723,456-byte Debian 13.7 netinst ISO through a private one-seeder/one-leecher torrent. The fixture SHA-256 was `a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355`; each receiver's final payload matched it. Each client ran alone. A fixed qBittorrent 5.2.3 leecher received at 4 MiB/s, and client order rotated between rounds.

| Client | Median CPU-s/GiB ↓ (range) | Median cgroup anonymous peak MiB ↓ (range) |
|---|---:|---:|
| TorrentNG | **1.33** (1.32–1.43) | 16.9 (16.3–18.1) |
| qBittorrent | 2.39 (2.30–2.46) | **6.2** (6.1–6.2) |
| Transmission | **1.19** (1.16–1.81) | 16.4 (14.1–17.1) |
| Deluge | 2.68 (2.15–2.97) | 82.7 (80.7–82.7) |
| rTorrent | **0.70** (0.52–0.84) | **1.0** (1.0–1.0) |

On this seed workload, TorrentNG used about 44% less CPU per GiB than qBittorrent and 50% less than Deluge. It used about 12% more CPU than Transmission and 90% more than rTorrent. Its anonymous memory peak was about 80% below Deluge, close to Transmission, 2.7 times qBittorrent, and 16.9 times rTorrent.

These are whole-client measurements, but they cover one host and one controlled seed workload. There are three rounds, fewer than the benchmark's ten-trial qualification target. The host was shared and load varied substantially. The results do not establish download efficiency, multi-torrent capacity, idle use, or a general ranking. Memory categories also matter: in round 1, qBittorrent peaked at about 769 MiB cgroup `memory.current`, including 756 MiB file cache and 6.1 MiB anonymous memory; rTorrent peaked at about 761 MiB current, including 756 MiB file cache and 1.0 MiB anonymous memory. Docker working-set counters differed by client, so the table uses cgroup anonymous peaks for the memory comparison.

Rounds 1 and 2 used clean source commit `24e521aed22a9cfb4b905c75a208e06077ad6332` (source fingerprint `da08674a960f9f43044ffc64483c95d272434844434b2db795fc25141772d6ca`). Round 3 used `c3fee17d7ffec859edab3336e07cf5c1533b6614` (fingerprint `cd263342ecfb77c4b319f787c21b7a8bd715522a19d0223a30f36b2c4a9005aa`). The intervening source changes contain OIDC branding and related tests/docs; no native engine or benchmark runtime implementation changed. Exact per-round source, image, host, order, and workload details are in each report.

## Controlled-download failure

The controlled-download comparison did not produce a valid five-client result set. An initial full panel was interrupted after TorrentNG stalled; later TorrentNG-only reproductions timed out. In the third diagnostic run, TorrentNG reached 2.76% (about 21.9 MB of the fixture), then reported zero active peers and made no further progress before the 60-second timeout. qBittorrent's seeder API recorded 22,560,768 bytes uploaded to the TorrentNG peer, and the TorrentNG daemon logged an incoming `Choke` from that peer. No completed SHA-256 means this run contributes no CPU/GiB score. The logs establish the stalled transfer, but they do not yet establish why the peer stayed choked or why the connection stopped progressing.

The private torrent used by these controlled-download runs has infohash `bde512393bf3804515e883cc3188c408783d7b72`, confirmed in the TorrentNG daemon log and qBittorrent snapshots. The raw generated reports mistakenly printed the official Debian torrent's hash (`7acf8fb590b2060dd9c3146ef770169d593433b0`). The benchmark report writer now records the generated private hash and has a regression test. The original reports remain unchanged as raw evidence.

The fixed download seeder was qBittorrent 5.2.3. TorrentNG used the exact current main source at `f886f4699997119366fe80d7161a1820bf10879a`, fingerprint `306507753fd2d5847f6d282b479a660ca59dc9b8946405b0eea14227ae194fef`. The current storage observation was portable `pread`, maximum batch size 1, with no registered files or fixed buffers. That is the measured configuration; the isolated batching results from the supplied patch do not substitute for a complete client comparison.

## Evidence files

Each seed round includes its original Markdown report, time-series CSV, cgroup/memory profile, and runner log:

- [Seed round 1](seed-main-round1.md), [samples](seed-main-round1-samples.csv), [memory profile](seed-main-round1-memory-profile.json), [log](seed-main-round1.log)
- [Seed round 2](seed-main-round2.md), [samples](seed-main-round2-samples.csv), [memory profile](seed-main-round2-memory-profile.json), [log](seed-main-round2.log)
- [Seed round 3](seed-main-round3-latest.md), [samples](seed-main-round3-latest-samples.csv), [memory profile](seed-main-round3-latest-memory-profile.json), [log](seed-main-round3-latest.log)

The `download-main-*` files preserve the incomplete full panel and three targeted reproductions, including time series, memory profiles, daemon logs, and sanitized seeder API snapshots where captured. The [setup and preflight diagnostics](diagnostics/README.md) preserve earlier harness and image attempts. None of these files are scored rows.

The [artifact manifest](source-artifacts-manifest.txt) records the original task output paths. [SHA256SUMS](SHA256SUMS) covers this report, all copied trial evidence, and the retained test log.

## Validation

`python3 scripts/test_client_performance_benchmark.py` passed all 50 tests, including the new check that controlled-download reports use the private torrent hash actually generated for the local fixture. The [test output](validation/python-benchmark-tests.log) is retained. No engine source changed in this report commit.
