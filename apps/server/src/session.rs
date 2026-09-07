use axum::http::{HeaderMap, header};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{AppState, game_expiration, http_support::ApiError};

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
    let (session, active) = resolve_presented_session(state, headers).await?;
    if !active {
        return Err(ApiError::session_invalid());
    }
    Ok(session)
}

pub(crate) async fn presented_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedSession, ApiError> {
    resolve_presented_session(state, headers)
        .await
        .map(|(session, _)| session)
}

async fn resolve_presented_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(AuthenticatedSession, bool), ApiError> {
    let token = session_token(headers)?;
    loop {
        let (session_id, participant_id, active, expired) = sqlx::query_as::<_, (Uuid, Uuid, bool, bool)>(
        r"
        SELECT guest_sessions.id, device_sessions.participant_id,
               device_sessions.status = 'active',
               COALESCE(games.access_expired_at IS NOT NULL OR clock_timestamp() >= games.expires_at, FALSE)
        FROM guest_sessions
        JOIN device_sessions ON device_sessions.guest_session_id = guest_sessions.id
        JOIN participants ON participants.id = device_sessions.participant_id
        LEFT JOIN games ON games.room_id = participants.room_id
        WHERE guest_sessions.token_digest =
              'sha256:' || encode(sha256(convert_to($1, 'UTF8')), 'hex')
          AND guest_sessions.expires_at > clock_timestamp()
        ",
    )
    .bind(token)
    .fetch_optional(&state.database)
    .await
    .map_err(|error| ApiError::internal_with("resolve presented guest session", error))?
    .ok_or_else(ApiError::session_invalid)?;
        if expired {
            if game_expiration::participant_game_expired(&state.database, participant_id).await? {
                return Err(ApiError::game_expired());
            }
            // A renewal won the root lock. Session revocation may also have
            // committed during that wait, so neither old flag is authoritative.
            continue;
        }
        return Ok((
            AuthenticatedSession {
                session_id,
                participant_id,
            },
            active,
        ));
    }
}

pub(crate) async fn session_is_active(
    state: &AppState,
    session: AuthenticatedSession,
) -> Result<bool, ApiError> {
    loop {
        let status = sqlx::query_as::<_, (bool, bool)>(
            r"
            SELECT device_sessions.status = 'active' AND guest_sessions.expires_at > clock_timestamp(),
                   COALESCE(games.access_expired_at IS NOT NULL OR clock_timestamp() >= games.expires_at, FALSE)
            FROM guest_sessions
            JOIN device_sessions ON device_sessions.guest_session_id = guest_sessions.id
            JOIN participants ON participants.id = device_sessions.participant_id
            LEFT JOIN games ON games.room_id = participants.room_id
            WHERE guest_sessions.id = $1 AND device_sessions.participant_id = $2
            ",
        )
        .bind(session.session_id)
        .bind(session.participant_id)
        .fetch_optional(&state.database)
        .await
        .map_err(|error| ApiError::internal_with("revalidate guest session", error))?;
        let Some((active, expired)) = status else {
            return Ok(false);
        };
        if expired
            && !game_expiration::participant_game_expired(&state.database, session.participant_id)
                .await?
        {
            // The root holder renewed retention while we waited. Revalidate
            // both session status and expiration against its committed state.
            continue;
        }
        return Ok(active && !expired);
    }
}

pub(crate) async fn session_is_active_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    session: AuthenticatedSession,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        r"
        SELECT EXISTS(
            SELECT 1
            FROM guest_sessions
            JOIN device_sessions ON device_sessions.guest_session_id = guest_sessions.id
            JOIN participants ON participants.id = device_sessions.participant_id
            LEFT JOIN games ON games.room_id = participants.room_id
            WHERE guest_sessions.id = $1
              AND device_sessions.participant_id = $2
              AND guest_sessions.expires_at > clock_timestamp()
              AND device_sessions.status = 'active'
              AND (games.id IS NULL OR (games.access_expired_at IS NULL AND games.expires_at > clock_timestamp()))
        )
        ",
    )
    .bind(session.session_id)
    .bind(session.participant_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("revalidate guest session in transaction", error))
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

/// Private-use WebSocket close code shared by the game and security channels.
pub(crate) async fn inactive_session_close_code(
    state: &AppState,
    session: AuthenticatedSession,
) -> u16 {
    if game_expiration::participant_game_expired(&state.database, session.participant_id)
        .await
        .unwrap_or(false)
    {
        4001
    } else {
        1008
    }
}
