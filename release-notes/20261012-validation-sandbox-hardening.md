---
category: security
audience: operators
area: ci-security
action: none
breaking: false
---

Local validation snapshots exclude dotted and extensionless credentials, private/API keys, access/refresh-token and client-secret material, and non-source files under credential folders while retaining source modules such as TokenStore.ts. Output logs remain bounded, with a visible marker when diagnostics are truncated.
