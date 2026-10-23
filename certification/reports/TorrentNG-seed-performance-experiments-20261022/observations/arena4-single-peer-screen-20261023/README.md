# glibc arena cap 4 screen — 2026-10-23

**Decision: do not promote or expand.** The candidate used more CPU in all three paired 64 MiB transfers. PSS and anonymous-memory effects were mixed by pair, despite slightly lower unpaired medians.

| Treatment | Median CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Current allocator default | 5.522 | 18.625 MiB | 6.004 MiB |
| `MALLOC_ARENA_MAX=4` | 5.761 (+9.7% paired median) | 18.360 MiB (+1.0% paired median) | 5.855 MiB (+4.4% paired median) |

The candidate ran first in all three pairs. CPU pressure at completion was 5.33% avg10 / 6.43% avg60 / 6.09% avg300; the screen is preliminary. All six transfers verified the same fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. Since arena cap 2 already has a clearer memory-only signal and arena 4 did not improve CPU, no extra confirmation was spent here. The existing allocator default remains in effect.

The same release binary was used for both arms (SHA-256 `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263`), so the only intended treatment difference was the environment override. Full sanitized paired samples and method settings are in [`summary.json`](summary.json). The original raw probe output remains task scratch and is not copied into the repository.
