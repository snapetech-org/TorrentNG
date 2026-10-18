---
category: fixed
audience: users, operators
area: transfers
action: none
breaking: false
---
Hybrid downloads and rechecks now validate both the v1 SHA-1 piece hash and the authenticated v2 Merkle root. Data whose two integrity views disagree is no longer marked valid.
