# TorrentNG client performance benchmark — incomplete

- Started (UTC): 2026-10-24T21:11:10+00:00
- Source commit: b27fcd9337a22bc231b74d0640083f4a823a9091
- Source tree: cfd1a43b00b75f0bf6f75f30aa0ffc33562a014d
- Host: kspld0; 32 logical CPUs; Linux 7.1.7-arch1-1 (x86_64)
- Host load average at start: 35.99/36.89/34.74
- Failure: RuntimeError: rTorrent SCGI Unix socket path is too long for Linux (116 bytes; maximum 107): /var/tmp/torrentng-perf-counters-20261024/torrentng-client-bench-20261024T211110Z-0u2imks3/config/rtorrent/scgi.sock. Set a shorter TNG_CLIENT_BENCH_WORKDIR or TNG_CLIENT_BENCH_CONFIG_ROOT.
- Cleanup: isolated Compose project, containers, and named volumes removed; no completed benchmark image was available to retain
