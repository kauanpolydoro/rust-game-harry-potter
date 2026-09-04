use axum::http::{HeaderMap, header};
use uuid::Uuid;

use crate::{AppState, http_support::ApiError};

const SESSION_BYTES: usize = 32;

#[derive(Clone, Copy)]
pub(crate) struct AuthenticatedSession {
    pub(crate) session_id: Uuid,
    pub(crate) participant_id: Uuid,
}

pub(crate) async fn authenticated_participant(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Uuid, ApiError> {
    authenticated_session(state, headers)
        .await
        .map(|session| session.participant_id)
}

pub(crate) async fn authenticated_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedSession, ApiError> {
    let token = session_token(headers)?;
    sqlx::query_as::<_, (Uuid, Uuid)>(
        r"
        SELECT guest_sessions.id, device_sessions.participant_id
        FROM guest_sessions
        JOIN device_sessions ON device_sessions.guest_session_id = guest_sessions.id
        WHERE guest_sessions.token_digest =
              'sha256:' || encode(sha256(convert_to($1, 'UTF8')), 'hex')
          AND guest_sessions.expires_at > clock_timestamp()
          AND device_sessions.status = 'active'
        ",
    )
    .bind(token)
    .fetch_optional(&state.database)
    .await
    .map_err(|error| ApiError::internal_with("authenticate guest session", error))?
    .map(|(session_id, participant_id)| AuthenticatedSession {
        session_id,
        participant_id,
    })
    .ok_or_else(ApiError::session_invalid)
}

pub(crate) async fn session_is_active(
    state: &AppState,
    session: AuthenticatedSession,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        r"
        SELECT EXISTS(
            SELECT 1
            FROM guest_sessions
            JOIN device_sessions ON device_sessions.guest_session_id = guest_sessions.id
            WHERE guest_sessions.id = $1
              AND device_sessions.participant_id = $2
              AND guest_sessions.expires_at > clock_timestamp()
              AND device_sessions.status = 'active'
        )
        ",
    )
    .bind(session.session_id)
    .bind(session.participant_id)
    .fetch_one(&state.database)
    .await
    .map_err(|error| ApiError::internal_with("revalidate guest session", error))
}

fn session_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .map(str::trim)
        .find_map(|cookie| cookie.strip_prefix("__Host-session="))
        .filter(|token| {
            token.len() == SESSION_BYTES * 2 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(ApiError::session_invalid)
}
