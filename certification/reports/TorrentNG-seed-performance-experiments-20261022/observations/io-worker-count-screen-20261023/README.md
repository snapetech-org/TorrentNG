# Storage I/O worker-count screen — 2026-10-23

**Status: do not change the production default.** Setting `storage.io_worker_threads = 1` improved CPU and memory in repeated single-peer seed screens, but a three-peer confirmation used more CPU in every round while reducing memory. Keep the existing default of `2`; the one-worker setting remains a memory option, and a lazy second worker is the next prototype to test.

## Results

All arms used the same release daemon binary, portable `pread`, 16 KiB requests, a 32-request peer window, and a 4 MiB/s local receiver. The deterministic 64 MiB fixture was SHA-256 verified on every transfer with digest `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. The only TorrentNG configuration difference was `io_worker_threads = 1` versus the explicit default `io_worker_threads = 2`.

| Screen | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| One round | Default, 2 workers | 3.844 | 18.864 MiB | 5.965 MiB |
| One round | Candidate, 1 worker | **3.585 (-6.7%)** | **17.708 MiB (-6.1%)** | **5.227 MiB (-12.4%)** |
| One round | rTorrent 0.16.25 | 2.075 | 7.479 MiB | 3.078 MiB |
| Three-round median | Default, 2 workers | 5.132 | 18.912 MiB | 6.082 MiB |
| Three-round median | Candidate, 1 worker | **3.921 (-23.6%)** | **17.774 MiB (-6.0%)** | **5.219 MiB (-14.2%)** |
| Three-round median | rTorrent 0.16.25 | 2.412 | 7.580 MiB | 3.082 MiB |

In the three rounds, the one-worker candidate used less CPU by 1.7%, 26.6%, and 23.6%; its PSS was lower by 6.0%, 7.5%, and 4.2%, and anonymous peak was lower by 13.2%, 20.0%, and 14.0%. Each 64 MiB transfer took 16.0 seconds at the fixed receiver rate. Randomized orders were:

1. rTorrent, one worker, default two workers.
2. One worker, rTorrent, default two workers.
3. rTorrent, default two workers, one worker.

The median one-worker result remained above rTorrent in this same screen: 1.62× its CPU, 2.34× its PSS, and 1.69× its anonymous memory. This probe is not the official whole-client comparison, whose latest published result remains 1.90 CPU-s/GiB and 19.82 MiB PSS for TorrentNG versus 0.73 and 10.74 MiB for rTorrent.

## Three-peer follow-up

The first three-peer screen was encouraging: with three simultaneous loopback leechers, the one-worker arm used 20.3% less CPU and 8.1% less PSS than the two-worker control.

| One-round screen | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Default, 2 workers | 3.759 | 19.286 MiB | 6.563 MiB |
| Candidate, 1 worker | **2.996 (-20.3%)** | **17.724 MiB (-8.1%)** | **5.121 MiB (-22.0%)** |
| rTorrent 0.16.25 | 1.723 | 7.550 MiB | 3.086 MiB |

A three-round randomized confirmation reversed the CPU result consistently while retaining a memory reduction:

| Build | Median CPU-s/GiB | Median PSS | Median anonymous |
|---|---:|---:|---:|
| Default, 2 workers | 2.838 | 18.736 MiB | 6.168 MiB |
| Candidate, 1 worker | 3.321 (**+17.0%**) | 17.889 MiB (**-4.5%**) | 5.270 MiB (**-14.6%**) |
| rTorrent 0.16.25 | 2.212 | 7.479 MiB | 3.086 MiB |

The one-worker CPU deltas were **+11.9%, +19.0%, and +17.0%** across randomized rounds. PSS fell by 4.5%, 4.5%, and 5.6%; anonymous peak fell by 14.6%, 13.4%, and 18.0%. All three peers verified the same 64 MiB SHA-256 in each run and received data at the same 4 MiB/s rate. The one-worker median remained 1.50× rTorrent CPU and 2.39× its PSS.

This workload changes the decision: a fixed one-worker default would trade a single-peer CPU reduction for a repeatable multi-peer CPU regression. The lower memory remains useful evidence. A bounded pool that starts one worker and starts the configured second worker when I/O backlog grows may retain the idle/single-peer savings without serializing busier reads; that prototype has not been implemented or measured yet.

## Limits and decision

Host load averages in the available snapshots ranged from about 16 to 23 on a 32-CPU machine; other compute-heavy work was active. The probe did not capture per-run host load, so exact CPU deltas are screening evidence, but the multi-peer CPU direction was the same in all three randomized rounds. The worker pool is shared across path-backed storage schedulers; downloads, rechecks, and broader multi-torrent concurrency remain unmeasured.

Keep the existing default at two workers. Preserve the one-worker option for memory-sensitive, low-concurrency cases and test lazy scaling as a separate candidate. No production source or release default changed, and the current best source and published comparison remain intact.

Three harness-only launch attempts failed before a complete comparison: the first labeled the rTorrent executable as a TorrentNG client; the second omitted the repository `scripts/` directory from `PYTHONPATH`; the third pointed rTorrent at the wrong scratch root after the control and one-worker arms had completed. Their causes and small logs are preserved under [`failed-attempts/`](failed-attempts/); none is application failure evidence.

## Reproduction and artifacts

`probe-io-worker-count.py` is the repository seed probe with a test-only TOML insertion: it explicitly sets two workers for the `control` label and one for `one_worker`. `commands.txt` records the single-peer runs; [`multi-peer/commands.txt`](multi-peer/commands.txt) records the concurrent runs and [`multi-peer/runner.py`](multi-peer/runner.py) reproduces them. Raw JSON summaries bind the fixture hash, binary hashes, run order, process CPU, PSS, anonymous peak, and transfer verification. Per-run configs, logs, metrics, and compressed `smaps` are retained. Generated payload copies, API tokens, bootstrap passwords, session databases, and rTorrent resume state are excluded.

TorrentNG binary SHA-256: `e4e63a0446632660200781f9c0f355a47c80b123be5cf6001125782c79dad276`; rTorrent 0.16.25 binary SHA-256: `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`.
