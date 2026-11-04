# TorrentNG PR #243 status metric RTL isolation review

Review date: 2026-11-04

## Pins

- PR #243 product head before remediation: `11ff6fa2618e842e92716b7e92867769200e32e5` (tree `770415a3fcd9353884ac245b53798af9610c0aa3`).
- Live PR base during review: `0de7ddcf2f16a4399da8fd7904dc35f458b4837c`.
- Remediation commit: `a41887bf6d5d09a328fcf5bcddc9cfb8e888a968`.

## Finding and remediation

The static review found that Arabic RTL direction was enabled while several mixed or numeric status values rendered without bidi isolation. `StatusBar.tsx` now sets `opaqueValue` on the four torrent counts, download and upload speeds, downloaded and uploaded totals, disk summary, and connection summary. The existing `Badge` renderer wraps those dynamic values in `<bdi dir="auto" lang="und">`; firewall and DHT/PEX values already used that path.

An independent code review of the exact diff confirmed that all ten value callsites are covered and that the change closes the identified metric-isolation gap.

## Limited headed-browser review

On 2026-11-04, the exact remediation source commit `a41887bf6d5d09a328fcf5bcddc9cfb8e888a968` was reviewed in an isolated headed Chrome session on the Agent GUI VM at 1440×1000. The Support → Appearance setting was changed to Arabic and the catalog was allowed to finish loading before inspection. Arabic labels rendered, the application shell and settings sidebar mirrored to RTL, and the status footer remained readable. Dynamic values shown in the accessible snapshot included `1.6 MB/s`, `524 KB/s`, `9.9 GB`, `1.2 GB`, `600 GB / 1 TB`, `42 +3`, and `:51413`; the screenshot preserves their visual context.

The footer spans wider than this viewport and uses its horizontal scrollbar, so the full status row is not visible at once. The preview used a task-owned mock API with synthetic values (`1,234` total torrents, `0` rendered, `17` cached); it did not connect to a live backend, display populated torrent rows, or perform torrent actions. This visual review is limited to Arabic shell direction and status-value readability. It does not establish full workflow acceptance, native-speaker approval, mobile coverage, or overall PR acceptance.

Screenshot: [`statusbar-arabic-rtl.png`](statusbar-arabic-rtl.png), SHA-256 `df07f019775a22e4c6c6ec8dae61231fb09301695a2b01b58d52793e0d289a8a`.

## Review limits

The static review also noted an assurance limitation in `localeSourceCoverage.test.ts`: the recorded producer-path mappings are not checked against actual callsites. This remains documented and unchanged. No tests, builds, or CI jobs were run. The source commit’s automatic local pre-commit hook ran Semgrep on `StatusBar.tsx` (210 rules, zero findings); this is hook output, not full validation. No workflow changes or native-speaker review were performed. The PR remains open and draft; no merge, release, or tag was made.
