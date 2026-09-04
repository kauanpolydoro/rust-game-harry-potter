use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use game_domain::HeroId;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::{Uuid, Variant};

use crate::{
    AppState,
    content_catalog::ContentManifestOption,
    http_support::{ApiError, idempotency_key, no_store_json},
    match_runtime,
    session::authenticated_participant,
};

mod credentials;
mod postgres;

use credentials::{hash_password, validate_display_name, validate_password, verify_password};

const ROOM_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const ROOM_CODE_LENGTH: usize = 8;
const RECOVERY_TOKEN_LENGTH: usize = 64;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms", post(create_room))
        .route("/api/rooms/{room_code}", get(find_room))
        .route("/api/rooms/{room_code}/participants", post(join_room))
        .route("/api/session/recover", post(recover_participation))
        .route(
            "/api/session/recovery-password",
            put(rotate_recovery_password),
        )
        .route(
            "/api/session/recovery-credential",
            post(regenerate_own_recovery_credential),
        )
        .route(
            "/api/rooms/current/participants/{position}/recovery-credential",
            post(regenerate_assisted_recovery_credential),
        )
        .route("/api/session/hero", put(select_hero))
        .route("/api/session/readiness", put(set_readiness))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRoomRequest {
    display_name: String,
    recovery_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinRoomRequest {
    display_name: String,
    hero_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectHeroRequest {
    hero_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetReadinessRequest {
    ready: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoverParticipationRequest {
    #[serde(rename = "recovery_token")]
    token: String,
    #[serde(rename = "recovery_password")]
    password: String,
    #[serde(rename = "recovery_attempt_id")]
    attempt_id: String,
    replace_session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotateRecoveryPasswordRequest {
    current_recovery_password: String,
    new_recovery_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegenerateOwnRecoveryCredentialRequest {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegenerateAssistedRecoveryCredentialRequest {
    host_assistance_risk_acknowledged: bool,
}

#[derive(Serialize)]
struct FindRoomResponse {
    room: RoomSummary,
    heroes: Vec<HeroAvailability>,
}

#[derive(Serialize)]
struct RoomSummary {
    code: String,
    status: String,
}

#[derive(Serialize)]
struct ParticipantSummary {
    display_name: String,
    role: String,
    position: i16,
    ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    hero: Option<HeroSummary>,
}

#[derive(Serialize)]
pub(crate) struct LobbyResponse {
    room: RoomSummary,
    participant: ParticipantSummary,
    participants: Vec<ParticipantSummary>,
    heroes: Vec<HeroAvailability>,
    content_options: Vec<ContentManifestOption>,
}

#[derive(Serialize)]
struct ParticipantAccessResponse {
    #[serde(flatten)]
    lobby: LobbyResponse,
    recovery_token: String,
}

#[derive(Serialize)]
struct RotateRecoveryPasswordResponse {
    password_generation: i64,
    security_event: SecurityEvent,
}

#[derive(Serialize)]
struct RegenerateRecoveryCredentialResponse {
    delivery: &'static str,
    participant: RecoveryParticipant,
    recovery_generation: i64,
    recovery_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    risk_message_key: Option<&'static str>,
    security_event: RecoveryCredentialSecurityEvent,
}

#[derive(Serialize)]
struct RecoveryParticipant {
    display_name: String,
    position: i16,
}

#[derive(Serialize)]
struct RecoveryCredentialSecurityEvent {
    event_version: u16,
    cursor: i64,
    #[serde(rename = "type")]
    event_type: &'static str,
    actor_position: i16,
    target_position: i16,
    delivery: &'static str,
    recovery_generation: i64,
    occurred_at: String,
}

#[derive(Serialize)]
struct SecurityEvent {
    event_version: u16,
    cursor: i64,
    #[serde(rename = "type")]
    event_type: &'static str,
    actor_position: i16,
    password_generation: i64,
    occurred_at: String,
}

#[derive(Serialize)]
struct HeroSummary {
    id: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct HeroAvailability {
    id: &'static str,
    name: &'static str,
    available: bool,
}

#[derive(FromRow)]
struct StoredRoomCreation {
    participant_id: Uuid,
    display_name: String,
    recovery_password_hash: String,
}

#[derive(FromRow)]
struct StoredRoomJoin {
    participant_id: Uuid,
    room_code: String,
    display_name: String,
    hero_id: String,
}

#[derive(FromRow)]
struct StoredParticipant {
    id: Uuid,
    display_name: String,
    role: String,
    position: i16,
    hero_id: Option<String>,
    ready: bool,
}

#[derive(FromRow)]
struct StoredRecoveryCandidate {
    credential_id: Uuid,
    room_id: Uuid,
    participant_id: Uuid,
    guest_identity_id: Uuid,
    game_id: Option<Uuid>,
    recovery_password_hash: String,
    recovery_epoch: i64,
    password_generation: i64,
    recovery_generation: i64,
    status: String,
    recovery_attempt_id: Option<Uuid>,
    replaced_device_session_id: Option<Uuid>,
    session_max_age_seconds: Option<i64>,
}

#[derive(FromRow)]
struct StoredRecoveryRoom {
    room_id: Uuid,
    recovery_password_hash: String,
    recovery_epoch: i64,
    password_generation: i64,
}

#[derive(FromRow)]
struct StoredRecoveryPasswordAuthority {
    room_id: Uuid,
    participant_id: Uuid,
    role: String,
    recovery_password_hash: String,
    password_generation: i64,
}

#[derive(FromRow)]
struct StoredSecurityEvent {
    sequence: i64,
    actor_position: i16,
    password_generation: i64,
    occurred_at: String,
}

#[derive(FromRow)]
struct StoredRecoveryPasswordRotation {
    actor_participant_id: Uuid,
    request_fingerprint: String,
    password_generation: i64,
    sequence: i64,
    actor_position: i16,
    occurred_at: String,
}

#[derive(FromRow)]
struct StoredRecoveryParticipant {
    participant_id: Uuid,
    display_name: String,
    position: i16,
    recovery_generation: i64,
}

#[derive(FromRow)]
struct StoredRecoveryCredentialSecurityEvent {
    sequence: i64,
    actor_position: i16,
    target_position: i16,
    recovery_generation: i64,
    occurred_at: String,
}

#[derive(FromRow)]
struct StoredRecoveryCredentialRegeneration {
    actor_participant_id: Uuid,
    target_participant_id: Uuid,
    delivery: String,
    request_fingerprint: String,
    recovery_generation: i64,
    sequence: i64,
    actor_position: i16,
    target_display_name: String,
    target_position: i16,
    occurred_at: String,
}

struct RecoveryCredentialRegenerationCommand {
    idempotency_key: String,
    actor_participant_id: Uuid,
    target_position: Option<i16>,
    delivery: &'static str,
    request_fingerprint: String,
}

#[derive(FromRow)]
struct StoredDeviceSession {
    id: Uuid,
    slot: i16,
    created_at: String,
}

struct AuthenticatedRecovery {
    candidate: StoredRecoveryCandidate,
    token_hmac: String,
    attempt_id: Uuid,
    replace_session_id: Option<Uuid>,
    session_token: String,
}

#[derive(Serialize)]
struct RecoverySessionSummary {
    id: String,
    label: String,
    created_at: String,
}

#[derive(Serialize)]
struct RecoveryReplacementRequiredResponse {
    status: &'static str,
    sessions: Vec<RecoverySessionSummary>,
}

#[derive(Serialize)]
struct RecoveredLobbyResponse {
    kind: &'static str,
    recovery_token: String,
    lobby: LobbyResponse,
}

#[derive(Serialize)]
struct RecoveredGameResponse {
    kind: &'static str,
    recovery_token: String,
    game: match_runtime::GameProjectionResponse,
}

struct StoredLobby {
    room_code: String,
    room_status: String,
    current_participant_id: Uuid,
    participants: Vec<StoredParticipant>,
}

async fn create_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateRoomRequest>,
) -> Result<Response, ApiError> {
    let idempotency_key = idempotency_key(&headers)?;
    let display_name = validate_display_name(&request.display_name)?;
    validate_password(&request.recovery_password)?;

    if let Some(stored) = postgres::load_room_creation(&state.database, &idempotency_key).await? {
        return replay_room_creation(
            &state,
            stored,
            &idempotency_key,
            display_name,
            request.recovery_password,
        )
        .await;
    }
    require_session_grant_key(&idempotency_key)?;

    let password_hash = hash_password(request.recovery_password.clone()).await?;
    let mut transaction =
        state.database.begin().await.map_err(|error| {
            ApiError::internal_with("identity access application operation", error)
        })?;
    let room_id = Uuid::new_v4();
    let participant_id = Uuid::new_v4();
    let guest_session_id = Uuid::new_v4();
    let claimed = postgres::claim_room_creation(
        &mut transaction,
        &idempotency_key,
        room_id,
        participant_id,
        guest_session_id,
    )
    .await?;

    if !claimed {
        transaction.rollback().await.map_err(|error| {
            ApiError::internal_with("identity access application operation", error)
        })?;
        let stored = postgres::load_room_creation(&state.database, &idempotency_key)
            .await?
            .ok_or_else(ApiError::internal)?;
        return replay_room_creation(
            &state,
            stored,
            &idempotency_key,
            display_name,
            request.recovery_password,
        )
        .await;
    }

    let session_token =
        state.idempotent_session_token("create_room", &idempotency_key, participant_id);
    let recovery_token =
        state.idempotent_recovery_token("create_room", &idempotency_key, participant_id);
    let recovery_token_hmac = state.recovery_token_hmac(&recovery_token);
    let stored = postgres::persist_room_creation(
        &mut transaction,
        postgres::NewRoomCreation {
            room_id,
            participant_id,
            guest_session_id,
            display_name,
            password_hash: &password_hash,
        },
        &session_token,
        &recovery_token_hmac,
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("identity access application operation", error))?;

    room_created_response(&state, stored, &idempotency_key).await
}

async fn find_room(
    State(state): State<AppState>,
    Path(room_code): Path<String>,
) -> Result<Response, ApiError> {
    let normalized_code = room_code.to_ascii_uppercase();
    let room = postgres::find_open_room(&state.database, &normalized_code)
        .await?
        .ok_or_else(ApiError::room_not_found)?;
    let selected_heroes = postgres::selected_room_heroes(&state.database, &normalized_code).await?;

    let mut response = Json(FindRoomResponse {
        room: RoomSummary {
            code: room.0,
            status: room.1,
        },
        heroes: hero_availability(&selected_heroes),
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn join_room(
    State(state): State<AppState>,
    Path(room_code): Path<String>,
    headers: HeaderMap,
    Json(request): Json<JoinRoomRequest>,
) -> Result<Response, ApiError> {
    let idempotency_key = idempotency_key(&headers)?;
    let normalized_code = room_code.to_ascii_uppercase();
    let display_name = validate_display_name(&request.display_name)?;
    let hero = parse_hero(&request.hero_id)?;

    if let Some(stored) = postgres::load_room_join(&state.database, &idempotency_key).await? {
        return replay_room_join(
            &state,
            stored,
            &idempotency_key,
            &normalized_code,
            display_name,
            hero,
            &headers,
        )
        .await;
    }
    require_session_grant_key(&idempotency_key)?;

    let mut transaction =
        state.database.begin().await.map_err(|error| {
            ApiError::internal_with("identity access application operation", error)
        })?;
    let room_id = postgres::lock_open_room(&mut transaction, &normalized_code)
        .await?
        .ok_or_else(ApiError::room_unavailable)?;
    let participant_id = Uuid::new_v4();
    let guest_session_id = Uuid::new_v4();
    let claimed = postgres::claim_room_join(
        &mut transaction,
        &idempotency_key,
        room_id,
        participant_id,
        guest_session_id,
    )
    .await?;

    if !claimed {
        transaction.rollback().await.map_err(|error| {
            ApiError::internal_with("identity access application operation", error)
        })?;
        let stored = postgres::load_room_join(&state.database, &idempotency_key)
            .await?
            .ok_or_else(ApiError::internal)?;
        return replay_room_join(
            &state,
            stored,
            &idempotency_key,
            &normalized_code,
            display_name,
            hero,
            &headers,
        )
        .await;
    }

    let position = postgres::next_room_position(&mut transaction, room_id).await?;
    if postgres::room_has_hero(&mut transaction, room_id, hero.as_str()).await? {
        return Err(ApiError::hero_unavailable());
    }

    let session_token =
        state.idempotent_session_token("join_room", &idempotency_key, participant_id);
    let recovery_token =
        state.idempotent_recovery_token("join_room", &idempotency_key, participant_id);
    let recovery_token_hmac = state.recovery_token_hmac(&recovery_token);
    let stored = postgres::persist_room_join(
        &mut transaction,
        postgres::NewRoomJoin {
            room_id,
            participant_id,
            guest_session_id,
            room_code: &normalized_code,
            display_name,
            hero_id: hero.as_str(),
            position,
        },
        &session_token,
        &recovery_token_hmac,
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("identity access application operation", error))?;

    room_joined_response(&state, stored, &idempotency_key).await
}

async fn rotate_recovery_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RotateRecoveryPasswordRequest>,
) -> Result<Response, ApiError> {
    let key = idempotency_key(&headers)?;
    require_session_grant_key(&key)?;
    validate_password(&request.new_recovery_password)?;
    let participant_id = authenticated_participant(&state, &headers).await?;
    let request_fingerprint = state.recovery_request_fingerprint(
        "rotate_recovery_password",
        &key,
        &[
            request.current_recovery_password.as_bytes(),
            request.new_recovery_password.as_bytes(),
        ],
    );
    if let Some(stored) = postgres::load_recovery_password_rotation(&state.database, &key).await? {
        return replay_recovery_password_rotation(stored, participant_id, &request_fingerprint);
    }
    let observed = postgres::load_recovery_password_authority(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::session_invalid)?;
    if observed.role != "host" {
        return Err(ApiError::not_room_host());
    }
    if request.current_recovery_password.is_empty()
        || request.current_recovery_password.chars().count() > 128
    {
        return Err(ApiError::recovery_confirmation_failed());
    }
    let password_check_permit = state
        .try_recovery_password_check()
        .ok_or_else(ApiError::recovery_unavailable)?;
    if !verify_password(
        request.current_recovery_password,
        observed.recovery_password_hash.clone(),
    )
    .await?
    {
        return Err(ApiError::recovery_confirmation_failed());
    }
    let new_password_hash = hash_password(request.new_recovery_password).await?;
    drop(password_check_permit);

    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|error| ApiError::internal_with("recovery password rotation", error))?;
    let locked = postgres::lock_recovery_password_authority(&mut transaction, participant_id)
        .await?
        .ok_or_else(ApiError::recovery_confirmation_failed)?;
    if let Some(stored) =
        postgres::load_recovery_password_rotation_in_transaction(&mut transaction, &key).await?
    {
        let replayed =
            replay_recovery_password_rotation(stored, participant_id, &request_fingerprint);
        transaction
            .rollback()
            .await
            .map_err(|error| ApiError::internal_with("recovery password rotation", error))?;
        return replayed;
    }
    let locked = Some(locked)
        .filter(|locked| {
            locked.room_id == observed.room_id
                && locked.participant_id == observed.participant_id
                && locked.role == observed.role
                && locked.recovery_password_hash == observed.recovery_password_hash
                && locked.password_generation == observed.password_generation
        })
        .ok_or_else(ApiError::recovery_confirmation_failed)?;
    let claimed = postgres::claim_recovery_password_rotation(
        &mut transaction,
        &key,
        locked.room_id,
        locked.participant_id,
        &request_fingerprint,
    )
    .await?;
    if !claimed {
        let stored =
            postgres::load_recovery_password_rotation_in_transaction(&mut transaction, &key)
                .await?
                .ok_or_else(ApiError::internal)?;
        return replay_recovery_password_rotation(stored, participant_id, &request_fingerprint);
    }
    let event = postgres::rotate_recovery_password(
        &mut transaction,
        locked.room_id,
        locked.participant_id,
        &new_password_hash,
    )
    .await?;
    let room_id = locked.room_id;
    postgres::complete_recovery_password_rotation(&mut transaction, &key, &event).await?;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("recovery password rotation", error))?;
    state.signal_security_event(room_id);

    Ok(recovery_password_rotation_response(event))
}

fn replay_recovery_password_rotation(
    stored: StoredRecoveryPasswordRotation,
    participant_id: Uuid,
    request_fingerprint: &str,
) -> Result<Response, ApiError> {
    if stored.actor_participant_id != participant_id
        || stored.request_fingerprint != request_fingerprint
    {
        return Err(ApiError::idempotency_conflict());
    }
    Ok(recovery_password_rotation_response(StoredSecurityEvent {
        sequence: stored.sequence,
        actor_position: stored.actor_position,
        password_generation: stored.password_generation,
        occurred_at: stored.occurred_at,
    }))
}

fn recovery_password_rotation_response(event: StoredSecurityEvent) -> Response {
    no_store_json(
        StatusCode::OK,
        RotateRecoveryPasswordResponse {
            password_generation: event.password_generation,
            security_event: SecurityEvent {
                event_version: 1,
                cursor: event.sequence,
                event_type: "recovery_password_rotated",
                actor_position: event.actor_position,
                password_generation: event.password_generation,
                occurred_at: event.occurred_at,
            },
        },
    )
}

async fn regenerate_own_recovery_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_request): Json<RegenerateOwnRecoveryCredentialRequest>,
) -> Result<Response, ApiError> {
    regenerate_recovery_credential(state, headers, None, true).await
}

async fn regenerate_assisted_recovery_credential(
    State(state): State<AppState>,
    Path(position): Path<i16>,
    headers: HeaderMap,
    Json(request): Json<RegenerateAssistedRecoveryCredentialRequest>,
) -> Result<Response, ApiError> {
    regenerate_recovery_credential(
        state,
        headers,
        Some(position),
        request.host_assistance_risk_acknowledged,
    )
    .await
}

async fn regenerate_recovery_credential(
    state: AppState,
    headers: HeaderMap,
    target_position: Option<i16>,
    risk_acknowledged: bool,
) -> Result<Response, ApiError> {
    let command = recovery_credential_regeneration_command(
        &state,
        &headers,
        target_position,
        risk_acknowledged,
    )
    .await?;
    if let Some(stored) =
        postgres::load_recovery_credential_regeneration(&state.database, &command.idempotency_key)
            .await?
    {
        return replay_recovery_credential_regeneration(&state, stored, &command);
    }

    let (committed_room_id, response) =
        commit_recovery_credential_regeneration(&state, &command).await?;
    if let Some(room_id) = committed_room_id {
        state.signal_security_event(room_id);
    }
    Ok(response)
}

async fn recovery_credential_regeneration_command(
    state: &AppState,
    headers: &HeaderMap,
    target_position: Option<i16>,
    risk_acknowledged: bool,
) -> Result<RecoveryCredentialRegenerationCommand, ApiError> {
    let idempotency_key = idempotency_key(headers)?;
    require_session_grant_key(&idempotency_key)?;
    let actor_participant_id = authenticated_participant(state, headers).await?;
    if target_position.is_some() && !risk_acknowledged {
        return Err(ApiError::host_assistance_risk_not_acknowledged());
    }
    let delivery = if target_position.is_some() {
        "host_assisted"
    } else {
        "direct"
    };
    let target_position_value = target_position.map(|position| position.to_string());
    let request_fingerprint = match target_position_value.as_deref() {
        Some(position) => state.recovery_request_fingerprint(
            "regenerate_recovery_credential",
            &idempotency_key,
            &[
                delivery.as_bytes(),
                position.as_bytes(),
                b"risk-acknowledged",
            ],
        ),
        None => state.recovery_request_fingerprint(
            "regenerate_recovery_credential",
            &idempotency_key,
            &[delivery.as_bytes()],
        ),
    };
    Ok(RecoveryCredentialRegenerationCommand {
        idempotency_key,
        actor_participant_id,
        target_position,
        delivery,
        request_fingerprint,
    })
}

async fn commit_recovery_credential_regeneration(
    state: &AppState,
    command: &RecoveryCredentialRegenerationCommand,
) -> Result<(Option<Uuid>, Response), ApiError> {
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|error| ApiError::internal_with("recovery credential regeneration", error))?;
    let (authority, participant) =
        lock_recovery_credential_participants(&mut transaction, command).await?;
    if let Some(replayed) = replay_or_claim_recovery_credential_regeneration(
        state,
        &mut transaction,
        command,
        &authority,
        &participant,
    )
    .await?
    {
        transaction
            .rollback()
            .await
            .map_err(|error| ApiError::internal_with("recovery credential regeneration", error))?;
        return Ok((None, replayed));
    }

    let participant = postgres::advance_recovery_generation(&mut transaction, participant).await?;
    postgres::supersede_active_recovery_credentials(&mut transaction, participant.participant_id)
        .await?;
    let recovery_token = state.idempotent_recovery_token(
        "regenerate_recovery_credential",
        &command.idempotency_key,
        participant.participant_id,
    );
    let recovery_token_hmac = state.recovery_token_hmac(&recovery_token);
    postgres::insert_recovery_credential(
        &mut transaction,
        participant.participant_id,
        &recovery_token_hmac,
    )
    .await?;
    let event = postgres::append_recovery_credential_security_event(
        &mut transaction,
        authority.room_id,
        command.actor_participant_id,
        participant.participant_id,
        command.delivery,
        participant.recovery_generation,
    )
    .await?;
    postgres::complete_recovery_credential_regeneration(
        &mut transaction,
        &command.idempotency_key,
        participant.recovery_generation,
        event.sequence,
    )
    .await?;
    let room_id = authority.room_id;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("recovery credential regeneration", error))?;
    let response = recovery_credential_regeneration_response(
        recovery_token,
        participant,
        event,
        command.delivery,
    );
    Ok((Some(room_id), response))
}

async fn lock_recovery_credential_participants(
    transaction: &mut Transaction<'_, Postgres>,
    command: &RecoveryCredentialRegenerationCommand,
) -> Result<(StoredRecoveryPasswordAuthority, StoredRecoveryParticipant), ApiError> {
    let authority =
        postgres::lock_recovery_password_authority(transaction, command.actor_participant_id)
            .await?
            .ok_or_else(ApiError::session_invalid)?;
    if command.target_position.is_some() && authority.role != "host" {
        return Err(ApiError::not_room_host());
    }
    let participant = match command.target_position {
        Some(position) => postgres::lock_recovery_participant_by_position(
            transaction,
            authority.room_id,
            position,
        )
        .await?
        .ok_or_else(ApiError::room_participant_not_found)?,
        None => postgres::lock_recovery_participant(
            transaction,
            authority.room_id,
            command.actor_participant_id,
        )
        .await?
        .ok_or_else(ApiError::session_invalid)?,
    };
    if command.target_position.is_some()
        && participant.participant_id == command.actor_participant_id
    {
        return Err(ApiError::recovery_assistance_not_required());
    }
    Ok((authority, participant))
}

async fn replay_or_claim_recovery_credential_regeneration(
    state: &AppState,
    transaction: &mut Transaction<'_, Postgres>,
    command: &RecoveryCredentialRegenerationCommand,
    authority: &StoredRecoveryPasswordAuthority,
    participant: &StoredRecoveryParticipant,
) -> Result<Option<Response>, ApiError> {
    if let Some(stored) = postgres::load_recovery_credential_regeneration_in_transaction(
        transaction,
        &command.idempotency_key,
    )
    .await?
    {
        return replay_recovery_credential_regeneration(state, stored, command).map(Some);
    }
    let claimed = postgres::claim_recovery_credential_regeneration(
        transaction,
        &command.idempotency_key,
        authority.room_id,
        command.actor_participant_id,
        participant.participant_id,
        command.delivery,
        &command.request_fingerprint,
    )
    .await?;
    if claimed {
        return Ok(None);
    }
    let stored = postgres::load_recovery_credential_regeneration_in_transaction(
        transaction,
        &command.idempotency_key,
    )
    .await?
    .ok_or_else(ApiError::internal)?;
    replay_recovery_credential_regeneration(state, stored, command).map(Some)
}

fn replay_recovery_credential_regeneration(
    state: &AppState,
    stored: StoredRecoveryCredentialRegeneration,
    command: &RecoveryCredentialRegenerationCommand,
) -> Result<Response, ApiError> {
    if stored.actor_participant_id != command.actor_participant_id
        || stored.delivery != command.delivery
        || stored.request_fingerprint != command.request_fingerprint
        || command
            .target_position
            .is_some_and(|position| stored.target_position != position)
        || (command.target_position.is_none()
            && stored.target_participant_id != command.actor_participant_id)
    {
        return Err(ApiError::idempotency_conflict());
    }
    let recovery_token = state.idempotent_recovery_token(
        "regenerate_recovery_credential",
        &command.idempotency_key,
        stored.target_participant_id,
    );
    Ok(recovery_credential_regeneration_response(
        recovery_token,
        StoredRecoveryParticipant {
            participant_id: stored.target_participant_id,
            display_name: stored.target_display_name,
            position: stored.target_position,
            recovery_generation: stored.recovery_generation,
        },
        StoredRecoveryCredentialSecurityEvent {
            sequence: stored.sequence,
            actor_position: stored.actor_position,
            target_position: stored.target_position,
            recovery_generation: stored.recovery_generation,
            occurred_at: stored.occurred_at,
        },
        command.delivery,
    ))
}

fn recovery_credential_regeneration_response(
    recovery_token: String,
    participant: StoredRecoveryParticipant,
    event: StoredRecoveryCredentialSecurityEvent,
    delivery: &'static str,
) -> Response {
    no_store_json(
        StatusCode::OK,
        RegenerateRecoveryCredentialResponse {
            delivery,
            participant: RecoveryParticipant {
                display_name: participant.display_name,
                position: participant.position,
            },
            recovery_generation: participant.recovery_generation,
            recovery_token,
            risk_message_key: (delivery == "host_assisted")
                .then_some("participant.recovery.host_assisted_impersonation_risk"),
            security_event: RecoveryCredentialSecurityEvent {
                event_version: 1,
                cursor: event.sequence,
                event_type: "recovery_credential_regenerated",
                actor_position: event.actor_position,
                target_position: event.target_position,
                delivery,
                recovery_generation: event.recovery_generation,
                occurred_at: event.occurred_at,
            },
        },
    )
}

async fn recover_participation(
    State(state): State<AppState>,
    Json(request): Json<RecoverParticipationRequest>,
) -> Result<Response, ApiError> {
    let recovery = authenticate_recovery_request(&state, request).await?;

    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|error| ApiError::internal_with("participant recovery transaction", error))?;
    let locked = lock_authenticated_recovery(&mut transaction, &recovery).await?;

    let successor_recovery_token = state.idempotent_recovery_token(
        "recover_participation",
        &recovery.attempt_id.to_string(),
        locked.participant_id,
    );
    let session_max_age = if locked.status == "consumed"
        && locked.recovery_attempt_id == Some(recovery.attempt_id)
        && locked.replaced_device_session_id == recovery.replace_session_id
    {
        locked
            .session_max_age_seconds
            .ok_or_else(ApiError::recovery_failed)?
    } else {
        if locked.status != "active" {
            return Err(ApiError::recovery_failed());
        }
        let active_sessions =
            postgres::lock_active_device_sessions(&mut transaction, locked.participant_id).await?;
        let replacement = match (active_sessions.len(), recovery.replace_session_id) {
            (2, None) => {
                return Ok(no_store_json(
                    StatusCode::CONFLICT,
                    RecoveryReplacementRequiredResponse {
                        status: "replacement_required",
                        sessions: active_sessions
                            .into_iter()
                            .map(recovery_session_summary)
                            .collect(),
                    },
                ));
            }
            (2, Some(replace_session_id)) => Some(
                active_sessions
                    .iter()
                    .find(|session| session.id == replace_session_id)
                    .ok_or_else(ApiError::recovery_failed)?,
            ),
            (0 | 1, None) => None,
            _ => return Err(ApiError::recovery_failed()),
        };
        let slot = replacement.map_or_else(
            || {
                if active_sessions.iter().any(|session| session.slot == 1) {
                    2
                } else {
                    1
                }
            },
            |session| session.slot,
        );
        let successor_token_hmac = state.recovery_token_hmac(&successor_recovery_token);
        postgres::consume_recovery_credential(
            &mut transaction,
            &locked,
            postgres::NewRecoveredSession {
                guest_session_id: Uuid::new_v4(),
                device_session_id: Uuid::new_v4(),
                recovery_attempt_id: recovery.attempt_id,
                session_token: &recovery.session_token,
                slot,
                replacement,
                successor_token_hmac: &successor_token_hmac,
            },
        )
        .await?
    };
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("participant recovery transaction", error))?;
    if let Some(game_id) = locked.game_id {
        state.signal_game_synchronization(game_id);
    }

    let mut response =
        recovered_participation_response(&state, locked.participant_id, successor_recovery_token)
            .await?;
    set_session_cookie(&mut response, &recovery.session_token, session_max_age);
    Ok(response)
}

async fn lock_authenticated_recovery(
    transaction: &mut Transaction<'_, Postgres>,
    recovery: &AuthenticatedRecovery,
) -> Result<StoredRecoveryCandidate, ApiError> {
    let candidate = &recovery.candidate;
    let locked_room = postgres::lock_recovery_room(transaction, candidate.room_id)
        .await?
        .ok_or_else(ApiError::recovery_failed)?;
    let locked_participant = postgres::lock_recovery_participant(
        transaction,
        candidate.room_id,
        candidate.participant_id,
    )
    .await?
    .ok_or_else(ApiError::recovery_failed)?;
    if candidate.status == "active"
        && (locked_room.room_id != candidate.room_id
            || locked_room.recovery_password_hash != candidate.recovery_password_hash
            || locked_room.recovery_epoch != candidate.recovery_epoch
            || locked_room.password_generation != candidate.password_generation
            || locked_participant.recovery_generation != candidate.recovery_generation)
    {
        return Err(ApiError::recovery_failed());
    }
    postgres::lock_recovery_candidate(transaction, &recovery.token_hmac, recovery.attempt_id)
        .await?
        .filter(|locked| {
            locked.credential_id == candidate.credential_id
                && locked.room_id == candidate.room_id
                && locked.participant_id == candidate.participant_id
                && locked.guest_identity_id == candidate.guest_identity_id
                && locked.game_id == candidate.game_id
                && locked.recovery_password_hash == candidate.recovery_password_hash
                && locked.recovery_epoch == candidate.recovery_epoch
                && locked.password_generation == candidate.password_generation
                && locked.recovery_generation == candidate.recovery_generation
        })
        .ok_or_else(ApiError::recovery_failed)
}

async fn authenticate_recovery_request(
    state: &AppState,
    request: RecoverParticipationRequest,
) -> Result<AuthenticatedRecovery, ApiError> {
    let password_check_permit = state
        .try_recovery_password_check()
        .ok_or_else(ApiError::recovery_unavailable)?;
    let token_is_well_formed = request.token.len() == RECOVERY_TOKEN_LENGTH
        && request.token.bytes().all(|byte| byte.is_ascii_hexdigit());
    let token_for_hmac = if token_is_well_formed {
        request.token.as_str()
    } else {
        "malformed-recovery-token"
    };
    let token_hmac = state.recovery_token_hmac(token_for_hmac);
    let recovery_attempt_id = Uuid::parse_str(&request.attempt_id)
        .ok()
        .filter(|attempt_id| {
            attempt_id.get_version_num() == 4 && attempt_id.get_variant() == Variant::RFC4122
        });
    let replace_session_id = request
        .replace_session_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .ok()
        .filter(|replacement| {
            replacement.is_none_or(|session_id| {
                session_id.get_version_num() == 4 && session_id.get_variant() == Variant::RFC4122
            })
        });
    let candidate =
        postgres::load_recovery_candidate(&state.database, &token_hmac, recovery_attempt_id)
            .await?;
    let password_is_bounded =
        !request.password.is_empty() && request.password.chars().count() <= 128;
    let password_matches = if let Some(candidate) =
        candidate.as_ref().filter(|_| password_is_bounded)
    {
        verify_password(
            request.password.clone(),
            candidate.recovery_password_hash.clone(),
        )
        .await?
    } else {
        let _timing_equalizer = hash_password("invalid participant recovery".to_owned()).await?;
        false
    };
    let Some(candidate) = candidate.filter(|_| {
        token_is_well_formed
            && recovery_attempt_id.is_some()
            && replace_session_id.is_some()
            && password_matches
    }) else {
        return Err(ApiError::recovery_failed());
    };
    let recovery_attempt_id = recovery_attempt_id.expect("the candidate requires a valid attempt");
    let replace_session_id = replace_session_id.expect("the candidate requires a valid choice");
    let session_token = state.recovered_session_token(&request.token, candidate.participant_id);
    drop(password_check_permit);
    Ok(AuthenticatedRecovery {
        candidate,
        token_hmac,
        attempt_id: recovery_attempt_id,
        replace_session_id,
        session_token,
    })
}

async fn recovered_participation_response(
    state: &AppState,
    participant_id: Uuid,
    recovery_token: String,
) -> Result<Response, ApiError> {
    if let Some(projection) =
        match_runtime::projection_for_participant(state, participant_id).await?
    {
        return Ok(no_store_json(
            StatusCode::OK,
            RecoveredGameResponse {
                kind: "game",
                recovery_token,
                game: projection,
            },
        ));
    }

    let lobby = postgres::load_lobby(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(
        StatusCode::OK,
        RecoveredLobbyResponse {
            kind: "lobby",
            recovery_token,
            lobby: lobby_response(state, lobby),
        },
    ))
}

fn recovery_session_summary(session: StoredDeviceSession) -> RecoverySessionSummary {
    RecoverySessionSummary {
        id: session.id.to_string(),
        label: format!("Sessão {}", session.slot),
        created_at: session.created_at,
    }
}

pub(crate) async fn lobby_for_participant(
    state: &AppState,
    participant_id: Uuid,
) -> Result<LobbyResponse, ApiError> {
    let lobby = postgres::load_lobby(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::session_invalid)?;
    Ok(lobby_response(state, lobby))
}

async fn select_hero(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SelectHeroRequest>,
) -> Result<Response, ApiError> {
    let hero = parse_hero(&request.hero_id)?;
    let participant_id = authenticated_participant(&state, &headers).await?;
    let mut transaction =
        state.database.begin().await.map_err(|error| {
            ApiError::internal_with("identity access application operation", error)
        })?;
    let (room_id, room_status, _) =
        postgres::lock_participant_room(&mut transaction, participant_id)
            .await?
            .ok_or_else(ApiError::session_invalid)?;
    if room_status != "open" {
        return Err(ApiError::room_sealed());
    }

    if postgres::room_hero_owned_by_other(&mut transaction, room_id, participant_id, hero.as_str())
        .await?
    {
        return Err(ApiError::hero_unavailable());
    }

    postgres::update_participant_hero(&mut transaction, participant_id, hero.as_str()).await?;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("identity access application operation", error))?;
    let lobby = postgres::load_lobby(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;

    Ok(no_store_json(StatusCode::OK, lobby_response(&state, lobby)))
}

async fn set_readiness(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetReadinessRequest>,
) -> Result<Response, ApiError> {
    let participant_id = authenticated_participant(&state, &headers).await?;
    let mut transaction =
        state.database.begin().await.map_err(|error| {
            ApiError::internal_with("identity access application operation", error)
        })?;
    let (_, room_status, hero_id) =
        postgres::lock_participant_room(&mut transaction, participant_id)
            .await?
            .ok_or_else(ApiError::session_invalid)?;
    if room_status != "open" {
        return Err(ApiError::room_sealed());
    }
    if request.ready && hero_id.is_none() {
        return Err(ApiError::invalid_participant_heroes());
    }

    postgres::update_participant_readiness(&mut transaction, participant_id, request.ready).await?;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("identity access application operation", error))?;
    let lobby = postgres::load_lobby(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;

    Ok(no_store_json(StatusCode::OK, lobby_response(&state, lobby)))
}

async fn replay_room_join(
    state: &AppState,
    stored: StoredRoomJoin,
    idempotency_key: &str,
    room_code: &str,
    display_name: &str,
    hero: HeroId,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    if stored.room_code != room_code
        || stored.display_name != display_name
        || stored.hero_id != hero.as_str()
    {
        return Err(ApiError::idempotency_conflict());
    }
    if !is_session_grant_key(idempotency_key)
        && authenticated_participant(state, headers).await? != stored.participant_id
    {
        return Err(ApiError::session_invalid());
    }

    room_joined_response(state, stored, idempotency_key).await
}

async fn room_joined_response(
    state: &AppState,
    stored: StoredRoomJoin,
    idempotency_key: &str,
) -> Result<Response, ApiError> {
    let session_token =
        state.idempotent_session_token("join_room", idempotency_key, stored.participant_id);
    let recovery_token =
        state.idempotent_recovery_token("join_room", idempotency_key, stored.participant_id);
    let session_max_age =
        postgres::ensure_room_join_session_token(&state.database, idempotency_key, &session_token)
            .await?;
    let lobby = postgres::load_lobby(&state.database, stored.participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    let mut response = no_store_json(
        StatusCode::CREATED,
        ParticipantAccessResponse {
            lobby: lobby_response(state, lobby),
            recovery_token,
        },
    );
    set_session_cookie(&mut response, &session_token, session_max_age);
    Ok(response)
}

fn lobby_response(state: &AppState, stored: StoredLobby) -> LobbyResponse {
    let selected_heroes: Vec<String> = stored
        .participants
        .iter()
        .filter_map(|participant| participant.hero_id.clone())
        .collect();
    let mut current_participant = None;
    let participants = stored
        .participants
        .into_iter()
        .map(|participant| {
            let is_current = participant.id == stored.current_participant_id;
            let summary = participant_summary(participant);
            if is_current {
                current_participant = Some(ParticipantSummary {
                    display_name: summary.display_name.clone(),
                    role: summary.role.clone(),
                    position: summary.position,
                    ready: summary.ready,
                    hero: summary.hero.as_ref().map(|hero| HeroSummary {
                        id: hero.id,
                        name: hero.name,
                    }),
                });
            }
            summary
        })
        .collect();

    LobbyResponse {
        room: RoomSummary {
            code: stored.room_code,
            status: stored.room_status,
        },
        participant: current_participant.expect("the current participant belongs to the lobby"),
        participants,
        heroes: hero_availability(&selected_heroes),
        content_options: state.content.options(),
    }
}

fn hero_availability(selected_heroes: &[String]) -> Vec<HeroAvailability> {
    HeroId::ALL
        .into_iter()
        .map(|hero| HeroAvailability {
            id: hero.as_str(),
            name: hero.name(),
            available: !selected_heroes
                .iter()
                .any(|selected| selected == hero.as_str()),
        })
        .collect()
}

fn participant_summary(stored: StoredParticipant) -> ParticipantSummary {
    ParticipantSummary {
        display_name: stored.display_name,
        role: stored.role,
        position: stored.position,
        ready: stored.ready,
        hero: stored
            .hero_id
            .as_deref()
            .and_then(|hero_id| hero_id.parse().ok())
            .map(hero_summary),
    }
}

fn set_session_cookie(response: &mut Response, token: &str, max_age_seconds: i64) {
    let cookie = format!(
        "__Host-session={token}; Path=/; Max-Age={max_age_seconds}; Secure; HttpOnly; SameSite=Strict"
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("hexadecimal session tokens make a valid cookie"),
    );
}

fn require_session_grant_key(key: &str) -> Result<(), ApiError> {
    if !is_session_grant_key(key) {
        return Err(ApiError::invalid_idempotency_key());
    }
    Ok(())
}

fn is_session_grant_key(key: &str) -> bool {
    Uuid::parse_str(key).is_ok_and(|parsed| {
        parsed.get_version_num() == 4 && parsed.get_variant() == Variant::RFC4122
    })
}

async fn replay_room_creation(
    state: &AppState,
    stored: StoredRoomCreation,
    idempotency_key: &str,
    display_name: &str,
    password: String,
) -> Result<Response, ApiError> {
    let password_matches = verify_password(password, stored.recovery_password_hash.clone()).await?;

    if stored.display_name != display_name || !password_matches {
        return Err(ApiError::idempotency_conflict());
    }

    room_created_response(state, stored, idempotency_key).await
}

async fn room_created_response(
    state: &AppState,
    stored: StoredRoomCreation,
    idempotency_key: &str,
) -> Result<Response, ApiError> {
    let session_token =
        state.idempotent_session_token("create_room", idempotency_key, stored.participant_id);
    let recovery_token =
        state.idempotent_recovery_token("create_room", idempotency_key, stored.participant_id);
    let session_max_age = postgres::ensure_room_creation_session_token(
        &state.database,
        idempotency_key,
        &session_token,
    )
    .await?;
    let lobby = postgres::load_lobby(&state.database, stored.participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    let mut response = no_store_json(
        StatusCode::CREATED,
        ParticipantAccessResponse {
            lobby: lobby_response(state, lobby),
            recovery_token,
        },
    );
    set_session_cookie(&mut response, &session_token, session_max_age);
    Ok(response)
}

fn parse_hero(value: &str) -> Result<HeroId, ApiError> {
    value.parse().map_err(|_| ApiError::invalid_hero())
}

const fn hero_summary(hero: HeroId) -> HeroSummary {
    HeroSummary {
        id: hero.as_str(),
        name: hero.name(),
    }
}

fn random_room_code() -> Result<String, ApiError> {
    let mut random = [0_u8; ROOM_CODE_LENGTH];
    getrandom::fill(&mut random)
        .map_err(|error| ApiError::internal_with("identity access application operation", error))?;
    Ok(random
        .into_iter()
        .map(|byte| ROOM_CODE_ALPHABET[usize::from(byte) % ROOM_CODE_ALPHABET.len()] as char)
        .collect())
}
