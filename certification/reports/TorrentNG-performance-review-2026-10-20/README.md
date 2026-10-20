# TorrentNG performance review and implementation handoff

Prepared: 2026-10-20 (UTC)

## Read first

1. **PERFORMANCE_REVIEW.md** — findings and recommended order of work.
2. **IMPLEMENTATION_HANDOFF.md** — concrete changes, invariants, and acceptance checks.
3. **SOURCE_REFERENCES.md** — links pinned to the reviewed commits.
4. **evidence/** — original checked-in benchmark reports, samples, and memory profiles.
5. **EVIDENCE_SUMMARY.json** — derived summaries of CPU repeats and the memory discrepancy.

This package contains an audit and implementation plan. It contains no applied
performance patch, replacement binary, or newly executed transfer benchmark.

## Source scope

- TorrentNG: https://github.com/snapetech-org/TorrentNG
- Reviewed commit: `8bd30a474b72309a4b95327f32bbd89caeb590dc`
- slskR: https://github.com/snapetech-org/slskR
- Reviewed commit: `2207a9208f017cc14e31e1f01d95ef53639e011f`
- Latest examined transfer candidate reports identify `3191f56e43300003e969541be4b2db01f6b13161` plus a recorded tracked-diff fingerprint.
- Candidate baseline reports identify `7545fc0`; retain their full embedded fingerprints when reproducing.

Do not describe the older transfer reports as a new benchmark of reviewed HEAD.
Current source includes later changes. The benchmark artifacts record dirty-tree
status and fingerprints; a commit ID alone does not fully identify those builds.

## Confidence

- **High:** the described code paths exist in the reviewed snapshot.
- **Moderate:** storage ownership and admission changes are useful optimization targets.
- **Unknown:** the end-to-end savings or whether they will beat reference clients.
- **Moderate:** the container healthcheck is a plausible contributor to transient
  cgroup-only memory spikes; process attribution has not proved that explanation.

The copies under evidence are historical source artifacts. Their statements,
including stale descriptions of a removed upload request-rate guard, must be
interpreted against the current code and the review.

`SHA256SUMS.txt` covers all other package files. Extract the ZIP before following
relative links. Original report-to-sample links work inside evidence/torrentng.

