# Combined worker and arena screen — interrupted, 2026-10-23

**No performance result.** The proposed combination was one storage I/O worker plus `MALLOC_ARENA_MAX=1`, compared with one worker alone. CPU PSI had risen to 11.70% avg10 and 12.47% avg60 at launch. The run was interrupted during its first paced transfer; the probe marked it failed with zero completed/verified runs. No CPU or memory value from this attempt is scored, and no conclusion about the combination is drawn.

The partial probe output and logs are retained at `/tmp/torrentng-seed-goal-20261022/worker1-arena1-combo-screen-20261023` as failure evidence. No production source or defaults were changed. Retry only in a quieter window if the combination remains useful after testing against the current best PGO build.
