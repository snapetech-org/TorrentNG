# Upload read coalescing: 8 ms and 16 ms evidence

These artifacts compare the 8 ms control with a candidate that changes only
`UPLOAD_READ_COALESCE_MAX_DELAY` to 16 ms from common source base
`4341f2819ee4bd10160f153c13c67d27346ce19b`.

## Contents

- `experiment.patch` is the one-line source change.
- `executables/` contains gzip-compressed release and full-profile PGO daemons
  for both settings. `SHA256SUMS.uncompressed` records hashes of the extracted
  binaries.
- `release-64m-3x/`, `release-128m-3x/`, `pgo-64m-3x/`, and
  `pgo-rtorrent-128m-3x/` contain normalized summaries and their original
  captured summaries. The last comparison also includes rTorrent 0.16.25.
- `trace-8ms-16m/` and `trace-16ms-16m/` contain the debug traces used to count
  storage read batch sizes. Do not use these debug-logging runs for CPU scores.
- `runs/` contains per-run configs, logs, metrics, compressed `smaps`, and
  fixture metainfo. Generated API/bootstrap credentials and payload copies
  were excluded.

All scored transfers passed fixture SHA-256 verification. Summaries list the
binary and fixture hashes, treatment order, active storage backend, duration,
CPU, PSS, and anonymous memory. The parent [`manifest.json`](../manifest.json)
hashes every retained evidence file except itself and also records the PGO
profile hash.
