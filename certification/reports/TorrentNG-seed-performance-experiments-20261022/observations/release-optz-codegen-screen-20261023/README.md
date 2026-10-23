# Release compiler-profile screen — 2026-10-23

**Decision: do not promote.** Smaller-code optimization (`opt-level=z`) reduced process PSS consistently, but CPU was worse in the order-balanced confirmation. Adding host-specific native code generation did not recover CPU parity. The production profile remains `opt-level=s`; candidate source/build outputs stayed out of `main`.

## Results

All trials used a 16 MiB private loopback seed, SHA-256 verified, at 4 MiB/s with 16 KiB blocks and a 32-request window. The same source revision and Rust 1.94 toolchain were used. Two three-pair screens covered `opt-level=z`; the initial screen ran z first each time and the confirmation ran current first each time, balancing order across the combined six pairs. The native-codegen refinement used three pairs with randomized current-first twice.

| Candidate | Paired median CPU change | Paired median PSS change | Paired median anonymous-memory change | Candidate wins | Binary size |
|---|---:|---:|---:|---:|---:|
| `opt-level=z` | +17.5% | -5.3% | -0.3% | CPU 2/6; PSS 6/6 | 25.65 MB (+4.0%) |
| `opt-level=z` + `target-cpu=native` | +4.5% | -4.7% | -2.2% | CPU 1/3; PSS 3/3 | 25.84 MB (+4.8%) |

For the balanced six-pair z comparison, CPU was lower in only two pairs, PSS was lower in all six, and anonymous memory was lower in three. Host CPU pressure reached 9.12% PSI avg10 after its confirmation. The native refinement lowered CPU in one of three pairs and increased the executable size further. These are not joint CPU-and-memory improvements and are not expanded to an rTorrent comparison.

## Build and evidence

- Current `opt-level=s` executable: 24,663,632 bytes, SHA-256 `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263`.
- `opt-level=z` candidate: 25,650,368 bytes, SHA-256 `220a6d5f1713a60b6e1109bb551d179c1d5871abdfd7c8070ffeb6d0670c3d27`.
- `opt-level=z` plus `-C target-cpu=native`: 25,835,640 bytes, SHA-256 `c2c75f7db5dc62ccd1f987f1f199e30889291bf7661b101440094c53ebde1823`.
- Both candidate builds succeeded from source revision `87c7e224` with Rust 1.94.0. The current executable was restored after each build.

Exact paired measurements, orders, binary identities, and fixture verification are in [`summary.json`](summary.json); captured host pressure is in [`host-load.txt`](host-load.txt); commands and validation are in [`validation.txt`](validation.txt).
