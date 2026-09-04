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

use super::codec::{command_domain_state, decode_persisted_event, decode_persisted_snapshot};
use super::{GameProjectionResponse, StoredGameEvent, postgres, projection_for_participant};
use crate::{
    AppState,
    http_support::ApiError,
    session::{AuthenticatedSession, authenticated_session, session_is_active},
};

const REALTIME_SUBPROTOCOL_V1: &str = "hogwarts.realtime.v1";
const REALTIME_SUBPROTOCOL_V2: &str = "hogwarts.realtime.v2";
const REALTIME_REPLAY_LIMIT: u64 = 100;
const REALTIME_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);
const REALTIME_SESSION_REVALIDATION_INTERVAL: Duration = Duration::from_secs(30);
const REALTIME_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const REALTIME_CLIENT_WATCHDOG: Duration = Duration::from_secs(60);
const REALTIME_PRESENCE_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(5);
const REALTIME_ONLINE_WINDOW_SECONDS: i64 = 40;
const REALTIME_RECONNECTING_WINDOW_SECONDS: i64 = 60;
const REALTIME_MAX_CONNECTION_AGE: Duration = Duration::from_hours(6);
const REALTIME_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum RealtimeProtocol {
    V1,
    V2,
}

impl RealtimeProtocol {
    const fn version(self) -> u16 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    const fn subprotocol(self) -> &'static str {
        match self {
            Self::V1 => REALTIME_SUBPROTOCOL_V1,
            Self::V2 => REALTIME_SUBPROTOCOL_V2,
        }
    }

    const fn publishes_presence(self) -> bool {
        matches!(self, Self::V2)
    }
}

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
    digest: Option<String>,
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
struct RealtimeSynchronizedMessage<'a> {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    cursor: i64,
    snapshot_version: u16,
    digest: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct RealtimePresenceMessage {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    game_id: String,
    participants: Vec<RealtimeParticipantPresence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_participant_position: Option<i16>,
    blocked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct RealtimeParticipantPresence {
    position: i16,
    status: &'static str,
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

struct RealtimePosition {
    cursor: Option<i64>,
    snapshot_version: Option<u16>,
    digest: Option<String>,
    synchronized: bool,
}

#[derive(Clone, Copy)]
struct RealtimeConnectionContext<'a> {
    state: &'a AppState,
    session: AuthenticatedSession,
    game_id: Uuid,
    connection_id: Uuid,
    protocol: RealtimeProtocol,
}

enum RealtimeLoopAction {
    Continue,
    Presence,
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
    let mut accepts_v1 = false;
    let mut accepts_v2 = false;
    for requested in websocket.requested_protocols() {
        accepts_v1 |= requested.as_bytes() == REALTIME_SUBPROTOCOL_V1.as_bytes();
        accepts_v2 |= requested.as_bytes() == REALTIME_SUBPROTOCOL_V2.as_bytes();
    }
    let protocol = if accepts_v2 {
        RealtimeProtocol::V2
    } else if accepts_v1 {
        RealtimeProtocol::V1
    } else {
        return Err(ApiError::upgrade_required());
    };

    let session = authenticated_session(&state, &headers).await?;
    let participant_id = session.participant_id;
    let projection = projection_for_participant(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::game_action_not_allowed)?;
    let game_id = Uuid::parse_str(&projection.game.id)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;

    Ok(websocket
        .protocols([protocol.subprotocol()])
        .max_message_size(4 * 1024)
        .on_upgrade(move |socket| {
            serve_game_events(socket, state, session, game_id, query, protocol)
        }))
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
    protocol: RealtimeProtocol,
) {
    let participant_id = session.participant_id;
    let mut position = RealtimePosition {
        cursor: query.cursor.and_then(|value| i64::try_from(value).ok()),
        snapshot_version: query.snapshot_version,
        digest: query.digest,
        synchronized: false,
    };
    let signal = state.subscribe_to_game_events(game_id);
    let presence_signal = state.subscribe_to_game_presence(game_id);
    let shutdown = state.subscribe_to_shutdown();
    if *shutdown.borrow() {
        close_socket(&mut socket, close_code::RESTART, "server is shutting down").await;
        drop(signal);
        drop(presence_signal);
        state.prune_game_event_channel(game_id);
        state.prune_game_presence_channel(game_id);
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
        protocol,
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
        drop(presence_signal);
        state.prune_game_event_channel(game_id);
        state.prune_game_presence_channel(game_id);
        return;
    }

    let Some((connection_id, mut last_presence)) =
        register_connection_presence(&mut socket, &state, session, game_id, protocol).await
    else {
        drop(signal);
        drop(presence_signal);
        state.prune_game_event_channel(game_id);
        state.prune_game_presence_channel(game_id);
        return;
    };
    let context = RealtimeConnectionContext {
        state: &state,
        session,
        game_id,
        connection_id,
        protocol,
    };

    realtime_event_loop(
        &mut socket,
        context,
        &mut position,
        &mut last_presence,
        signal,
        presence_signal,
        shutdown,
    )
    .await;
    disconnect_presence(&state, connection_id, game_id, participant_id).await;
    state.prune_game_event_channel(game_id);
    state.prune_game_presence_channel(game_id);
}

async fn register_connection_presence(
    socket: &mut WebSocket,
    state: &AppState,
    session: AuthenticatedSession,
    game_id: Uuid,
    protocol: RealtimeProtocol,
) -> Option<(Uuid, Option<RealtimePresenceMessage>)> {
    let participant_id = session.participant_id;
    let connection_id = Uuid::new_v4();
    match postgres::register_realtime_connection(
        &state.database,
        connection_id,
        game_id,
        participant_id,
        session.session_id,
    )
    .await
    {
        Ok(true) => state.signal_game_presence(game_id),
        Ok(false) => {
            close_socket(socket, close_code::POLICY, "session is no longer active").await;
            return None;
        }
        Err(_error) => {
            tracing::warn!(%game_id, %participant_id, "realtime presence registration failed");
            close_socket(socket, close_code::ERROR, "presence registration failed").await;
            return None;
        }
    }

    let mut last_presence = None;
    if protocol.publishes_presence()
        && let Err(error) =
            send_realtime_presence(socket, state, game_id, &mut last_presence, true, protocol).await
    {
        tracing::warn!(error = %error, %game_id, %participant_id, "initial realtime presence failed");
        close_socket(socket, close_code::ERROR, "initial presence failed").await;
        disconnect_presence(state, connection_id, game_id, participant_id).await;
        return None;
    }
    Some((connection_id, last_presence))
}

async fn realtime_event_loop(
    socket: &mut WebSocket,
    context: RealtimeConnectionContext<'_>,
    position: &mut RealtimePosition,
    last_presence: &mut Option<RealtimePresenceMessage>,
    mut signal: broadcast::Receiver<()>,
    mut presence_signal: broadcast::Receiver<()>,
    mut shutdown: watch::Receiver<bool>,
) {
    let RealtimeConnectionContext {
        state,
        session,
        game_id,
        connection_id,
        protocol,
    } = context;
    let participant_id = session.participant_id;
    let now = tokio::time::Instant::now();
    let mut reconciliation = realtime_interval(
        now + REALTIME_RECONCILIATION_INTERVAL + reconciliation_jitter(game_id, participant_id),
        REALTIME_RECONCILIATION_INTERVAL,
    );
    let mut session_revalidation = realtime_interval(
        now + REALTIME_SESSION_REVALIDATION_INTERVAL,
        REALTIME_SESSION_REVALIDATION_INTERVAL,
    );
    let mut heartbeat = realtime_interval(
        now + REALTIME_HEARTBEAT_INTERVAL,
        REALTIME_HEARTBEAT_INTERVAL,
    );
    let mut presence_reconciliation = realtime_interval(
        now + REALTIME_PRESENCE_RECONCILIATION_INTERVAL,
        REALTIME_PRESENCE_RECONCILIATION_INTERVAL,
    );
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
                    state,
                    session,
                    connection_id,
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
            notification = presence_signal.recv(), if protocol.publishes_presence() => {
                match notification {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        RealtimeLoopAction::Presence
                    }
                    Err(broadcast::error::RecvError::Closed) => RealtimeLoopAction::Continue,
                }
            }
            _ = reconciliation.tick() => RealtimeLoopAction::Synchronize,
            _ = presence_reconciliation.tick(), if protocol.publishes_presence() => {
                RealtimeLoopAction::Presence
            },
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

        if !apply_realtime_loop_action(action, socket, context, position, last_presence).await {
            return;
        }
    }
}

fn realtime_interval(first_tick: tokio::time::Instant, period: Duration) -> tokio::time::Interval {
    let mut interval = tokio::time::interval_at(first_tick, period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

async fn apply_realtime_loop_action(
    action: RealtimeLoopAction,
    socket: &mut WebSocket,
    context: RealtimeConnectionContext<'_>,
    position: &mut RealtimePosition,
    last_presence: &mut Option<RealtimePresenceMessage>,
) -> bool {
    let RealtimeConnectionContext {
        state,
        session,
        game_id,
        protocol,
        ..
    } = context;
    match action {
        RealtimeLoopAction::Continue => true,
        RealtimeLoopAction::Stop => false,
        RealtimeLoopAction::Presence => {
            synchronize_presence(
                socket,
                context,
                last_presence,
                "realtime presence synchronization failed",
            )
            .await
        }
        RealtimeLoopAction::Synchronize => {
            if !synchronize_connection(socket, state, session, game_id, position, protocol).await {
                return false;
            }
            !protocol.publishes_presence()
                || synchronize_presence(
                    socket,
                    context,
                    last_presence,
                    "realtime presence after official synchronization failed",
                )
                .await
        }
    }
}

async fn synchronize_presence(
    socket: &mut WebSocket,
    context: RealtimeConnectionContext<'_>,
    last_presence: &mut Option<RealtimePresenceMessage>,
    failure_message: &'static str,
) -> bool {
    let RealtimeConnectionContext {
        state,
        session,
        game_id,
        protocol,
        ..
    } = context;
    if let Err(error) =
        send_realtime_presence(socket, state, game_id, last_presence, false, protocol).await
    {
        tracing::warn!(error = %error, %game_id, participant_id = %session.participant_id, failure_message);
        close_socket(socket, close_code::ERROR, "presence synchronization failed").await;
        return false;
    }
    true
}

async fn handle_client_message(
    socket: &mut WebSocket,
    message: Option<Result<Message, axum::Error>>,
    game_id: Uuid,
    state: &AppState,
    session: AuthenticatedSession,
    connection_id: Uuid,
    last_client_activity: &mut tokio::time::Instant,
) -> RealtimeLoopAction {
    let participant_id = session.participant_id;
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
            match postgres::touch_realtime_connection(
                &state.database,
                connection_id,
                participant_id,
                session.session_id,
            )
            .await
            {
                Ok(true) => {
                    state.signal_game_presence(game_id);
                    RealtimeLoopAction::Continue
                }
                Ok(false) => {
                    close_socket(socket, close_code::POLICY, "session is no longer active").await;
                    RealtimeLoopAction::Stop
                }
                Err(_error) => {
                    tracing::warn!(%game_id, %participant_id, "realtime presence heartbeat failed");
                    close_socket(socket, close_code::ERROR, "presence heartbeat failed").await;
                    RealtimeLoopAction::Stop
                }
            }
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
    protocol: RealtimeProtocol,
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
        protocol,
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

async fn disconnect_presence(
    state: &AppState,
    connection_id: Uuid,
    game_id: Uuid,
    participant_id: Uuid,
) {
    if let Err(_error) =
        postgres::disconnect_realtime_connection(&state.database, connection_id).await
    {
        tracing::warn!(%game_id, %participant_id, "realtime presence disconnect failed");
    }
    state.signal_game_presence(game_id);
}

async fn send_realtime_presence(
    socket: &mut WebSocket,
    state: &AppState,
    game_id: Uuid,
    last_presence: &mut Option<RealtimePresenceMessage>,
    force: bool,
    protocol: RealtimeProtocol,
) -> Result<(), RealtimeFailure> {
    let presence = realtime_presence(&state.database, game_id, protocol).await?;
    if !force && last_presence.as_ref() == Some(&presence) {
        return Ok(());
    }
    send_realtime_message(socket, &presence).await?;
    *last_presence = Some(presence);
    Ok(())
}

async fn realtime_presence(
    database: &sqlx::PgPool,
    game_id: Uuid,
    protocol: RealtimeProtocol,
) -> Result<RealtimePresenceMessage, RealtimeFailure> {
    let (snapshot_json, stored_participants) = postgres::game_presence(
        database,
        game_id,
        REALTIME_ONLINE_WINDOW_SECONDS,
        REALTIME_RECONNECTING_WINDOW_SECONDS,
    )
    .await
    .map_err(|_| RealtimeFailure::Database("load game presence"))?
    .ok_or(RealtimeFailure::InvalidData("game presence is unavailable"))?;
    let persisted = decode_persisted_snapshot(&snapshot_json)
        .map_err(|_| RealtimeFailure::InvalidData("decode presence decision state"))?;
    let domain_state = command_domain_state(&persisted)
        .map_err(|_| RealtimeFailure::InvalidData("restore presence decision state"))?;
    let required_participant_position =
        game_domain::required_participant_for_decision(&domain_state).map(i16::from);
    let participants = stored_participants
        .into_iter()
        .map(|(position, status)| {
            let status = match status.as_str() {
                "online" => "online",
                "reconnecting" => "reconnecting",
                "offline" => "offline",
                _ => {
                    return Err(RealtimeFailure::InvalidData(
                        "presence status is not supported",
                    ));
                }
            };
            Ok(RealtimeParticipantPresence { position, status })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if required_participant_position.is_some_and(|required| {
        !participants
            .iter()
            .any(|participant| participant.position == required)
    }) {
        return Err(RealtimeFailure::InvalidData(
            "required participant is absent from presence",
        ));
    }
    let blocked = required_participant_position.is_some_and(|required| {
        participants
            .iter()
            .any(|participant| participant.position == required && participant.status != "online")
    });

    Ok(RealtimePresenceMessage {
        protocol_version: protocol.version(),
        message_type: "presence",
        game_id: game_id.to_string(),
        participants,
        required_participant_position,
        blocked,
    })
}

async fn synchronize_socket(
    socket: &mut WebSocket,
    state: &AppState,
    participant_id: Uuid,
    game_id: Uuid,
    position: &mut RealtimePosition,
    force_snapshot: bool,
    protocol: RealtimeProtocol,
) -> Result<(), RealtimeFailure> {
    let observed = load_realtime_position(&state.database, participant_id, game_id).await?;
    if !force_snapshot
        && position.cursor == observed.cursor
        && position.snapshot_version == observed.snapshot_version
        && position.digest == observed.digest
    {
        if !position.synchronized {
            send_realtime_synchronized(socket, &observed, protocol).await?;
            position.synchronized = true;
        }
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
    let anchor_matches =
        requested_anchor_matches(&state.database, participant_id, game_id, position).await?;
    let needs_snapshot = force_snapshot
        || position.snapshot_version != Some(current_snapshot_version)
        || requested_cursor.is_none_or(|value| value > current_cursor)
        || replay_gap_requires_snapshot(requested_cursor, current_cursor)
        || !anchor_matches;

    if needs_snapshot {
        send_realtime_snapshot(
            socket,
            projection,
            current_cursor,
            current_snapshot_version,
            position,
            protocol,
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
            protocol,
        )
        .await?;
        return Ok(());
    }

    let message = RealtimeEventBatchMessage {
        protocol_version: protocol.version(),
        message_type: "events",
        from_cursor,
        cursor: current_cursor,
        events,
        projection,
    };
    let digest = message.projection.snapshot.digest.clone();
    send_realtime_message(socket, &message).await?;
    position.cursor = Some(current_cursor);
    position.snapshot_version = Some(current_snapshot_version);
    position.digest = Some(digest);
    position.synchronized = true;
    Ok(())
}

fn replay_gap_requires_snapshot(requested_cursor: Option<i64>, current_cursor: i64) -> bool {
    requested_cursor
        .and_then(|value| current_cursor.checked_sub(value))
        .and_then(|distance| u64::try_from(distance).ok())
        .is_none_or(|distance| distance > REALTIME_REPLAY_LIMIT)
}

async fn requested_anchor_matches(
    database: &sqlx::PgPool,
    participant_id: Uuid,
    game_id: Uuid,
    position: &RealtimePosition,
) -> Result<bool, RealtimeFailure> {
    let (Some(cursor), Some(snapshot_version), Some(digest)) = (
        position.cursor,
        position.snapshot_version,
        position.digest.as_deref(),
    ) else {
        return Ok(false);
    };
    let stored =
        postgres::game_state_anchor_for_participant(database, participant_id, game_id, cursor)
            .await
            .map_err(|_| RealtimeFailure::Database("load game state replay anchor"))?;
    Ok(stored.is_some_and(|(stored_version, stored_digest)| {
        i16::try_from(snapshot_version).ok() == Some(stored_version) && digest == stored_digest
    }))
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
        digest: Some(stored.2),
        synchronized: true,
    })
}

async fn send_realtime_synchronized(
    socket: &mut WebSocket,
    observed: &RealtimePosition,
    protocol: RealtimeProtocol,
) -> Result<(), RealtimeFailure> {
    let (Some(cursor), Some(snapshot_version), Some(digest)) = (
        observed.cursor,
        observed.snapshot_version,
        observed.digest.as_deref(),
    ) else {
        return Err(RealtimeFailure::InvalidData(
            "current synchronization coordinates are incomplete",
        ));
    };
    send_realtime_message(
        socket,
        &RealtimeSynchronizedMessage {
            protocol_version: protocol.version(),
            message_type: "synchronized",
            cursor,
            snapshot_version,
            digest,
        },
    )
    .await
}

async fn send_realtime_snapshot(
    socket: &mut WebSocket,
    projection: GameProjectionResponse,
    cursor: i64,
    snapshot_version: u16,
    position: &mut RealtimePosition,
    protocol: RealtimeProtocol,
) -> Result<(), RealtimeFailure> {
    let digest = projection.snapshot.digest.clone();
    let message = RealtimeSnapshotMessage {
        protocol_version: protocol.version(),
        message_type: "snapshot",
        cursor,
        projection,
    };
    send_realtime_message(socket, &message).await?;
    position.cursor = Some(cursor);
    position.snapshot_version = Some(snapshot_version);
    position.digest = Some(digest);
    position.synchronized = true;
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

#[cfg(test)]
mod tests {
    use super::replay_gap_requires_snapshot;

    #[test]
    fn replay_gap_is_bounded_to_the_incremental_recovery_window() {
        assert!(!replay_gap_requires_snapshot(Some(0), 100));
        assert!(replay_gap_requires_snapshot(Some(0), 101));
        assert!(replay_gap_requires_snapshot(Some(2), 1));
        assert!(replay_gap_requires_snapshot(None, 1));
    }
}
