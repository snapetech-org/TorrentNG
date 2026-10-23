# TorrentNG

TorrentNG is a torrent manager for people who seed a lot. It ships as a native
desktop app, a headless server with a browser interface, and a bridge that puts
the same interface in front of a client you already run. All three use one
BitTorrent engine and one API.

It runs on your computer or server. There is no hosted TorrentNG account. The
files stay on the storage you configure.

![TorrentNG showing Debian, Ubuntu, and Fedora Linux downloads](docs/assets/torrentng-default-dark-linux-isos.png)

For local MCP access, see the [SnapeTech MCP integration guide](docs/integrations/snapetech-mcp.md).
## Is it useful for you?

TorrentNG may help if you manage torrents on a home server, NAS, or other
self-hosted machine and want to control them from one browser page. It can also
provide familiar automation APIs for supported clients.

If you already have a client and are happy with its interface, TorrentNG adds
something new to set up and maintain. If you just want a torrent app on your
own computer, install **TorrentNG Desktop**: it bundles the engine and needs no
server, Docker, or configuration.

## Choose what to install

TorrentNG has three parts. You normally need only one of them.

| You want to... | Install | What it is |
|---|---|---|
| Run torrents on this computer, with a normal app | **TorrentNG Desktop** | A native app (Linux, macOS, Windows) with the engine included. Low memory and CPU; no browser or server. It can also connect to a TorrentNG Server. |
| Run torrents on a home server, NAS, or seedbox and use it from a browser | **TorrentNG Server** (`torrentngd` + WebUI) | The headless engine with the browser interface and APIs. Available as Docker images, release binaries, and packages. |
| Keep qBittorrent, rTorrent, Transmission, or Deluge and add the TorrentNG interface | **TorrentNG WebUI Bridge** (`torrentng`) | A web interface and API for the client you already run. It replaces ruTorrent or the client's own WebUI and adds automation APIs, while your client keeps its files and runs transfers. |

Downloads are on the [releases page](https://github.com/snapetech-org/TorrentNG/releases):
`torrentng-desktop-*` for the Desktop app, `torrentngd-native-*` for the Server.
See the [Desktop guide](docs/DESKTOP.md) for install steps and what each file is.
The Server quick start is below. To connect an existing client, start with the
[WebUI Bridge deployment guide](docs/DEPLOYMENT.md). The parts use different services
and configuration.

## Try TorrentNG Server

You need Docker with Docker Compose v2 and OpenSSL. This starts the Engine on
your own machine and keeps its state and downloads in Docker volumes.

From the repository directory, create a private token file and start TorrentNG:

```sh
umask 077
printf 'TORRENTNG_API_TOKEN=%s\n' "$(openssl rand -hex 32)" > .env
docker compose --env-file .env -f deploy/native/compose.yml up --build
```

Open [http://localhost:28082](http://localhost:28082). At the login screen,
enter the value after `TORRENTNG_API_TOKEN=` from `.env` in either field. Keep
`.env` private; the same token is needed if you recreate the container. The
file is ignored by Git.

The WebUI is available only from this machine by default. Port `44444` is open
for BitTorrent peers. To let another person reach the WebUI, set up an
authenticated HTTPS reverse proxy first; see the
[Engine deployment guide](docs/NATIVE_DEPLOYMENT.md).

To stop the service, press Ctrl+C. The named volumes keep your downloads and
settings. **Do not add `-v` to `docker compose down` unless you intend to delete
those volumes.**

## Before using a real library

TorrentNG is pre-1.0. Settings and APIs may change. Back up your client state
and downloaded files before importing, moving, or deleting data. Migration
starts with a report; review it before using `--apply`. Some imported progress
may need a full data check before another client can resume it.

The dated [Debian ISO comparison and native memory investigation](certification/reports/native-memory-optimization-20261014-agentvm.md)
records public download, controlled seeding, and anonymous memory against
qBittorrent, Transmission, Deluge, and rTorrent. TorrentNG averaged 0.49 MiB/s
on its one public-swarm pass, slower than the completed peers at 5.60–16.82
MiB/s; in the controlled seed test it averaged 3.56 MiB/s, close to their
3.61–3.74 MiB/s. Its measured anonymous peaks were 8.3 MiB in public download
and 7.4 MiB while seeding. That was about 2.1–3.2 times less than qBittorrent
and Transmission, and 11–12 times less than Deluge. The peer ratios combine
sequential runs, with Transmission and Deluge measured in a later rerun; they
are not a general ranking. A separate system-allocator run used less memory
than the previous jemalloc run (18.8 MiB public / 25.3 MiB seed), while seed
rates were 3.56 vs 3.66 MiB/s. This allocator comparison is not a concurrent
A/B. A mimalloc follow-up used more anonymous memory (19.8 MiB public / 23.6
MiB seed) and did not complete the public download within 30 minutes. A
qBittorrent check right afterward completed in 39.6 seconds at 19.08 MiB/s,
which flags a current TorrentNG download gap; sequential testing leaves the
precise cause unresolved. The results do not show orders-of-magnitude savings
against every peer; rTorrent did not start, and public swarm speeds varied
sharply. The earlier
[unoptimized baseline](certification/reports/client-performance-benchmark-20261013.md)
is retained for reference. Neither report establishes a maximum torrent count.
See the dated [release evidence](docs/RELEASE_EVIDENCE.md) for what was
exercised and where.

A corrected comparison on clean main commit `cfb63f91` completed both
controlled download and seed transfers for TorrentNG, qBittorrent, Transmission,
Deluge, and rTorrent, with SHA-256 verification. In this one-pass baseline,
TorrentNG's download used 6.504 CPU-s/GiB and 6.3 MiB anonymous memory, versus
8.218 CPU-s/GiB and 16.6 MiB for qBittorrent; TorrentNG transferred more slowly
(3.07 versus 3.83 MiB/s). On the seed workload, TorrentNG used 1.33 CPU-s/GiB
and 16.6 MiB anonymous memory: lower CPU than qBittorrent and Deluge, higher
than Transmission and rTorrent, and higher memory than qBittorrent and
rTorrent. These are single-pass, shared-host observations, not a client
ranking. The [corrected comparison and raw evidence](certification/reports/TorrentNG-native-download-efa3a607/comparison/round2-corrected-cfb63f91/README.md)
is the pre-change baseline for the seed-upload optimization.

An exploratory Linux TCP file-range seed candidate recorded 4.3 MiB sampled
anonymous peak versus 16.6 MiB in the clean-source baseline (74% lower), while
measured CPU was 5.51 versus 1.33 CPU-s/GiB (4.14 times higher). Transfer rate
stayed near 3.9 MiB/s under the shared 4 MiB/s receiver cap. The candidate was reverted
because the CPU cost missed our goal; the memory gain and CPU loss are both
preserved as design evidence for later work. These were single-pass observations
on a shared host, not a causal estimate or a client ranking. See the
[candidate results and raw evidence](certification/reports/TorrentNG-native-download-efa3a607/comparison/round3-sendfile-e9727943/README.md)
and the [three-round whole-client seed comparison](certification/reports/TorrentNG-whole-client-comparison-20261021/README.md).
The next review archive repeated the `sendfile` recommendation and added no new
patch. Its source and older benchmark snapshots are recorded in the
[archive assessment](certification/reports/TorrentNG-download-review-20261021/README.md).

On 2026-10-22, a private local screen against source-built rTorrent/libtorrent
v0.16.25 measured active-window CPU at 1.28 CPU-s/GiB for TorrentNG and 0.50
for rTorrent, with active goodput of 3.53 and 3.85 MiB/s. Anonymous seed-memory
peaks were 14.5 MiB and 3.4 MiB. rTorrent's first payload appeared about 63
seconds after its peer connected; the run did not record the peer's interest,
choke, or request state. This was one exploratory pair, not a ranking. See the
[current-source screen report](certification/reports/TorrentNG-current-rtorrent-screen-20261022.md).

A separate hot-file probe found that a single mapped view, read in 16 KiB
blocks, used 56% less CPU per GiB than a reused-buffer `pread` path across three
paired samples, but the absolute saving was 0.060 CPU-s/GiB. That is about 8%
of the 0.78 CPU-s/GiB gap in the separate screen, only a magnitude comparison.
The probe excludes scheduler, socket, and cold-file costs, so mapped reads
alone will not close the gap. A prototype is worth pursuing only if it can also
remove repeated scheduler or buffer-ownership work; the [method and raw
measurements](certification/reports/TorrentNG-seed-read-path-probe-20261022.md)
are recorded separately.

The final compact-default comparison reported three-run medians of 1.90 CPU
seconds and 19.82 MiB PSS for TorrentNG, versus 0.73 seconds and 10.74 MiB for
rTorrent. PSS improved 18.3% against TorrentNG's previous best configuration;
CPU did not improve, and remains about 2.60× rTorrent's. The final summary and
its evidence limits are in the [CPU/RAM results note](certification/reports/TorrentNG-CPU-RAM-final-summary-20261022.md),
with the [full report and verified run summaries](certification/reports/TorrentNG-native-cpu-ram-20261022.md).
Follow-up matched local screens found a release-default improvement: increasing
the upload-read coalescing window from 8 ms to 16 ms reduced median CPU by
10.6% and also reduced PSS and anonymous memory on the 128 MiB release fixture.
The paired 64 MiB release screen moved in the same direction. The matching PGO
128 MiB screen reduced median CPU by 2.6%, PSS by 2.8%, and anonymous memory by
5.5%; its 64 MiB screen had a 5.5% anonymous-memory increase, which is recorded
alongside the gains. The 16 ms setting was retained at that stage; the exact 8 ms baseline, 16 ms
candidate, raw summaries, logs, and diagnostics remain in the [seed performance
experiments report](certification/reports/TorrentNG-seed-performance-experiments-20261022/README.md).
It was later superseded by the measured 32 ms/eight-block default described below.

A further measured source improvement is now retained on main as commit `39554462`: upload reads collect for up to 32 ms and combine up to eight contiguous 16 KiB blocks. In three randomized 64 MiB release rounds, CPU fell 22.2% (4.504 to 3.506 CPU-s/GiB), PSS fell 1.5% (18.875 to 18.586 MiB), and anonymous peak fell 7.5% (6.500 to 6.012 MiB), with every transfer SHA-256 verified. Fresh PGO screens improved on matched PGO controls at 64 MiB (3.016 vs 3.481 CPU-s/GiB; 15.791 vs 16.037 MiB PSS) and 128 MiB (3.348 vs 4.019; 15.593 vs 16.184 MiB PSS). PGO remains opt-in, and at 128 MiB the candidate still used about 1.42× rTorrent CPU and 2.05× its PSS. Sixteen-block experiments did not improve CPU and PSS together; all failed and losing patches and measurements are preserved. The detailed method, matched results, optional allocator tradeoff, and exact artifacts are in the [seed experiment report](certification/reports/TorrentNG-seed-performance-experiments-20261022/README.md#current-main-improvement-bounded-eight-block-reads) and [evidence directory](certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/seed-goal-next-avenues-20261022/README.md).

The earlier matched 128 MiB comparison left the 16 ms PGO candidate at about
1.43× rTorrent 0.16.25 CPU, 2.05× its PSS, and 2.04× its anonymous memory. The
newer 32 ms/eight-block PGO screen reports 1.42× rTorrent CPU and 2.05× its PSS;
its 15.59 MiB PSS is the lowest recorded 128 MiB PGO median in these local
screens. Earlier full-profile PGO evidence remains archived for comparison.
PGO is still opt-in. The separate io_uring screen confirmed the requested
backend but measured about 13% more CPU than `pread` for roughly 2.5% lower PSS;
that tradeoff and the worker-startup correction are documented in the same
report. Fresh PGO profiles trained from the current source lowered CPU in the
transfer-window screen, but raised PSS by 11.2%; full-process training also
lost in its one-pair screen. Both candidates and the exact evidence are
preserved without changing the release or PGO defaults.
The follow-up upload-rate admission batching refactor also failed to beat the
16 ms release on CPU and PSS together in two randomized 64 MiB screens. Its
patch and verified measurements are retained in the same report for later
profiling.
Six one-round 64 MiB screens tested jemalloc, allocator refinements,
`target-cpu=native`, Rust 1.97, and `opt-level=3`. None improved CPU and PSS
together, so the current system allocator, portable target, and compact
release profile remain in place. The exact builds, verified summaries, and
diagnostic profile are in the
[low-risk tuning screen](certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/low-risk-tuning-screen-20261022/README.md).
The separate [mapped-window probe](certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/mapped-window-read-probe-20261022/README.md)
found an 8.6% read-stage CPU reduction against batched `pread`, but no whole-client result.
I also rebuilt PGO with a profile trained for `target-cpu=native` to avoid the
control-flow mismatch in the earlier portable-profile attempt. In one matched
64 MiB screen, it used 7.6% more CPU and 11.3% more PSS than the retained PGO
control, though anonymous memory was 5.2% lower. All transfers verified, but
the host was busy and this one-round result is only screening evidence. The
candidate was not promoted; its exact profile, binaries, and run data are
preserved in the [native-target PGO screen](certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/pgo-native-target-screen-20261022/README.md).
An equal-weight blend of the current-source full-process and transfer-window
profiles looked promising in one pass, then lost its three-round screen: CPU
was 5.9% higher and PSS 9.3% higher than the PGO control, though anonymous
memory was 5.2% lower. The initial and repeated results, binary, and merged
profile are preserved in the [PGO blend screen](certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/pgo-profile-blend-20261022/README.md).
Fat-LTO PGO made the executable smaller but raised CPU by 7.7% and PSS by
15.0% versus the PGO control in its verified screen. A three-round
`MALLOC_ARENA_MAX=1` run lowered PSS by 7.4% and anonymous memory by 19.5%,
with 5.4% more CPU; this remains an optional memory tradeoff, not a new default.
The one-round arena-cap-2 screen raised CPU by 42.3%. Exact results and
reproducible artifacts are in the
[runtime and codegen screen](certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/runtime-tuning-screen-20261022/README.md).
An exact-binary 256 MiB CPU profile showed broad peer/task polling among the
largest resolved TorrentNG frames, but 47.1% of its sample weight mapped to
libc without reliable symbol attribution. It is diagnostic-only and does not
support the peer-read map refactor as a large target. The run changed no source
or defaults; limits and the raw profile are in the
[hot-path profile note](certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/pgo-hotpath-profile-20261022/README.md).

## Learn more

- [Install TorrentNG Desktop](docs/DESKTOP.md)
- [Connect an existing client](docs/DEPLOYMENT.md)
- [Run the TorrentNG Engine](docs/NATIVE_DEPLOYMENT.md)
- [Install on Unraid](deploy/unraid/README.md)
- [Configuration](docs/CONFIGURATION.md)
- [OIDC sign-in setup and operations](docs/OIDC_AUTHENTICATION.md) · [shared provider catalog](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-provider-catalog.md)
- [Migration and export](docs/MIGRATION.md)
- [Client performance benchmark](docs/CLIENT_PERFORMANCE_BENCHMARK.md) · [dated comparison and memory investigation](certification/reports/native-memory-optimization-20261014-agentvm.md)
- [API and compatibility limits](docs/API.md) · [client matrices](docs/CLIENT_COMPATIBILITY_MATRICES.md)
- [Backup and restore](docs/BACKUP_RESTORE.md)
- [Documentation index](docs/README.md)
- [Localization roadmap](docs/localization-roadmap.md) — language coverage, regional number behavior, and remaining translation work

## Development

The source tree has separate workspaces for the built-in Engine and the
compatible-client service. The commands below are for contributors, not
required for the Docker quick start.

```sh
cargo build --release -p torrentngd
```

The default Server build includes the native and qBittorrent APIs. To keep
using Transmission or Deluge RPC against a direct `torrentngd` deployment,
build with `--features full-compat` (or the matching single API feature).
See [Configuration](docs/CONFIGURATION.md#compatibility-api-build-features)
for all build options and the API migration note.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development checks. The current
release qualification sequence and the hardware-dependent work to run later
are in the [at-home validation plan](docs/AT_HOME_VALIDATION.md).

## Support

Ask for setup help or report a problem on
[Discord](https://discord.gg/5PyXBfvS6T). For a security issue, follow
[SECURITY.md](SECURITY.md) and do not post it publicly.

If TorrentNG is useful to you, [support its development on Ko-fi](https://ko-fi.com/snapetech).

## Legal use

Users are responsible for the content they download, seed, or manage and for
complying with applicable laws and licenses.

## License

TorrentNG is available under `AGPL-3.0-or-later OR Commercial`. Unless you have
a separate signed commercial license, the AGPL applies. See [LICENSE](LICENSE)
and [COMMERCIAL-LICENSE.md](COMMERCIAL-LICENSE.md).

TorrentNG is not affiliated with or endorsed by qBittorrent, rTorrent,
Transmission, Deluge, or other projects named in this repository.

## Local security scanning

Run `./scripts/security-scan-local.sh` for local Semgrep CE and the Trivy scanners that fit this repository. The installed pre-commit hook runs Semgrep on staged source/config files only; Trivy runs on demand or in CI. This public repository also runs Semgrep CE in GitHub Actions on changes and weekly. See [security-scanning.md](docs/security-scanning.md). Use `./scripts/update-security-tools.sh` to refresh local scanner data and the Trivy database.
