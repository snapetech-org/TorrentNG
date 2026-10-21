export type OidcProviderPresentation = { label: string; icon: string; setupUrl: string }

const GENERIC_PROVIDER: OidcProviderPresentation = {
  label: 'identity provider', icon: 'generic',
  setupUrl: 'https://openid.net/developers/how-connect-works/',
}

const PROVIDERS: Record<string, OidcProviderPresentation> = {
  google: { label: 'Google', icon: 'google', setupUrl: 'https://developers.google.com/identity/openid-connect/openid-connect' },
  'microsoft-entra': { label: 'Microsoft Entra ID', icon: 'microsoft', setupUrl: 'https://learn.microsoft.com/en-us/entra/identity-platform/v2-protocols-oidc' },
  okta: { label: 'Okta', icon: 'okta', setupUrl: 'https://developer.okta.com/docs/guides/implement-grant-type/-/main/' },
  auth0: { label: 'Auth0', icon: 'auth0', setupUrl: 'https://auth0.com/docs/quickstart/webapp/express/index' },
  keycloak: { label: 'Keycloak', icon: 'keycloak', setupUrl: 'https://www.keycloak.org/docs/latest/server_admin/' },
  authentik: { label: 'authentik', icon: 'authentik', setupUrl: 'https://docs.goauthentik.io/add-secure-apps/providers/oauth2/create-oauth2-provider/' },
  zitadel: { label: 'ZITADEL', icon: 'zitadel', setupUrl: 'https://zitadel.com/docs/guides/integrate/login/oidc/oauth-recommended-flows' },
  'amazon-cognito': { label: 'Amazon Cognito', icon: 'amazon-cognito', setupUrl: 'https://docs.aws.amazon.com/cognito/latest/developerguide/federation-endpoints.html' },
  'apple-sign-in': { label: 'Apple', icon: 'apple', setupUrl: 'https://developer.apple.com/documentation/signinwithapple/configuring-your-environment-for-sign-in-with-apple' },
  gitlab: { label: 'GitLab', icon: 'gitlab', setupUrl: 'https://docs.gitlab.com/integration/openid_connect_provider/' },
}

export function getOidcProviderPresentation(provider: unknown): OidcProviderPresentation {
  if (typeof provider !== 'string') return GENERIC_PROVIDER
  const key = provider.trim().toLowerCase()
  return Object.prototype.hasOwnProperty.call(PROVIDERS, key) ? PROVIDERS[key] : GENERIC_PROVIDER
}
