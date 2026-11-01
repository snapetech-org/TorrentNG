# TorrentNG populated-catalog localization review

This review reconciles all populated target cells in the 12 direct locale catalogs at PR #243 head `57f2d6ae2f01f322ca2c9e7e49607d199912a3bd` (tree `498f38fac00ff999135f0aadb1a0eaeabf2d54f5`). The independent changed-target batches are pinned to that same head and tree. The four accepted corrections are applied by the follow-up commit that adds this evidence.

## Coverage

- The existing-cell corpus contains 13,704 cells. Of these, 13,654 still match the earlier exact-target review. Six changed targets had separate exact candidate adjudications. The remaining 44 changed targets were independently reviewed in two slices (5 and 39 cells).
- The new-cell corpus contains 3,372 cells: 3,263 match the primary semantic review and 109 match its independent cross-review.
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
- `policy/` contains the protected-literal policy audit.
- `first-slice/` and `remaining-39/` contain the independent exact-head selections, contexts, decisions, eligibility reconciliations, manifests, and checksums for the 44 changed targets.

The evidence records that native-speaker approval is still required and is not claimed here. Final headed-browser review must be repeated against the updated PR head after these catalog changes.
