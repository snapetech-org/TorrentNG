# Native seed performance follow-up — 2026-10-22

**Status:** experimental screening only. The current release defaults remain in place. PGO remains the strongest measured candidate. The io_uring startup fix makes the requested backend selectable on constrained hosts, but matched screening found a CPU regression and only a small memory reduction, so it is not promoted as a performance win.

The objective remains to meet or beat current rTorrent in both process CPU and memory. These local tests do not reach that objective. The last published whole-client comparison remains the reference: TorrentNG at 1.90 CPU seconds and 19.82 MiB PSS versus rTorrent at 0.73 seconds and 10.74 MiB PSS.

## Method

- Source checkout: `7ba8bb6a5b3505d4d263e35c649e24a9f0761d44`; no production Rust source changes were made during these screens.
- Host: `kspld0`, AMD Ryzen 9 9950X3D.
- Rust toolchain: `rustc 1.97.0 (2d8144b78 2026-07-07)`, LLVM 22.1.6.
- Workload: private loopback seed, one 16 KiB block size, a 32-block request window, 4 MiB/s paced receiver, tracker/DHT/PEX disabled, and the portable `pread` storage backend for TorrentNG.
- Payloads were deterministic (probe seed `20261022`) and SHA-256 verified. The 64, 128, and 256 MiB fixtures have hashes `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`, `7fae01c69f5254b9fbf3cdf6db292cc5da49ee3fa4bfbb995de8aa3b66dd6e67`, and `937d7bd20b42ad9d0130419a685ce1e481f4330ab2a211dde1e16403c7854f81` respectively. Raw summaries and logs are kept under [`observations/`](observations/); oversized generated payload copies are omitted and can be regenerated from the seed and size documented there.
- CPU is process CPU time from first payload to payload completion, normalized per GiB. PSS and anonymous memory are active-window peaks. Transfer duration is a validity check, not a performance score.

These are controlled local screens, not the repository's official whole-client qualification. Their source-built binaries and load shape differ from the release comparison. The 128 MiB and 64 MiB three-client screens include the same local benchmark rTorrent binary as a comparator.

## Strongest candidate: full-profile PGO

The same full-profile PGO binary improved on the unprofiled build in repeated 128 MiB and 256 MiB screens. The 128 MiB result is consistently positive across all three paired rounds; at 256 MiB the CPU gain is small and round-to-round results vary. Memory improved in both sizes.

| Fixture | Build | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---|---:|---:|---:|
| 128 MiB, 3 rounds | Release baseline | 5.04 | 19.43 MiB | 6.53 MiB |
| 128 MiB, 3 rounds | Full-profile PGO | **3.68** (-27.0%) | **16.10 MiB** (-17.1%) | **6.28 MiB** (-3.8%) |
| 256 MiB, 3 rounds | Release baseline | 4.88 | 19.44 MiB | 6.54 MiB |
| 256 MiB, 3 rounds | Full-profile PGO | **4.60** (-5.7%) | **16.22 MiB** (-16.6%) | **6.29 MiB** (-3.8%) |

The follow-up three-client 128 MiB comparison reused the exact fixture and binaries, randomized order across three rounds, and verified every transfer. Medians were:

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Release baseline | 5.70 | 19.38 MiB | 6.55 MiB |
| Full-profile PGO | **3.68** (-35.4%) | **15.80 MiB** (-18.5%) | **6.08 MiB** (-7.2%) |
| rTorrent 0.16.25 | 2.15 | 7.90 MiB | 3.04 MiB |

PGO still used 1.71× rTorrent CPU and 2.00× its PSS. This strengthens the case for preserving and qualifying the PGO build while making clear it does not meet the target. The three-round CPU values were 3.680, 3.415, and 3.702 CPU-s/GiB for PGO; 4.301, 6.729, and 5.700 for baseline; and 2.106, 2.258, and 2.152 for rTorrent. All nine transfers passed SHA-256 against fixture `7fae01c69f5254b9fbf3cdf6db292cc5da49ee3fa4bfbb995de8aa3b66dd6e67`.

The PGO candidate binary used for the 128/256 MiB tables has SHA-256 `4b95eafd08d685e89272b7bc6bb24e37b906bbab6bfaa208a14f5026df801547`; the baseline binary has SHA-256 `368bb71e0d7bfae9322aeb3a2dfac7648d57e515984c852d433713b78bae8754`. The merged profile SHA-256 is `45c1f0352f9fe9f1622b783ced1a48a93269cc2eb79ce77549468e35401fc4ed`; the compressed profile and its training summary are preserved in [`observations/pgo-full-training/`](observations/pgo-full-training/). The training run used a 64 MiB verified seed and an instrumented daemon with SHA-256 `18d9aa4e4221df8b03e76b4ece08943514cbb04b98ab7d06c81879e705a530e9`.

The opt-in [`build_native_pgo_release.sh`](../../../scripts/build_native_pgo_release.sh) accepts the archived compressed profile, pins Rust 1.97.0, applies the workspace release settings, and prints the profile and output-binary hashes. Three clean builds from the same source and profile produced different binary hashes (`4b95eafd08d685e89272b7bc6bb24e37b906bbab6bfaa208a14f5026df801547`, `9697ac512212d97cc3d3c23f8c5f38cfbdfae7cf30d390c77bb04e946c64c6cf`, and `80d20641f4611979bfdd9cd12254bab524436267bdf9333938f9a786b453041e`); the exact cause of this codegen variation is unresolved, so those hashes identify individual artifacts rather than a byte-for-byte reproducibility promise. The full-profile behavior was screened on the `80d206…` script-built binary in a separate 64 MiB, one-round comparison. It used 3.21 CPU-s/GiB, 15.72 MiB PSS, and 5.97 MiB anonymous memory, versus the release baseline at 4.43, 18.85 MiB, and 6.01 MiB; rTorrent measured 2.40, 7.88 MiB, and 3.03 MiB. All three transfers passed SHA-256 verification. This confirms the direction in a quick screen but does not qualify that exact artifact; keep the build opt-in and re-screen each built binary before broader use. The profile was trained on one 64 MiB seed fixture, so PGO still needs download, multi-peer, and official release qualification before becoming a default.

An earlier three-client, three-round 64 MiB screen measured PGO at 4.007 CPU-s/GiB and 15.63 MiB PSS, against baseline at 6.529 and 18.80 MiB, and rTorrent at 2.310 and 8.33 MiB. PGO remained about 1.74× rTorrent CPU and 1.88× its PSS. A separate one-round 256 MiB comparison likewise left PGO above rTorrent: 3.565 versus 2.112 CPU-s/GiB and 16.25 versus 8.29 MiB PSS. Those observations show PGO is useful, but does not meet the goal.

The transfer-only PGO profile did not beat the full profile in its one-round 64 MiB screen: 3.832 versus 3.527 CPU-s/GiB and 18.22 versus 13.21 MiB PSS. PGO is not enabled by the product release profile. The archived profile now supports rebuilding and screening the candidate, but clean binary hashes vary and the profile lacks download/multi-peer training coverage. Keep the build opt-in until a clean-build artifact passes repeated qualification.

## Source-level candidates and reversions

Every rejected or inconsistent source candidate is kept as a patch under [`experiments/`](experiments/) so it can be revisited without losing its result.

| Candidate | Result | Decision |
|---|---|---|
| Increase upload batch target to 8 | One 64 MiB round: CPU rose 14.9%, PSS 7.1%, and anonymous peak 8.4% versus baseline. | Reverted; keep [`upload-batch8.patch`](experiments/upload-batch8.patch). |
| Cache-hit single-lookup refactor | One round looked faster, but the three-round median was 5.801 CPU-s/GiB versus 3.780 for the PGO control; PSS was essentially tied. | Reverted as inconsistent; keep [`cache-hit-single-hash.patch`](experiments/cache-hit-single-hash.patch). |
| Active stream kept outside the cache map | At 64 MiB under `opt-level=s`, the three-round median improved CPU 16.3% and PSS was tied. At 256 MiB, the single candidate round used 9.8% more CPU with effectively tied PSS. In a refreshed PGO comparison, it was 6.7% slower and used 0.46 MiB more PSS than the PGO control. | Reverted as size/profile dependent; keep [`stream-cache-active.patch`](experiments/stream-cache-active.patch). |
| Wait for a fourth request before dispatch | The runtime setting disabled the read elevator, so the probe did not exercise batching and is not a scored result. | Do not infer a gain; preserve [`fourth-request-probe.patch`](experiments/fourth-request-probe.patch) for a corrected test. |
| io_uring startup and queue-sized ring | The earlier requested arm actually selected `pread`. Direct worker startup now selects io_uring at queue depth 64. A three-round verified comparison found higher CPU with slightly lower PSS and anonymous memory. | Keep the startup correction and explicit backend available; do not promote io_uring as a performance winner or change the default queue depth. Results and invalid attempts are retained under [`observations/io-uring-queue-depth-64/`](observations/io-uring-queue-depth-64/). |

### io_uring startup attribution and measured tradeoff

The old `TNG_STORAGE_BACKEND=uring` screen was not an io_uring result: its runtime gauge showed `pread`. A focused reproduction found that selection allocated and registered a probe ring, then the real worker failed with `ENOMEM`; starting the worker directly at queue depth 64 succeeded. The selector now starts the actual worker and falls back to `pread` only if that startup fails. The benchmark harness accepts a bounded `--storage-io-queue-depth` setting and records the active backend label, preventing another silent fallback from being scored as io_uring. Queue depth and release defaults were not changed.

Three randomized 64 MiB rounds compared the same release binary with `pread` and io_uring, using queue depth 64 for both. All six transfers matched fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`, and the diagnostic gauge confirmed the requested backend in every round.

| Backend | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---:|---:|---:|
| `pread` | **4.46** | 19.11 MiB | 6.41 MiB |
| io_uring, queue depth 64 | 5.02 (+12.5%) | **18.62 MiB** (-2.5%) | **6.09 MiB** (-5.0%) |

The CPU cost is higher, while the memory reductions are small and these screens do not establish a general performance win. The one-round cross-client screen and follow-up PGO screen on the same 64 MiB fixture were:

| Build | Backend | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| Release | `pread` | 5.91 | 19.01 MiB | 6.51 MiB |
| Release | io_uring, queue depth 64 | 8.42 | 18.58 MiB | 6.01 MiB |
| rTorrent 0.16.25 | — | 3.46 | 7.90 MiB | 3.04 MiB |
| Updated-source PGO | `pread` | 4.03 | 16.18 MiB | 6.49 MiB |
| Updated-source PGO | io_uring, queue depth 64 | 6.15 | 15.97 MiB | 5.87 MiB |
| rTorrent 0.16.25 | — | 2.28 | 7.90 MiB | 3.04 MiB |

The updated-source PGO build emitted profile-mismatch warnings for the changed backend functions, so those two rows are screening evidence only. Neither screen displaces the retained full-profile PGO result of 3.68 CPU-s/GiB / 15.80 MiB PSS at 128 MiB.

The first three-client attempt is preserved but invalid: the harness correctly removed inherited loader variables, and rTorrent then exited 127 because `libtorrent.so.53` was not found. The valid retry passed the pinned `LD_LIBRARY_PATH` explicitly. The exact output binary hashes, selected-backend snapshots, process maps, logs, and original plus relocated summaries are in the evidence directory above. The code and harness correction were committed on `main` as `85dc7800`.

The experiments also keep evidence that did not fit a clean score: an initial rTorrent launch failed because its loader could not find `libtorrent.so.53`; the later valid runs use a pinned `LD_LIBRARY_PATH`. Failed and diagnostic summaries remain in the observations directory and are excluded from performance claims.

## Upstream rTorrent path and next source experiment

The comparator is built from the official [rTorrent v0.16.25 release](https://github.com/rakshasa/rtorrent/releases/tag/v0.16.25). Its paired [libTorrent v0.16.25 README](https://github.com/rakshasa/libtorrent/blob/v0.16.25/README.md) describes transferring directly from file pages to the network stack. That supports testing file-page-backed payload ownership. It does not establish that raw `sendfile` is the right implementation: our sendfile measurement traded a large anonymous-memory reduction for a 4.14× CPU increase.

TorrentNG currently resolves a verified v1 upload to a file span, then calls `read_owned_at_with_priority`; storage schedules a positioned read into an owned frame, and the peer writer sends the resulting `Bytes`. `rt-storage/src/verify.rs` explicitly avoids `mmap` for verification. The source-level follow-up is therefore a bounded, read-only mapped-range lease for eligible verified seed reads, keeping the current vectored frame header and buffered fallback. Its first screen must check CPU, PSS, anonymous memory, scheduling fairness, cancellation, path-replacement fencing, and the behavior when file pages are cold. A map-per-block implementation is not a candidate: mapping lifetime and reuse must be bounded and measured first.

## Reproducibility and evidence

[`observations/`](observations/) contains the complete probe JSON and captured client logs for the scored and invalid screens. [`manifest.json`](observations/manifest.json) records each file's byte length and SHA-256; it also records the uncompressed PGO profile hash. The summaries bind measurements to binary checksums, fixture hashes, round order, transfer completion, and payload verification.

The benchmark probe's regression tests cover its process sampling, profile-window setup, configuration, and cleanup behavior. They must pass before using the probe for another qualification run. Re-run a promising candidate against the same baseline and current rTorrent binary in randomized, paired rounds at both 128 and 256 MiB, then qualify it using the official release comparison before changing defaults.

The separate supplied source-review archive is assessed in [`TorrentNG-rtorrent-hot-path-review-20261022/README.md`](../TorrentNG-rtorrent-hot-path-review-20261022/README.md). Its source files match the reviewed `7ba8bb6a` checkout, so it adds no code change.
