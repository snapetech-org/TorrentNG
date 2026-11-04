# TorrentNG PR #243 status metric RTL isolation review

Review date: 2026-11-04

## Pins

- PR #243 product head before remediation: `11ff6fa2618e842e92716b7e92867769200e32e5` (tree `770415a3fcd9353884ac245b53798af9610c0aa3`).
- Live PR base during review: `0de7ddcf2f16a4399da8fd7904dc35f458b4837c`.
- Remediation commit: `a41887bf6d5d09a328fcf5bcddc9cfb8e888a968`.

## Finding and remediation

The static review found that Arabic RTL direction was enabled while several mixed or numeric status values rendered without bidi isolation. `StatusBar.tsx` now sets `opaqueValue` on the four torrent counts, download and upload speeds, downloaded and uploaded totals, disk summary, and connection summary. The existing `Badge` renderer wraps those dynamic values in `<bdi dir="auto" lang="und">`; firewall and DHT/PEX values already used that path.

An independent code review of the exact diff confirmed that all ten value callsites are covered and that the change closes the identified metric-isolation gap. No screenshot or headed-browser result is claimed for this remediation.

## Review limits

The static review also noted an assurance limitation in `localeSourceCoverage.test.ts`: the recorded producer-path mappings are not checked against actual callsites. This remains documented and unchanged. No tests, builds, or CI jobs were run. The source commit’s automatic local pre-commit hook ran Semgrep on `StatusBar.tsx` (210 rules, zero findings); this is hook output, not full validation. No workflow changes or native-speaker review were performed. The PR remains open and draft; no merge, release, or tag was made.
