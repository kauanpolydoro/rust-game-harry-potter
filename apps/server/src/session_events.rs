use std::time::Duration;

use axum::{
    Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket, close_code},
    },
    http::{HeaderMap, header},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::{
    AppState,
    http_support::ApiError,
    session::{AuthenticatedSession, authenticated_session, session_is_active},
};

mod postgres;

const SESSION_EVENTS_SUBPROTOCOL: &str = "hogwarts.session.v1";
const SESSION_EVENTS_REPLAY_LIMIT: i64 = 100;
const SESSION_EVENTS_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);
const SESSION_EVENTS_SESSION_REVALIDATION_INTERVAL: Duration = Duration::from_secs(30);
const SESSION_EVENTS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const SESSION_EVENTS_MAX_CONNECTION_AGE: Duration = Duration::from_hours(6);
const SESSION_EVENTS_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionEventsQuery {
    cursor: Option<u64>,
}

#[derive(FromRow)]
struct StoredSecurityPosition {
    room_id: Uuid,
    cursor: i64,
}

#[derive(FromRow, Serialize)]
struct SessionSecurityEvent {
    event_version: i16,
    cursor: i64,
    #[serde(rename = "type")]
    event_type: String,
    actor_position: i16,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_position: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password_generation: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_generation: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_sessions: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_session_preserved: Option<bool>,
    occurred_at: String,
}

#[derive(Serialize)]
struct SecuritySnapshotMessage {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    cursor: i64,
    events: Vec<SessionSecurityEvent>,
}

#[derive(Serialize)]
struct SecurityEventsMessage {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    from_cursor: i64,
    cursor: i64,
    events: Vec<SessionSecurityEvent>,
}

#[derive(Debug)]
enum SessionEventsFailure {
    Database(&'static str),
    Serialize(String),
    Send(String),
    WriteTimeout,
    InvalidPosition,
}

struct SessionEventSignals {
    security: broadcast::Receiver<()>,
    revocation: broadcast::Receiver<()>,
    shutdown: watch::Receiver<bool>,
}

struct SessionEventIntervals {
    reconciliation: tokio::time::Interval,
    session_revalidation: tokio::time::Interval,
    heartbeat: tokio::time::Interval,
}

impl SessionEventIntervals {
    fn new(now: tokio::time::Instant) -> Self {
        Self {
            reconciliation: session_event_interval(
                now + SESSION_EVENTS_RECONCILIATION_INTERVAL,
                SESSION_EVENTS_RECONCILIATION_INTERVAL,
            ),
            session_revalidation: session_event_interval(
                now + SESSION_EVENTS_SESSION_REVALIDATION_INTERVAL,
                SESSION_EVENTS_SESSION_REVALIDATION_INTERVAL,
            ),
            heartbeat: session_event_interval(
                now + SESSION_EVENTS_HEARTBEAT_INTERVAL,
                SESSION_EVENTS_HEARTBEAT_INTERVAL,
            ),
        }
    }
}

impl std::fmt::Display for SessionEventsFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(operation) => {
                write!(formatter, "database operation failed: {operation}")
            }
            Self::Serialize(error) => write!(formatter, "message serialization failed: {error}"),
            Self::Send(error) => write!(formatter, "message send failed: {error}"),
            Self::WriteTimeout => formatter.write_str("message write timed out"),
            Self::InvalidPosition => formatter.write_str("security event position is invalid"),
        }
    }
}

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/api/session/events", get(session_events))
}

async fn session_events(
    State(state): State<AppState>,
    Query(query): Query<SessionEventsQuery>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    require_origin(&state, &headers)?;
    if !websocket
        .requested_protocols()
        .any(|protocol| protocol.as_bytes() == SESSION_EVENTS_SUBPROTOCOL.as_bytes())
    {
        return Err(ApiError::upgrade_required());
    }
    let session = authenticated_session(&state, &headers).await?;
    let position = postgres::load_security_position(&state.database, session.participant_id)
        .await?
        .ok_or_else(ApiError::session_invalid)?;
    let requested_cursor = query.cursor.and_then(|cursor| i64::try_from(cursor).ok());

    Ok(websocket
        .protocols([SESSION_EVENTS_SUBPROTOCOL])
        .max_message_size(1024)
        .on_upgrade(move |socket| {
            serve_session_events(socket, state, session, position.room_id, requested_cursor)
        }))
}

fn require_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
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

async fn serve_session_events(
    mut socket: WebSocket,
    state: AppState,
    session: AuthenticatedSession,
    room_id: Uuid,
    requested_cursor: Option<i64>,
) {
    let signal = state.subscribe_to_security_events(room_id);
    let revocation_signal = state.subscribe_to_session_revocation(session.session_id);
    let shutdown = state.subscribe_to_shutdown();
    if *shutdown.borrow() {
        close_socket(&mut socket, close_code::RESTART, "server is shutting down").await;
        drop(signal);
        drop(revocation_signal);
        state.prune_security_event_channel(room_id);
        state.prune_session_revocation_channel(session.session_id);
        return;
    }
    match session_is_active(&state, session).await {
        Ok(true) => {}
        Ok(false) => {
            close_socket(
                &mut socket,
                close_code::POLICY,
                "session is no longer active",
            )
            .await;
            drop(signal);
            drop(revocation_signal);
            state.prune_security_event_channel(room_id);
            state.prune_session_revocation_channel(session.session_id);
            return;
        }
        Err(_) => {
            tracing::warn!(%room_id, participant_id = %session.participant_id, "initial session revalidation failed");
            close_socket(
                &mut socket,
                close_code::ERROR,
                "session revalidation failed",
            )
            .await;
            drop(signal);
            drop(revocation_signal);
            state.prune_security_event_channel(room_id);
            state.prune_session_revocation_channel(session.session_id);
            return;
        }
    }
    let mut cursor = requested_cursor.unwrap_or(0);
    if let Err(error) = synchronize_until_current(
        &mut socket,
        &state,
        session.participant_id,
        room_id,
        &mut cursor,
        true,
    )
    .await
    {
        tracing::warn!(error = %error, %room_id, participant_id = %session.participant_id, "initial security event synchronization failed");
        close_socket(
            &mut socket,
            close_code::ERROR,
            "initial synchronization failed",
        )
        .await;
        drop(signal);
        drop(revocation_signal);
        state.prune_security_event_channel(room_id);
        state.prune_session_revocation_channel(session.session_id);
        return;
    }

    session_event_loop(
        &mut socket,
        &state,
        session,
        room_id,
        &mut cursor,
        SessionEventSignals {
            security: signal,
            revocation: revocation_signal,
            shutdown,
        },
    )
    .await;
    state.prune_security_event_channel(room_id);
    state.prune_session_revocation_channel(session.session_id);
}

async fn session_event_loop(
    socket: &mut WebSocket,
    state: &AppState,
    session: AuthenticatedSession,
    room_id: Uuid,
    cursor: &mut i64,
    signals: SessionEventSignals,
) {
    let now = tokio::time::Instant::now();
    let mut intervals = SessionEventIntervals::new(now);
    let SessionEventSignals {
        mut security,
        mut revocation,
        mut shutdown,
    } = signals;
    let connection_lifetime = tokio::time::sleep(SESSION_EVENTS_MAX_CONNECTION_AGE);
    tokio::pin!(connection_lifetime);

    loop {
        let synchronize = tokio::select! {
            message = socket.recv() => {
                if !handle_client_message(socket, message).await {
                    return;
                }
                false
            }
            notification = security.recv() => {
                if matches!(notification, Ok(()) | Err(broadcast::error::RecvError::Lagged(_))) {
                    match session_is_active(state, session).await {
                        Ok(true) => true,
                        Ok(false) => {
                            close_socket(socket, close_code::POLICY, "session is no longer active").await;
                            return;
                        }
                        Err(_) => {
                            close_socket(socket, close_code::ERROR, "session revalidation failed").await;
                            return;
                        }
                    }
                } else {
                    false
                }
            }
            notification = revocation.recv() => {
                if matches!(notification, Ok(()) | Err(broadcast::error::RecvError::Lagged(_))) {
                    match session_is_active(state, session).await {
                        Ok(false) => {
                            close_socket(socket, close_code::POLICY, "session is no longer active").await;
                            return;
                        }
                        Ok(true) => false,
                        Err(_) => {
                            close_socket(socket, close_code::ERROR, "session revalidation failed").await;
                            return;
                        }
                    }
                } else {
                    false
                }
            }
            _ = intervals.reconciliation.tick() => true,
            _ = intervals.session_revalidation.tick() => {
                if let Ok(true) = session_is_active(state, session).await {
                    false
                } else {
                    close_socket(socket, close_code::POLICY, "session is no longer active").await;
                    return;
                }
            }
            _ = intervals.heartbeat.tick() => {
                if send_message(socket, Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
                false
            }
            () = &mut connection_lifetime => {
                close_socket(socket, close_code::AWAY, "connection lifetime reached").await;
                return;
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    close_socket(socket, close_code::RESTART, "server is shutting down").await;
                    return;
                }
                false
            }
        };
        if synchronize
            && let Err(error) = synchronize_until_current(
                socket,
                state,
                session.participant_id,
                room_id,
                cursor,
                false,
            )
            .await
        {
            tracing::warn!(error = %error, %room_id, participant_id = %session.participant_id, "security event synchronization failed");
            close_socket(socket, close_code::ERROR, "synchronization failed").await;
            return;
        }
    }
}

fn session_event_interval(
    first_tick: tokio::time::Instant,
    period: Duration,
) -> tokio::time::Interval {
    let mut interval = tokio::time::interval_at(first_tick, period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

async fn handle_client_message(
    socket: &mut WebSocket,
    message: Option<Result<Message, axum::Error>>,
) -> bool {
    match message {
        Some(Ok(Message::Close(_)) | Err(_)) | None => false,
        Some(Ok(Message::Text(_) | Message::Binary(_))) => {
            close_socket(
                socket,
                close_code::UNSUPPORTED,
                "client data messages are not supported",
            )
            .await;
            false
        }
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => true,
    }
}

async fn synchronize_until_current(
    socket: &mut WebSocket,
    state: &AppState,
    participant_id: Uuid,
    room_id: Uuid,
    cursor: &mut i64,
    mut initial: bool,
) -> Result<(), SessionEventsFailure> {
    loop {
        let position = postgres::load_security_position(&state.database, participant_id)
            .await
            .map_err(|_| SessionEventsFailure::Database("load security position"))?
            .filter(|position| position.room_id == room_id)
            .ok_or(SessionEventsFailure::InvalidPosition)?;
        if *cursor < 0 || *cursor > position.cursor {
            *cursor = 0;
        }
        let from_cursor = *cursor;
        let events = postgres::load_security_events(
            &state.database,
            room_id,
            participant_id,
            from_cursor,
            position.cursor,
            SESSION_EVENTS_REPLAY_LIMIT,
        )
        .await
        .map_err(|_| SessionEventsFailure::Database("load security events"))?;
        let next_cursor = if i64::try_from(events.len()).ok() == Some(SESSION_EVENTS_REPLAY_LIMIT) {
            events.last().map_or(position.cursor, |event| event.cursor)
        } else {
            position.cursor
        };
        if !initial && next_cursor == from_cursor {
            return Ok(());
        }
        if initial {
            send_json(
                socket,
                &SecuritySnapshotMessage {
                    protocol_version: 1,
                    message_type: "security_snapshot",
                    cursor: next_cursor,
                    events,
                },
            )
            .await?;
        } else {
            send_json(
                socket,
                &SecurityEventsMessage {
                    protocol_version: 1,
                    message_type: "security_events",
                    from_cursor,
                    cursor: next_cursor,
                    events,
                },
            )
            .await?;
        }
        *cursor = next_cursor;
        initial = false;
        if next_cursor >= position.cursor {
            return Ok(());
        }
    }
}

async fn send_json(
    socket: &mut WebSocket,
    value: &impl Serialize,
) -> Result<(), SessionEventsFailure> {
    let serialized = serde_json::to_string(value)
        .map_err(|error| SessionEventsFailure::Serialize(error.to_string()))?;
    send_message(socket, Message::Text(serialized.into())).await
}

async fn send_message(
    socket: &mut WebSocket,
    message: Message,
) -> Result<(), SessionEventsFailure> {
    tokio::time::timeout(SESSION_EVENTS_WRITE_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| SessionEventsFailure::WriteTimeout)?
        .map_err(|error| SessionEventsFailure::Send(error.to_string()))
}

async fn close_socket(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = send_message(
        socket,
        Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })),
    )
    .await;
}
