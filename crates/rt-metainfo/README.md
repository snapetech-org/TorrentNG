# rt-metainfo

torrent metadata parser: .torrent and magnet, infohash, path sanitization.

## Status: Implemented — TorrentNG client support

The `swarm` module builds sanitized public counterparts and proves compatible
shared payloads: v1 piece hashes and layout, v2 file roots and layout, or both
for hybrid torrents. It preserves supplied public info bytes exactly and offers
a bounded allocation-reservation API. See [Swarm Switch](../../docs/SWARM_SWITCH.md).
