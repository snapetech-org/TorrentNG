---
category: fixed
audience: users, operators
area: networking
action: none
breaking: false
---
V1-only magnet metadata lookups no longer advertise the BEP 52 v2-upgrade flag. This avoids handshake mismatches with clients such as libtorrent and improves v1 metadata discovery compatibility.
