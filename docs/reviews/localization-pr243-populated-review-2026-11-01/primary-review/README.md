# TorrentNG PR #243 populated-cell semantic review

## Pin and scope

This pass reviewed every populated target cell in the prepared corpus for PR #243 at head `ad256fee0e65449d0b01062ff7e87e05bd0f6dc8`, tree `2df9a688fbae93e4627cd0e96ba02cbbf6e3d944`: 1,142 source rows × 12 locales = 13,704 cells. Inputs were the checksum-pinned bundle named in `manifest.json`; no catalogs or PR files were edited.

Locales: es, zh-Hans, hi, de, ja, fr, pt-BR, ar, ko, zh-Hant, vi, id.

## Results

- `findings.jsonl` contains exact cells with high-confidence semantic/grammar corrections and source-code context.
- `uncertain-candidates.jsonl` keeps terminology and context cases separate for native torrent-UI review.
- `literal-policy-deviations.jsonl` records all 21 row-scoped invariant-literal mismatches from the prepared audit. Some are semantically acceptable localized equivalents; they are listed as policy deviations, not automatically as semantic mistranslations.
- `reviewed-cells.jsonl` is the 13,704-cell coverage ledger.

Placeholder/ICU safety: prepared audit reported 0 placeholder mismatches. Catalogs are keyed plain strings rather than ICU messages. Dynamic placeholders were preserved in the reviewed cells. This review did not run tests/builds/scans and did not call the local Qwen model.

Native-speaker/layout status: not performed. The exact uncertain cells are listed separately; language idiom beyond those candidates remains subject to native review.

## Evidence and limits

Each finding includes the exact cell ID, English source, current target, suggested wording, rationale, SHA-256 hashes for source/target values, and relevant source callsites. The input manifest and batch SHA-256 values, live PR head/tree, source messages hash, all 46 inspected source-file hashes, and all 12 locale-catalog hashes are pinned in `manifest.json`.
