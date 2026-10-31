# TorrentNG headless QA and public release plan

## Goal and release standard

Exercise TorrentNG's repository-owned behavior with repeatable headless checks, find and fix actionable defects, and assemble fresh evidence for one exact release revision. The work covers the native engine, the separate compatibility sidecar, WebUI, APIs, persistence and recovery, packaging, security, and release automation.

"Bug free" cannot be proven by a finite test run. The release standard is therefore: no known open release-blocking defect; every required source and runtime gate has fresh, reviewable evidence tied to the exact release commit and artifact; and the strict release-readiness gate reports `PASS`. Missing, stale, skipped, warning, or failed required evidence blocks a public release. Local unit tests alone are not a release qualification.

## Security scanning policy

Per the owner directive dated 2026-10-30, CodeQL analysis is disabled for
snapetech-org repositories. TorrentNG intentionally has no
`.github/workflows/codeql.yml`, and the runner-policy test keeps it absent.
Historical CodeQL results remain archival; they are not current release
evidence. Continue the private-JIT Semgrep, Trivy, dependency-audit, and secret
scanning gates.

## Risk areas to cover

1. Torrent identity and metainfo parsing: v1, v2, hybrid, magnets, piece layers, malformed and oversized inputs, exact hash verification.
2. Peer, tracker, DHT, PEX, uTP, webseed, and transfer lifecycle: framing, timeouts, cancellation, duplicate requests, rate limits, bounded queues, completion, retries, and shutdown.
3. Storage and filesystem safety: path/root confinement, symlinks, no-overwrite behavior, short I/O, concurrent moves/imports/deletes/rechecks, durability modes, rollback, device scheduling, and failure recovery.
4. Persistence and orchestration: ordered database writes, cancellation fencing, panic containment, migrations, crash recovery, stale completions, registry/database projection consistency, backup and restore.
5. API/authentication and compatibility: native API, qBittorrent, Transmission, Deluge, rTorrent, access control, session isolation, redaction, pagination, WebSocket reconnects, and capability manifests.
6. WebUI: user-visible workflows, keyboard/accessibility, large lists, mobile viewport, error/loading/empty states, WebSocket updates, production assets, and browser regressions.
7. Release/deployment: reproducible locked builds, MSRV, release binary smoke, Docker and package metadata, workflow permissions, provenance, dependency security, and release-note/tag policy.

## Execution sequence

Run these stages against a clean checkout of the intended release commit. During investigation, tests may run against a dirty worktree, but that result is diagnostic only and cannot satisfy exact-commit release evidence. Use locked dependencies, record the complete source SHA, host/toolchain, command, exit code, duration, and report path. Preserve failing logs. Keep disposable data and reports in task-scoped scratch; reuse dependency/build caches and do not delete shared caches.

### 0. Establish the source and environment

- Record branch, full `HEAD`, worktree status, submodules, Rust/Cargo/Node/npm versions, OS/architecture, available storage, Docker access, toolchains, browsers, and available test mounts.
- Stop release qualification on a dirty or changing tree. Never reuse a report whose source revision or artifact digest differs from the candidate.
- Run `git diff --check` and verify generated output stays outside the source checkout where the harness supports it.

### 1. Static, schema, and script checks

- Rust formatting: `cargo fmt --all -- --check`; repeat with the sidecar and desktop manifests.
- Contract checks: `python3 scripts/validate_openapi.py`; validate release-note and workflow policy.
- Run all repository Python harness regressions with `python3 -m unittest discover -s scripts -p 'test_*.py'`; run each `scripts/*_selftest.sh`.
- Run shell syntax/static checks (for example `bash -n` and ShellCheck when installed), YAML/workflow validation, Docker Compose config validation, Kubernetes manifest validation, and YunoHost package checks.
- Review changed configuration, manifests, permissions, secrets, and package metadata; do not treat a static review as runtime qualification.

### 2. Rust unit and integration matrix

- Native workspace: `cargo test --workspace --all-targets --locked -- --test-threads=1`.
- Feature-sensitive paths from CI: `cargo test --locked -p torrentngd --bin torrentngd --features full-compat` and `cargo test --locked -p rt-storage -p rt-engine --features seed-mmap --lib`.
- Strict lint: `cargo clippy --workspace --all-targets --locked -- -D warnings` and the full-compat Clippy command from CI.
- Sidecar (separate workspace): format, locked build, locked all-target tests with one test thread, and strict Clippy using `--manifest-path sidecar/Cargo.toml`.
- Desktop package: format, strict Clippy, build, and tests using `--manifest-path crates/torrentng-desktop/Cargo.toml`.
- Repeat the applicable native and sidecar suites on declared MSRVs (Rust 1.88 and 1.97 respectively). If a toolchain is unavailable, record it as blocked; a newer compiler pass does not replace this check.
- Add focused tests for every defect found. Test correctness before and after the fix, then rerun the affected package and final broad matrix.

### 3. WebUI headless matrix

- Install from `webui/package-lock.json` in an isolated install directory or use a verified existing lock-consistent install; do not remove shared `node_modules` or caches.
- Run `npm test`, `npm run build` (TypeScript plus production Vite bundle), `npm run lint`, and `npm run test:e2e -- --reporter=line` with headless Chromium.
- Confirm the E2E projects cover desktop and mobile emulation, critical add/control/settings flows, accessibility, scale, and visual snapshots. Keep browser outputs and traces in task-scoped scratch.
- Test packaged assets and a production build against the API, including authentication failures, empty/error/loading states, reconnect/resync, and stale request handling.

### 4. API, persistence, recovery, and local integration

- Run `scripts/api_facade_certification.sh`, migration corpus certification in strict mode, storage feature matrix, backup/restore certification, authenticated release-binary smoke, and native/sidecar security configuration reviews.
- Run the backend fault matrix and API-load harness with bounded, disposable state. Check crash/restart, DB failure, cancellation, rollback, saturation, request-size limits, and cleanup behavior.
- Run local Docker interoperability for qBittorrent, Transmission, Deluge, and rTorrent when Docker is available. Do not infer public swarm or hardware behavior from mocked/unit tests.
- Verify upgrade and restore using the exact candidate artifact and a disposable data directory. Preserve the prior artifact and do not use an active torrent library as test data.

### 5. Security and supply-chain gates

- Audit every lockfile (root, sidecar, fuzz, desktop) with RustSec and WebUI dependencies with `npm audit --audit-level=high`.
- Run Semgrep, Trivy/container scans, dependency audits, secret scanning, license/package checks, and workflow permission/runner policy validation on the exact candidate commit. Review all exceptions and suppressions.
- Exercise authorization boundaries, API key/OIDC login, secret redaction, URL/config redaction, path traversal/symlink defenses, request limits, hostile metainfo/protocol frames, and token/session revocation.
- Record scanner/tool versions and database freshness; missing tools or stale vulnerability databases are not passes.

### 6. Fuzz, fault, scale, and performance evidence

- Run the CI-bounded parser/protocol fuzz smoke targets, then maintain a longer fuzz campaign with retained seeds and crash triage before release. A short smoke is not a fuzzing campaign.
- Run deterministic fault-injection suites and process-crash cuts at storage/database checkpoints. Verify restart recovery and data integrity, not just process survival.
- Run scale/load checks for API, torrent churn, transfer, and WebSocket updates with resource observations bound to the exact run. Synthetic 10k/100k catalog tests are diagnostic unless a named acceptance gate says otherwise.
- Compare whole-client performance only with pinned clients, matched workloads, exact artifacts, and CPU/RAM/network/disk evidence. Do not make superiority claims from microbenchmarks or a single trial.

### 7. Deployment and release artifact matrix

- Build, inspect, and smoke the exact optimized binaries/images. Verify architecture, version, source revision, checksums, SBOM/provenance, signatures where configured, release notes, and immutable image/tag promotion rules.
- Validate supported install/update paths (Docker/Compose, native package formats, YunoHost, Kubernetes/systemd as applicable) in disposable environments; test config migration, permissions, health, rollback, and backup restore.
- Run the release workflow's required jobs on the authorized hosted/private runner. A local approximation cannot replace required hosted CI or runner isolation checks.

### 8. External release qualification

Run against the exact merged commit/artifact on suitable, authorized infrastructure:

- Dedicated physical storage targets for the supported storage matrix, durability, io_uring, and real-root move/import gates.
- Local disposable client matrix plus opted-in public legal-torrent transfers and required public-network behavior.
- Source- and image-bound 24-hour soak; add loaded-swarm/transfer soak where that behavior is claimed or required.
- Crash-cut, recovery, real-client interoperability, target-OS/package, browser, and network-condition checks not represented by local unit tests.
- Fresh security/hosted CI evidence and any private-runner-only release steps.

Preserve source/image identity in all external reports. Idle-daemon soak, local facade tests, stale qualification reports, and one public torrent cannot stand in for the other gates.

## Agent roles and coordination

- **QA lead:** owns the exact revision, risk/coverage matrix, test execution log, defect triage, fixes, and final strict readiness decision.
- **Native/runtime agent:** maps tests to engine, storage, DB/jobs, network/protocol, migration, and recovery risks; runs the native matrix and focused repros.
- **Web/API agent:** runs headless UI, accessibility, API facade, sidecar, desktop, and packaging checks without sharing mutable test state.
- **Release/security agent:** checks workflows, dependency/security scans, artifact provenance, certification freshness, and external gates.

Agents must report exact commands, revisions, results, and artifact/log paths. Coordinate shared ports, Docker resources, databases, and storage mounts; serialize conflicting suites. Do not publish, deploy, or create public swarm traffic without the required explicit opt-in for that environment.

## Release decision checklist

- [ ] Candidate is a clean, immutable commit; every report and binary/image names that full SHA/digest.
- [ ] All required source, feature, sidecar, desktop, WebUI, API, migration, storage, recovery, security, dependency, and artifact gates pass.
- [ ] CI/private-runner checks and declared MSRV checks pass on the candidate.
- [ ] Required real-device, client/public-swarm, crash/recovery, and soak gates have fresh passing evidence; no required row is skipped, stale, warning, or missing.
- [ ] No open release-blocking defect; all known exceptions have an owner, rationale, and expiry.
- [ ] `scripts/release_evidence_suite.sh` and `scripts/release_readiness_gate.sh` pass in strict scope against the same evidence directory.

## Current execution record

### Run on 2026-10-25

#### Source and environment

- Candidate application and deployment revision: `329b9946876032c5d7642e476566983391631fb4` (`main`, pushed to `origin/main`). The plan and this record are committed separately; that documentation-only commit does not change the tested application or deployment files. No release tag was created.
- Host: `kspld0`, EndeavourOS Linux, x86_64; Rust/Cargo 1.94.0; Node 24.22.1/npm 11.19.1; Playwright 1.64.1; Docker 29.9.2/Compose 5.6.1.
- The QA commits between `5fd8d5b9` and the candidate change deployment manifests, interop test setup, documentation, and browser snapshots. They do not change Rust engine or `webui/src` application code. Broad engine, fuzz, API-load, sidecar, desktop, and MSRV results below were run at `5fd8d5b9`; exact-candidate workspace, storage, release-artifact, API, WebUI, security, and Docker interoperability gates were also run at `329b9946`.
- Evidence and logs are retained under `/tmp/torrentng-headless-qa-20261025-01` on `kspld0`. All local torrent tests used generated fixtures, a task-local Docker stack, loopback-bound host ports, and disabled DHT/PEX/NAT/UPnP/LSD. No public torrent traffic was initiated.

#### Fixes made while qualifying

- Set the native Kubernetes container to a read-only root filesystem and added task-local writable volumes at `/tmp`, `/downloads`, and `/run/secrets`, matching the paths used by the entrypoint and runtime.
- Hardened interop cookie-jar initialization for qBittorrent and Deluge: create a private mode-0600 jar safely and reject symlink targets.
- Changed the interop QA image's default Cargo features to `system-allocator,full-compat`. The API-facade certification probes optional Transmission and Deluge routes, so the QA target now enables those routes. The production image defaults are unchanged.
- These were deployment/test-harness issues. No native-engine or WebUI application defect was reproduced during this run.

#### Results

| Area | Result | Evidence |
|---|---|---|
| Repository scripts and contracts | PASS | 459 Python harness tests; OpenAPI validator covered 67 paths and 93 operations; shell syntax, ShellCheck, policy/self-tests, YAML/Compose/Kubernetes validation passed. |
| Native workspace and storage | PASS | Exact-candidate local release gate: formatting, locked workspace tests, and storage feature matrix passed. Full-compat tests: 32 passed, 1 ignored; seed-mmap storage/engine tests: 371 passed. Strict Clippy passed. |
| Sidecar and desktop | PASS | Sidecar: 122 tests and strict Clippy; desktop: 38 unit tests plus engine roundtrip, strict Clippy, and build. Rust 1.88 native and Rust 1.97 sidecar checks passed. These components had no source changes after `5fd8d5b9`. |
| WebUI UX/UI | PASS with runner limitation | Isolated runner-profile run: 57 unit tests; production build, lint, and npm audit passed (0 vulnerabilities); Playwright desktop/mobile/accessibility/scale suite: 30 passed, 10 expected skips, 0 failures after refreshing only the task-local snapshots. The exact-candidate integrated run on `kspld0` passed build/lint and 27 E2E checks, skipped 10, and reported 3 hosted visual-baseline diffs (two desktop, one mobile). The rendered pages were otherwise stable; failure screenshots and traces are retained. No local-only baselines were committed. |
| API, persistence, and recovery | PASS | Exact-candidate API-facade certification, authenticated release-binary smoke, backup/restore, migration corpus, and native/sidecar config reviews passed. The authenticated binary smoke exercised health, torrent lists/transfers, qBittorrent facade, metrics, and clean shutdown. |
| Local client interoperability | PASS | Exact-candidate full-compat matrix: all 10 base cases passed (four existing-client-to-native downloads, four native-to-client downloads, mesh swarm, and 25-torrent churn). All 4 extended cases also passed (webseed-only, private explicit peer, restart recovery, and REST/qBittorrent/Transmission/Deluge facades). QA image digest: `sha256:b99c0445182cf8a3a5bba5ddfccd26d84143a9be8f08dd779ed71254553d7e5f`. |
| API load and fuzz smoke | PASS | Earlier candidate run: 125,394 successful API requests, 0 errors/timeouts, p50 1.79 ms, p95 3.32 ms, p99 4.19 ms; four SSE clients completed without lag. Eleven CI-bounded fuzz targets ran for 60 seconds each with no crash. No relevant engine source changed after that run. |
| Security and supply chain | FAIL | npm and RustSec audits found no actionable high/critical dependency vulnerabilities; container image scan found none. Semgrep previously reported 0 findings across 3,136 tracked files, with staged checks clean on follow-up commits. `scripts/security_scan.sh` failed Trivy config scan on `deploy/docker/Dockerfile:126` (DS-0002, `USER root` during PUID/PGID initialization before privilege drop). This startup design initializes as root before dropping privileges and remains an open scan finding. RustSec also reports unmaintained `paste` and desktop dependency advisories; the tracked Deluge private key is a self-signed interop fixture. |
| External qualification | NOT RUN / BLOCKED | External preflight passed Docker and migration-corpus checks, but warned that public-torrent opt-in and a real storage target were unset. The 24-hour soak was explicitly deferred. Physical storage durability/io_uring/move-import, public-network transfer, hosted release CI, and private-runner gates remain outstanding. |

#### Release decision and retained reports

- Exact-candidate `scripts/local_release_gate.sh` result: `FAIL` because the local runner did not match the three hosted visual snapshots; real-device storage certification was skipped because no authorized target mount was available. The other listed gates passed. Report: `/tmp/torrentng-headless-qa-20261025-01/reports-final/local-release-329b9946.md`. Failure screenshots, error contexts, and traces are retained in `/tmp/torrentng-headless-qa-20261025-01/failure-evidence/webui-hosted-baseline/`; task-generated `webui/test-results` output was removed from the checkout.
- Exact-candidate `scripts/security_scan.sh` result: `FAIL` on the Trivy Dockerfile finding above. Report: `/tmp/torrentng-headless-qa-20261025-01/reports-final/security-image-329b9946.md`.
- `scripts/release_readiness_gate.sh` and `scripts/release_evidence_suite.sh` in strict scope both returned `FAIL`. The evidence directory does not contain the required current public/client, hardware, soak, hosted-CI, and other certification reports; the WebUI and security findings are also non-clean. Reports: `/tmp/torrentng-headless-qa-20261025-01/reports-final/release-readiness-strict-329b9946.md` and `/tmp/torrentng-headless-qa-20261025-01/reports-final/release-evidence-suite-strict-329b9946.md`.
- The exact-candidate external preflight is `PASS_WITH_WARNINGS`: `/tmp/torrentng-headless-qa-20261025-01/reports-final/external-preflight-329b9946.md`. Local interop reports are `interop-base-fullcompat-329b9946.md` and `interop-extended-fullcompat-329b9946.md` in the same directory.
- The CodeQL, Semgrep Community, and Trivy push runs for `329b9946` were canceled before jobs started when documentation-only commits advanced `main`. On docs-only revision `f059fc91`, their push runs were pending with no jobs started at 22:35 UTC. The test-only CI workflow was manually dispatched on `f059fc91` and was queued with its jobs waiting for runners; no CI job had started at 22:35 UTC. Duplicate manual security dispatches were canceled after the push runs appeared. The release/publish workflow was not dispatched. No production artifact was published.
- **Release status: NOT READY for public release.** This run supplies broad local evidence but cannot prove the software bug-free. Clear the visual-runner mismatch, obtain passing physical-storage/public-network/soak and hosted CI evidence, resolve the security scan finding, and rerun the strict release evidence and readiness gates on the same final source and image before release.

The execution record is append-only for future candidate runs. A pass on a dirty development checkout is diagnostic evidence only and does not clear the release checklist above.

### Run on 2026-10-25 — candidate `c4329b2e`

#### Source and scope

- Candidate application/deployment commit: `c4329b2e2e74aa0703ccceaf74165417aae88d82` (`main`, pushed to `origin/main`).
- The run fixed actionable release QA blockers in security-scan failure handling, runtime image policy annotations, YunoHost bundle ownership, and the physical-storage benchmark fixture. These changes are in the preceding commit; the app UI/engine source is unchanged from `15b2ec73933f46ac836cfb94b5f47797a27b2438`.
- Local/browser traffic used generated fixtures and loopback-only test services. No public torrent traffic was initiated.
- The storage qualification ran from a clean exact-SHA worktree on `kspls0`, against its HDD-backed ext4 LVM target. Reports are copied into `certification/reports/`; complete logs and scanner JSON/SARIF remain in `/tmp/torrentng-release-readiness-20261025-02/` on `kspld0`.

#### Results

| Area | Result | Evidence |
|---|---|---|
| UX/UI and accessibility | Local PASS; hosted visual check pending repair | 57 unit tests, production build, lint, npm audit (0 findings), 30 desktop/mobile browser tests passed locally, 10 expected skips, and no serious automated accessibility findings. Desktop/mobile snapshots matched in the pinned runner and were reviewed. The first hosted run failed three screenshot checks because its unpinned renderer differed; see the hosted runner follow-up below. |
| Native and release binary | PASS with scope limits | Optimized locked daemon build and authenticated loopback API smoke passed on `c4329b2e`; API list/transfer and qBittorrent facades, metrics, and SIGTERM shutdown passed. Broad local workspace-quality results were from base `15b2ec73`; they are not relabeled as exact-candidate hosted CI. Exact-candidate formatting, workflow/security harnesses, storage feature compile, and clean-checkout storage certification passed. |
| Backup/restore | PASS | Exact-candidate SQLite integrity, session archive, daemon restore, and state/mutation checks passed. See `certification/reports/backup-restore-certification-c4329b2e.md`. |
| Physical storage | PASS for tested target and workload | Exact-candidate physical HDD matrix, io_uring selection, real-root move/import, and storage self-test passed on `kspls0` LVM/ext4. In the synthetic shuffled fixture, 4,096 requests coalesced to 1 backend read. Sequential readahead reduced 4,096 submitted reads to 133 backend reads (30.8× read-count reduction). Timing was a single noisy trial and does not support a general speed or throughput claim. See the four `*-c4329b2e.md` storage reports in `certification/reports/`. |
| Local security | PASS with hosted confirmation pending | Semgrep scanned 3,137 tracked files (531 rules run), 0 findings; Trivy found 0 HIGH/CRITICAL vulnerabilities across 8 lockfiles, 0 secrets, and 0 misconfigurations in 9 config files. RustSec reported 5 policy-allowed advisories. See `certification/reports/security-scan-20261025-c4329b2e.md`. |
| External preflight | PASS_WITH_WARNINGS | Fresh `kspls0` preflight validated Docker access, writable physical target, and migration corpus. Public torrent opt-in was unset and the 24-hour soak was explicitly deferred. See `certification/reports/external-evidence-preflight-c4329b2e.md`. |
| Hosted CI/security | PENDING | At record time, candidate CI `40244559651`, Semgrep `40244390980`, CodeQL `40244390962`, and Trivy `40244390929` had not completed. Local results do not substitute for these hosted runs. |
| 24-hour soak/public network | INCOMPLETE | No source/image-bound 24-hour soak or public swarm transfer was run. The older 2026-10-03 soak report has no matching source/image identity. Public torrents remain an explicit opt-in. See `certification/reports/soak-final-20261025-c4329b2e.md`. |

#### Release disposition

**NOT READY for public release.** Local UX/UI, release-binary, restore, security, and target-specific storage checks are favorable, but hosted CI and hosted security are pending, and source/image-bound soak and opted-in public-network qualification have not been run. The strict certification status and release evidence suite must remain authoritative; no “bug-free” claim is justified by this finite test matrix.

The previously queued release workflow for `v20261025-2` (`40241691247`) was canceled before its jobs ran and before publication. It targets older source `15b2ec73933f46ac836cfb94b5f47797a27b2438`. No new tag was created and no release was published.

At record time the exact-candidate workflows were still pending/in progress; append their final results in a follow-up section rather than rewriting this run record. The release checklist above remains open until all required gates have fresh evidence on the same final candidate and artifact.

#### Hosted WebUI runner follow-up

- GitHub CI run `40244559651` on `c4329b2e` passed WebUI unit tests, build, and lint, then failed three visual snapshot comparisons in job `121323279457`. The failure artifact is preserved under `/tmp/torrentng-release-readiness-20261025-02/hosted-webui-failure/` and summarized in `certification/reports/webui-hosted-40244559651.md`.
- Desktop and mobile rendered content remained legible and the controls matched; screenshot differences were font/spacing geometry. The same WebUI tree passed in Playwright `1.64.1-noble` with image digest `sha256:47b510dd9230bf0b9bfc9b20102241e577ea7a99334ab4c3ca19f19770e9b8ea`.
- Updated `.github/workflows/ci.yml` to run the hosted WebUI job in that exact digest and verify the resolved Playwright package version before browser tests. This preserves the 1% visual threshold and existing snapshots. The hosted rerun on the workflow-fix commit is still required; until it passes, the WebUI hosted gate remains open.
