
### bolt-default

| Build | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB |
| --- | ---: | ---: | ---: |
| pgo-relocs | 3.241 | 15.589 | 5.902 |
| pgo-bolt | 3.133 | 18.773 | 5.871 |
| rtorrent | 2.097 | 10.532 | 2.590 |

`pgo-relocs → pgo-bolt`: paired median CPU ratio 1.038; PSS ratio 1.204.


### layout-followup

| Build | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB |
| --- | ---: | ---: | ---: |
| pgo-bolt-4k | 3.234 | 17.375 | 6.223 |
| rtorrent | 2.466 | 10.697 | 2.590 |
| pgo-link-order | 3.689 | 13.445 | 6.906 |
| pgo-relocs | 3.744 | 16.230 | 6.543 |

`pgo-relocs → pgo-bolt-4k`: paired median CPU ratio 0.864; PSS ratio 1.071.

`pgo-relocs → pgo-link-order`: paired median CPU ratio 0.985; PSS ratio 0.818.


### link-order-qualification

| Build | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB |
| --- | ---: | ---: | ---: |
| pgo-relocs | 3.442 | 15.786 | 5.928 |
| rtorrent | 2.538 | 10.581 | 2.592 |
| pgo-link-order | 3.493 | 13.193 | 6.414 |

`pgo-relocs → pgo-link-order`: paired median CPU ratio 0.989; PSS ratio 0.836.

97.5% intervals: CPU [0.978, 1.092], PSS [0.792, 0.880]. Joint resource gate: `inconclusive_or_regression`.

`rtorrent → pgo-link-order`: paired median CPU ratio 1.312; PSS ratio 1.242.

97.5% intervals: CPU [1.212, 1.512], PSS [1.183, 1.298]. Joint resource gate: `inconclusive_or_regression`.
