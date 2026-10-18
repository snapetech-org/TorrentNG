---
category: security
audience: users, operators
area: transfer
action: none
breaking: false
---
Peer and webseed requests now share bounded process and per-torrent admission. This limits outstanding source work across concurrent downloads and reduces memory pressure when peers or webseeds respond slowly.
