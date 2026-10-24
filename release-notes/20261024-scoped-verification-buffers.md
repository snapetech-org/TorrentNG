---
category: changed
audience: users, operators
area: storage
action: none
breaking: false
---

Linux operators can test bounded verification scratch reuse with `TNG_STORAGE_ANON_LARGE_FRAMES=1`. Idle large buffers are released after verification to reduce retained RAM. This remains experimental; buffered file I/O and normal release defaults are unchanged, and rTorrent parity is not established.
