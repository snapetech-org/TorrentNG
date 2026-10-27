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

Before submitting changes, run the checks that cover the code you changed. The
standard repository commands use Cargo, Python's unittest runner, and npm
directly:

```sh
cargo fmt --all -- --check
python3 scripts/validate_openapi.py
python3 -m unittest \
  scripts.test_protected_target \
  scripts.test_backend_burndown_api_load \
  scripts.test_workflow_security \
  scripts.test_release_notes
cargo test --workspace --all-targets --locked -- --test-threads=1
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked -p torrentngd --bin torrentngd --features full-compat
cargo clippy --locked -p torrentngd --bin torrentngd --features full-compat -- -D warnings
cargo fmt --manifest-path sidecar/Cargo.toml -- --check
cargo test --locked --manifest-path sidecar/Cargo.toml -- --test-threads=1
cargo clippy --all-targets --locked --manifest-path sidecar/Cargo.toml -- -D warnings
cargo fmt --manifest-path crates/torrentng-desktop/Cargo.toml -- --check
cargo build --locked -p torrentngd
cargo clippy --all-targets --locked --manifest-path crates/torrentng-desktop/Cargo.toml -- -D warnings
cargo test --locked --manifest-path crates/torrentng-desktop/Cargo.toml -- --test-threads=1
(cd webui && npm ci && npm test && npx playwright install chromium && npm run build && npm run lint)
```

Install the repository hooks once with `git config core.hooksPath .githooks`.
The pre-push hook checks that stable Docker tag promotion still follows the
release policy; GitLab runs the same check for pushed branches and tags.
