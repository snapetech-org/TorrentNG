# TorrentNG WebUI UX/UI Certification

- Generated: 2026-10-25
- Candidate source commit: `c4329b2e2e74aa0703ccceaf74165417aae88d82`
- Frontend source and `webui/package-lock.json` are unchanged from `15b2ec73933f46ac836cfb94b5f47797a27b2438`; the pinned Playwright run below exercised that same frontend tree.
- Runner: pinned `mcr.microsoft.com/playwright:v1.64.1-noble`
- Detailed log: `/tmp/torrentng-release-readiness-20261025-02/logs/webui-pinned-image.log`
- Screenshots reviewed: desktop and mobile visual-cert snapshots under `/tmp/torrentng-release-readiness-20261025-02/webui/tests/e2e/webui-visual-cert.spec.ts-snapshots/`

| Gate | Result | Evidence |
|---|---|---|
| WebUI unit tests | PASS | 57 tests across 10 files |
| Production TypeScript/Vite build | PASS | `tsc && vite build` |
| Lint | PASS | ESLint with zero warnings |
| npm audit | PASS | 0 vulnerabilities across 237 installed packages |
| Desktop/mobile browser matrix | PASS | 30 passed, 10 expected viewport-specific skips, 0 failed |
| Accessibility | PASS | Automated desktop/mobile checks found no serious violations |
| Local visual regression | PASS | Pinned desktop/mobile snapshots matched; screenshots were reviewed |
| Keyboard, responsive layout, large lists, selection, settings and dialogs | PASS | Covered by the browser matrix; desktop 15k-row scale path passed |
| Hosted browser matrix | FAIL | GitHub CI run 40244559651 failed three visual snapshot comparisons on the unpinned runner; unit tests, build and lint passed. See `webui-hosted-40244559651.md` and the retained failure artifact. |

The repository CI workflow now pins the WebUI job to the same Playwright image digest used by the passing local run. The hosted rerun on that workflow change is still required.

Overall status: FAIL

This certifies the tested WebUI source and browser environment. It does not cover assistive-technology testing with human users or every browser/device combination.
