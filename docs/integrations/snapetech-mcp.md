# SnapeTech MCP

TorrentNG does not run an MCP server inside the application. The shared
`snapetech-mcp` process connects to the TorrentNG Server API using MCP stdio
transport. Its quickstart is in the sibling `snapetech-mcp/README.md`.

## Tools and API access

| MCP tool | TorrentNG endpoint | Result |
| --- | --- | --- |
| `torrentng_status` | `GET /health` | Reachability only. |
| `torrentng_list_torrents` | `GET /api/v1/torrents` | Up to twenty projected summaries per bounded snapshot page. |
| `torrentng_get_transfer_summary` | `GET /api/v1/transfer/info` | Aggregate byte totals, transfer rates, connection state, and DHT node count. |

Configure `SNAPETECH_MCP_APPS=torrentng`,
`SNAPETECH_MCP_TORRENTNG_URL=<TorrentNG API base URL>`, and
`SNAPETECH_MCP_TORRENTNG_API_KEY=<dedicated token>` in the MCP host's protected
environment. The bridge sends the token as `Authorization: Bearer`. For the
native server, the token can be supplied through the private `api_tokens`
configuration or the deployment's `TORRENTNG_API_TOKEN` secret.

## Credential and data boundaries

The current API token is also valid for the WebUI and other authenticated API
operations; no read-only token role was identified in the reviewed source.
Use a separate randomly generated token for this MCP process and keep it
private. The MCP server itself only issues GET requests and excludes info
hashes, tracker URLs, filesystem paths, and credentials from its result, but
the upstream token remains broader than that tool surface.

Keep the MCP process attached to a trusted local client. Prefer HTTPS for
remote instances; HTTP to a non-loopback address requires an explicit app ID
in `SNAPETECH_MCP_ALLOW_HTTP`. Never expose the stdio server through a TCP
bridge or reverse proxy.

## Competitor feature coverage

The [qBittorrent WebUI API](https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-%28qBittorrent-5.0%29) and [Transmission RPC](https://github.com/transmission/transmission/blob/main/docs/rpc-spec.md) cover filtered torrent lists, properties, files,
trackers/peers, queue and transfer limits, categories/labels/tags, RSS/search, and
client settings. Community MCP projects expose much of that surface, including
mutations; for example, [qBittorrent MCP](https://github.com/arr-mcps/qbittorrent-mcp) reports tools for torrent control, search/RSS,
categories, tags, and settings.

TorrentNG provides a native engine and a WebUI bridge for existing clients. Snapetech
MCP exposes snapshot-paged torrent summaries and aggregate transfer statistics. It
omits hashes, tracker URLs/messages, peer identifiers, paths, and file listings. Queue
controls, adding/removing torrents, file selection, client preferences, and filesystem
import/move remain outside the reusable read-only baseline; the current API token is
broader than the MCP tool surface.

**Confidence:** high for the published competitor API surfaces; moderate for direct comparison because TorrentNG includes a native engine and an existing-client bridge.

Source contract: `docs/THREAT_MODEL.md`,
`deploy/native/config.toml`, and the authenticated API router.
