# Controlled download panel 1 — aborted startup

This is an incomplete run, not a whole-client comparison. It used source commit `730550e7be0231db82f6a9ccd3cfc7669b6e798c`, the pinned Debian 13.7 netinst fixture (756 MiB, SHA-256 `a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355`), a single task-local qBittorrent seeder, and live OpenTracker scrape validation.

| Client | Result | CPU-s/GiB | Goodput | SHA-256 |
| --- | --- | ---: | ---: | --- |
| qBittorrent | Complete | 7.057 | 3.83 MiB/s | verified |
| TorrentNG | Complete | 5.498 | 3.46 MiB/s | verified |
| rTorrent | Not measured | — | — | — |
| Transmission | Not run | — | — | — |
| Deluge | Not run | — | — | — |

The harness waited for rTorrent's task-local SCGI Unix socket. The configured socket pathname exceeded Linux's 107-byte `sun_path` limit, so rTorrent could not be reached and the run was interrupted before the startup timeout. The report, raw samples, process/memory profile, and console output are preserved beside this note.

The two completed rows transferred through the verified local seeder after the harness's live-scrape gate. They are single observations from an incomplete randomized panel and are not ranked as a client comparison. A path-length preflight was added before client startup; the next panel uses a shorter work directory.
