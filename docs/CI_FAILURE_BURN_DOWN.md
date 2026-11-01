# TorrentNG CI Failure Burn-down

## 2026-10-31 private-JIT visual baseline correction

The historical recommendation below to route WebUI certification through
GitHub-hosted Ubuntu is superseded by the 2026-10-26 owner directive: all CI,
test, scan, package, and release jobs must use compatible private ephemeral JIT
runners. The current `webui` job runs on `ng-shared-main-jit`.

Private-JIT run `41169356206` on `040896a07fe17994723ce53227cacf5ad151fb43`
failed only three visual comparisons while its unit tests, build, lint,
accessibility checks, and other browser cases passed. The desktop workspace and
settings screenshots from that run match the existing private-JIT baselines
byte-for-byte; the mobile workspace differs by one pixel. The test was still
hard-coded to the obsolete `hosted` snapshot profile. The profile and baseline
correction is isolated in the candidate change; its exact-head private-JIT CI
result is required before this failure is considered closed.

Status: **not green** as of 2026-10-26. CodeQL (`40252195029`), Trivy
(`40252194955`), and Semgrep (`40252195011`) are pending or queued for
`30fe11b6afdbad34d0191001c221ebd073edf103`; there is no CI run for that SHA.
A manual CI run on `83c24ea` reached the WebUI job but failed during setup
because its private JIT runner has no Docker CLI, so the pinned Playwright
container never started. The WebUI job now uses GitHub's Ubuntu 24.04 runner,
which provides Docker for that container. CI and security checks must complete
on the runner fix before those gates can close.

## Failures fixed

| Run / job | Actual failure | Fix | Evidence |
| --- | --- | --- | --- |
| `33906940869` / Backup and restore drill | The clean GitHub checkout did not contain the ignored recovery fixture. | `718e571` makes the drill create a valid tiny v1 fixture when the ignored local fixture is absent. | Final backup/restore job passed in `33916500668`. |
| `33906940869` / Backend fault-containment matrix | `yield_now()` was not a durable scheduling boundary on the hosted runner; the pause test raced the worker. | `718e571` changed the test to bounded deadline polling of durable SQLite state. | Full TorrentNG-client suite and final fault job passed in `33916500668`. |
| `33908414275` / `native-quality` | `tar -xOzf ... | grep -q` caused GNU tar to receive a broken pipe under `pipefail`. | `dc4ab9a` extracts the archive before checking its contents; `b748f59` applied the same fix to archive listing. | Certification bundle self-test and final `native-quality` job passed in `33916500668`. |
| `33909060233` / `native-quality` | Cleanup became visible on disk before the terminal job row was durably removed, so the test asserted too early. | `8a85615` waits for both filesystem cleanup and the empty durable job projection. | The regression passed repeatedly locally and in final hosted CI `33916500668`. |
| Final interop harness path | `curl | grep -q` let `grep` exit early and surfaced curl's SIGPIPE as a false failure under `pipefail`. | `83b70ce` captures the metrics response and checks it with a here-string. | Current 28/28 Docker matrix and final CI `33916500668` are green. |
| `35014022485` / `native-quality` | A pure-v2 magnet recheck updated runtime `amount_left` but left the tracker detail row at its pre-recheck `left_bytes` value. | `7f66041` flushes runtime and tracker state after the recheck before acknowledging the command, and retains peer availability updates across rechecks. | The regression passed repeatedly locally; all ten jobs passed in hosted CI `35016899959`. |
| `35016119210` / `webui` | The accessibility scan sometimes sampled a properties dialog during its 140 ms entrance animation, producing a transient contrast violation. | `56d77e5` waits for the dialog opacity to settle at `1` before running axe. | Ten repeated dialog scans, the full local browser suite, and hosted CI `35016899959` passed. |
| `40244559651` / `webui` | The visual comparisons differed on the unpinned rendering environment. | Pin the Playwright `1.64.1-noble` image digest, verify the package/image version, keep the 1% threshold, and run the job on GitHub's Ubuntu 24.04 runner so Docker can start the image. | The initial failure artifact is retained. The exact-SHA hosted rerun is pending. |
| `40246072603` / `webui` | The private JIT runner had no Docker CLI, so the job-level Playwright container failed before checkout or browser tests. | Route this isolated, secret-free job to `ubuntu-24.04`; assert the runner and image digest in `scripts/test_workflow_runner_policy.py`. | The pinned-image job has not yet completed on the updated runner. |

## Other CI hardening included

- `e226c03` waits for daemon readiness after the interop restart before
  probing facade endpoints.
- `70852de` makes the local security scan fail closed when required tooling is
  unavailable; explicitly allowing a blocked local tool produces a warning,
  not a false clean pass.
- `18b836c` makes the backup/recovery evidence portable and diagnosable in a
  clean checkout.

The cancelled intermediate CI runs were superseded by later pushes while the
same failure burn-down was in progress. They are not evidence of a passing
check. Dependabot alerts, CodeQL alerts, and secret-scanning alerts were last
reported as zero in the prior audit; this document does not treat that as a
current exact-SHA security result.

## Current boundary

The CI workflow is configured to cover TorrentNG-client quality, both
declared MSRV floors, fuzz smoke, the compatible-client service, WebUI,
dependency security, backup/restore, API/SSE load, and deterministic fault
containment. No complete CI run has passed on the current `main` head. The
latest CodeQL and filesystem scan runs for that head are pending, not passed.
CI does not certify target-device
storage, public-swarm behavior, branch-protection enforcement, a 24-hour soak, production-corpus
allocator/fairness profiling, or optional extended-capacity claims.
