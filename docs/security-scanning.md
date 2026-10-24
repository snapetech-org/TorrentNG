# Security scanning

Run `./scripts/security-scan-local.sh` to scan this checkout locally. Semgrep Community Edition uses the free `p/default` community rules with metrics disabled; the source stays on the machine. The rule pack is fetched from the public Semgrep Registry when it is missing or refreshed, so the first scan needs network access. For a disconnected run, set `SEMGREP_CONFIG` to a local rules directory and `SEMGREP_OFFLINE=1` after caching the Semgrep package.

Semgrep runs on supported source and security configuration. It defaults to two worker jobs for memory-limited environments; set `SEMGREP_JOBS` to change that. Trivy selects `secret` for source/config, `vuln` when dependency manifests or lockfiles are present, and `misconfig` for container, workflow, Kubernetes, Helm, or Terraform files.

The installed pre-commit hook runs Semgrep on staged source/config files; Trivy runs during full local scans and any configured CI workflow.

A weekly public GitHub Actions job runs the current official Semgrep CE image and refreshes `p/default` rules; scan output is uploaded to GitHub code scanning.

The weekly Trivy filesystem job uses the current Trivy release and refreshes its vulnerability database.

Scans report findings without failing by default so existing findings can be triaged. Set `SEMGREP_FAIL_ON_FINDINGS=1` or `TRIVY_EXIT_CODE=1` to make a local scan fail on findings. Trivy downloads its vulnerability and check databases automatically; use `TRIVY_OFFLINE=1` only after those databases are cached. The default timeout is 20 minutes for slow first downloads; set `TRIVY_TIMEOUT` to change it.

Run `./scripts/update-security-tools.sh` to refresh the local Semgrep package cache, pull the current Semgrep fallback image, and refresh Trivy's database. Public scan workflows use weekly schedules to exercise current scanner releases and rules. Existing private runner workflows retain their current version and runner policy.

Semgrep uses the POSIX I/O backend by default for compatibility with restricted Linux runners. Set `SEMGREP_EIO_BACKEND` only if your runner needs a different backend.

## Reviewed Semgrep exceptions

`.semgrepignore` excludes only archived source snapshots under
`certification/reports/TorrentNG-deep-audit-*/source/`; the live files remain
in the scan. Inline `nosemgrep` annotations document protocol-required
BitTorrent v1 SHA-1 hashes, owner-only directory modes, runtime entrypoints
that drop privileges, and a GPG marker string that contains no key material.

## Reviewed RustSec exceptions

The sidecar's OpenID Connect implementation uses RSA and P-256/P-384 only to
verify identity-provider signatures with public keys. It does not decrypt or
sign with private keys, so the secret-dependent timing findings do not apply
to this usage. RustSec has no fixed release for `RUSTSEC-2023-0071`; the
exception remains narrow to that public-key verification path.

The sidecar currently resolves `RUSTSEC-2026-0387` through
`openidconnect`'s P-256/P-384 ID-token signature-verification dependencies.
That verifier receives provider public keys and signatures; TorrentNG does
not use the affected arithmetic on private key material. The dependency chain
does not currently offer a compatible `crypto-bigint` 0.5.x release containing
the upstream fix, so `.cargo/audit.toml` narrowly ignores this advisory. This
does not mean the dependency is patched. Reassess the exception when the
OIDC/elliptic-curve dependency chain adopts `crypto-bigint >=0.7.6`.
