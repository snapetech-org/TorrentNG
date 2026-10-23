# PGO worker-count and rTorrent comparison — 2026-10-23

Three randomized 64 MiB rounds compared the exact same batch-8 PGO daemon at two storage workers and one worker, plus the pinned rTorrent 0.16.25 binary. TorrentNG used `pread`; all nine transfers verified the same SHA-256 fixture.

| Client/build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| TorrentNG batch-8 PGO, 2 workers | 3.123 | 15.746 MiB | 5.969 MiB |
| TorrentNG batch-8 PGO, 1 worker | 2.944 | 14.884 MiB | 5.176 MiB |
| rTorrent 0.16.25 | 2.738 | 8.119 MiB | 3.098 MiB |

One worker reduced paired median PSS 5.5% and anonymous memory 13.3% versus two workers, with lower memory in all three rounds. CPU was mixed: paired median +0.3%, with one worker faster in only one of three pairs. Against rTorrent, one-worker PGO remained 7.5% higher in CPU, 83.3% higher in PSS, and 67.1% higher in anonymous memory. This is a measured memory tradeoff, not a CPU win and not parity. No production default changed.

CPU pressure rose during the screen and then fell; the randomized order balanced the first client across the three rounds. Treat the CPU result as a screen. Exact rounds, binary hashes, and host-pressure snapshots are in [`summary.json`](summary.json).
