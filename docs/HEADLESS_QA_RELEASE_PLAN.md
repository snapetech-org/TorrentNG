# TorrentNG headless QA and public release plan

## Goal and release standard

Exercise TorrentNG's repository-owned behavior with repeatable headless checks, find and fix actionable defects, and assemble fresh evidence for one exact release revision. The work covers the native engine, the separate compatibility sidecar, WebUI, APIs, persistence and recovery, packaging, security, and release automation.

"Bug free" cannot be proven by a finite test run. The release standard is therefore: no known open release-blocking defect; every required source and runtime gate has fresh, reviewable evidence tied to the exact release commit and artifact; and the strict release-readiness gate reports `PASS`. Missing, stale, skipped, warning, or failed required evidence blocks a public release. Local unit tests alone are not a release qualification.

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
- Run CodeQL, Semgrep, Trivy/container scans, secret scanning, license/package checks, and workflow permission/runner policy validation on the exact candidate commit. Review all exceptions and suppressions.
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
- [ ] No open release-blocking defect; all known exceptions have an owner, rationale, expiry, and explicit release approval.
- [ ] `scripts/release_evidence_suite.sh` and `scripts/release_readiness_gate.sh` pass in strict scope against the same evidence directory.
- [ ] Human release owner reviews the artifacts, notes, and evidence before publication.

## Current execution record

The execution record is appended below with the tested revision/worktree state, exact commands, outcomes, limitations, and remaining external gates. A pass on a dirty development checkout is useful diagnostic evidence only and does not clear the release checklist above.
