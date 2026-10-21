TorrentNG deep performance audit — source 0f726a89

Open REVIEW.html for the findings, evidence, CPU/RAM explanation, and priorities.
Read IMPLEMENTATION-HANDOFF.txt for the concrete refactor and validation sequence.

The audit includes:
  - A compiled reproduction using the UNMODIFIED production RateTree module.
  - Results from all 19 existing tests in that module.
  - A separate synthetic worker-handoff cost experiment and raw results.
  - Relevant unmodified source files and a SHA-256 source manifest.
  - Previously published comparison data, clearly identified as older evidence.

Run the reproducible checks on Linux with a recent Rust compiler:
  bash repro/run_checks.sh
Or select a compiler explicitly:
  RUSTC=/absolute/path/to/rustc bash repro/run_checks.sh

The rate reproduction confirms a failure case; it is not a production fix.
Removing a peer in its control case isolates the cause, but is not the proposed
repair: live connections and outstanding grants must remain intact.

No full-daemon benchmark was executed for this audit. The synthetic queue
measurements are not TorrentNG CPU measurements and must not be advertised as
an application speedup. Current-main full-client attribution still requires a
profile on the benchmark host. Source copied here remains at the reviewed commit.

Existing repo source was not edited or pushed. This package is review material
and reproducible evidence for the implementing agent.
