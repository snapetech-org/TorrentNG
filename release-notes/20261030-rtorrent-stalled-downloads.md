---
category: fixed
audience: users, operators
area: rtorrent
action: none
breaking: false
---
The rTorrent integration now refreshes incomplete torrents even when their transfer rate is zero, and treats an empty nonblocking SCGI accept queue as normal. This keeps stalled-download state current and prevents idle listener polling from being reported as an accept failure.
