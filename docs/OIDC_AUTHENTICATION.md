# OIDC authentication

TorrentNG supports server-side OpenID Connect authorization-code sign-in with
PKCE for ordinary providers. Apple uses its documented form-post flow without
PKCE. A successful callback creates the WebUI's normal signed, HttpOnly
`tng_session` and qBittorrent-compatible `SID` cookies. Provider tokens are
not returned to the browser or persisted. Password and API-token login remain
available. This adapter follows the [shared cross-project OIDC contract](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-cross-project-template.md).

Configure a confidential web client with this exact callback path:

```toml
[auth.oidc]
provider = "keycloak"
issuer_url = "https://auth.example.lan/application/o/torrentng/"
client_id = "torrentng"
client_secret = "replace-me"
redirect_uri = "https://torrent.example.lan/api/v1/auth/oidc/callback"
allowed_emails = ["operator@example.com"]
allowed_email_domains = []
allow_any_verified_user = false
allow_private_issuer = false
```

The equivalent environment variables use the `TNG_OIDC_` prefix (the legacy
`RTNG_OIDC_` prefix is also accepted): `ISSUER_URL`, `CLIENT_ID`,
`CLIENT_SECRET`, `REDIRECT_URI`, `ALLOWED_EMAILS`, `ALLOWED_EMAIL_DOMAINS`,
`ALLOW_ANY_VERIFIED_USER`, and `ALLOW_PRIVATE_ISSUER`. Email and domain lists
accept comma, semicolon, or whitespace separators. All four core settings must
be supplied together. Issuer metadata must exactly match the configured
issuer. The callback must use HTTPS and its path must be
`/api/v1/auth/oidc/callback`.

Set `TNG_OIDC_PROVIDER=keycloak` (or `RTNG_OIDC_PROVIDER=keycloak`) to brand
the login button and choose its bundled provider mark. This optional hint does
not change the issuer or validation. See the [provider preset catalog](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-provider-catalog.json)
for supported IDs and official setup links.

For the full human-readable provider list, issuer examples, official setup
pages, and provider-specific scope/claim notes, see the shared
[provider catalog](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-provider-catalog.md).
The [cross-project implementation contract](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-cross-project-template.md)
and [rollout validation record](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-rollout-validation.md)
cover parity requirements and the verification already completed across the
nine apps.

Admission requires a verified email matching an exact listed address or
domain. An empty allowlist denies sign-in unless
`allow_any_verified_user = true` is explicitly enabled. TorrentNG's current
WebUI has one operator permission level; every admitted identity receives that
same full operator access. Provider group claims do not grant or change
privileges, and an OIDC identity is not linked to an existing local login by
email. Local password and API-token login remain recovery paths.

Issuer and discovered endpoint hosts that resolve to private or reserved
addresses are rejected by default; redirects from provider endpoints are
disabled, and each request is pinned to its validated addresses to prevent DNS
rebinding. OIDC requests connect directly and bypass environment proxy settings.
Set `allow_private_issuer = true` only when the configured provider and its
endpoints are intentionally hosted on a private network. The callback still
must use HTTPS.

The flow stores a one-time state and nonce (plus a PKCE verifier for ordinary
providers) on the server for ten minutes. Each attempt has its own Secure,
HttpOnly, SameSite=Lax state cookie so concurrent sign-ins in one browser do
not overwrite each other. Up to 16 attempts can be pending; additional starts
are rejected until one completes or expires. The callback validates the
provider signature and required ID-token claims, and restricts the post-login
return path to a local path. Pending state is process-local, so a service
restart invalidates in-flight sign-ins. OIDC sessions use the existing
24-hour app-session lifetime.

The callback accepts the ordinary provider GET response and Apple's required
URL-encoded POST response. Reverse proxies must preserve the callback method
and form body.

## Sign in with Apple

Set `issuer_url` to exactly `https://appleid.apple.com`, use the Apple Services
ID as `client_id`, and set `client_secret` to Apple's signed client-secret JWT.
Register `https://<public-host>/api/v1/auth/oidc/callback` as the exact HTTPS
return URL. TorrentNG requests only `openid email`, uses
`response_mode=form_post`, and omits PKCE for Apple's documented web flow. The
callback accepts Apple's bounded form POST and binds it to the one-time state
cookie. Apple may encode `email_verified` as a JSON boolean or string; only
`true` is admitted. Rotate the JWT before its six-month maximum expiry. See
Apple's [environment setup](https://developer.apple.com/documentation/signinwithapple/configuring-your-environment-for-sign-in-with-apple)
and [web authorization flow](https://developer.apple.com/documentation/signinwithapple/incorporating-sign-in-with-apple-into-other-platforms).

## Enable, verify, and recover

The four core values—issuer, client ID, client secret, and redirect URI—must be
present together. Keep the secret in the protected server-side TorrentNG
configuration or deployment secret source. Do not put it in the WebUI, a
checked-in TOML file, support logs, or a README. Register the exact
`/api/v1/auth/oidc/callback` URL from the public HTTPS origin. If the WebUI is
mounted below a reverse-proxy path, include the path in both `redirect_uri` and
the provider's allowed callback list.

An exact verified-email or domain admission grants TorrentNG's current full
operator access. `allow_any_verified_user = true` grants that same access to
every verified provider user, so keep it off unless provider-side client
assignment independently limits membership. OIDC groups do not change local
permissions and email does not link an OIDC identity to a password account.

Before publishing the login action, test one admitted and one rejected identity
from the public URL. Check that the `tng_session` and `SID` cookies are issued,
that logout revokes the OIDC session, and that local password or API-token
recovery still works. Pending state is process-local and expires after ten
minutes; a restart invalidates an in-flight callback. Apply new config, then
start a fresh sign-in. To disable OIDC, remove the complete `[auth.oidc]`
configuration or its environment-variable equivalent.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Provider rejects the callback | Use the exact HTTPS URL ending `/api/v1/auth/oidc/callback`; compare host, port, path, reverse-proxy prefix, and trailing slash with both config and provider. |
| Discovery/token/key request is rejected | Verify exact issuer metadata and outbound DNS. Private/reserved targets are blocked by default; opt in only for a trusted internal IdP. TorrentNG OIDC requests bypass environment HTTP proxy settings. |
| State or nonce validation fails | Start a new login after a restart or ten-minute expiry. Keep the same public hostname and browser, preserve the Secure/HttpOnly state cookie, and check proxy scheme/host forwarding. |
| Sign-in fails admission | Confirm `email_verified`, email/domain list formatting, and the provider's email claim mapping. An empty allow-list denies access unless allow-any is explicitly on. |
| Access is unexpectedly broad | Review `allow_any_verified_user` and provider app assignment; all admitted identities receive the same TorrentNG operator access. |
| Apple form callback fails | Confirm Services ID, Apple issuer, exact callback, signed client-secret JWT, HTTPS origin, and form-post cookie support. |

When requesting help, include the TorrentNG version, provider ID, redacted
issuer host, callback path, error code, and timestamp. Remove authorization
codes, tokens, client secrets, and raw claims.
