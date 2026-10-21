# TorrentNG native upload fix

The patch fixes a reproduced CPU failure in the native Rust upload path. Existing ready-only batching works when requests arrive together. When a peer keeps 32 requests outstanding but replenishes them one at a time, it collapses to almost one storage operation per 16 KiB block—even though most reads hit the cache. Reusing payload buffers does not remove that per-request scheduling, admission, queue, and completion work.

The combined fix reduced median native-daemon CPU by **39.6%** in three matched local A/B trials of that request pattern. It does **not** establish a RAM win or a ranking against other clients. The main remaining measured tradeoff is approximately **5.8% more logical backend read bytes** in this pattern.

## What to apply

Apply only `torrentng-native-fix.patch` at the ZIP root. It targets current main **7c2dd481ca224da8f5e47b783678b6110146ba9b**. The four complete modified files are also under `source/`. Changes are local; nothing was pushed or merged upstream.

```sh
git apply --check /path/to/torrentng-native-fix.patch
git apply /path/to/torrentng-native-fix.patch
```

The files under `evidence/experiments/` and `evidence/measured-d57218d.patch` are historical evidence, not additional patches to apply.

## The native changes

1. `crates/rt-engine/src/torrent_task.rs`: the peer loop can collect staggered upload requests for up to 8 ms, capped at the existing four-block batch. It learns the peer's demonstrated pipeline width. Serial requesters dispatch immediately; a peer shrinking its window recovers immediate service after one solitary deadline flush. Choke controls and v2 uploads bypass collection. Existing admission, cancellation, rate limits, generation checks, payload accounting, and bounded batch size remain in use.

2. `crates/rt-storage/src/scheduler.rs`: consuming a requested range now updates every overlapping read-ahead window, including when a backend read crosses the end of an existing window. A single 64-bit word remembers fully consumed granules arriving before the prefix or suffix, without allocating per hit. Unread gaps remain cached. Returned Bytes views retain their existing backing-memory leases after cache references retire. Partial out-of-order granules remain conservatively cached; this is not a claim that every possible access pattern releases every consumed byte immediately.

The cache change is necessary alongside coalescing: the coalescing-only experiment improved CPU but retained 7.5 MiB of cache payload and reached 16.43 MiB anonymous RAM. The combined candidate's final cache was empty in two of three staggered trials and retained 0.5 MiB in the other. Its anonymous RAM stayed between 7.98 and 8.91 MiB.

## Completed native-daemon evidence

These are actual `torrentngd` processes serving the complete file to a deterministic BEP 3 receiver. They are not storage-only microbenchmarks. No sidecar was used, and no browser or WebUI was active. All listed transfers passed the full payload SHA-256 check.

Primary workload: 128 MiB, 16 KiB wire blocks, 1 MiB pieces, one private v1 torrent, one TCP peer, 32 outstanding requests, one new request per received Piece, receiver capped at 4 MiB/s. Current/fixed order was rotated over three rounds. Each run used a fresh daemon. Trials ran sequentially with compilation and unit tests stopped.

| Median, three trials per variant | Current | Combined fix |
| --- | ---: | ---: |
| Native process CPU-s/GiB | 15.8725 | 9.5933 |
| Sampled peak process anonymous RAM, MiB | 8.0273 | 8.4766 |
| Transfer wall time, s | 32.00011 | 32.00013 |
| Storage read operations, including cache hits | 8,158 | 2,814 |
| Backend read operations | 264 | 288 |
| Backend read MiB/GiB | 1,025.00 | 1,084.29 |
| Cache payload bytes after transfer | 393,216 | 0 |

CPU ranges were 15.0124–15.9460 current and 8.9895–9.9359 fixed. Anonymous RAM ranges were 7.9844–10.7383 current and 7.9805–8.9141 fixed. RAM varies with the pre-transfer process baseline; its overlapping ranges and higher fixed median do not support a RAM-improvement claim. The cache-retention defect is separately supported by its targeted tests and the coalescing-only experiment.

The already-batched control replenished four requests together. Two rotated rounds gave median CPU 8.0023 current / 6.8000 fixed, with the same 32-second completion and 2,048 storage reads in both variants. This is a regression guard with limited repetitions, not a separate broad performance claim. Anonymous RAM medians were 7.7852 / 8.4355 MiB; logical backend reads were 1,024.75 / 1,026.75 MiB/GiB.

Small-window guards used a 16 MiB file with no receiver rate cap, under the same configured native 32 MiB/s limit and its initial token burst. A one-request window finished in 0.40145 s current / 0.40120 s fixed; a two-request window finished in 0.40770 / 0.42005 s. All hashes passed. The timer does not impose an 8 ms delay on every serial request. These short guards are not sustained maximum-throughput measurements.

Earlier diagnostic comparisons are retained too. The previous 1564fa32 changes already reduced CPU substantially when requests arrived four at a time, but showed no CPU win over 16da409 when refills arrived singly. A separate rate-waiter mutex experiment had overlapping CPU results and was rejected; it is absent from the delivered source.

## Source binding and limits

The timed binaries were built from clean **d57218d** and that revision plus the native candidate. Main advanced to **7c2dd48** during the work. Its only change under `crates/`, `Cargo.toml`, and `Cargo.lock` is `#[cfg(test)]` on a previously unused helper. The delivered storage file is byte-identical to the measured candidate; the engine file is byte-identical after removing that upstream annotation. This was mechanically checked. The latest-base unit suites were rerun. `provenance.json`, `evidence/measured-provenance.json`, and `evidence/upstream-native-change.patch` preserve the distinction; the CPU rows have not been relabeled as new exact-7c2dd48 binary measurements.

All local binaries used Rust 1.94.0, the GNU target/system allocator, release optimization with LTO disabled and 16 codegen units, two Tokio workers, and the pread storage backend. This differs from the production Rust 1.96/musl/thin-LTO Docker build. Absolute CPU and RAM values here must not be compared with the older five-client report's numbers or used to predict a percentage reduction there.

CPU is the kernel process CPU clock summed over daemon threads. The timed window starts before the first upload request and ends after receiving the payload, before receiver hashing. Native seed recheck, compilation, startup, and post-transfer metric collection are outside it. Memory is process RssAnon sampled every 250 ms, not cgroup anonymous memory. The harness reconciles its PID namespace with the outer /proc mount before memory sampling. Storage counters are collected after the asynchronous metrics snapshot refresh completes.

The native seed recheck warms file data. Backend byte counters describe logical reads, not physical disk traffic. Cold storage, multipeer/multitorrent load, download efficiency, v2 performance, and competitor rankings remain unmeasured here. Docker and competitor binaries were unavailable in this environment. The exact cause of the historical qBittorrent-receiver score is therefore still unconfirmed; this work proves one concrete request-cadence failure mode and its local fix.

Confidence is high for transferred-byte integrity and operation counts, moderate for the CPU improvement in this exact repeated workload, low for a RAM difference, and unknown for whole-client superiority against competitors.

## Validation

- Latest-base native suites: **762 engine + 344 storage passed**, zero failures. Storage includes four new tests: all 24 permutations of four wire-block completion orders; overlapping windows; an unread partial-granule gap; and a real backend read straddling a cached tail.
- Latest-base benchmark suite: **45 passed**. The patch also fixes main's invalid `from unittest.mock import call, mock` import, which prevented test collection here.
- The final A/B and guard set contains **14 complete hash-verified transfers**: six staggered, four burst, and four small-window trials.
- `git diff --check` passed. The patch passed `git apply --cached --check` against a temporary index populated from 7c2dd48.
- Two release archive construction attempts failed with a zero-length object mapping error; sequential retries completed. Both failed and successful build logs are preserved. No build work ran inside timed windows.

## Faster proof cycle

Main's new **7c2dd48** implementation already supplies pinned local `controlled-seed` mode, bounded persistent BuildKit caches, and optional image retention/reuse. Those changes are retained. The delivered benchmark change adds a report checkpoint after each client and labels clients not yet run as `NOT RUN`; a later interruption no longer hides earlier completed measurements. New tests cover a later-client interruption and success/failure handling. Docker integration for these reporting additions was not run here.

On the benchmark host, first screen the native daemon against a pinned local fixture; use the same fixture, receiver, storage policy, and production build flags for before/after comparisons. Then run the five-client comparison on a candidate that survived the screen. Keep the public-swarm interoperability run separate from the resource comparison.

```sh
TNG_CLIENT_BENCH_MODE=controlled-seed \
TNG_CLIENT_BENCH_CLIENTS=torrentngd \
TNG_CLIENT_BENCH_SEED_SOURCE=/absolute/path/to/fixture.bin \
TNG_CLIENT_BENCH_SEED_SHA256=<previously-verified-sha256> \
TNG_CLIENT_BENCH_KEEP_IMAGE=1 \
TNG_CLIENT_BENCH_REPORT=/absolute/path/outside-checkout/native-screen.md \
python3 scripts/client_performance_benchmark.py
```

Replace the angle-bracket digest placeholder before running. Leave `TNG_CLIENT_BENCH_CLIENTS` unset for the full five-client pass. Repeat matched windows in balanced order before making a ranking claim. Keep reports outside the checkout while reusing an image: the existing source fingerprint includes repository files, so writing reports there can change the fingerprint. Reuse only an image whose commit, source fingerprint, and allocator labels pass the existing checks.

Acceptance on the production host should require the CPU benefit to survive the real qBittorrent receiver, no material sustained-throughput or memory regression, and acceptable cold-cache disk reads. If actual peers already replenish in full bursts, the 39.6% staggered result should not be expected. The current patch has an explicit CPU/read-amplification tradeoff rather than proof of being better in every resource dimension.

## Reproduce the local native A/B

Use separate target directories for clean baseline and candidate checkouts, retain each binary, and stop compilation before measurement. The local build command was:

```sh
CARGO_TARGET_DIR=/absolute/task-target \
CARGO_PROFILE_RELEASE_LTO=false \
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
cargo build --release --locked -p torrentngd -j 1
```

The included probe creates deterministic fixtures, checks their SHA-256, seeds through the native API, and receives the entire payload through the peer protocol. It requires Linux /proc and process CPU clocks. It binds only loopback for the test peer/API and explicitly enables loopback peer egress in its task-local configuration.

```sh
python3 reproduce/compare_native.py \
  --case current=/absolute/baseline/torrentngd \
  --case fixed=/absolute/candidate/torrentngd \
  --output /absolute/new-results-directory \
  --fixtures /absolute/fixture-cache \
  --rounds 3 --mib 128 --refill 1
```

Use `--refill 4` for the burst control. For the small-window guard, call `reproduce/native_seed_probe.py` directly with `--mib 16 --rate-mib 0 --window 1 --refill 1`, then window 2. Results must pass hashing before `result.json` is written. Do not use Python optimization flags that disable assertions.

`results.json` contains every per-trial row and per-workload medians. `evidence/` contains the raw results, memory samples, final metrics, configuration, and logs. Binaries, generated payloads, Cargo targets, and session databases are intentionally excluded from this small handoff ZIP.
