# Controlled download panel 1 — invalid setup

This diagnostic panel is **not a client comparison**. The benchmark used source commit `d2f7269364572403714b7352248c1e1812a89d2f`, clean at startup, and the pinned 756 MiB Debian 13.7 fixture with SHA-256 `a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355`.

| Client | Outcome | Notes |
| --- | --- | --- |
| qBittorrent | PASS, hash verified | First receiver, before the harness restarted the tracker. Printed result: 7.880 CPU-s/GiB. |
| TorrentNG | PASS, hash verified | Started with zero peers and required an explicit peer address. Printed result: 5.296 CPU-s/GiB. Excluded because peer injection changed the trial. |
| Transmission | PASS, hash verified | Started without a live seeder; proceeded after restarting the task-local qBittorrent seeder and reannouncing Transmission. Printed result: 1.541 CPU-s/GiB. Excluded because setup added an uncontrolled wait and intervention. |
| Deluge | Incomplete | Still at zero peers when the panel was stopped. |
| rTorrent | Not run | The panel was stopped before this client. |

The task-local tracker was restarted between clients. The harness then accepted qBittorrent's cached tracker status (`status=working`, `num_seeds=1`) as proof that the restarted tracker knew about the seeder. A live OpenTracker scrape showed `complete=0, incomplete=1`, so there was no seeder registered. The qBittorrent reannounce endpoint returned HTTP 200 but did not change the live scrape; restarting the disposable seeder container caused a fresh announce and changed the live scrape to `complete=1`.

After the missing seeder was restored, the clients began receiving data. The interrupted panel therefore demonstrates a benchmark setup bug, not a TorrentNG peer-wire failure. Its non-qBittorrent resource values must not be ranked or used as whole-client performance claims.

The raw samples, memory profile, and console log are retained beside this note. The run's full local workdir was removed after the benchmark process exited; the verified source ISO is preserved at its pre-existing fixture path.
