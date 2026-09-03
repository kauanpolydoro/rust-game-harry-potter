use std::time::Duration;

use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket, close_code},
    },
    http::{HeaderMap, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use super::codec::decode_persisted_event;
use super::{GameProjectionResponse, StoredGameEvent, postgres, projection_for_participant};
use crate::{
    AppState,
    http_support::ApiError,
    session::{AuthenticatedSession, authenticated_session, session_is_active},
};

const REALTIME_PROTOCOL_VERSION: u16 = 1;
const REALTIME_SUBPROTOCOL: &str = "hogwarts.realtime.v1";
const REALTIME_REPLAY_LIMIT: u64 = 100;
const REALTIME_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);
const REALTIME_SESSION_REVALIDATION_INTERVAL: Duration = Duration::from_secs(30);
const REALTIME_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const REALTIME_CLIENT_WATCHDOG: Duration = Duration::from_secs(60);
const REALTIME_MAX_CONNECTION_AGE: Duration = Duration::from_hours(6);
const REALTIME_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
enum RealtimeFailure {
    Database(&'static str),
    InvalidData(&'static str),
    Serialize(String),
    Send(String),
    WriteTimeout,
}

impl std::fmt::Display for RealtimeFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(operation) => {
                write!(formatter, "database operation failed: {operation}")
            }
            Self::InvalidData(reason) => write!(formatter, "invalid persisted data: {reason}"),
            Self::Serialize(error) => write!(formatter, "message serialization failed: {error}"),
            Self::Send(error) => write!(formatter, "message send failed: {error}"),
            Self::WriteTimeout => formatter.write_str("message write timed out"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RealtimeQuery {
    cursor: Option<u64>,
    snapshot_version: Option<u16>,
}

#[derive(Serialize)]
struct RealtimeSnapshotMessage {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    cursor: i64,
    projection: GameProjectionResponse,
}

#[derive(Serialize)]
struct RealtimeEventBatchMessage {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    from_cursor: i64,
    cursor: i64,
    events: Vec<RealtimeGameEvent>,
    projection: GameProjectionResponse,
}

#[derive(Serialize)]
struct RealtimeGameEvent {
    event_version: i16,
    #[serde(rename = "type")]
    event_type: &'static str,
    sequence: i64,
    state_version: i64,
    turn: u32,
    actor_position: i16,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
}

#[derive(Clone, Copy)]
struct RealtimePosition {
    cursor: Option<i64>,
    snapshot_version: Option<u16>,
}

enum RealtimeLoopAction {
    Continue,
    Synchronize,
    Stop,
}

pub(super) async fn game_events(
    State(state): State<AppState>,
    Query(query): Query<RealtimeQuery>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    require_realtime_origin(&state, &headers)?;
    if !websocket
        .requested_protocols()
        .any(|protocol| protocol.as_bytes() == REALTIME_SUBPROTOCOL.as_bytes())
    {
        return Err(ApiError::upgrade_required());
    }

    let session = authenticated_session(&state, &headers).await?;
    let participant_id = session.participant_id;
    let projection = projection_for_participant(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::game_action_not_allowed)?;
    let game_id = Uuid::parse_str(&projection.game.id)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;

    Ok(websocket
        .protocols([REALTIME_SUBPROTOCOL])
        .max_message_size(4 * 1024)
        .on_upgrade(move |socket| serve_game_events(socket, state, session, game_id, query)))
}

fn require_realtime_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
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

async fn serve_game_events(
    mut socket: WebSocket,
    state: AppState,
    session: AuthenticatedSession,
    game_id: Uuid,
    query: RealtimeQuery,
) {
    let participant_id = session.participant_id;
    let mut position = RealtimePosition {
        cursor: query.cursor.and_then(|value| i64::try_from(value).ok()),
        snapshot_version: query.snapshot_version,
    };
    let signal = state.subscribe_to_game_events(game_id);
    let shutdown = state.subscribe_to_shutdown();
    if *shutdown.borrow() {
        close_socket(&mut socket, close_code::RESTART, "server is shutting down").await;
        drop(signal);
        state.prune_game_event_channel(game_id);
        return;
    }
    let force_initial_snapshot = query.cursor.is_none() || position.cursor.is_none();
    if let Err(error) = synchronize_socket(
        &mut socket,
        &state,
        participant_id,
        game_id,
        &mut position,
        force_initial_snapshot,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            %game_id,
            %participant_id,
            "initial realtime synchronization failed"
        );
        close_socket(
            &mut socket,
            close_code::ERROR,
            "initial synchronization failed",
        )
        .await;
        drop(signal);
        state.prune_game_event_channel(game_id);
        return;
    }

    realtime_event_loop(
        &mut socket,
        &state,
        session,
        game_id,
        &mut position,
        signal,
        shutdown,
    )
    .await;
    state.prune_game_event_channel(game_id);
}

async fn realtime_event_loop(
    socket: &mut WebSocket,
    state: &AppState,
    session: AuthenticatedSession,
    game_id: Uuid,
    position: &mut RealtimePosition,
    mut signal: broadcast::Receiver<()>,
    mut shutdown: watch::Receiver<bool>,
) {
    let participant_id = session.participant_id;
    let now = tokio::time::Instant::now();
    let mut reconciliation = tokio::time::interval_at(
        now + REALTIME_RECONCILIATION_INTERVAL + reconciliation_jitter(game_id, participant_id),
        REALTIME_RECONCILIATION_INTERVAL,
    );
    reconciliation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut session_revalidation = tokio::time::interval_at(
        now + REALTIME_SESSION_REVALIDATION_INTERVAL,
        REALTIME_SESSION_REVALIDATION_INTERVAL,
    );
    session_revalidation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut heartbeat = tokio::time::interval_at(
        now + REALTIME_HEARTBEAT_INTERVAL,
        REALTIME_HEARTBEAT_INTERVAL,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let connection_lifetime = tokio::time::sleep(REALTIME_MAX_CONNECTION_AGE);
    tokio::pin!(connection_lifetime);
    let mut last_client_activity = now;

    loop {
        let action = tokio::select! {
            message = socket.recv() => {
                handle_client_message(
                    socket,
                    message,
                    game_id,
                    participant_id,
                    &mut last_client_activity,
                )
                .await
            }
            notification = signal.recv() => {
                match notification {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        RealtimeLoopAction::Synchronize
                    }
                    Err(broadcast::error::RecvError::Closed) => RealtimeLoopAction::Continue,
                }
            }
            _ = reconciliation.tick() => RealtimeLoopAction::Synchronize,
            _ = session_revalidation.tick() => {
                validate_realtime_session(socket, state, session, game_id).await
            }
            _ = heartbeat.tick() => {
                send_realtime_heartbeat(
                    socket,
                    game_id,
                    participant_id,
                    last_client_activity,
                )
                .await
            }
            () = &mut connection_lifetime => {
                close_socket(socket, close_code::AWAY, "connection lifetime reached").await;
                RealtimeLoopAction::Stop
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    close_socket(
                        socket,
                        close_code::RESTART,
                        "server is shutting down",
                    )
                    .await;
                    RealtimeLoopAction::Stop
                } else {
                    RealtimeLoopAction::Continue
                }
            }
        };

        match action {
            RealtimeLoopAction::Continue => {}
            RealtimeLoopAction::Stop => return,
            RealtimeLoopAction::Synchronize => {
                if !synchronize_connection(socket, state, session, game_id, position).await {
                    return;
                }
            }
        }
    }
}

async fn handle_client_message(
    socket: &mut WebSocket,
    message: Option<Result<Message, axum::Error>>,
    game_id: Uuid,
    participant_id: Uuid,
    last_client_activity: &mut tokio::time::Instant,
) -> RealtimeLoopAction {
    match message {
        Some(Ok(Message::Close(frame))) => {
            tracing::info!(
                %game_id,
                %participant_id,
                ?frame,
                "realtime client closed the connection"
            );
            RealtimeLoopAction::Stop
        }
        Some(Ok(Message::Text(_) | Message::Binary(_))) => {
            close_socket(
                socket,
                close_code::UNSUPPORTED,
                "client data messages are not supported",
            )
            .await;
            RealtimeLoopAction::Stop
        }
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
            *last_client_activity = tokio::time::Instant::now();
            RealtimeLoopAction::Continue
        }
        Some(Err(error)) => {
            tracing::warn!(
                error = %error,
                %game_id,
                %participant_id,
                "realtime receive failed"
            );
            RealtimeLoopAction::Stop
        }
        None => {
            tracing::info!(%game_id, %participant_id, "realtime peer disconnected");
            RealtimeLoopAction::Stop
        }
    }
}

async fn send_realtime_heartbeat(
    socket: &mut WebSocket,
    game_id: Uuid,
    participant_id: Uuid,
    last_client_activity: tokio::time::Instant,
) -> RealtimeLoopAction {
    if last_client_activity.elapsed() >= REALTIME_CLIENT_WATCHDOG {
        tracing::warn!(%game_id, %participant_id, "realtime client watchdog expired");
        close_socket(socket, close_code::POLICY, "heartbeat timeout").await;
        return RealtimeLoopAction::Stop;
    }

    match tokio::time::timeout(
        REALTIME_WRITE_TIMEOUT,
        socket.send(Message::Ping(Vec::new().into())),
    )
    .await
    {
        Ok(Ok(())) => RealtimeLoopAction::Continue,
        Ok(Err(error)) => {
            tracing::warn!(
                error = %error,
                %game_id,
                %participant_id,
                "realtime heartbeat send failed"
            );
            RealtimeLoopAction::Stop
        }
        Err(_) => {
            tracing::warn!(%game_id, %participant_id, "realtime heartbeat write timed out");
            RealtimeLoopAction::Stop
        }
    }
}

async fn validate_realtime_session(
    socket: &mut WebSocket,
    state: &AppState,
    session: AuthenticatedSession,
    game_id: Uuid,
) -> RealtimeLoopAction {
    match revalidate_socket_session(state, session, game_id).await {
        Ok(true) => RealtimeLoopAction::Continue,
        Ok(false) => {
            close_socket(socket, close_code::POLICY, "session is no longer active").await;
            RealtimeLoopAction::Stop
        }
        Err(_) => {
            close_socket(socket, close_code::ERROR, "session revalidation failed").await;
            RealtimeLoopAction::Stop
        }
    }
}

async fn synchronize_connection(
    socket: &mut WebSocket,
    state: &AppState,
    session: AuthenticatedSession,
    game_id: Uuid,
    position: &mut RealtimePosition,
) -> bool {
    match revalidate_socket_session(state, session, game_id).await {
        Ok(true) => {}
        Ok(false) => {
            close_socket(socket, close_code::POLICY, "session is no longer active").await;
            return false;
        }
        Err(_) => {
            close_socket(socket, close_code::ERROR, "session revalidation failed").await;
            return false;
        }
    }

    if let Err(error) = synchronize_socket(
        socket,
        state,
        session.participant_id,
        game_id,
        position,
        false,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            %game_id,
            participant_id = %session.participant_id,
            "realtime synchronization failed"
        );
        close_socket(socket, close_code::ERROR, "synchronization failed").await;
        return false;
    }
    true
}
async fn revalidate_socket_session(
    state: &AppState,
    session: AuthenticatedSession,
    game_id: Uuid,
) -> Result<bool, ApiError> {
    session_is_active(state, session)
        .await
        .inspect_err(|_error| {
            tracing::warn!(
                %game_id,
                participant_id = %session.participant_id,
                "realtime session revalidation failed"
            );
        })
}

fn reconciliation_jitter(game_id: Uuid, participant_id: Uuid) -> Duration {
    let game = game_id.as_bytes();
    let participant = participant_id.as_bytes();
    let mixed = u16::from_be_bytes([game[0] ^ participant[0], game[1] ^ participant[1]]);
    Duration::from_millis(u64::from(mixed % 5_000))
}

async fn close_socket(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let result = tokio::time::timeout(
        REALTIME_WRITE_TIMEOUT,
        socket.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(error = %error, code, reason, "realtime close failed"),
        Err(_) => tracing::warn!(code, reason, "realtime close write timed out"),
    }
}

async fn synchronize_socket(
    socket: &mut WebSocket,
    state: &AppState,
    participant_id: Uuid,
    game_id: Uuid,
    position: &mut RealtimePosition,
    force_snapshot: bool,
) -> Result<(), RealtimeFailure> {
    let observed = load_realtime_position(&state.database, participant_id, game_id).await?;
    if !force_snapshot
        && position.cursor == observed.cursor
        && position.snapshot_version == observed.snapshot_version
    {
        return Ok(());
    }

    let projection = match projection_for_participant(&state.database, participant_id).await {
        Ok(Some(projection)) if projection.game.id == game_id.to_string() => projection,
        Ok(Some(_) | None) => {
            return Err(RealtimeFailure::InvalidData(
                "participant projection does not match the socket game",
            ));
        }
        Err(_) => return Err(RealtimeFailure::Database("load participant projection")),
    };
    let current_cursor = projection.snapshot.cursor;
    let Ok(current_snapshot_version) = u16::try_from(projection.snapshot.snapshot_version) else {
        return Err(RealtimeFailure::InvalidData(
            "projection Snapshot version cannot be represented",
        ));
    };
    let requested_cursor = position.cursor;
    let replay_distance = requested_cursor
        .and_then(|value| current_cursor.checked_sub(value))
        .and_then(|value| u64::try_from(value).ok());
    let needs_snapshot = force_snapshot
        || position.snapshot_version != Some(current_snapshot_version)
        || requested_cursor.is_none_or(|value| value > current_cursor)
        || replay_distance.is_none_or(|distance| distance > REALTIME_REPLAY_LIMIT);

    if needs_snapshot {
        send_realtime_snapshot(
            socket,
            projection,
            current_cursor,
            current_snapshot_version,
            position,
        )
        .await?;
        return Ok(());
    }

    let Some(from_cursor) = requested_cursor else {
        return Err(RealtimeFailure::InvalidData(
            "compatible realtime cursor is missing",
        ));
    };
    if from_cursor == current_cursor {
        return Ok(());
    }
    let events = match postgres::game_events_for_participant(
        &state.database,
        participant_id,
        game_id,
        from_cursor,
        current_cursor,
    )
    .await
    {
        Ok(stored) => stored
            .iter()
            .map(|event| realtime_event(event, participant_id))
            .collect::<Result<Vec<_>, _>>(),
        Err(error) => Err(error),
    };
    let events = events.map_err(|_| RealtimeFailure::InvalidData("decode persisted events"))?;
    if !events_are_contiguous(&events, from_cursor, current_cursor) {
        send_realtime_snapshot(
            socket,
            projection,
            current_cursor,
            current_snapshot_version,
            position,
        )
        .await?;
        return Ok(());
    }

    let message = RealtimeEventBatchMessage {
        protocol_version: REALTIME_PROTOCOL_VERSION,
        message_type: "events",
        from_cursor,
        cursor: current_cursor,
        events,
        projection,
    };
    send_realtime_message(socket, &message).await?;
    position.cursor = Some(current_cursor);
    position.snapshot_version = Some(current_snapshot_version);
    Ok(())
}

async fn load_realtime_position(
    database: &sqlx::PgPool,
    participant_id: Uuid,
    game_id: Uuid,
) -> Result<RealtimePosition, RealtimeFailure> {
    let stored = postgres::game_cursor_for_participant(database, participant_id, game_id)
        .await
        .map_err(|_| RealtimeFailure::Database("load game cursor"))?
        .ok_or(RealtimeFailure::InvalidData("game cursor is unavailable"))?;
    let snapshot_version = u16::try_from(stored.1).map_err(|_| {
        RealtimeFailure::InvalidData("persisted Snapshot version cannot be represented")
    })?;
    Ok(RealtimePosition {
        cursor: Some(stored.0),
        snapshot_version: Some(snapshot_version),
    })
}

async fn send_realtime_snapshot(
    socket: &mut WebSocket,
    projection: GameProjectionResponse,
    cursor: i64,
    snapshot_version: u16,
    position: &mut RealtimePosition,
) -> Result<(), RealtimeFailure> {
    let message = RealtimeSnapshotMessage {
        protocol_version: REALTIME_PROTOCOL_VERSION,
        message_type: "snapshot",
        cursor,
        projection,
    };
    send_realtime_message(socket, &message).await?;
    position.cursor = Some(cursor);
    position.snapshot_version = Some(snapshot_version);
    Ok(())
}

fn realtime_event(
    stored: &StoredGameEvent,
    participant_id: Uuid,
) -> Result<RealtimeGameEvent, ApiError> {
    let payload = decode_persisted_event(&stored.payload_json)?;
    let metadata_matches = i16::try_from(payload.event_version).ok() == Some(stored.event_version)
        && payload.event_type == stored.event_type
        && i64::try_from(payload.sequence).ok() == Some(stored.sequence)
        && i64::try_from(payload.state_version).ok() == Some(stored.state_version)
        && i16::from(payload.actor_position) == stored.actor_position;
    if !metadata_matches || stored.event_type != "dark_arts_completed" {
        return Err(ApiError::internal());
    }

    Ok(RealtimeGameEvent {
        event_version: stored.event_version,
        event_type: "dark_arts_completed",
        sequence: stored.sequence,
        state_version: stored.state_version,
        turn: payload.turn,
        actor_position: stored.actor_position,
        command_id: (stored.actor_participant_id == participant_id)
            .then(|| stored.command_id.to_string()),
    })
}

fn events_are_contiguous(
    events: &[RealtimeGameEvent],
    from_cursor: i64,
    current_cursor: i64,
) -> bool {
    let Some(expected_count) = current_cursor
        .checked_sub(from_cursor)
        .and_then(|count| usize::try_from(count).ok())
    else {
        return false;
    };
    events.len() == expected_count
        && events
            .iter()
            .zip((from_cursor + 1)..=current_cursor)
            .all(|(event, expected)| event.sequence == expected)
}

async fn send_realtime_message(
    socket: &mut WebSocket,
    value: &impl Serialize,
) -> Result<(), RealtimeFailure> {
    let serialized = serde_json::to_string(value)
        .map_err(|error| RealtimeFailure::Serialize(error.to_string()))?;
    tokio::time::timeout(
        REALTIME_WRITE_TIMEOUT,
        socket.send(Message::Text(serialized.into())),
    )
    .await
    .map_err(|_| RealtimeFailure::WriteTimeout)?
    .map_err(|error| RealtimeFailure::Send(error.to_string()))
}
