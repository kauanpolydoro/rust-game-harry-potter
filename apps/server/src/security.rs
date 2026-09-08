use axum::{
    extract::{ConnectInfo, MatchedPath, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{AppState, http_support::ApiError};
use std::net::SocketAddr;

mod password_work;
mod rate_limits;
pub(crate) use password_work::PasswordWork;

pub(crate) async fn acknowledge_websocket_close(socket: &mut axum::extract::ws::WebSocket) {
    // The protocol queues its close acknowledgement on receive and flushes it on
    // the next poll. Bound that poll so a peer cannot retain the connection.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), socket.recv()).await;
}
pub(crate) use rate_limits::RateLimits;

pub(crate) async fn response_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}

pub(crate) async fn protect_request(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if request.uri().path().starts_with("/api/") && !state.accepts_traffic().await {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if request
        .uri()
        .path_and_query()
        .is_some_and(|uri| uri.as_str().len() > 2048)
    {
        return ApiError::invalid_request(StatusCode::URI_TOO_LONG).into_response();
    }
    if request
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len() + value.len())
        .sum::<usize>()
        > 16 * 1024
    {
        return ApiError::invalid_request(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE)
            .into_response();
    }
    if !request.method().is_safe() {
        if let Err(error) = require_origin(&state, request.headers()) {
            return error.into_response();
        }
        let mut csrf = request.headers().get_all("x-csrf-protection").iter();
        if csrf.next().is_none_or(|value| value != "1") || csrf.next().is_some() {
            return ApiError::csrf_required().into_response();
        }
    }
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str);
    if route.starts_with("/api/") {
        let sensitive = matches!(
            route,
            "/api/rooms"
                | "/api/session/recover"
                | "/api/session/recovery-password"
                | "/api/rooms/current/protection"
        );
        let discovery = matches!(
            route,
            "/api/rooms/{room_code}" | "/api/rooms/{room_code}/participants"
        );
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map_or_else(|| "unknown-peer".to_owned(), |peer| peer.0.ip().to_string());
        let class = if route.starts_with("/api/telemetry") {
            "telemetry-peer"
        } else if sensitive {
            "password-peer"
        } else if discovery {
            "discovery-peer"
        } else {
            "api-peer"
        };
        let limit = if sensitive || discovery { 60 } else { 600 };
        if !state.admit_request(class, peer.as_bytes(), limit) {
            return ApiError::rate_limited(route == "/api/session/recover").into_response();
        }
    }
    next.run(request).await
}

pub(crate) fn require_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let origin = origins
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::origin_not_allowed)?;
    if origins.next().is_some() || origin != state.application_origin() {
        return Err(ApiError::origin_not_allowed());
    }
    Ok(())
}
