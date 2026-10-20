---
category: security
audience: users, operators
area: network
action: none
breaking: false
---
TLS certificate verification now rejects affected chains and uses an updated root bundle that removes distrusted emSign certificates. A service using those chains may stop connecting until its certificate chain is replaced with a trusted one.
