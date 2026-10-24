# Excluded benchmark attempts

These attempts are retained for audit and debugging and are excluded from all comparison summaries.

- `excluded-stale-rtorrent-pin.*`: a shared-worktree checkout changed to `main` before rTorrent started. The report expected 0.16.26, but the actual container image was 0.16.25. All results from this pass are excluded.
- `excluded-fixed-order-01.*`: all five transfers passed SHA-256, but controlled-seed mode ran the fixed client tuple instead of the recorded randomized order. The measurements are not used for ranking.
- `excluded-fixed-order-02.*`: same fixed-order harness defect; TorrentNG also timed out without a connected leecher. The other four transfers passed SHA-256. The entire pass is excluded.

The harness now iterates the recorded randomized order in controlled-seed mode, with a regression test that checks actual execution order.

- `excluded-buildkit-startup.*`: no client transfer started. The Docker-container BuildKit builder was absent and its first bootstrap stalled while pulling the BuildKit image; partial report, CSV header, and memory profile are retained.

- `excluded-no-peer-01.*`: partial randomized run on `ea8484dd` in order Deluge, qBittorrent, TorrentNG, rTorrent, Transmission. Deluge and qBittorrent completed SHA-256, but TorrentNG received one tracker peer record without forming a peer connection. The readiness check was the cause: it waited for the registered seeder (`complete >= 1`) but not for the leecher (`incomplete >= 1`) before forcing the seeder to reannounce. The later inbound peer then arrived before TorrentNG had an authorized tracker peer. The harness now waits for both counts; all five clients passed in each of the three following randomized rounds. This failed attempt is excluded. Its report, CSV, memory profile, and sanitized Compose override are retained; generated configuration files are omitted because they contain task-local credentials. The scored qBittorrent comparator was 5.3.0 (`e887…`); the fixed qBittorrent leecher used for every client was 5.2.3 (`2be…`).
