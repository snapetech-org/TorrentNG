---
category: added
audience: users, operators
area: swarm-switch
action: Back up the native database before upgrading; schema 12 requires a backup to downgrade.
breaking: false
---
Swarm Switch compares private tracker reports and public DHT observations before using an identical public counterpart. Retain the private torrent to restore verified private seeding; deleting that copy leaves an independent public torrent. Available with the TorrentNG engine.
