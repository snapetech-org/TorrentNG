# Swarm Switch

Swarm Switch is a reversible public recovery session for a private torrent in
TorrentNG's native engine. Use it to finish a slow download, repair missing pieces,
or keep seeding when a private tracker has retired a torrent. It is available in
the WebUI detail panel and Properties → Swarm.

## User workflow

1. Open **Swarm Switch** on a private torrent. Choose a generated counterpart or
   provide a known public `.torrent`.
2. Select **Compare swarms**. Review the exact original and candidate hashes,
   tracker reports, DHT observations, timestamps and reachable-peer sample.
3. Select **Use public swarm**. TorrentNG parks the private torrent, creates a
   linked public torrent at the same location, and checks the existing data.
   The public identity downloads missing pieces or seeds complete data.
4. Refresh **Compare swarms**, then select **Restore private swarm** from either linked torrent. TorrentNG stops
   public transfers, removes the public session metadata, verifies the files
   against the retained original, then resumes private downloading/seeding.
5. Alternatively, **Delete retained private copy** keeps the public torrent and
   payload files but permanently removes this session's restore action.

Both entries remain visible while linked: the original is parked and the public
entry shows actual transfer progress and its own upload/download counters. This
keeps automation that addresses the original infohash honest; starting the parked
original is rejected with an instruction to use Restore private swarm. Standard
file-preserving deletion of the original through any compatibility API also
makes the public torrent independent. Deleting shared payload data, deleting the
public entry, or moving/renaming shared files requires restoring first.

A torrent that was originally added as public has no private copy and no restore
action. TorrentNG does not invent a private identity, tracker URL, passkey, ratio,
or download history. Re-adding an original `.torrent` is a separate user action.

## Identity and compatibility

The infohash covers the bencoded **info** dictionary. The `private` flag is inside
that dictionary. Changing it creates a different swarm, even when the files and
piece hashes are identical. Turning on DHT for the original private hash would
not find an independently packaged public torrent and would mix discovery
policies. Swarm Switch never does that.

Generated candidates preserve the content layout and offer four explicit forms:

| Candidate | Info changes |
| --- | --- |
| Remove private flag | Remove `private` |
| Set private flag to zero | Replace `private=1` with `private=0` |
| Remove private flag and tracker source | Remove `private` and `source` |
| Zero private flag and remove tracker source | Set `private=0`, remove `source` |

There is no universally correct public hash. The actual public release might use
other names, file ordering, piece sizes, padding, or publisher fields. The
comparison probes the selected exact candidate; it does not promise content
search or join unrelated hashes. No peers found is a useful observation, not a
proof that no compatible public release exists. A user can choose to seed an
empty public counterpart intentionally.

For an uploaded public torrent, the exact info bytes are retained. All candidates
are packaged with only `info` and authenticated BEP 52 `piece layers`: private
tracker URLs, webseed URLs, DHT node lists, comments and other outer metadata are
excluded. Other info fields remain part of the selected identity and become
public with it. The UI states this before activation. Restoration stops public
participation; it cannot retract metadata or addresses already shared.

Shared write access requires a proof of identical content:

- V1: identical file paths, lengths, order, offsets and padding; identical piece
  length and SHA-1 piece hashes.
- V2: identical file paths, lengths, padding and authenticated per-file Merkle
  roots. Different v2 piece lengths are allowed because those roots identify
  the same bytes independently of piece length.
- Hybrid: both validations must pass. Cross-version substitutions are rejected.
- Renamed/remapped session files are rejected until the original layout is
  restored. Same names and sizes alone are insufficient.
- Pending magnets cannot switch until complete metainfo exists. Candidate input
  is capped at 8 MiB, with parser allocations charged to the metadata governor.

A v1 public release with different piece sizes cannot safely write into a partial
private download through this feature. Use a separately downloaded public release
and ordinary file verification/import for that case. This avoids overwriting
verified private pieces with content that has only a matching filename or size.

## Comparison semantics

Comparison is explicitly requested and does not announce either candidate as a
new participant. The original private hash is sent only to its existing trackers.
Public queries use only the candidate public hash. Tracker credentials stay on the
server; responses show tracker origins with paths, queries and userinfo removed.

| Display | Meaning |
| --- | --- |
| Private seeds / leechers | Individual tracker HTTP or UDP scrape report |
| Private last known report | Stored announce/scrape counts when a fresh scrape fails; timestamp retained |
| Public discovered peers | Unique allowed endpoints returned by the bounded DHT lookup |
| Public estimated seeds / peers | BEP 33 bloom-filter estimates, only when provided by responding nodes |
| Reachable in sample | Successful matching-infohash TCP handshakes from up to 12 discovered peers |
| Observed seeds | Sampled peers advertising all pieces through bitfield/HaveAll; a lower bound |
| Unknown | Unsupported, unavailable, saturated estimate, timeout, or no usable report; never converted to zero |

DHT discovery is not a swarm census. Bloom filters count IP addresses and are
estimates; NAT, stale announcements, unsupported extensions and incomplete DHT
coverage affect them. The peer sample requests no blocks and uploads no payload.
An unreachable or unclassified peer is not asserted to be a leecher. Encrypted-
only and uTP-only peers may not answer the TCP sample. Private tracker counts are
shown individually, never summed across overlapping populations.

Probes have separate concurrency limits (two comparisons, three DHT queries per
lookup, four sampled peers), 64 queried nodes, 256 returned peers, 8 bootstrap
hosts and an eight-second DHT deadline. Each peer sample has a two-second deadline.
Tracker scrapes are bounded to eight tracker rows and five seconds each. They
share configured outbound address restrictions, DNS admission and peer bans.
Hybrid comparisons use the v1 discovery hash, matching the engine’s DHT registration.
Pure v2 uses its truncated SHA-256 discovery key while retaining the full v2 session identity.

A comparison does not require globally enabling DHT; activation does. The UI
requires refreshing a comparison older than one minute before switching in either direction. Interrupted transitions can always be restored without a comparison.

## Lifecycle and persistence contract

The original `.torrent`, tracker overrides, labels, original file choices and
accounting remain authoritative. Public transfer totals are independent. Public
activation copies file selection and transfer limits; it never attributes public
traffic to the private tracker. The new actor runs the existing data-durability
and completion gates.

The engine owns ordering; UI/API calls cannot independently pause, copy and
resume around concurrent commands. A schema-12 `swarm_switches` journal reserves
both identities before switching:

| Phase | Network/storage ownership | Recovery |
| --- | --- | --- |
| Preparing | Original is drained; public remains paused until setup commits | Park both; allow explicit private restore |
| Public | Private is parked; public is the sole active identity | Restore public's persisted lifecycle; retain private fence |
| Restoring | Drain public and invalidate private fastresume | Park both; allow retry |
| No relation | Ordinary independent torrent lifecycle | Normal engine recovery |

The durable relation is installed before quiescence. The old actor drains writes,
closes peers, sends its stopped event and is joined before a replacement starts.
DHT registrations are removed during retirement. A failed drain never starts the
other actor. Pending promotions are cancelled; inactive identities cannot be
promoted by incoming peers, trackers, compatibility APIs or queue control.

On restore, deleting the public DB row, removing the link and setting the original
to Checking are one transaction. Original fastresume is invalidated and the fresh
actor receives Checking even if an older checkpoint survives a filesystem crash.
The retained private artifact is validated before retiring the public session; a missing or corrupt file leaves the public session intact.
The original cannot seed based on its old bitmap. Existing crash-safety barriers
still govern completed/seeding claims.

Interrupted switch records are recovered before actors load. Stable public
sessions retain their lifecycle state; every original stays parked. An
interrupted transition parks both and remains visible for an explicit retry.
No automatic fallback publishes a private hash to DHT.

This changes the native DB schema to 12. Older binaries reject the newer schema
instead of ignoring exclusive-ownership records. Use a pre-upgrade database
backup when downgrading binaries; do not manually decrement `user_version`.

## Native API

All routes are under `/api/v1/torrents/{hash}/swarm-switch`:

| Method and suffix | Result |
| --- | --- |
| GET | Current relationship, restore availability, DHT state and active progress |
| POST `/preview` | Validate candidate and compare both swarms without switching |
| POST `/public` | Activate the explicitly expected candidate hash |
| POST `/restore` | Restore the retained private identity with verification |

Preview/public bodies accept `variant` and optional `public_torrent_base64`.
`variant=supplied` requires the file; generated variants reject an uploaded file.
Public activation additionally requires `expected_public_hash` from the preview.
Unknown JSON fields are rejected. Existing authentication, token permissions,
CSRF and request-body limits apply; comparison requires mutation authorization
because it initiates network probes. GET is read-only.

Repeated activation of the same active candidate is idempotent. A different
candidate requires restore first. Restore through the original identity is
idempotent after success. Restoring an independent public torrent fails explicitly.

The WebUI Bridge forwards these routes when connected to the TorrentNG engine.
Other backend types report this capability as unavailable.

The feature is capability-gated with `supports_swarm_switch`. Third-party engines
must not expose a raw private-flag override in its place: they do not participate
in the native exclusivity, recovery or accounting contract.

## Verification contract

Deterministic tests cover candidate identity and credential stripping, v1/v2/
hybrid content compatibility, unknown-versus-zero counts, authenticated probe
responses, bloom estimates, journal recovery, private-copy deletion, shared-file
fences, lifecycle reversal, and UI/API availability. Local fixtures should supply
all content and discovery; no user's tracker credentials or real private torrent
are needed. Live DHT results remain observations of the network at the displayed
time, not a guaranteed performance improvement.

Protocol references: [BEP 3](https://www.bittorrent.org/beps/bep_0003.html),
[BEP 27](https://www.bittorrent.org/beps/bep_0027.html),
[BEP 52](https://www.bittorrent.org/beps/bep_0052.html),
[BEP 5](https://www.bittorrent.org/beps/bep_0005.html),
[BEP 15](https://www.bittorrent.org/beps/bep_0015.html),
[BEP 33](https://www.bittorrent.org/beps/bep_0033.html).

## Validation evidence (2026-10-24)

- Native workspace: 2,423 tests passed with two existing process-inspection
  tests excluded after confirming this runner cannot see its own PID through
  their process probe. The excluded tests are
  `source_state::tests::an_explicit_lock_held_by_a_live_process_means_running`
  and `source_state::tests::this_process_is_seen_running_with_its_own_name`.
- Six runtime lifecycle tests cover actual public payload transfer and restart,
  private restoration, v2/hybrid verification, interrupted phases, private-copy
  deletion, missing original metainfo, standalone public torrents, and stale
  candidate rejection. Local protocol fixtures cover DHT transaction validation,
  zero/unknown observations, peer availability and authenticated tracker scrapes.
- Native workspace Clippy and full-compatibility daemon Clippy pass with warnings
  denied. Full-compatibility daemon tests: 32 passed.
- Bridge feature tests and all 122 API integration tests pass; Bridge Clippy
  passes. Five existing rTorrent Unix-socket tests cannot bind sockets in this
  runner (`Operation not permitted`); their source is unchanged.
- WebUI: 57 unit tests, lint and production build pass. The browser workspace
  checks pass on desktop and mobile (10 passed, four existing viewport-specific
  skips), including compare → public → compare → private. Candidate controls are
  checked against viewport bounds. Browser validation used Chromium 134 because
  the configured browser download was unavailable in this runner.
- OpenAPI, 61 repository Python tests, formatting, and the commit security scan
  pass; Semgrep reported zero findings across the changed source files.

These are fixture-based checks. They do not claim that a particular public
counterpart exists or that a live tracker will permit scraping. The seven
runner-limited checks above still need a host that supports their OS primitives.
