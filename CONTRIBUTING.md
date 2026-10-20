# Contributing

Contributions should preserve the project's security model: rTorrent SCGI is a
trusted integration interface only, and all browser or automation access must
go through TorrentNG's WebUI/API surface. The compatible-client service is the
only direct SCGI client in rTorrent deployments.

By contributing, you agree that your contribution is provided under the project
license: `AGPL-3.0-or-later OR Commercial`.

Do not contribute code or assets that you do not have the right to license.
Do not add functionality whose primary purpose is piracy, copyright
infringement, or evading lawful access controls.

Before submitting changes, run the relevant checks:

```sh
python3 scripts/validate_development.py --plan
python3 scripts/validate_development.py --tests-only
python3 scripts/validate_development.py
```

The validation runner and its task-selection options are documented in
[`docs/development/validation-engine.md`](docs/development/validation-engine.md).

Install the repository hooks once with `git config core.hooksPath .githooks`.
The pre-push hook checks that stable Docker tag promotion still follows the
release policy; GitLab runs the same check for pushed branches and tags.

## Pull requests and merges

Use `main` as the target. Open a pull request only when current GitHub rules require one; push directly
when they allow it. Keep a pull request current with `main` only when those rules require it. Merge
after the checks and approvals required by current rules are satisfied. An explicit maintainer's
direction to merge is sufficient; no additional review or approval is required beyond those rules.
Use any merge method enabled in GitHub.
