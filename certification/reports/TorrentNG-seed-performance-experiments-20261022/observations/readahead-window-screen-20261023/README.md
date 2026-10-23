# Peer read-ahead window screen — 2026-10-23

**Status: keep the 512 KiB default.** Smaller 128 and 256 KiB windows looked faster in the first randomized sweep, but lost CPU in the two-round confirmation and did not reduce memory.

All arms used the same eight-block release binary and a 64 MiB loopback seed fixture offered at 4 MiB/s. The candidate configurations changed only `storage.peer_read_readahead_bytes`; the exact generated TOML files are preserved with each run. Storage was pinned to `pread`. All TorrentNG and rTorrent transfers verified the same fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

## Initial and window-sweep screens

| Screen / build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Initial: default 512 KiB | 3.736 | 18.49 MiB | 6.01 MiB |
| Initial: 256 KiB | 3.631 (-2.8%) | 18.65 MiB (+0.16 MiB) | 6.00 MiB |
| Sweep: default 512 KiB | 4.081 | 18.57 MiB | 6.00 MiB |
| Sweep: 128 KiB | 3.644 (-10.7%) | 18.73 MiB (+0.16 MiB) | 6.01 MiB |
| Sweep: 256 KiB | 3.543 (-13.2%) | 18.72 MiB (+0.15 MiB) | 6.00 MiB |

The first sweep suggested a CPU opportunity but no memory gain, so both smaller windows went into a randomized two-round confirmation.

## Two-round confirmation medians

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Default 512 KiB | 3.603 | 18.607 MiB | 6.002 MiB |
| 128 KiB | 4.120 (+14.3%) | 18.727 MiB (+0.6%) | 6.021 MiB (+0.3%) |
| 256 KiB | 3.819 (+6.0%) | 19.126 MiB (+2.8%) | 6.506 MiB (+8.4%) |
| rTorrent 0.16.25 | 2.543 | 7.621 MiB | 3.08 MiB |

The confirmation reversed the initial CPU signal. Neither smaller window improved CPU and memory together, so no default or config changed. The raw summaries, exact probe copies, configs, logs, diagnostics, and load snapshots are retained in this folder. Kernel `smaps` captures are gzip-compressed byte-for-byte. The probe copies differ from the checked-in harness only by injecting test-only readahead values into generated TOML. Payload copies, temporary API tokens, session databases, and bootstrap passwords are excluded.
