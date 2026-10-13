# Development validation engine

TorrentNG's validation runner coordinates the existing project checks without
replacing Cargo, unittest, Vitest, or the certification scripts. Its inventory
tracks Rust unit and integration test sources, Python test modules, WebUI tests,
and the certification self-tests.

From the repository root:

```sh
python3 scripts/validate_development.py --plan
python3 scripts/validate_development.py --plan --json
python3 scripts/validate_development.py --tests-only
python3 scripts/validate_development.py --task workspace-tests
python3 scripts/validate_development.py
```

Local execution of test tasks requires the trusted Linux bubblewrap launcher.
When evaluating an unreviewed branch, copy `trusted-test-launcher.mjs` and
`trusted-test-sandbox-policy.mjs` from the same reviewed revision to a location
outside the checkout, keeping them together. The launcher snapshots only paths
in the Git index, hides the original checkout and common host configuration and
data roots, and runs candidate code offline. Dotted and extensionless credential
names, private/API keys, access/refresh-token names, client-secret names, and
credential material with source-like extensions are excluded. Source modules
under credential-named folders remain available unless their own names identify
credential material. Suffixed dotenv credentials such as tokens.env.local are
excluded, while example templates such as tokens.env.example remain available.
It opens each workspace, run-data, dependency, cache, and marker source before
masking host paths, then binds those pinned sources through inherited file
descriptors. Stage new candidate files first.

```sh
node /path/to/trusted-test-launcher.mjs "$PWD" -- python3 scripts/validate_development.py --tests-only
```

Run planning for an unreviewed candidate through that launcher too, because
planning imports its validation code. GitHub Actions remains responsible for
hosted validation.

The plan is read-only. Execution checks the plan against the current Git
candidate, runs each selected native command, records its output in a private
temporary evidence directory, and writes a JSON receipt there. Rust, Python,
and Vitest case totals are read from their native summaries after each command
finishes. During long commands, the runner prints a task-level status at start,
completion, and once per minute to stderr; task output is streamed there too.
These messages appear in a terminal or live GitHub Actions log while the run is
in progress. The final machine-readable summary is written to stdout.
The pump queue is bounded to 16 output chunks, and each private task log is
capped at 16 MiB with a visible truncation marker. The receipt hashes the bytes
actually persisted to that log; case parsing retains only the final 8 MiB.

WebUI tasks require `webui/node_modules` to exist. The runner does not install
dependencies or start hardware, network, live-transfer, soak, or release
certification jobs. Those environment-specific gates remain separate from the
deterministic local suite. The storage certification self-test verifies that
the release provenance gate rejects dirty checkouts, so it is opt-in locally
with `--task storage-certification-selftest`; CI selects it explicitly from a
clean checkout.

Child commands inherit only the executable path, platform process variables,
locale, timezone, and the Rust toolchain location. Each task gets a run-owned
home, temporary, XDG, and Python cache directory; Cargo uses a private cache for
the run. Task-declared settings remain explicit, `CI` is forced to `true`, and
Python user-site packages are disabled. Ambient credentials, proxy settings,
loader options, and user configuration paths do not reach child processes.
