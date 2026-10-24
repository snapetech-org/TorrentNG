# Upload-read coalescing candidate — screened out before transfer

- Status: **not retained or scored; no transfer was started**.
- Started (UTC): 2026-10-24 04:05:21.
- Source commit: `5a030f7f543f964db219a8f8bf4451f71bb6e689` (`main`).
- Source tree fingerprint: `9a0b7124f5e3f7ad4d7f26946f094d13b4388cf42b77e1ef5f9820986019bbc2`.
- Candidate settings applied only to TorrentNG: minimum upload-read batch 2 requests,
  maximum collection delay 8 ms.
- Fixture: 134,217,728 bytes; SHA-256
  `789a368ff6d8b3871c733f709d721867dbb932b0297a2c173e654e980aa84399`.
- Host: `kspld0`, 32 logical CPUs. Load average at start was
  `30.88 / 37.43 / 39.67` (1/5/15 minutes).
- The release `torrentngd` image build was interrupted after about 10 minutes while
  `rustc` was linking the daemon. No application image was produced and no swarm
  transfer or performance measurement occurred.
- The controlled-seed benchmark uses an internal Docker network and a local
  SHA-256-pinned fixture; no public swarm was contacted.
- Partial [sample CSV](TorrentNG-upload-coalesce-candidate-20261023-samples.csv)
  contains only its header. The
  [memory profile](TorrentNG-upload-coalesce-candidate-20261023-memory-profile.json)
  contains no transfer samples.
- Cleanup: isolated Compose project, containers, and volumes removed; the
  task-scoped BuildKit builder removed; persistent TorrentNG BuildKit cache retained.
  Docker confirmed the interrupted build had produced no candidate image. The
  task scratch directory was removed and verified absent.

This run provides no CPU, RAM, throughput, or storage-counter evidence for the
candidate. Its peer-loop regression test separately verified two contiguous
requests enter one upload read batch and a solitary request is flushed after the
configured deadline. That establishes functional behavior only.

The policy was not retained after checking the separate current-main batch
distribution diagnostic: 127 of 129 observed storage batches already contained
eight blocks, with one initial singleton and one seven-block tail. That makes
adding a collection delay a weak CPU hypothesis for this traffic pattern. The
distribution run is also diagnostic-only and does not establish other peers'
request patterns. The exact unpromoted implementation is preserved in the
[candidate patch](TorrentNG-upload-coalesce-candidate-20261023.patch), SHA-256
`c14661bd8cce409c525b68d4464e11a19144cf89886c2a04d92cf6725ee3c416`.

The build did not complete before interruption, so there is no whole-client
result. This candidate should be reconsidered only if measurements show a
meaningful singleton-batch rate under representative traffic.
