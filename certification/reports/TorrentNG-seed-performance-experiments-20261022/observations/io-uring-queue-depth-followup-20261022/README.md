# io_uring queue-depth follow-up

This follow-up revisits io_uring after the first 64-entry queue screen showed a
CPU regression. It uses the same release daemon for both backends and changes
only `TNG_STORAGE_BACKEND` and the bounded storage queue depth. The executable
SHA-256 is `63e316c8727c00e3c3356f48076ee7afeb4654b6050435f1c037c1c30247df99`.
The 16 MiB fixture SHA-256 is
`132646868d8a9abf88d5fe16244ec101009f6b76613437514dbd41769d3943d3`; the
64 MiB fixture SHA-256 is
`b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.
Transfers were limited to 4 MiB/s, used one local peer, and disabled DHT, PEX,
and trackers. Every scored transfer passed SHA-256 verification. The runtime
backend metric confirmed the requested backend in every scored run.

| Fixture | Queue depth | Backend | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---|---:|---:|---:|
| 16 MiB, one pair | 4 | `pread` | 6.868 | 18.47 MiB | 5.87 MiB |
| 16 MiB, one pair | 4 | io_uring | 6.524 (-5.0%) | 17.79 MiB (-3.7%) | 5.21 MiB (-11.2%) |
| 64 MiB, one pair | 4 | `pread` | 8.439 | 19.14 MiB | 6.78 MiB |
| 64 MiB, one pair | 4 | io_uring | 7.044 (-16.5%) | 17.89 MiB (-6.5%) | 5.34 MiB (-21.2%) |
| 64 MiB, three pairs, median | 4 | `pread` | **5.628** | 18.51 MiB | 5.96 MiB |
| 64 MiB, three pairs, median | 4 | io_uring | 6.040 (+7.3%) | **18.50 MiB (-0.04%)** | 6.11 MiB (+2.5%) |
| 16 MiB, one pair | 8 | `pread` | 5.242 | 18.58 MiB | 5.86 MiB |
| 16 MiB, one pair | 8 | io_uring | 5.847 (+11.5%) | 18.11 MiB (-2.5%) | 5.53 MiB (-5.6%) |

The depth-4 one-pair results initially looked promising, especially at 64 MiB.
The randomized three-pair repeat did not reproduce that CPU gain: io_uring used
more CPU in all three rounds (+18.4%, +12.1%, and +6.3% paired). Its median PSS
was effectively tied, and its median anonymous peak was 2.5% higher. The
single-pair results remain in the record as mixed early evidence; the repeat
determines the current decision.

The depth-1 attempt is invalid as a performance comparison. Its `pread`
control saturated the scheduler and logged repeated `scheduler queue full`
errors before producing a complete transfer. The probe was stopped, and the
failed summary and log are retained. It does not establish an io_uring result
at depth 1.

**Decision:** keep the existing io_uring startup correction and explicit
backend selectable, but keep `pread` as the default and do not change the
release queue depth. The depth-4 repeat failed the CPU screen, while depth 8
also used more CPU in its small screen. No production source or configuration
was changed by these experiments. The established 16 ms upload coalescing and
current PGO gains remain intact. Revisit queue sizing only with a workload or
mechanism that addresses why the one-pair gains did not repeat.

Raw probe summaries, run logs, redacted per-run configurations, fixture
metainfo, backend gauges, run order, binary identity, and verified transfer
hashes are retained beside this report. Generated payloads, per-run databases,
API tokens, and other temporary session files are excluded.
