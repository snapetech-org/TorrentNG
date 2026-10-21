# Corrected native client comparison baseline

This is one controlled download pass and one controlled seed pass from the
same clean TorrentNG source, commit `cfb63f91b79ead5ce46a3ffc32a79b6250df0a53`
(fingerprint `d17321068f89eba7b22371fa85a30ba373784813948a5a55e1d4380fd81724dd`).
All five clients completed the same 792,723,456-byte Debian 13.7 netinst
fixture and the receiver payload matched SHA-256
`a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355`.

The clean source and private-tracker corrections make these valid matched
observations. They remain **one pass per client**, on a shared host; they do
not establish an overall ranking. The download client order was
qBittorrent, TorrentNG, rTorrent, Deluge, Transmission. The seed order was
TorrentNG, qBittorrent, Transmission, Deluge, rTorrent. Both workloads used a
fixed qBittorrent 5.2.3 counterpart and a 4 MiB/s counterpart rate cap. The
reports include full image digests, host load, configuration, and method.

## Controlled download

| Client | Average MiB/s | CPU s/GiB ↓ | Peak cgroup anonymous MiB ↓ | Verified |
|---|---:|---:|---:|---|
| TorrentNG | 3.07 | 6.504 | 6.3 | SHA-256 |
| qBittorrent | 3.83 | 8.218 | 16.6 | SHA-256 |
| Transmission | 3.72 | 1.498 | 7.9 | SHA-256 |
| Deluge | 3.89 | 6.940 | 84.8 | SHA-256 |
| rTorrent | 3.89 | 1.875 | 1.1 | SHA-256 |

TorrentNG used less CPU per GiB and less anonymous memory than qBittorrent and
Deluge in this pass. It transferred more slowly than every peer. Transmission
and rTorrent used less CPU, and rTorrent used less anonymous memory. This is a
current download signal to investigate, not evidence of general superiority.

## Controlled seed

| Client | Average upload MiB/s | CPU s/GiB ↓ | Peak cgroup anonymous MiB ↓ | Verified |
|---|---:|---:|---:|---|
| TorrentNG | 3.88 | 1.33 | 16.6 | SHA-256 |
| qBittorrent | 3.88 | 2.28 | 6.2 | SHA-256 |
| Transmission | 3.77 | 1.17 | 18.5 | SHA-256 |
| Deluge | 3.88 | 2.37 | 82.7 | SHA-256 |
| rTorrent | 3.89 | 0.55 | 1.0 | SHA-256 |

TorrentNG used less CPU per GiB than qBittorrent and Deluge, but more than
Transmission and rTorrent. Its anonymous peak was lower than Transmission and
Deluge, and higher than qBittorrent and rTorrent. The seed transfer rate was
near the common 4 MiB/s receiver ceiling for all clients, so it is not a
maximum-throughput test.

The measured seed CPU agrees with the earlier three-round report's TorrentNG
median (1.33 CPU-s/GiB), while this pass records 16.6 MiB anonymous memory.
The rTorrent source advice targets this seed path; the next comparison should
hold this source result as the before measurement and use a different client
order.

## Artifacts

- [Controlled-download report](controlled-download-corrected-round1b.md), [time series](controlled-download-corrected-round1b-samples.csv), [memory profile](controlled-download-corrected-round1b-memory-profile.json), and [runner output](controlled-download-corrected-round1b-console.log)
- [Controlled-seed report](controlled-seed-corrected-round1b.md), [time series](controlled-seed-corrected-round1b-samples.csv), and [memory profile](controlled-seed-corrected-round1b-memory-profile.json)

The seed runner output was not redirected to a file; the report, time series,
and memory profile are preserved. `SHA256SUMS` covers the copied source reports
and raw artifacts.
