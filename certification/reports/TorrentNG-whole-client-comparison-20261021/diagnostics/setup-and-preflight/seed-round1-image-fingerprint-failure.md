# TorrentNG client performance benchmark — incomplete

- Started (UTC): 2026-10-21T18:49:48+00:00
- Source commit: 7c2dd481ca224da8f5e47b783678b6110146ba9b
- Source tree: e39e63abbc56f91b35e5124a920793b9eea6fdb2
- Host: kspld0; 32 logical CPUs; Linux 7.1.7-arch1-1 (x86_64)
- Host load average at start: 7.54/7.40/9.44
- Failure: RuntimeError: TorrentNG image metadata does not match this benchmark checkout: ['7c2dd481ca224da8f5e47b783678b6110146ba9b', '023413a5b5c92f59cb4bc33818323d2799058b87cd4241529c984aaa379c5c13', 'system-allocator'] != ['7c2dd481ca224da8f5e47b783678b6110146ba9b', '52d861e5c7edb4acb90b45fdd7a40fa2871a90f7ff259d6ecd242cfcb80a73a6', 'system-allocator']
- Cleanup: isolated Compose project, containers, and named volumes removed; benchmark image removal failed
- Partial samples: [seed-round1-recovered-samples.csv](seed-round1-image-fingerprint-failure-samples.csv)
