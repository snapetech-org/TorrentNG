# Twelve-block upload batch screen — 2026-10-22

**Status: not promoted.** The 12-block candidate passed engine tests and a multi-peer wire check, but its first matched 64 MiB screen did not improve CPU or PSS over the retained eight-block source. Host load was high; preserve it as a candidate for a quiet-host repeat rather than treating this single round as a conclusive regression.

## Candidate and validation

The isolated patch raises the maximum contiguous upload-read batch from eight 16 KiB blocks (128 KiB) to twelve (192 KiB) and raises the per-connection read admission limit to twelve. The existing 32 ms collection window and global 64-job / 16 MiB limits are unchanged. The candidate daemon has SHA-256 `e81e8e64dd6c7c841112a7af02d24d4d67b4d77efab18b80450cd2534f407eb1`.

- `cargo build --release -p torrentngd` completed with the Rust 1.94.0 toolchain.
- `cargo test -p rt-engine` passed all 769 tests; `cargo fmt --all -- --check` passed.
- Three simultaneous local peers each completed a 16 MiB transfer through the candidate and matched fixture SHA-256 `132646868d8a9abf88d5fe16244ec101009f6b76613437514dbd41769d3943d3`.

## One-round matched screen

The 64 MiB transfer used the same fixture (SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`), `pread`, a 4 MiB/s paced receiver, 16 KiB requests, and a 32-request window. Order was batch12, rTorrent, then the eight-block control. All three completed and verified.

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Current eight-block control | **3.270** | **19.23 MiB** | **6.50 MiB** |
| Twelve-block candidate | 3.331 (+1.9%) | 19.68 MiB (+2.4%) | 6.77 MiB (+4.3%) |
| rTorrent 0.16.25 | 2.178 | 7.69 MiB | 3.07 MiB |

The candidate did not beat current main in this round. Host load was already elevated before the run (11.81/14.72/16.63) and rose afterward (16.87/14.69/16.32) on the 32-CPU host. Because this was one round under active background load, it is screening evidence only. Keep the eight-block setting on main; retain the 12-block patch and exact binary for later review.

The raw matched summary, three-peer correctness summary, runner, load snapshots, and compressed candidate binary are stored beside this note. The zero-context patch [`../../experiments/upload-batch12.patch`](../../experiments/upload-batch12.patch) reapplies to the reviewed main base with `git apply --unidiff-zero`.
