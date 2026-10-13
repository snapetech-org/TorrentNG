# Agent instructions - TorrentNG

## Communication and authority

These interaction rules apply to every model interface using this repository:

- Never praise a question or validate a premise before answering.
- Correct false premises directly when evidence supports it.
- Assess estimates independently and use explicit confidence levels: high, moderate, low, or unknown.
- Do not invent evidence. State when a result is unknown.
- Distinguish implemented behavior, automated verification, visual inspection, and live interoperability evidence.
- Preserve upstream attribution and follow CONTRIBUTING.md and the repository license.
- Do not publish, merge, deploy, or release without maintainer authorization.

## Required project reading

- Read README.md and CONTRIBUTING.md before product or workflow changes.
- Read docs/README.md to find current architecture, deployment, security, compatibility, and release guidance.
- Read docs/development/validation-engine.md before selecting or reporting checks.
- Read docs/THREAT_MODEL.md before changing authentication, network exposure, or service boundaries.
- Read docs/CRASH_SAFETY.md and docs/MIGRATION.md before changing storage durability, recovery, import, export, or migration.
- Read release-notes/README.md before preparing a user-facing, security, operational, or user-facing documentation change.

## TorrentNG product boundaries

- TorrentNG has three distinct arrangements: TorrentNG Desktop, the torrentngd Engine with WebUI, and the compatible-client Bridge. Preserve each service's ownership of transfers, files, and configuration.
- In a compatible-client deployment, rTorrent SCGI is a trusted integration interface. Browser and automation access must go through TorrentNG's authenticated WebUI/API; do not expose SCGI directly to browser clients.
- Preserve client-specific compatibility limits. A shared UI or API field does not prove that every client supports the same operation; update the relevant capability matrix and tests when support changes.
- Storage, resume state, crash recovery, and migration are data-integrity boundaries. Keep migration report-first and require the documented apply step. Do not run imports, deletes, moves, or migrations against a live client or user data during tests.
- Use temporary state directories, local fixtures, and disposable containers for verification. A fixture or simulated peer test does not establish swarm, tracker, or multi-client interoperability.
- Keep the documented license terms and product identity clear. TorrentNG is not affiliated with the client projects whose compatibility APIs it implements.

## Validation and evidence

- Follow CONTRIBUTING.md and docs/development/validation-engine.md. The repository entry points are python3 scripts/validate_development.py --plan, --tests-only, and the full run.
- A plan is discovery only. Report executed tasks, actual native test counts, skips, exclusions, and missing prerequisites separately.
- Run the relevant focused checks while editing. Use the full validation engine for a complete gate request, not a hand-picked subset presented as complete.
- Keep external trackers, peers, and user accounts out of local checks. Use the at-home validation plan only for its explicitly controlled hardware and interoperability evidence.
- Report UI visual inspection separately from automated checks; source checks do not prove the UI works at desktop or narrow widths.

## Release notes

Changes to user-facing behavior, security, operations, or user-facing documentation need a new fragment under release-notes/. Keep fragments append-only and preview them using release-notes/README.md. For an internal-only change, include the exact marker release-note: none on a separate line in the PR description.
