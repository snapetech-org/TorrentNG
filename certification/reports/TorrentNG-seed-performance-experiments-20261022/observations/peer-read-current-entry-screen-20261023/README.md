# Peer-read current-entry screen — 2026-10-23

**Status: not promoted.** Keeping the active sequential read stream outside `PeerReadStreams`' path map gave a small 16 MiB win, then used 13.0% more CPU at 64 MiB. PSS and anonymous memory were effectively tied. The size-dependent result is not a dependable improvement.

The candidate was built from source revision `4d13f9d0ecdc13f877be73f3d0a266ef1f52abea`. The screen used the `pread` backend, a local receiver limited to 4 MiB/s, and SHA-256-verified fixtures. The 16 MiB screen used three randomized orders and included rTorrent 0.16.25; the 64 MiB screen used three paired control/candidate rounds. Payload hashes and per-round binary identities are in [`summary.json`](summary.json). The candidate patch is preserved compressed as [`peer-read-current-entry.patch.gz`](peer-read-current-entry.patch.gz).

## Median results

| Fixture | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| 16 MiB, 3 rounds | Control | 5.107 | 18.762 MiB | 6.039 MiB |
| 16 MiB, 3 rounds | Current-entry candidate | 4.881 (-4.4%) | 18.724 MiB (-0.2%) | 5.992 MiB (-0.8%) |
| 16 MiB, 3 rounds | rTorrent 0.16.25 | 2.882 | 7.452 MiB | 3.086 MiB |
| 64 MiB, 3 rounds | Control | 3.595 | 18.731 MiB | 6.027 MiB |
| 64 MiB, 3 rounds | Current-entry candidate | 4.061 (+13.0%) | 18.642 MiB (-0.5%) | 6.039 MiB (+0.2%) |

The 64 MiB screen had no rTorrent arm, so it is a control/candidate comparison only. Its CPU regression outweighs the 0.5% PSS reduction, while anonymous memory increased slightly. Keep the existing implementation; the earlier small-fixture improvement is retained here for future comparison, not promoted as a general gain.

All three transfers per arm completed and verified. The candidate passed the **354 `rt-storage` library tests**. No production change was made from these screens.
