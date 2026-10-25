# TorrentNG Backup/Restore Certification

- Date UTC: 2026-10-25T23:28:50Z
- Host: kspld0
- Commit: c4329b2e
- Binary: /home/keith/Documents/code/TorrentNG/target/release/torrentngd
- Binary SHA-256: 74fbbd96dad5d71541267b668c9f7414667ca35d210362f4624f2bf36b6dadc3
- Fixture: /tmp/torrentng-release-readiness-20261025-02/torrentng-backup-restore.cho79b/backup-restore.torrent

This is a disposable TorrentNG-client drill. It creates one paused torrent in a
temporary session, performs a SQLite online backup plus session archive, tears
down the source daemon, restores the archive into a different session/data
root, starts a second daemon, and verifies that the restored state is usable.
Payload bytes are intentionally not part of this drill; production payloads
remain a separate storage-backup responsibility.

## Checks

| Check | Result | Detail |
|---|---|---|
| Source daemon startup and health | PASS | authenticated TorrentNG health endpoint |
| Persisted torrent creation | PASS | hash=d504621befce387c0019d20363fdf8369703a5c9 |
| Source state projection | PASS | category and tag survived API read |
| Source SQLite integrity | PASS | PRAGMA integrity_check=ok |
| Archive creation and listing | PASS | SQLite online backup plus session metadata archive |
| Restored SQLite integrity | PASS | PRAGMA integrity_check=ok |
| Restored daemon startup and health | PASS | restored config and session accepted |
| Restored torrent identity and metadata | PASS | hash, state, category, and tags match source |
| Post-restore mutation | PASS | category update committed through restored worker-backed DB |

- Source session: one paused torrent with persisted category and tag state
- Restored session: state listed, then a category mutation committed
- Archive bytes: 7688
- Source SQLite integrity: ok
- Restored SQLite integrity: ok

Overall status: PASS
