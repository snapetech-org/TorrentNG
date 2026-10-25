# TorrentNG Local Security Scan

- Generated: 2026-10-25
- Source commit: `c4329b2e2e74aa0703ccceaf74165417aae88d82`
- Semgrep: `1.181.1`, `p/default`, 531 rules executed over 3,137 tracked files; 0 findings.
- Trivy: `0.76.1`, CI/fail-closed mode; 0 HIGH/CRITICAL vulnerabilities across 8 lockfiles, 0 secrets, and 0 IaC misconfigurations across 9 configuration files.
- Logs: `/tmp/torrentng-release-readiness-20261025-02/logs/semgrep-final.log` and `/tmp/torrentng-release-readiness-20261025-02/logs/trivy-ci-final.log`.
- RustSec audits: all four lockfiles audited; 5 policy-allowed advisories remain (four unmaintained packages and one yanked package), recorded in `/tmp/torrentng-release-readiness-20261025-02/logs/cargo-audit.log`.
- Hosted security runs for this commit: Semgrep run `40244390980`, CodeQL run `40244390962`, and Trivy run `40244390929`; all are pending/in progress at the time of this report.

## Commands

- `scripts/security-scan-local.sh --trivy-only --ci`
- Full Semgrep Community scan using the repository's `p/default` policy.
- RustSec audit for root, sidecar, fuzz, and desktop lockfiles.

## Interpretation

Local scan result: PASS. Hosted confirmation: PENDING. The overall local certification status remains `PASS_WITH_WARNINGS` until the candidate's hosted CodeQL, Semgrep, Trivy, and CI workflows finish successfully. This report does not imply that every advisory is exploitable or that no undiscovered vulnerability exists.

Overall status: PASS_WITH_WARNINGS
