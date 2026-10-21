TorrentNG follow-up review — 16da409

Start with REVIEW.txt. Evidence/probe-results.txt contains the executable observations.
Source/ holds selected exact reviewed files, not a complete buildable repository.
SOURCE-MANIFEST.json records revision and source hashes. PACKAGE-CHECKSUMS.sha256 covers package members.

The reproductions detect current defects. A pass means the old behavior was reproduced, not that TorrentNG is correct.
For admission, the probe imports the real scheduler and supplies its simple identity types; it replays the batch acquisition pattern rather than running the entire engine.
Cache and codec cases call the actual current crates.

To reproduce with Rust/Cargo installed and dependencies available:
  python3 repro/run-probes.py --repo /absolute/path/to/TorrentNG

The runner checks the revision, creates a temporary standalone Cargo project, and does not edit the repository.
It builds rt-storage/rt-peer-wire dependencies; it does not build/run the daemon or Docker images.
Optional: --cargo /path/to/cargo --target-dir /path/to/build-cache --offline
A repository with a different revision is rejected unless --allow-different-revision is supplied.
After fixes, update the behavior assertions to express corrected invariants; current assertions intentionally detect the reviewed problems.

See evidence/build.log and evidence/packaged-run.log for this review's build scope.

