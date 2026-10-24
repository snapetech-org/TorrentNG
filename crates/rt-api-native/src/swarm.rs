//! Native-only Swarm Switch routes. Mutations share normal authentication/CSRF.
use crate::{
    handlers::{require_mutation_auth, validate_native_info_hash},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine as _;
use rt_engine::engine::swarm_switch::SwarmVariant;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRequest {
    #[serde(default)]
    variant: SwarmVariant,
    public_torrent_base64: Option<String>,
    expected_public_hash: Option<String>,
}

fn error(code: StatusCode, message: impl Into<String>) -> Response {
    (
        code,
        Json(serde_json::json!({"error": {"code": "swarm_switch", "message": message.into()}})),
    )
        .into_response()
}

fn metainfo(request: &CandidateRequest) -> Result<Option<Vec<u8>>, (StatusCode, &'static str)> {
    request
        .public_torrent_base64
        .as_ref()
        .map(|raw| {
            if raw.len() > 11_184_812 {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Public metainfo exceeds 8 MiB",
                ));
            }
            let bytes = base64::prelude::BASE64_STANDARD.decode(raw).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Public metainfo is not valid base64",
                )
            })?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Public metainfo exceeds 8 MiB",
                ));
            }
            Ok(bytes)
        })
        .transpose()
}

pub async fn status(State(state): State<AppState>, Path(hash): Path<String>) -> Response {
    if let Err(e) = validate_native_info_hash(&hash) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let Some(engine) = &state.engine else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Swarm Switch requires the TorrentNG engine",
        );
    };
    match engine.swarm_status(hash).await {
        Ok(status) => Json(status).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

pub async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(hash): Path<String>,
    Json(request): Json<CandidateRequest>,
) -> Response {
    // Comparison performs network probes. Require the same write authorization
    // as an explicit torrent action, including for a nominally read-only result.
    if let Some(response) = require_mutation_auth(&state, &headers) {
        return response;
    }
    if let Err(e) = validate_native_info_hash(&hash) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let raw = match metainfo(&request) {
        Ok(raw) => raw,
        Err((status, message)) => return error(status, message),
    };
    let Some(engine) = &state.engine else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Swarm Switch requires the TorrentNG engine",
        );
    };
    match engine.preview_swarm(hash, request.variant, raw).await {
        Ok(preview) => Json(preview).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

pub async fn public(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(hash): Path<String>,
    Json(request): Json<CandidateRequest>,
) -> Response {
    if let Some(response) = require_mutation_auth(&state, &headers) {
        return response;
    }
    if let Err(e) = validate_native_info_hash(&hash) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let raw = match metainfo(&request) {
        Ok(raw) => raw,
        Err((status, message)) => return error(status, message),
    };
    let Some(expected) = request.expected_public_hash else {
        return error(
            StatusCode::BAD_REQUEST,
            "Compare swarms before selecting the public candidate",
        );
    };
    let Some(engine) = &state.engine else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Swarm Switch requires the TorrentNG engine",
        );
    };
    match engine
        .switch_to_public(hash, request.variant, raw, expected)
        .await
    {
        Ok(status) => Json(status).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

pub async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(hash): Path<String>,
) -> Response {
    if let Some(response) = require_mutation_auth(&state, &headers) {
        return response;
    }
    if let Err(e) = validate_native_info_hash(&hash) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let Some(engine) = &state.engine else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Swarm Switch requires the TorrentNG engine",
        );
    };
    match engine.restore_private_swarm(hash).await {
        Ok(status) => Json(status).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn swarm_routes_require_auth_and_reject_invalid_requests_before_engine_work() {
        let state = AppState::with_tokens(None, vec!["secret".into()]);
        let app = crate::router::build_router(state);
        let path = format!("/api/v1/torrents/{}/swarm-switch/preview", "a".repeat(40));
        let response = app
            .clone()
            .oneshot(
                Request::post(&path)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        for (body, expected) in [
            (
                r#"{"variant":"invented"}"#,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (r#"{"force":true}"#, StatusCode::UNPROCESSABLE_ENTITY),
            (
                r#"{"variant":"supplied","public_torrent_base64":"!"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (r#"{}"#, StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post(&path)
                        .header("authorization", "Bearer secret")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{body}");
        }
        let path = path.replace("/preview", "/public");
        let response = app
            .oneshot(
                Request::post(path)
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
