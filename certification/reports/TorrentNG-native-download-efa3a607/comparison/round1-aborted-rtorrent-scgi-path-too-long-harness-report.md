# TorrentNG client performance benchmark — incomplete

- Started (UTC): 2026-10-21T22:11:11+00:00
- Source commit: 730550e7be0231db82f6a9ccd3cfc7669b6e798c
- Source tree: f1e81ae803fc0c3f589a096438a52266795547d9
- Host: kspld0; 32 logical CPUs; Linux 7.1.7-arch1-1 (x86_64)
- Host load average at start: 3.27/4.03/3.83
- Failure: interrupted
- Cleanup: isolated Compose project, containers, and named volumes removed; benchmark image retained for reuse: torrentng/torrentngd:benchmark-20261021t221111z; BuildKit builder removed; TorrentNG-specific cache retained (maximum 16GB)
- Partial samples: [controlled-download-corrected-round1-samples.csv](controlled-download-corrected-round1-samples.csv)
