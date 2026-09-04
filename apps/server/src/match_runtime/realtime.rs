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

use game_domain::DecisionPoint;

use super::codec::{command_domain_state, decode_persisted_event, decode_persisted_snapshot};
use super::{
    GameProjectionResponse, PersistedDecisionPoint, PersistedEffectOutcome,
    PersistedEffectTargetBinding, PersistedEndTurnOutcome, PersistedEngineControl,
    PersistedEventChoice, PersistedTurnStep, StoredGameEvent, postgres, projection_for_participant,
};
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
#[serde(untagged)]
enum RealtimeGameEvent {
    Legacy(RealtimeLegacyGameEvent),
    TurnCompleted(RealtimeTurnCompletedGameEvent),
    ChoiceResolved(RealtimeChoiceResolvedGameEvent),
}

impl RealtimeGameEvent {
    const fn sequence(&self) -> i64 {
        match self {
            Self::Legacy(event) => event.sequence,
            Self::TurnCompleted(event) => event.sequence,
            Self::ChoiceResolved(event) => event.sequence,
        }
    }
}

#[derive(Serialize)]
struct RealtimeLegacyGameEvent {
    event_version: i16,
    #[serde(rename = "type")]
    event_type: &'static str,
    sequence: i64,
    state_version: i64,
    turn: u32,
    actor_position: i16,
    #[serde(skip_serializing_if = "Option::is_none")]
    choice_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    choice_cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    card_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    targets: Option<Vec<PersistedEffectTargetBinding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    villain_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refill_card_id: Option<String>,
    effects: Vec<PersistedEffectOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effect_stop: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    choice: Option<RealtimeChoiceSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prng_counter: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
}

#[derive(Serialize)]
struct RealtimeChoiceSummary {
    status: &'static str,
    id: String,
    cause: String,
    responsible_position: u8,
    kind: String,
    options: Vec<String>,
    min: u16,
    max: u16,
}

#[derive(Serialize)]
struct RealtimeTurnCompletedGameEvent {
    event_version: i16,
    #[serde(rename = "type")]
    event_type: &'static str,
    sequence: i64,
    state_version: i64,
    turn: u32,
    actor_position: i16,
    end_turn: Vec<PersistedEndTurnOutcome>,
    steps: Vec<PersistedTurnStep>,
    control: RealtimeEngineControl,
    prng_counter: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
}

#[derive(Serialize)]
struct RealtimeChoiceResolvedGameEvent {
    event_version: i16,
    #[serde(rename = "type")]
    event_type: &'static str,
    sequence: i64,
    state_version: i64,
    turn: u32,
    actor_position: i16,
    choice_id: String,
    choice_cause: String,
    selected_options: Vec<String>,
    steps: Vec<PersistedTurnStep>,
    control: RealtimeEngineControl,
    prng_counter: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
}

#[derive(Serialize)]
struct RealtimeEngineControl {
    status: String,
    turn: u32,
    phase: String,
    active_position: u8,
    queued_phases: Vec<String>,
    queued_effect_count: usize,
    decision_point: RealtimeDecisionPoint,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RealtimeDecisionPoint {
    None,
    Automatic,
    PlayerIntent { responsible_position: u8 },
    EffectChoice { choice: RealtimeChoiceSummary },
}

struct RealtimeEventFields {
    event_type: &'static str,
    card_id: Option<String>,
    targets: Option<Vec<PersistedEffectTargetBinding>>,
    villain_id: Option<String>,
    amount: Option<u16>,
    cost: Option<u16>,
    refill_card_id: Option<String>,
    effect_stop: Option<String>,
    prng_counter: Option<u64>,
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
    let projection = projection_for_participant(&state, participant_id)
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
    let synchronization_signal = state.subscribe_to_game_synchronization(game_id);
    let presence_signal = state.subscribe_to_game_presence(game_id);
    let shutdown = state.subscribe_to_shutdown();
    if *shutdown.borrow() {
        close_socket(&mut socket, close_code::RESTART, "server is shutting down").await;
        drop(synchronization_signal);
        drop(presence_signal);
        state.prune_game_synchronization_channel(game_id);
        state.prune_game_presence_channel(game_id);
        return;
    }
    let force_initial_snapshot = query.cursor.is_none() || position.cursor.is_none();
    if !synchronize_connection(
        &mut socket,
        &state,
        session,
        game_id,
        &mut position,
        force_initial_snapshot,
        protocol,
    )
    .await
    {
        drop(synchronization_signal);
        drop(presence_signal);
        state.prune_game_synchronization_channel(game_id);
        state.prune_game_presence_channel(game_id);
        return;
    }

    let Some((connection_id, mut last_presence)) =
        register_connection_presence(&mut socket, &state, session, game_id, protocol).await
    else {
        drop(synchronization_signal);
        drop(presence_signal);
        state.prune_game_synchronization_channel(game_id);
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
        synchronization_signal,
        presence_signal,
        shutdown,
    )
    .await;
    disconnect_presence(&state, connection_id, game_id, participant_id).await;
    state.prune_game_synchronization_channel(game_id);
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
    mut synchronization_signal: broadcast::Receiver<()>,
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
            notification = synchronization_signal.recv() => {
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
            if !synchronize_connection(socket, state, session, game_id, position, false, protocol)
                .await
            {
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
    force_snapshot: bool,
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
        force_snapshot,
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
    let presence = realtime_presence(state, game_id, protocol).await?;
    if !force && last_presence.as_ref() == Some(&presence) {
        return Ok(());
    }
    send_realtime_message(socket, &presence).await?;
    *last_presence = Some(presence);
    Ok(())
}

async fn realtime_presence(
    state: &AppState,
    game_id: Uuid,
    protocol: RealtimeProtocol,
) -> Result<RealtimePresenceMessage, RealtimeFailure> {
    let (snapshot_json, stored_participants) = postgres::game_presence(
        &state.database,
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
        required_participant_position(domain_state.decision_point());
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

fn required_participant_position(decision_point: Option<&DecisionPoint>) -> Option<i16> {
    match decision_point {
        Some(DecisionPoint::PlayerIntent {
            responsible_position,
        }) => Some(i16::from(*responsible_position)),
        Some(DecisionPoint::EffectChoice(choice)) => Some(i16::from(choice.responsible_position)),
        None | Some(DecisionPoint::Automatic) => None,
    }
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

    let projection = match projection_for_participant(state, participant_id).await {
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
    if !realtime_event_metadata_matches(&payload, stored) {
        return Err(ApiError::internal());
    }
    let command_id =
        (stored.actor_participant_id == participant_id).then(|| stored.command_id.to_string());

    match (payload.event_version, payload.event_type.as_str()) {
        (1..=3, "dark_arts_completed")
        | (3, "choice_resolved")
        | (3 | 4, "card_played" | "attack_assigned" | "card_acquired") => {
            realtime_legacy_event(stored, payload, command_id)
        }
        (4, "turn_completed") => realtime_turn_completed_event(stored, payload, command_id),
        (4, "choice_resolved") => realtime_choice_resolved_event(stored, payload, command_id),
        _ => Err(ApiError::internal()),
    }
}

fn realtime_legacy_event(
    stored: &StoredGameEvent,
    payload: super::PersistedGameEvent,
    command_id: Option<String>,
) -> Result<RealtimeGameEvent, ApiError> {
    let fields = realtime_event_fields(&payload, &stored.event_type)?;
    let choice = payload
        .choice
        .as_ref()
        .map(realtime_choice_summary)
        .transpose()?;
    Ok(RealtimeGameEvent::Legacy(RealtimeLegacyGameEvent {
        event_version: stored.event_version,
        event_type: fields.event_type,
        sequence: stored.sequence,
        state_version: stored.state_version,
        turn: payload.turn,
        actor_position: stored.actor_position,
        choice_id: payload.choice_id,
        choice_cause: payload.choice_cause,
        selected_options: payload.selected_options,
        card_id: fields.card_id,
        targets: fields.targets,
        villain_id: fields.villain_id,
        amount: fields.amount,
        cost: fields.cost,
        refill_card_id: fields.refill_card_id,
        effects: payload.effects,
        effect_stop: fields.effect_stop,
        choice,
        prng_counter: fields.prng_counter,
        command_id,
    }))
}

fn realtime_turn_completed_event(
    stored: &StoredGameEvent,
    payload: super::PersistedGameEvent,
    command_id: Option<String>,
) -> Result<RealtimeGameEvent, ApiError> {
    let (Some(end_turn), Some(steps), Some(control)) =
        (payload.end_turn, payload.steps, payload.control)
    else {
        return Err(ApiError::internal());
    };
    Ok(RealtimeGameEvent::TurnCompleted(
        RealtimeTurnCompletedGameEvent {
            event_version: stored.event_version,
            event_type: "turn_completed",
            sequence: stored.sequence,
            state_version: stored.state_version,
            turn: payload.turn,
            actor_position: stored.actor_position,
            end_turn,
            steps,
            control: realtime_engine_control(control),
            prng_counter: payload.prng_counter.ok_or_else(ApiError::internal)?,
            command_id,
        },
    ))
}

fn realtime_choice_resolved_event(
    stored: &StoredGameEvent,
    payload: super::PersistedGameEvent,
    command_id: Option<String>,
) -> Result<RealtimeGameEvent, ApiError> {
    let (Some(choice_id), Some(choice_cause), Some(selected_options), Some(steps), Some(control)) = (
        payload.choice_id,
        payload.choice_cause,
        payload.selected_options,
        payload.steps,
        payload.control,
    ) else {
        return Err(ApiError::internal());
    };
    Ok(RealtimeGameEvent::ChoiceResolved(
        RealtimeChoiceResolvedGameEvent {
            event_version: stored.event_version,
            event_type: "choice_resolved",
            sequence: stored.sequence,
            state_version: stored.state_version,
            turn: payload.turn,
            actor_position: stored.actor_position,
            choice_id,
            choice_cause,
            selected_options,
            steps,
            control: realtime_engine_control(control),
            prng_counter: payload.prng_counter.ok_or_else(ApiError::internal)?,
            command_id,
        },
    ))
}

fn realtime_engine_control(control: PersistedEngineControl) -> RealtimeEngineControl {
    let decision_point = match control.decision_point {
        PersistedDecisionPoint::None => RealtimeDecisionPoint::None,
        PersistedDecisionPoint::Automatic => RealtimeDecisionPoint::Automatic,
        PersistedDecisionPoint::PlayerIntent {
            responsible_position,
        } => RealtimeDecisionPoint::PlayerIntent {
            responsible_position,
        },
        PersistedDecisionPoint::EffectChoice { choice } => RealtimeDecisionPoint::EffectChoice {
            choice: realtime_current_choice_summary(&choice),
        },
    };
    RealtimeEngineControl {
        status: control.status,
        turn: control.turn,
        phase: control.phase,
        active_position: control.active_position,
        queued_phases: control.queued_phases,
        queued_effect_count: control.queued_effects.len(),
        decision_point,
    }
}

fn realtime_choice_summary(
    choice: &PersistedEventChoice,
) -> Result<RealtimeChoiceSummary, ApiError> {
    let (id, cause, responsible_position, kind, options, min, max) = match choice {
        PersistedEventChoice::Current(choice) => (
            choice.id.clone(),
            choice.cause.clone(),
            choice.responsible_position,
            choice.kind.clone(),
            choice.options.clone(),
            choice.min,
            choice.max,
        ),
        PersistedEventChoice::Legacy(choice) => (
            choice.id.clone(),
            legacy_choice_cause(&choice.id, &choice.kind)?,
            choice.responsible_position,
            choice.kind.clone(),
            choice.options.clone(),
            choice.min,
            choice.max,
        ),
    };
    Ok(RealtimeChoiceSummary {
        status: "pending",
        id,
        cause,
        responsible_position,
        kind,
        options,
        min,
        max,
    })
}

fn realtime_current_choice_summary(choice: &super::PersistedEffectChoice) -> RealtimeChoiceSummary {
    RealtimeChoiceSummary {
        status: "pending",
        id: choice.id.clone(),
        cause: choice.cause.clone(),
        responsible_position: choice.responsible_position,
        kind: choice.kind.clone(),
        options: choice.options.clone(),
        min: choice.min,
        max: choice.max,
    }
}

fn legacy_choice_cause(id: &str, kind: &str) -> Result<String, ApiError> {
    let marker = match kind {
        "effect" => ":effect:",
        "target" => ":target:",
        _ => return Err(ApiError::internal()),
    };
    let (cause, index) = id.rsplit_once(marker).ok_or_else(ApiError::internal)?;
    if cause.is_empty() || index.parse::<usize>().is_err() {
        return Err(ApiError::internal());
    }
    Ok(cause.to_owned())
}

fn realtime_event_metadata_matches(
    payload: &super::PersistedGameEvent,
    stored: &StoredGameEvent,
) -> bool {
    i16::try_from(payload.event_version).ok() == Some(stored.event_version)
        && payload.event_type == stored.event_type
        && i64::try_from(payload.sequence).ok() == Some(stored.sequence)
        && i64::try_from(payload.state_version).ok() == Some(stored.state_version)
        && i16::from(payload.actor_position) == stored.actor_position
}

fn realtime_event_fields(
    payload: &super::PersistedGameEvent,
    stored_event_type: &str,
) -> Result<RealtimeEventFields, ApiError> {
    let event_type = realtime_event_type(payload, stored_event_type)?;
    let (effect_stop, prng_counter) = realtime_effect_progress(payload, event_type)?;
    Ok(match event_type {
        "card_played" => RealtimeEventFields {
            event_type,
            card_id: payload.card_id.clone(),
            targets: Some(payload.targets.clone()),
            villain_id: None,
            amount: None,
            cost: None,
            refill_card_id: None,
            effect_stop,
            prng_counter,
        },
        "attack_assigned" => RealtimeEventFields {
            event_type,
            card_id: None,
            targets: None,
            villain_id: payload.villain_id.clone(),
            amount: payload.amount,
            cost: None,
            refill_card_id: None,
            effect_stop,
            prng_counter,
        },
        "card_acquired" => RealtimeEventFields {
            event_type,
            card_id: payload.card_id.clone(),
            targets: None,
            villain_id: None,
            amount: None,
            cost: payload.cost,
            refill_card_id: payload.refill_card_id.clone(),
            effect_stop,
            prng_counter,
        },
        _ => RealtimeEventFields {
            event_type,
            card_id: None,
            targets: None,
            villain_id: None,
            amount: None,
            cost: None,
            refill_card_id: None,
            effect_stop,
            prng_counter,
        },
    })
}

fn realtime_event_type(
    payload: &super::PersistedGameEvent,
    stored_event_type: &str,
) -> Result<&'static str, ApiError> {
    match (payload.event_version, stored_event_type) {
        (1..=3, "dark_arts_completed") => Ok("dark_arts_completed"),
        (3, "choice_resolved") => Ok("choice_resolved"),
        (3 | 4, "card_played") => Ok("card_played"),
        (3 | 4, "attack_assigned") => Ok("attack_assigned"),
        (3 | 4, "card_acquired") => Ok("card_acquired"),
        _ => Err(ApiError::internal()),
    }
}

fn realtime_effect_progress(
    payload: &super::PersistedGameEvent,
    event_type: &str,
) -> Result<(Option<String>, Option<u64>), ApiError> {
    if payload.event_version == 1 && event_type == "dark_arts_completed" {
        return Ok((Some("stable".to_owned()), Some(0)));
    }
    if !matches!(
        event_type,
        "dark_arts_completed" | "choice_resolved" | "card_played"
    ) {
        return Ok((None, None));
    }

    let effect_stop = payload
        .effect_stop
        .as_ref()
        .ok_or_else(ApiError::internal)?;
    let Some(prng_counter) = payload.prng_counter else {
        return Err(ApiError::internal());
    };
    Ok((Some(effect_stop.clone()), Some(prng_counter)))
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
            .all(|(event, expected)| event.sequence() == expected)
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
    use game_domain::{
        DecisionPoint, EffectContinuation, EffectCursor, PendingEffectChoice,
        PendingEffectChoiceKind,
    };
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::{
        StoredGameEvent, realtime_event, replay_gap_requires_snapshot,
        required_participant_position,
    };

    fn stored_event(payload: &Value, actor_participant_id: Uuid) -> StoredGameEvent {
        let event_version = payload["event_version"]
            .as_i64()
            .and_then(|value| i16::try_from(value).ok())
            .expect("fixture event version must fit");
        let event_type = payload["type"]
            .as_str()
            .expect("fixture event type must be text")
            .to_owned();
        let sequence = payload["sequence"]
            .as_i64()
            .expect("fixture sequence must fit");
        let state_version = payload["state_version"]
            .as_i64()
            .expect("fixture state version must fit");
        let actor_position = payload["actor_position"]
            .as_i64()
            .and_then(|value| i16::try_from(value).ok())
            .expect("fixture actor position must fit");
        StoredGameEvent {
            event_version,
            event_type,
            command_id: Uuid::new_v4(),
            actor_participant_id,
            actor_position,
            sequence,
            state_version,
            payload_json: payload.to_string(),
        }
    }

    fn serialized_realtime_event(stored: &StoredGameEvent, viewer: Uuid) -> Value {
        let event = realtime_event(stored, viewer)
            .unwrap_or_else(|_| panic!("valid persisted event must become realtime output"));
        serde_json::to_value(event).expect("realtime event must serialize")
    }

    #[test]
    fn replay_gap_is_bounded_to_the_incremental_recovery_window() {
        assert!(!replay_gap_requires_snapshot(Some(0), 100));
        assert!(replay_gap_requires_snapshot(Some(0), 101));
        assert!(replay_gap_requires_snapshot(Some(2), 1));
        assert!(replay_gap_requires_snapshot(None, 1));
    }

    #[test]
    fn only_human_decisions_require_a_participant() {
        assert_eq!(required_participant_position(None), None);
        assert_eq!(
            required_participant_position(Some(&DecisionPoint::Automatic)),
            None
        );
        assert_eq!(
            required_participant_position(Some(&DecisionPoint::PlayerIntent {
                responsible_position: 2,
            })),
            Some(2)
        );
        let choice = PendingEffectChoice {
            id: "choice:dark".to_owned(),
            cause: "rule:dark".to_owned(),
            responsible_position: 3,
            kind: PendingEffectChoiceKind::Target,
            options: vec!["hero:one".to_owned(), "hero:two".to_owned()],
            min: 1,
            max: 1,
            continuation: EffectContinuation {
                choice_cursor: EffectCursor {
                    rule_id: "rule:dark".to_owned(),
                    path: Vec::new(),
                },
                queue: Vec::new(),
                steps_completed: 1,
            },
        };
        assert_eq!(
            required_participant_position(Some(&DecisionPoint::EffectChoice(choice))),
            Some(3)
        );
    }

    #[test]
    fn legacy_realtime_events_expose_only_effect_resolution_fields() {
        let actor = Uuid::new_v4();
        let payloads = [
            json!({
                "event_version": 1,
                "type": "dark_arts_completed",
                "sequence": 1,
                "state_version": 2,
                "turn": 1,
                "actor_position": 1
            }),
            json!({
                "event_version": 2,
                "type": "dark_arts_completed",
                "sequence": 2,
                "state_version": 3,
                "turn": 1,
                "actor_position": 1,
                "effects": [],
                "effect_stop": "stable",
                "choice": null,
                "prng_counter": 0
            }),
        ];

        for payload in payloads {
            let stored = stored_event(&payload, actor);
            let event = serialized_realtime_event(&stored, actor);
            assert_eq!(event["type"], "dark_arts_completed");
            assert_eq!(event["effects"], json!([]));
            assert_eq!(event["effect_stop"], "stable");
            assert!(event.get("choice").is_none());
            assert!(event.get("end_turn").is_none());
            assert!(event.get("steps").is_none());
            assert!(event.get("control").is_none());
            assert_eq!(event["command_id"], stored.command_id.to_string());
        }
    }

    #[test]
    fn choice_resolved_v3_remains_public_without_its_private_continuation() {
        let actor = Uuid::new_v4();
        let stored = stored_event(
            &json!({
                "event_version": 3,
                "type": "choice_resolved",
                "sequence": 2,
                "state_version": 3,
                "turn": 1,
                "actor_position": 2,
                "choice_id": "choice:effect:0",
                "choice_cause": "rule:previous",
                "selected_options": ["option:1"],
                "effects": [],
                "effect_stop": "choice",
                "choice": {
                    "id": "choice:target:1",
                    "cause": "rule:next",
                    "responsible_position": 3,
                    "kind": "target",
                    "options": ["hero:1", "hero:2"],
                    "min": 1,
                    "max": 1,
                    "continuation": {
                        "choice_cursor": { "rule_id": "rule:next", "path": [] },
                        "queue": [],
                        "steps_completed": 1
                    }
                },
                "prng_counter": 0
            }),
            actor,
        );

        let event = serialized_realtime_event(&stored, actor);
        assert_eq!(event["type"], "choice_resolved");
        assert_eq!(event["choice"]["responsible_position"], 3);
        assert!(event["choice"].get("continuation").is_none());
        assert_eq!(event["choice_id"], "choice:effect:0");
        assert_eq!(event["command_id"], stored.command_id.to_string());
    }

    #[test]
    fn turn_completed_realtime_event_exposes_only_v4_fields_and_redacts_private_state() {
        let actor = Uuid::new_v4();
        let stored = stored_event(
            &json!({
                "event_version": 4,
                "type": "turn_completed",
                "sequence": 1,
                "state_version": 2,
                "turn": 1,
                "actor_position": 1,
                "end_turn": [
                    { "type": "resource_reset", "resource": "attack", "before": 0 },
                    { "type": "resource_reset", "resource": "influence", "before": 0 }
                ],
                "steps": [
                    { "phase": "end_turn", "effects": [] },
                    { "phase": "dark_arts", "effects": [] },
                    { "phase": "villains", "effects": [] }
                ],
                "control": {
                    "status": "in_progress",
                    "turn": 2,
                    "phase": "hero_actions",
                    "active_position": 2,
                    "queued_phases": ["end_turn"],
                    "queued_effects": [],
                    "decision_point": {
                        "type": "player_intent",
                        "responsible_position": 2
                    }
                },
                "prng_counter": 0
            }),
            actor,
        );

        let actor_event = serialized_realtime_event(&stored, actor);
        assert_eq!(actor_event["type"], "turn_completed");
        assert_eq!(
            actor_event["end_turn"],
            json!([
                { "type": "resource_reset", "resource": "attack", "before": 0 },
                { "type": "resource_reset", "resource": "influence", "before": 0 }
            ])
        );
        assert_eq!(actor_event["steps"].as_array().map(Vec::len), Some(3));
        assert_eq!(actor_event["control"]["phase"], "hero_actions");
        assert_eq!(actor_event["control"]["queued_effect_count"], 0);
        assert!(actor_event["control"].get("queued_effects").is_none());
        assert!(actor_event.get("effects").is_none());
        assert!(actor_event.get("effect_stop").is_none());
        assert!(actor_event.get("choice").is_none());
        assert_eq!(actor_event["command_id"], stored.command_id.to_string());

        let observer_event = serialized_realtime_event(&stored, Uuid::new_v4());
        assert!(observer_event.get("command_id").is_none());
    }

    #[test]
    fn choice_resolved_v4_preserves_responsibility_without_exposing_continuation() {
        let actor = Uuid::new_v4();
        let stored = stored_event(
            &json!({
                "event_version": 4,
                "type": "choice_resolved",
                "sequence": 2,
                "state_version": 3,
                "turn": 1,
                "actor_position": 2,
                "choice_id": "choice:effect:0",
                "choice_cause": "rule:previous",
                "selected_options": ["option:1"],
                "steps": [{ "phase": "dark_arts", "effects": [] }],
                "control": {
                    "status": "in_progress",
                    "turn": 1,
                    "phase": "dark_arts",
                    "active_position": 1,
                    "queued_phases": ["villains", "hero_actions", "end_turn"],
                    "queued_effects": [],
                    "decision_point": {
                        "type": "effect_choice",
                        "choice": {
                            "id": "choice:target:1",
                            "cause": "rule:next",
                            "responsible_position": 3,
                            "kind": "target",
                            "options": ["hero:1", "hero:2"],
                            "min": 1,
                            "max": 1,
                            "continuation": {
                                "choice_cursor": {
                                    "rule_id": "rule:next",
                                    "path": []
                                },
                                "queue": [],
                                "steps_completed": 1
                            }
                        }
                    }
                },
                "prng_counter": 0
            }),
            actor,
        );

        let event = serialized_realtime_event(&stored, actor);
        assert_eq!(event["type"], "choice_resolved");
        assert_eq!(event["control"]["active_position"], 1);
        assert_eq!(
            event["control"]["decision_point"]["choice"]["responsible_position"],
            3
        );
        assert_eq!(event["control"]["queued_effect_count"], 0);
        assert!(event["control"].get("queued_effects").is_none());
        assert!(
            event["control"]["decision_point"]["choice"]
                .get("continuation")
                .is_none()
        );
        assert_eq!(event["command_id"], stored.command_id.to_string());
    }

    #[test]
    fn realtime_event_rejects_metadata_mismatch_and_cross_version_fields() {
        let actor = Uuid::new_v4();
        let mut mismatched = stored_event(
            &json!({
                "event_version": 1,
                "type": "dark_arts_completed",
                "sequence": 1,
                "state_version": 2,
                "turn": 1,
                "actor_position": 1
            }),
            actor,
        );
        mismatched.event_type = "turn_completed".to_owned();
        assert!(realtime_event(&mismatched, actor).is_err());

        let invalid_shape = stored_event(
            &json!({
                "event_version": 4,
                "type": "turn_completed",
                "sequence": 1,
                "state_version": 2,
                "turn": 1,
                "actor_position": 1,
                "effects": [],
                "end_turn": [
                    { "type": "resource_reset", "resource": "attack", "before": 0 },
                    { "type": "resource_reset", "resource": "influence", "before": 0 }
                ],
                "steps": [],
                "control": {},
                "prng_counter": 0
            }),
            actor,
        );
        assert!(realtime_event(&invalid_shape, actor).is_err());
    }

    #[test]
    fn hero_action_events_are_forwarded_without_synthetic_stop_or_prng_fields() {
        let participant_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
            .expect("the participant ID must be valid");
        let attack = realtime_json(
            &stored_action_event(
                "attack_assigned",
                &json!({
                    "event_version": 3,
                    "type": "attack_assigned",
                    "sequence": 4,
                    "state_version": 5,
                    "turn": 1,
                    "actor_position": 1,
                    "villain_id": "instance:villain:1",
                    "amount": 2,
                    "effects": []
                }),
                participant_id,
            ),
            participant_id,
        );
        assert_eq!(attack["type"], "attack_assigned");
        assert_eq!(attack["villain_id"], "instance:villain:1");
        assert_eq!(attack["amount"], 2);
        assert!(attack.get("effect_stop").is_none());
        assert!(attack.get("prng_counter").is_none());

        let acquisition = realtime_json(
            &stored_action_event(
                "card_acquired",
                &json!({
                    "event_version": 3,
                    "type": "card_acquired",
                    "sequence": 4,
                    "state_version": 5,
                    "turn": 1,
                    "actor_position": 1,
                    "card_id": "instance:market:1",
                    "cost": 3,
                    "refill_card_id": null,
                    "effects": []
                }),
                participant_id,
            ),
            participant_id,
        );
        assert_eq!(acquisition["type"], "card_acquired");
        assert_eq!(acquisition["card_id"], "instance:market:1");
        assert_eq!(acquisition["cost"], 3);
        assert!(acquisition.get("refill_card_id").is_none());
        assert!(acquisition.get("effect_stop").is_none());
        assert!(acquisition.get("prng_counter").is_none());
    }

    #[test]
    fn played_card_event_forwards_explicit_target_bindings() {
        let participant_id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
            .expect("the participant ID must be valid");
        let played = realtime_json(
            &stored_action_event(
                "card_played",
                &json!({
                    "event_version": 3,
                    "type": "card_played",
                    "sequence": 4,
                    "state_version": 5,
                    "turn": 1,
                    "actor_position": 1,
                    "card_id": "instance:starter:1",
                    "targets": [{
                        "selector_id": "target:ally",
                        "target_ids": ["hero:2"]
                    }],
                    "effects": [],
                    "effect_stop": "stable",
                    "choice": null,
                    "prng_counter": 0
                }),
                participant_id,
            ),
            participant_id,
        );

        assert_eq!(played["type"], "card_played");
        assert_eq!(played["card_id"], "instance:starter:1");
        assert_eq!(played["targets"][0]["selector_id"], "target:ally");
        assert_eq!(played["targets"][0]["target_ids"], json!(["hero:2"]));
        assert_eq!(played["effect_stop"], "stable");
        assert_eq!(played["prng_counter"], 0);
    }

    fn stored_action_event(
        event_type: &str,
        payload: &serde_json::Value,
        participant_id: Uuid,
    ) -> StoredGameEvent {
        StoredGameEvent {
            event_version: 3,
            event_type: event_type.to_owned(),
            command_id: Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")
                .expect("the command ID must be valid"),
            actor_participant_id: participant_id,
            actor_position: 1,
            sequence: 4,
            state_version: 5,
            payload_json: payload.to_string(),
        }
    }

    fn realtime_json(stored: &StoredGameEvent, participant_id: Uuid) -> serde_json::Value {
        let event = realtime_event(stored, participant_id)
            .ok()
            .expect("the persisted event must normalize");
        serde_json::to_value(event).expect("the realtime event must serialize")
    }
}
