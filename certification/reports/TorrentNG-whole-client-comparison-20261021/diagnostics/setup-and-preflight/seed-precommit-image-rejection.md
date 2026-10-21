# TorrentNG client performance benchmark — incomplete

- Started (UTC): 2026-10-21T19:10:02+00:00
- Source commit: 24e521aed22a9cfb4b905c75a208e06077ad6332
- Source tree: a83706e42237af37bada4a7244f4da12a19291e4
- Host: kspld0; 32 logical CPUs; Linux 7.1.7-arch1-1 (x86_64)
- Host load average at start: 28.36/29.83/20.23
- Failure: RuntimeError: TorrentNG image metadata does not match this benchmark checkout: ['7c2dd481ca224da8f5e47b783678b6110146ba9b', 'd7c6762c4d06fa998f6664a9d8c1cd0ac79f6145be4492731955cdc60046bd63', 'system-allocator'] != ['24e521aed22a9cfb4b905c75a208e06077ad6332', 'da08674a960f9f43044ffc64483c95d272434844434b2db795fc25141772d6ca', 'system-allocator']
- Cleanup: isolated Compose project, containers, and named volumes removed; benchmark image removal failed
- Partial samples: [seed-precommit-image-rejection-samples.csv](seed-precommit-image-rejection-samples.csv)
