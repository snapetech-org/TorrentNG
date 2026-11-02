# TorrentNG populated-catalog localization review

This review reconciles all populated target cells in the 12 direct locale catalogs at PR #243 head `57f2d6ae2f01f322ca2c9e7e49607d199912a3bd` (tree `498f38fac00ff999135f0aadb1a0eaeabf2d54f5`). The independent changed-target batches are pinned to that same head and tree. The four accepted corrections are applied by the follow-up commit that adds this evidence.

## Coverage

- The existing-cell corpus contains 13,704 cells. Of these, 13,654 still match the earlier exact-target review. Six changed targets had separate exact candidate adjudications. The remaining 44 changed targets were independently reviewed in two slices (5 and 39 cells).
- The new-cell corpus contains 3,372 cells: 3,247 were accepted by the original exact-hash semantic review, 109 target changes were confirmed by independent cross-review, and 16 unchanged singular-form targets were accepted after caller-context cross-review.
- Combined, the 17,076 populated catalog cells have terminal exact-target dispositions. The 44 newly reviewed changed targets produced 40 acceptances and four corrections, with no holds.

## Corrections applied

- Hindi `API-token login:` → `API टोकन लॉगिन:`.
- Traditional Chinese `Copy the real tracker URL` → `複製實際的追蹤器 URL`.
- Traditional Chinese `Tracker URL` → `追蹤器 URL`.
- Vietnamese `Per-torrent download slots` → `Số khe tải xuống cho mỗi torrent`.

## Evidence

- `primary-review/` contains the prior exact-hash catalog decisions and their manifests/attestations.
- `new-cells/` contains the semantic decisions for all 3,372 new cells.
- `cross-review/` contains the independent cross-review of 109 changed targets.
- `cross-review/caller-verified-singular-crossreview-16-2026-11-02.jsonl` records root’s cross-review of 16 unchanged singular-form targets. It pins source/target hashes and decisions; all 16 were accepted unchanged because their callsites select singular forms only for a count or cardinality of one. `cross-review/caller-verified-singular-crossreview-16-contexts.json` preserves the exact caller excerpts and hashes for the three source files plus hashes for all six affected locale catalogs. The companion SHA-256 file pins both records.
- `policy/` contains the protected-literal policy audit.
- `first-slice/` and `remaining-39/` contain the independent exact-head selections, contexts, decisions, eligibility reconciliations, manifests, and checksums for the 44 changed targets.

The evidence records that native-speaker approval is still required and is not claimed here. The initial browser review was followed by a current-head headed review recorded below.

## Current corrected-target cross-review

At PR head `b5e694e279a5060ff3a4dea790b54dc1b57ded86` / tree `a92a915714c63b793d567be2a858ec78f4a6fbe4`, an independent exact-current-hash review revisited the four target values proposed by the previous correction batch. All four current values were accepted unchanged at moderate confidence. No additional correction or hold was proposed. The evidence is in `corrected-target-crossreview-2026-11-01/`; it records the current catalog and callsite hashes and preserves the need for native-speaker signoff.

## Caller-verified singular-count cross-review

At PR head `e0736c65f62c65bd28d88bb310cb605d125e15e5` / tree `07b6f31e4d37b7e8b907b635f71c5a81fcba5e54`, Codex root cross-reviewed the 16 remaining exact-current-target cells previously flagged for singular-count context. The six preview-match targets, five pending-change targets, and five one-rule/one-run targets were accepted unchanged. Their source branches select singular keys only when the runtime count or cardinality is exactly one; plural branches cover all other values. No catalog target changed. The decision ledger, exact caller excerpts, and SHA-256 are in `cross-review/caller-verified-singular-crossreview-16-2026-11-02.jsonl`, `cross-review/caller-verified-singular-crossreview-16-contexts.json`, and their companion checksum file. Native-speaker approval remains outstanding.
