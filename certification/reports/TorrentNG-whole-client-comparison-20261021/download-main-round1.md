# TorrentNG client performance benchmark — incomplete

- Started (UTC): 2026-10-21T20:12:29+00:00
- Source commit: f886f4699997119366fe80d7161a1820bf10879a
- Source tree: 6eb7ec9a48789a94e7f1738786b4cd54b8c3c473
- Host: kspld0; 32 logical CPUs; Linux 7.1.7-arch1-1 (x86_64)
- Host load average at start: 6.57/5.20/7.02
- Failure: interrupted
- Cleanup: isolated Compose project, containers, and named volumes removed; benchmark image retained for reuse: torrentng/torrentngd:benchmark-20261021t201229z; BuildKit builder removed; TorrentNG-specific cache retained (maximum 16GB)
- Partial samples: [download-main-round1-samples.csv](download-main-round1-samples.csv)
