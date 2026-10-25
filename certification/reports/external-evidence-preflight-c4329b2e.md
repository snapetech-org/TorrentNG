# TorrentNG External Evidence Preflight

- Date UTC: 2026-10-25T23:33:03Z
- Commit: c4329b2
- Corpus directory: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus
- Storage target: /mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage
- Strict mode: 0
- 24h soak deferred: 1

## Checks

| Check | Result | Detail |
|---|---|---|
| Docker daemon | PASS | reachable |
| public torrent opt-in | WARN | set UNIVERSAL_LIVE_PUBLIC=1 only after approving public legal torrent downloads |
| real-device storage target | PASS | /mnt/datapool_lvm_media/.codex-tasks/torrentng-release-readiness-20261025-02/storage is writable |
| migration corpus coverage | PASS | all source-family directories contain migration evidence files |
| migration corpus manifest | PASS | validated by /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/reports/final/external-preflight-migration-corpus-20261025T233304Z.md |
| 24h soak | INFO | deferred by TNG_DEFER_24H_SOAK=1; this qualification run makes no 24-hour stability claim |

Overall status: PASS_WITH_WARNINGS
Warnings: 1
