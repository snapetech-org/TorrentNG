# Runtime and codegen tuning screen — 2026-10-22

These screens tested whether fat LTO or limiting glibc arenas could improve the
current opt-in PGO seed build. They did not replace the retained PGO winner or
change release/build defaults. Every listed transfer completed and matched the
64 MiB fixture SHA-256
`b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

## Fat-LTO PGO

One randomized-order round compared a portable-target, system-allocator PGO
build with `CARGO_PROFILE_RELEASE_LTO=fat` against the existing PGO control,
transfer-window thin PGO, and rTorrent 0.16.25. All used `pread`; the local
receiver was paced at 4 MiB/s with a 16 KiB block and 32-block request window.

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Fat-LTO PGO | 3.8804 | 18.96 MiB | 6.66 MiB |
| PGO control | **3.6020** | **16.50 MiB** | 6.51 MiB |
| Transfer-window PGO | 3.8511 | 18.84 MiB | 6.39 MiB |
| rTorrent 0.16.25 | 2.3329 | 7.59 MiB | 3.05 MiB |

Fat LTO used 7.7% more CPU and 15.0% more PSS than the control. Its compressed
executable is preserved for repeatability; the smaller on-disk binary did not
translate into lower resident memory. This was a single-round screen on a busy
host and is not a promotion candidate.

## glibc arena caps

The arena tests used the identical PGO-control binary
`8e8522729bdc06b8590995945e87544741370cb66c70eb13576d54f8e378f006`; only
`MALLOC_ARENA_MAX` differed. For `MALLOC_ARENA_MAX=1`, the one-round result was
3.6% more CPU for 9.3% lower PSS and 27.5% lower anonymous memory. A three-round
follow-up retained the memory reduction but had 5.4% higher median CPU:

| Setting | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---:|---:|---:|
| Default allocator settings | **3.9013** | 15.78 MiB | 6.02 MiB |
| `MALLOC_ARENA_MAX=1` | 4.1102 (+5.4%) | **14.61 MiB (-7.4%)** | **4.84 MiB (-19.5%)** |
| rTorrent 0.16.25 | 2.1834 | 7.69 MiB | 3.05 MiB |

The paired arena-cap CPU deltas were +5.4%, -18.7%, and +28.7%; the screen was
host-noisy and the arena-cap client ran first in every round. PSS and anonymous
memory were lower in all three paired rounds. This remains a documented,
memory-focused runtime option with a CPU tradeoff; it is not a default and does
not meet the CPU goal.

`MALLOC_ARENA_MAX=2` had only one round: 4.8688 CPU-s/GiB, 15.58 MiB PSS, and
5.62 MiB anonymous, versus 3.4215, 16.22 MiB, and 6.41 MiB for the same binary
without the cap. Its CPU was 42.3% higher for a 4.0% PSS reduction, so it was
not repeated.

## Scope and retained evidence

All tests used the local verified seed fixture and identical daemon binary for
the allocator comparisons. The busy host and short rounds make these screening
results, not release qualification. The archive retains summaries, client
logs, metrics, compressed `smaps`, fixture metainfo, and the compressed fat-LTO
executable with its uncompressed SHA-256. Generated payloads and temporary
credentials/databases are excluded. No production source, release default, or
PGO default changed.

The lesson for follow-up work is specific: arena limits can trade CPU for
anonymous memory, while fat LTO only reduced executable file size. Keep the
existing measured configuration unless a later matched screen improves the
resource goal without regressing another required metric.
