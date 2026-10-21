TorrentNG reassessment — db9e65ac — 2026-10-21

Open REVIEW.html for the report, or REPORT.txt for plain text.
IMPLEMENTATION-HANDOFF.txt gives the next agent the concrete remaining work.

The report separates implemented fixes, unfinished validation, current-source
probe results and unmeasured proposals. Main was fetched and inspected; no
production file, commit or remote branch was changed by this audit.

experimental-bounded-512k-pool.patch is a tested, bounded local prototype.
It is not an accepted whole-client CPU/RAM win and should not be blindly merged.
The strongest next architectural work is batching already queued upload blocks
and reducing repeated file/admission/completion work per useful GiB.

evidence/repository contains the last clean complete comparison, a later dirty
complete comparison and the latest incomplete candidate comparison. These are
not benchmarks of the latest db9e65a changes.
evidence/local contains this audit's real storage/codec probes and test results.
source contains selected exact-current source, its diff from the previous audit,
the commit log and the project license. It is not a full repository checkout.
SOURCE-MANIFEST.json records source identity, toolchain, probe scope and limits.

Reproduce the probes with Python 3.11+, cargo/rustc and an external clean checkout
at db9e65ac5807d7b3e3d80842d9e447b331fa81c5:

  python3 repro/run_probes.py /path/to/TorrentNG /tmp/tng-db9e65ac-reprobe

The output directory must not exist and must be outside the checkout. The script
builds baseline and isolated candidate, checks dependency versions/checksums
against the source lockfile, runs the focused pool tests and writes JSON results.
It does not edit the input checkout or build/run the daemon. Diagnostic operation
sizes above 16 KiB are storage requests, not changes to BitTorrent wire requests.

Rate core tests from the included exact-current source can run independently:
  rustc --edition=2021 --test -O repro/rate-current-tests.rs -o /tmp/tng-rate-tests
  /tmp/tng-rate-tests

The automatic rerun covers the main operation-size experiment and the bounded
pool candidate. The other raw files retain the exploratory read-ahead reduction
and unrestricted 512 KiB pool runs, including their unfavorable results.

CPU units are process CPU seconds per GiB for the local probe, cgroup CPU seconds
per verified GiB for the repository daemon runs. Allocated bytes are cumulative
allocation volume; pool capacities, governor credit, anonymous memory, RSS and
PSS are distinct. No whole-client improvement is claimed from the local probe.
