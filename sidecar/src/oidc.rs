//! Shared-contract OIDC login for the TorrentNG WebUI operator session.

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use openidconnect::url::Url;
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    reqwest, AuthType, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::net::lookup_host;

use crate::config::OidcConfig;

const FLOW_TTL: Duration = Duration::from_secs(10 * 60);
const SESSION_TTL_SECS: u64 = 24 * 60 * 60;
const MAX_PENDING_FLOWS: usize = 16;
const MAX_OIDC_SESSIONS: usize = 4_096;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1 << 20;

type ConfiguredCoreClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
    EndpointMaybeSet,
>;

#[derive(Debug)]
struct OidcHttpError;

impl fmt::Display for OidcHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OIDC provider request failed or exceeded its size limit")
    }
}

impl std::error::Error for OidcHttpError {}

struct BoundedOidcHttpClient {
    allow_private: bool,
}

impl<'a> openidconnect::AsyncHttpClient<'a> for BoundedOidcHttpClient {
    type Error = OidcHttpError;
    type Future = Pin<
        Box<
            dyn Future<Output = Result<openidconnect::http::Response<Vec<u8>>, Self::Error>>
                + Send
                + 'a,
        >,
    >;

    fn call(&'a self, request: openidconnect::http::Request<Vec<u8>>) -> Self::Future {
        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let url = Url::parse(&parts.uri.to_string()).map_err(|_| OidcHttpError)?;
            let host = url.host_str().ok_or(OidcHttpError)?;
            let addresses = resolve_target(&url, self.allow_private).await?;
            let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
                .map_err(|_| OidcHttpError)?;
            // Pin the socket addresses checked above. A separate validation
            // lookup followed by reqwest's own DNS lookup would allow a
            // rebinding hostname to switch to a private address in between.
            let client = reqwest::ClientBuilder::new()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .resolve_to_addrs(host, &addresses)
                .build()
                .map_err(|_| OidcHttpError)?;
            let response = client
                .request(method, url)
                .version(parts.version)
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(|_| OidcHttpError)?;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64)
            {
                return Err(OidcHttpError);
            }
            let status = response.status();
            let version = response.version();
            let headers = response.headers().clone();
            let mut stream = response;
            let mut body = Vec::new();
            while let Some(chunk) = stream.chunk().await.map_err(|_| OidcHttpError)? {
                if body.len().saturating_add(chunk.len()) > MAX_PROVIDER_RESPONSE_BYTES {
                    return Err(OidcHttpError);
                }
                body.extend_from_slice(&chunk);
            }
            let mut builder = openidconnect::http::Response::builder()
                .status(status)
                .version(version);
            if let Some(response_headers) = builder.headers_mut() {
                *response_headers = headers;
            }
            builder.body(body).map_err(|_| OidcHttpError)
        })
    }
}

struct PendingFlow {
    nonce: Nonce,
    verifier: Option<PkceCodeVerifier>,
    return_to: String,
    expires_at: Instant,
}

pub struct OidcLoginStart {
    pub authorize_url: String,
    pub state: String,
}

#[derive(Clone)]
pub struct OidcAuth {
    settings: OidcConfig,
    pending: Arc<Mutex<HashMap<String, PendingFlow>>>,
    sessions: Arc<Mutex<HashMap<String, u64>>>,
}

impl OidcAuth {
    pub fn new(settings: OidcConfig) -> Self {
        Self {
            settings,
            pending: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn enabled(&self) -> bool {
        !self.settings.issuer_url.is_empty()
    }

    pub async fn begin(&self, next: Option<&str>) -> Result<OidcLoginStart, &'static str> {
        if !self.enabled() {
            return Err("OIDC is not configured");
        }
        let apple = self.is_apple_issuer();
        let (authorize_url, csrf, nonce, verifier) = {
            let (client, _http, _) = self.client().await?;
            let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
            let request = client
                .authorize_url(
                    CoreAuthenticationFlow::AuthorizationCode,
                    CsrfToken::new_random,
                    Nonce::new_random,
                )
                .add_scope(Scope::new("openid".to_owned()))
                .add_scope(Scope::new("email".to_owned()))
                .add_scopes(if apple {
                    Vec::new()
                } else {
                    vec![Scope::new("profile".to_owned())]
                });
            let (authorize_url, csrf, nonce) = if apple {
                request.add_extra_param("response_mode", "form_post").url()
            } else {
                request.set_pkce_challenge(challenge).url()
            };
            (authorize_url, csrf, nonce, verifier)
        };

        let state = csrf.secret().clone();
        let pending = PendingFlow {
            nonce,
            verifier: (!apple).then_some(verifier),
            return_to: safe_return_to(next),
            expires_at: Instant::now() + FLOW_TTL,
        };
        let mut flows = self.pending.lock().expect("OIDC flow lock poisoned");
        flows.retain(|_, flow| flow.expires_at > Instant::now());
        if flows.len() >= MAX_PENDING_FLOWS {
            return Err("too many pending sign-ins");
        }
        flows.insert(state.clone(), pending);
        Ok(OidcLoginStart {
            authorize_url: authorize_url.to_string(),
            state,
        })
    }

    pub async fn finish(
        &self,
        state: &str,
        cookie_state: &str,
        code: &str,
    ) -> Result<(String, String, String), &'static str> {
        if state.is_empty()
            || state.len() > 256
            || code.is_empty()
            || code.len() > 8_192
            || !constant_time_equal(state.as_bytes(), cookie_state.as_bytes())
        {
            return Err("invalid OIDC transaction");
        }
        let flow = {
            let mut flows = self.pending.lock().expect("OIDC flow lock poisoned");
            let Some(flow) = flows.get(state) else {
                return Err("expired OIDC transaction");
            };
            if flow.expires_at <= Instant::now() {
                flows.remove(state);
                return Err("expired OIDC transaction");
            }
            flows.remove(state).expect("flow was present")
        };

        let (client, http, userinfo_endpoint) = self.client().await?;
        let exchange = client.exchange_code(AuthorizationCode::new(code.to_owned()));
        let exchange = match flow.verifier {
            Some(verifier) => exchange.set_pkce_verifier(verifier),
            None => exchange,
        };
        let tokens = exchange
            .request_async(&http)
            .await
            .map_err(|_| "OIDC token exchange failed")?;
        let id_token = tokens.id_token().ok_or("provider returned no ID token")?;
        let claims = id_token
            .claims(&client.id_token_verifier(), &flow.nonce)
            .map_err(|_| "provider ID token validation failed")?;
        let subject = claims.subject().as_str().to_owned();
        if subject.is_empty() || subject.len() > 1_024 {
            return Err("provider subject is invalid");
        }

        let email = match (claims.email(), claims.email_verified()) {
            (Some(email), Some(true)) => email.as_str().to_owned(),
            (_, Some(false)) => return Err("provider email is unverified"),
            _ => {
                let endpoint = userinfo_endpoint
                    .as_deref()
                    .ok_or("provider returned no email or user-info endpoint")?;
                let userinfo = http
                    .request_userinfo(endpoint, tokens.access_token().secret())
                    .await
                    .map_err(|_| "provider user-info request failed")?;
                if userinfo.get("sub").and_then(serde_json::Value::as_str) != Some(&subject)
                    || !json_boolean(userinfo.get("email_verified"))
                {
                    return Err("provider user-info identity is not verified");
                }
                userinfo
                    .get("email")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("provider did not return an email")?
                    .to_owned()
            }
        };
        let email = email.trim().to_ascii_lowercase();
        if !self.email_allowed(&email) {
            return Err("identity is not admitted");
        }
        Ok((email, subject, flow.return_to))
    }

    pub fn new_session_credential(&self) -> String {
        let now = unix_now();
        let credential = uuid::Uuid::new_v4().simple().to_string();
        let mut sessions = self.sessions.lock().expect("OIDC session lock poisoned");
        sessions.retain(|_, expires| *expires > now);
        if sessions.len() >= MAX_OIDC_SESSIONS {
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, expires)| *expires)
                .map(|(token, _)| token.clone())
            {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(credential.clone(), now + SESSION_TTL_SECS);
        credential
    }

    pub fn active_session_credentials(&self) -> Vec<String> {
        let now = unix_now();
        let mut sessions = self.sessions.lock().expect("OIDC session lock poisoned");
        sessions.retain(|_, expires| *expires > now);
        sessions.keys().cloned().collect()
    }

    pub fn revoke_session_credential(&self, credential: &str) -> bool {
        self.sessions
            .lock()
            .expect("OIDC session lock poisoned")
            .remove(credential)
            .is_some()
    }

    async fn client(
        &self,
    ) -> Result<(ConfiguredCoreClient, BoundedOidcHttpClient, Option<String>), &'static str> {
        let issuer_url =
            Url::parse(&self.settings.issuer_url).map_err(|_| "OIDC issuer URL is invalid")?;
        let redirect_uri =
            Url::parse(&self.settings.redirect_uri).map_err(|_| "OIDC callback URL is invalid")?;
        if !self.settings.allow_private_issuer {
            ensure_public_target(&issuer_url).await?;
        }
        let http = BoundedOidcHttpClient {
            allow_private: self.settings.allow_private_issuer,
        };
        let metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(self.settings.issuer_url.clone())
                .map_err(|_| "OIDC issuer URL is invalid")?,
            &http,
        )
        .await
        .map_err(|_| "OIDC discovery failed")?;
        if metadata.issuer().as_str() != self.settings.issuer_url {
            return Err("OIDC issuer metadata did not match the configured issuer");
        }
        if !self.settings.allow_private_issuer {
            let endpoints = serde_json::to_value(&metadata)
                .map_err(|_| "OIDC metadata could not be inspected")?;
            for name in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
                let endpoint = endpoints
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .ok_or("OIDC metadata omitted a required endpoint")?;
                let endpoint = Url::parse(endpoint).map_err(|_| "OIDC endpoint URL is invalid")?;
                if endpoint.scheme() != "https" {
                    return Err("OIDC endpoints must use HTTPS");
                }
                ensure_public_target(&endpoint).await?;
            }
            if let Some(endpoint) = endpoints
                .get("userinfo_endpoint")
                .and_then(serde_json::Value::as_str)
            {
                let endpoint = Url::parse(endpoint).map_err(|_| "OIDC endpoint URL is invalid")?;
                if endpoint.scheme() != "https" {
                    return Err("OIDC endpoints must use HTTPS");
                }
                ensure_public_target(&endpoint).await?;
            }
        }
        let authorization_endpoint = metadata.authorization_endpoint().clone();
        let token_endpoint = metadata
            .token_endpoint()
            .cloned()
            .ok_or("OIDC discovery omitted a token endpoint")?;
        let userinfo_endpoint = metadata
            .userinfo_endpoint()
            .map(|endpoint| endpoint.as_str().to_owned());
        let mut client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(self.settings.client_id.clone()),
            Some(ClientSecret::new(self.settings.client_secret.clone())),
        )
        .set_auth_uri(authorization_endpoint)
        .set_token_uri(token_endpoint)
        .set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_string())
                .map_err(|_| "OIDC callback URL is invalid")?,
        );
        if self.is_apple_issuer() {
            client = client.set_auth_type(AuthType::RequestBody);
        }
        Ok((client, http, userinfo_endpoint))
    }

    pub(crate) fn is_apple_issuer(&self) -> bool {
        self.settings.issuer_url.trim() == "https://appleid.apple.com"
    }

    fn email_allowed(&self, email: &str) -> bool {
        if self.settings.allow_any_verified_user {
            return true;
        }
        let address_match = self
            .settings
            .allowed_emails
            .iter()
            .any(|allowed| allowed.trim().eq_ignore_ascii_case(email));
        let domain = email
            .rsplit_once('@')
            .map(|(_, domain)| domain)
            .unwrap_or_default();
        address_match
            || self.settings.allowed_email_domains.iter().any(|allowed| {
                allowed
                    .trim()
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(domain)
            })
    }
}

impl BoundedOidcHttpClient {
    async fn request_userinfo(
        &self,
        endpoint: &str,
        access_token: &str,
    ) -> Result<serde_json::Value, OidcHttpError> {
        let request = openidconnect::http::Request::builder()
            .method("GET")
            .uri(endpoint)
            .header(
                openidconnect::http::header::AUTHORIZATION,
                format!("Bearer {access_token}"),
            )
            .body(Vec::new())
            .map_err(|_| OidcHttpError)?;
        let response = openidconnect::AsyncHttpClient::call(self, request).await?;
        if !response.status().is_success()
            || !response
                .headers()
                .get(openidconnect::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.to_ascii_lowercase().contains("json"))
        {
            return Err(OidcHttpError);
        }
        serde_json::from_slice(response.body()).map_err(|_| OidcHttpError)
    }
}

fn json_boolean(value: Option<&serde_json::Value>) -> bool {
    matches!(value, Some(serde_json::Value::Bool(true)))
        || matches!(value, Some(serde_json::Value::String(value)) if value == "true")
}

fn safe_return_to(raw: Option<&str>) -> String {
    let Some(value) = raw.filter(|value| value.len() <= 2_048) else {
        return "/".to_owned();
    };
    if !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return "/".to_owned();
    }
    value.to_owned()
}

#[derive(Deserialize, Default)]
pub(crate) struct OidcStartQuery {
    next: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub(crate) async fn oidc_enabled(
    State(state): State<crate::api::server::AppState>,
) -> impl IntoResponse {
    Json(serde_json::json!({
        "enabled": state.oidc_auth.enabled(),
        "provider": state.cfg.auth.oidc.provider,
    }))
}

pub(crate) async fn oidc_login(
    State(state): State<crate::api::server::AppState>,
    Query(query): Query<OidcStartQuery>,
) -> Response {
    let start = match state.oidc_auth.begin(query.next.as_deref()).await {
        Ok(start) => start,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "OIDC sign-in is unavailable",
            )
                .into_response();
        }
    };
    let same_site = if state.oidc_auth.is_apple_issuer() {
        "None"
    } else {
        "Lax"
    };
    let cookie = format!(
        "{}={}; Path=/api/v1/auth/oidc/callback; Max-Age={}; HttpOnly; SameSite={same_site}; Secure",
        oidc_state_cookie_name(&start.state),
        start.state,
        FLOW_TTL.as_secs(),
    );
    let mut response = Redirect::temporary(&start.authorize_url).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("generated OIDC state cookie is a valid header"),
    );
    response
}

pub(crate) async fn oidc_callback(
    State(state): State<crate::api::server::AppState>,
    Query(query): Query<OidcCallbackQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let state_value = query.state.as_deref().unwrap_or_default();
    let cookie_state = oidc_state_cookie(&headers, state_value);
    oidc_callback_parts(
        &state,
        state_value,
        cookie_state,
        query.code.as_deref(),
        query.error.is_some(),
    )
    .await
}

#[derive(Default)]
pub(crate) struct OidcCallbackForm {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn parse_oidc_callback_form(body: &[u8]) -> Option<OidcCallbackForm> {
    if body.len() > 16 * 1024 {
        return None;
    }
    let mut form = OidcCallbackForm::default();
    for (key, value) in openidconnect::url::form_urlencoded::parse(body) {
        let slot = match key.as_ref() {
            "code" => &mut form.code,
            "state" => &mut form.state,
            "error" => &mut form.error,
            _ => continue,
        };
        if value.len() > 8_192 || slot.replace(value.into_owned()).is_some() {
            return None;
        }
    }
    Some(form)
}

pub(crate) async fn oidc_callback_form(
    State(state): State<crate::api::server::AppState>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    if !state.oidc_auth.is_apple_issuer() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if content_type != Some("application/x-www-form-urlencoded") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(form) = parse_oidc_callback_form(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let state_value = form.state.as_deref().unwrap_or_default();
    let cookie_state = oidc_state_cookie(&headers, state_value);
    oidc_callback_parts(
        &state,
        state_value,
        cookie_state,
        form.code.as_deref(),
        form.error.is_some(),
    )
    .await
}

fn oidc_state_cookie<'a>(headers: &'a axum::http::HeaderMap, state: &str) -> &'a str {
    let cookie_name = oidc_state_cookie_name(state);
    let cookie_prefix = format!("{cookie_name}=");
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(';')
                .map(str::trim)
                .find_map(|part| part.strip_prefix(&cookie_prefix))
        })
        .unwrap_or_default()
}

async fn oidc_callback_parts(
    state: &crate::api::server::AppState,
    state_value: &str,
    cookie_state: &str,
    code: Option<&str>,
    callback_error: bool,
) -> Response {
    let same_site = if state.oidc_auth.is_apple_issuer() {
        "None"
    } else {
        "Lax"
    };
    let identity = if callback_error || code.is_none() {
        Err("provider rejected sign-in")
    } else {
        state
            .oidc_auth
            .finish(state_value, cookie_state, code.unwrap_or_default())
            .await
    };
    let (_email, _subject, return_to) = match identity {
        Ok(identity) => identity,
        Err(_) => return oidc_failure_response(same_site, state_value),
    };
    let credential = state.oidc_auth.new_session_credential();
    let cookie_value =
        crate::auth::session_cookie_value(state.cfg.auth.secret_key.as_deref(), &credential);
    let session_cookie = format!(
        "tng_session={cookie_value}; Path=/; Max-Age={SESSION_TTL_SECS}; HttpOnly; SameSite=Lax; Secure"
    );
    let sid_cookie = format!(
        "SID={cookie_value}; Path=/; Max-Age={SESSION_TTL_SECS}; HttpOnly; SameSite=Lax; Secure"
    );
    let mut response = Redirect::to(&return_to).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&session_cookie)
            .expect("generated OIDC session cookie is a valid header"),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&sid_cookie).expect("generated qBittorrent SID is a valid header"),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{}=; Path=/api/v1/auth/oidc/callback; Max-Age=0; HttpOnly; SameSite={same_site}; Secure",
            oidc_state_cookie_name(state_value)
        ))
        .expect("generated OIDC state cookie is a valid header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn oidc_failure_response(same_site: &str, state: &str) -> Response {
    let mut response = Redirect::to("/?oidc-error=1").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{}=; Path=/api/v1/auth/oidc/callback; Max-Age=0; HttpOnly; SameSite={same_site}; Secure",
            oidc_state_cookie_name(state)
        ))
        .expect("generated OIDC state cookie is a valid header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn oidc_state_cookie_name(state: &str) -> String {
    let digest = Sha256::digest(state.as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("tng_oidc_state_{suffix}")
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    left.len() == right.len() && bool::from(left.ct_eq(right))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn ensure_public_target(url: &Url) -> Result<(), &'static str> {
    resolve_target(url, false)
        .await
        .map(|_| ())
        .map_err(|_| "OIDC endpoint resolves to a private, reserved, or unavailable address")
}

async fn resolve_target(url: &Url, allow_private: bool) -> Result<Vec<SocketAddr>, OidcHttpError> {
    let host = url.host_str().ok_or(OidcHttpError)?;
    let port = url.port_or_known_default().ok_or(OidcHttpError)?;
    let addresses = lookup_host((host, port))
        .await
        .map_err(|_| OidcHttpError)?
        .collect::<Vec<SocketAddr>>();
    if addresses.is_empty()
        || (!allow_private && addresses.iter().any(|addr| !is_public_ip(addr.ip())))
    {
        return Err(OidcHttpError);
    }
    Ok(addresses)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_ipv4(ip),
        IpAddr::V6(ip) => public_ipv6(ip),
    }
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xe000) != 0x2000
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::{
        json_boolean, oidc_state_cookie, oidc_state_cookie_name, parse_oidc_callback_form,
    };
    use axum::http::{header, HeaderMap, HeaderValue};

    #[test]
    fn apple_form_post_parses_single_bounded_callback_fields() {
        let parsed = parse_oidc_callback_form(b"state=flow-1&code=auth-code").unwrap();
        assert_eq!(parsed.state.as_deref(), Some("flow-1"));
        assert_eq!(parsed.code.as_deref(), Some("auth-code"));
        assert!(parsed.error.is_none());
    }

    #[test]
    fn apple_form_post_rejects_duplicate_and_oversized_callback_fields() {
        assert!(parse_oidc_callback_form(b"state=a&state=b&code=auth-code").is_none());
        assert!(parse_oidc_callback_form(b"state=ok&code=one&code=two").is_none());
        let oversized = "x".repeat(16 * 1024 + 1);
        assert!(parse_oidc_callback_form(oversized.as_bytes()).is_none());
    }

    #[test]
    fn apple_email_verified_accepts_only_true_boolean_or_string() {
        assert!(json_boolean(Some(&serde_json::json!(true))));
        assert!(json_boolean(Some(&serde_json::json!("true"))));
        assert!(!json_boolean(Some(&serde_json::json!(false))));
        assert!(!json_boolean(Some(&serde_json::json!("false"))));
        assert!(!json_boolean(Some(&serde_json::json!("yes"))));
    }

    #[test]
    fn state_cookie_names_are_isolated_per_transaction() {
        let first = oidc_state_cookie_name("first-flow");
        let second = oidc_state_cookie_name("second-flow");

        assert!(first.starts_with("tng_oidc_state_"));
        assert_ne!(first, second);
    }

    #[test]
    fn state_cookie_lookup_selects_the_matching_concurrent_flow() {
        let first = oidc_state_cookie_name("first-flow");
        let second = oidc_state_cookie_name("second-flow");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_bytes(format!("{first}=first-flow; {second}=second-flow").as_bytes())
                .unwrap(),
        );

        assert_eq!(oidc_state_cookie(&headers, "second-flow"), "second-flow");
    }
}
