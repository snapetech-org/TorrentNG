# Explicit single-worker Tokio runtime experiment

- Source worktree: `<bench>/runtime-experiment-source`
- Source parent: `a8b0c42cdb30a2a4fda875f654e1a0217969033c`
- Changed source: `crates/torrentngd/src/main.rs`
- Final source diff: 1 file, 110 insertions, 13 deletions
- Final diff SHA-256: `375a474eefb810adfb368e98d56e358a354d95788ba958b0b1f9fb3d709204b9`
- Patch file: `runtime-current-thread-source.patch`

## Build

- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`
- Release profile: Cargo.toml normal release profile (`opt-level=3`, `codegen-units=1`, `lto=thin`, system allocator)
- Command: `CARGO_TARGET_DIR=<target-dir> cargo +1.97.0 build --locked --release --bin torrentngd -j4`
- Result: success; `Finished release profile [optimized] target(s) in 4m 25s`
- Build log: `runtime-experiment-source-build.log`
- Binary: `bin/torrentngd-current-thin197`
- Binary SHA-256: `b4b7d2ab72b360d97cadd9b0a6b1a222d0bfa750853bd73becd7373a27890b96`

## Tests

- Command: `CARGO_TARGET_DIR=<target-dir> cargo +1.97.0 test --locked --release -p torrentngd --bin torrentngd -j4`
- Result: 33 passed, 0 failed, 1 ignored
- Coverage includes explicit override selection, runtime flavor assertion, spawned async task, timer, loopback TCP round trip, and `spawn_blocking`.
- Test log: `runtime-experiment-source-tests.log`
- Two intermediate test-only failures were fixed before the passing run: a redundant `.unwrap()` after timeout `.expect()`, then constructing `tokio::time::timeout` before entering the runtime context. The passing test constructs timeout inside the `block_on` async context.
- The release binary was built before those final test-only edits. Production runtime code did not change after that build; intervening source edits were confined to the test hunk.

## Runtime selection

`TOKIO_WORKER_THREADS=1` selects `Builder::new_current_thread()`. Auto-detected one-worker configurations and all other explicit values preserve the existing multi-thread runtime behavior. No benchmarks were run as part of this source/test gate.

## Source-only safety review

- No production use of `block_in_place`, `Handle::block_on`, or Tokio blocking channel APIs was found in the daemon/engine paths reviewed.
- `spawn_blocking` is used for database/filesystem/CPU work and remains available on current-thread runtimes. The DB worker and storage housekeeping also use dedicated OS threads; their sleeps do not block Tokio's current thread.
- The root daemon future is driven with `Runtime::block_on`; spawned engine, signal, and HTTP tasks are therefore driven while that future yields. Shutdown awaits engine shutdown and the signal task before returning.
- The semantic risk is reduced concurrency: all Tokio async tasks and the root future are polled on the main thread. A long synchronous section in an async poll can stall network, timers, and shutdown work. Most identified expensive DB/filesystem work is offloaded, but the source review does not prove every callback is nonblocking. The explicit environment override keeps this behavior opt-in pending benchmark results.

## Suggested configuration documentation text (not applied)

`TOKIO_WORKER_THREADS` overrides the daemon's Tokio worker count. Setting it explicitly to `1` selects Tokio's current-thread runtime: the main thread drives asynchronous work, while `spawn_blocking` retains its separate blocking pool. When the setting is omitted, automatic worker selection keeps the existing multi-thread runtime, including on hosts that resolve to one worker. Values above one continue to use the multi-thread runtime.
