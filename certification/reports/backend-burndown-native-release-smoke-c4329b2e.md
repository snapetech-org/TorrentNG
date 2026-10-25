# TorrentNG Client Release-Binary Smoke

- Date UTC: 2026-10-25T23:28:47Z
- Host: kspld0
- Commit: c4329b2e
- Binary: /home/keith/Documents/code/TorrentNG/target/release/torrentngd
- Binary bytes: 25129520
- Binary SHA-256: 74fbbd96dad5d71541267b668c9f7414667ca35d210362f4624f2bf36b6dadc3
- Config: /tmp/torrentng-release-readiness-20261025-02/release-smoke/config.toml
- Worktree: clean

This run exercised the optimized production daemon binary with the
authenticated TorrentNG and qBittorrent facades, then sent SIGTERM and
waited for process exit. It is a deployment smoke test, not 100k-scale
capacity evidence.

## Checks

| Check | Result | Evidence |
|---|---|---|
| Startup and health | PASS | backend-burndown-native-release-smoke-c4329b2e.health.json |
| TorrentNG list envelope | PASS | backend-burndown-native-release-smoke-c4329b2e.torrents.json |
| TorrentNG transfer info | PASS | backend-burndown-native-release-smoke-c4329b2e.transfer.json |
| qBittorrent list | PASS | backend-burndown-native-release-smoke-c4329b2e.qbit.json |
| qBittorrent transfer info | PASS | backend-burndown-native-release-smoke-c4329b2e.qbit-transfer.json |
| Prometheus metrics | PASS | backend-burndown-native-release-smoke-c4329b2e.metrics.txt; 828 lines / 57514 bytes |
| SIGTERM and clean exit | PASS | backend-burndown-native-release-smoke-c4329b2e.log; 2 polls |

- Total smoke duration milliseconds: 486

Overall status: PASS
