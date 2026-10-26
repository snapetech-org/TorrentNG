---
category: changed
audience: users, operators
area: networking
action: none
breaking: false
---
The transfer engine now uses its request ledger to cancel duplicate block requests, avoiding a full peer scan for every accepted block and reducing overhead in busy swarms.
