---
category: security
audience: users, operators
area: network
action: none
breaking: false
---
TLS certificate verification now rejects affected chains and uses an updated root bundle that removes distrusted emSign certificates. Patched package versions are locked in the native, sidecar, desktop, and urtorrent-runner dependency sets. A service using those chains may stop connecting until its certificate chain is replaced with a trusted one.
