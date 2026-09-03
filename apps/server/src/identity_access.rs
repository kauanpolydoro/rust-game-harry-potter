use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{AppState, match_runtime};

mod postgres;

const ROOM_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const ROOM_CODE_LENGTH: usize = 8;
const SESSION_BYTES: usize = 32;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms", post(create_room))
        .route("/api/rooms/{room_code}", get(find_room))
        .route("/api/rooms/{room_code}/participants", post(join_room))
        .route("/api/session", get(restore_session))
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
struct LobbyResponse {
    room: RoomSummary,
    participant: ParticipantSummary,
    participants: Vec<ParticipantSummary>,
    heroes: Vec<HeroAvailability>,
    content_options: Vec<match_runtime::ContentManifestOption>,
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

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: ErrorCode,
    category: ErrorCategory,
    retry: RetryPolicy,
    message_key: MessageKey,
    details: ErrorDetails,
    correlation_id: String,
}

#[derive(Default, Serialize)]
struct ErrorDetails {}

pub(crate) struct ApiError {
    status: StatusCode,
    code: ErrorCode,
    category: ErrorCategory,
    retry: RetryPolicy,
    message_key: MessageKey,
}

#[derive(Serialize)]
enum ErrorCode {
    #[serde(rename = "IDEMPOTENCY_KEY_REQUIRED")]
    IdempotencyKeyRequired,
    #[serde(rename = "INVALID_IDEMPOTENCY_KEY")]
    InvalidIdempotencyKey,
    #[serde(rename = "INVALID_DISPLAY_NAME")]
    InvalidDisplayName,
    #[serde(rename = "WEAK_RECOVERY_PASSWORD")]
    WeakRecoveryPassword,
    #[serde(rename = "IDEMPOTENCY_KEY_REUSED")]
    IdempotencyKeyReused,
    #[serde(rename = "ROOM_NOT_FOUND")]
    RoomNotFound,
    #[serde(rename = "ROOM_UNAVAILABLE")]
    RoomUnavailable,
    #[serde(rename = "ROOM_FULL")]
    RoomFull,
    #[serde(rename = "INVALID_HERO")]
    InvalidHero,
    #[serde(rename = "HERO_UNAVAILABLE")]
    HeroUnavailable,
    #[serde(rename = "SESSION_INVALID")]
    SessionInvalid,
    #[serde(rename = "NOT_ROOM_HOST")]
    NotRoomHost,
    #[serde(rename = "ROOM_PARTICIPANT_COUNT_INVALID")]
    RoomParticipantCountInvalid,
    #[serde(rename = "ROOM_POSITIONS_INVALID")]
    RoomPositionsInvalid,
    #[serde(rename = "PARTICIPANT_HEROES_INVALID")]
    ParticipantHeroesInvalid,
    #[serde(rename = "PARTICIPANTS_NOT_READY")]
    ParticipantsNotReady,
    #[serde(rename = "CONTENT_NOT_PLAYABLE")]
    ContentNotPlayable,
    #[serde(rename = "ROOM_SEALED")]
    RoomSealed,
    #[serde(rename = "INTERNAL_ERROR")]
    Internal,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ErrorCategory {
    Validation,
    Conflict,
    NotFound,
    Authentication,
    Authorization,
    Internal,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RetryPolicy {
    AfterCorrection,
    WithNewIdempotencyKey,
    SafeToRetry,
}

#[derive(Serialize)]
enum MessageKey {
    #[serde(rename = "request.idempotency_key.required")]
    IdempotencyKeyRequired,
    #[serde(rename = "request.idempotency_key.invalid")]
    InvalidIdempotencyKey,
    #[serde(rename = "participant.display_name.invalid")]
    InvalidDisplayName,
    #[serde(rename = "room.recovery_password.weak")]
    WeakRecoveryPassword,
    #[serde(rename = "request.idempotency_key.reused")]
    IdempotencyKeyReused,
    #[serde(rename = "room.not_found")]
    RoomNotFound,
    #[serde(rename = "room.unavailable")]
    RoomUnavailable,
    #[serde(rename = "room.full")]
    RoomFull,
    #[serde(rename = "hero.invalid")]
    InvalidHero,
    #[serde(rename = "hero.unavailable")]
    HeroUnavailable,
    #[serde(rename = "session.invalid")]
    SessionInvalid,
    #[serde(rename = "room.host.required")]
    NotRoomHost,
    #[serde(rename = "room.participant_count.invalid")]
    RoomParticipantCountInvalid,
    #[serde(rename = "room.positions.invalid")]
    RoomPositionsInvalid,
    #[serde(rename = "room.heroes.invalid")]
    ParticipantHeroesInvalid,
    #[serde(rename = "room.participants.not_ready")]
    ParticipantsNotReady,
    #[serde(rename = "content.not_playable")]
    ContentNotPlayable,
    #[serde(rename = "room.sealed")]
    RoomSealed,
    #[serde(rename = "internal.error")]
    Internal,
}

#[derive(FromRow)]
struct StoredRoomCreation {
    participant_id: Uuid,
    display_name: String,
    recovery_password_hash: String,
    session_token: String,
}

#[derive(FromRow)]
struct StoredRoomJoin {
    participant_id: Uuid,
    room_code: String,
    display_name: String,
    hero_id: String,
    session_token: String,
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

struct StoredLobby {
    room_code: String,
    room_status: String,
    current_participant_id: Uuid,
    participants: Vec<StoredParticipant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeroId {
    Harry,
    Hermione,
    Neville,
    Ron,
}

impl HeroId {
    const ALL: [Self; 4] = [Self::Harry, Self::Hermione, Self::Neville, Self::Ron];

    fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "harry" => Ok(Self::Harry),
            "hermione" => Ok(Self::Hermione),
            "neville" => Ok(Self::Neville),
            "ron" => Ok(Self::Ron),
            _ => Err(ApiError::invalid_hero()),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Harry => "harry",
            Self::Hermione => "hermione",
            Self::Neville => "neville",
            Self::Ron => "ron",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Harry => "Harry",
            Self::Hermione => "Hermione",
            Self::Neville => "Neville",
            Self::Ron => "Ron",
        }
    }

    fn summary(self) -> HeroSummary {
        HeroSummary {
            id: self.as_str(),
            name: self.name(),
        }
    }
}

impl ApiError {
    fn invalid_request(code: ErrorCode, message_key: MessageKey) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            category: ErrorCategory::Validation,
            retry: RetryPolicy::AfterCorrection,
            message_key,
        }
    }

    fn weak_password() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: ErrorCode::WeakRecoveryPassword,
            category: ErrorCategory::Validation,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::WeakRecoveryPassword,
        }
    }

    pub(crate) fn idempotency_conflict() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::IdempotencyKeyReused,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::WithNewIdempotencyKey,
            message_key: MessageKey::IdempotencyKeyReused,
        }
    }

    fn room_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: ErrorCode::RoomNotFound,
            category: ErrorCategory::NotFound,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::RoomNotFound,
        }
    }

    pub(crate) fn room_unavailable() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: ErrorCode::RoomUnavailable,
            category: ErrorCategory::NotFound,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::RoomUnavailable,
        }
    }

    fn room_full() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::RoomFull,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::RoomFull,
        }
    }

    fn invalid_hero() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: ErrorCode::InvalidHero,
            category: ErrorCategory::Validation,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::InvalidHero,
        }
    }

    fn hero_unavailable() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::HeroUnavailable,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::HeroUnavailable,
        }
    }

    pub(crate) fn session_invalid() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: ErrorCode::SessionInvalid,
            category: ErrorCategory::Authentication,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::SessionInvalid,
        }
    }

    pub(crate) fn not_room_host() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: ErrorCode::NotRoomHost,
            category: ErrorCategory::Authorization,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::NotRoomHost,
        }
    }

    pub(crate) fn invalid_participant_count() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::RoomParticipantCountInvalid,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::RoomParticipantCountInvalid,
        }
    }

    pub(crate) fn invalid_positions() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::RoomPositionsInvalid,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::RoomPositionsInvalid,
        }
    }

    pub(crate) fn invalid_participant_heroes() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::ParticipantHeroesInvalid,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::ParticipantHeroesInvalid,
        }
    }

    pub(crate) fn participants_not_ready() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::ParticipantsNotReady,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::ParticipantsNotReady,
        }
    }

    pub(crate) fn content_not_playable() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: ErrorCode::ContentNotPlayable,
            category: ErrorCategory::Validation,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::ContentNotPlayable,
        }
    }

    pub(crate) fn room_sealed() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::RoomSealed,
            category: ErrorCategory::Conflict,
            retry: RetryPolicy::AfterCorrection,
            message_key: MessageKey::RoomSealed,
        }
    }

    pub(crate) fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: ErrorCode::Internal,
            category: ErrorCategory::Internal,
            retry: RetryPolicy::SafeToRetry,
            message_key: MessageKey::Internal,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    category: self.category,
                    retry: self.retry,
                    message_key: self.message_key,
                    details: ErrorDetails::default(),
                    correlation_id: Uuid::new_v4().to_string(),
                },
            }),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
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
        return replay_room_creation(&state, stored, display_name, request.recovery_password).await;
    }

    let password_hash = hash_password(request.recovery_password.clone()).await?;
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
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
        transaction
            .rollback()
            .await
            .map_err(|_| ApiError::internal())?;
        let stored = postgres::load_room_creation(&state.database, &idempotency_key)
            .await?
            .ok_or_else(ApiError::internal)?;
        return replay_room_creation(&state, stored, display_name, request.recovery_password).await;
    }

    let stored = postgres::persist_room_creation(
        &mut transaction,
        room_id,
        participant_id,
        guest_session_id,
        display_name,
        &password_hash,
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|_| ApiError::internal())?;

    room_created_response(&state, stored).await
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
    let hero = HeroId::parse(&request.hero_id)?;

    if let Some(stored) = postgres::load_room_join(&state.database, &idempotency_key).await? {
        return replay_room_join(&state, stored, &normalized_code, display_name, hero).await;
    }

    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
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
        transaction
            .rollback()
            .await
            .map_err(|_| ApiError::internal())?;
        let stored = postgres::load_room_join(&state.database, &idempotency_key)
            .await?
            .ok_or_else(ApiError::internal)?;
        return replay_room_join(&state, stored, &normalized_code, display_name, hero).await;
    }

    let position = postgres::next_room_position(&mut transaction, room_id).await?;
    if postgres::room_has_hero(&mut transaction, room_id, hero.as_str()).await? {
        return Err(ApiError::hero_unavailable());
    }

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
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|_| ApiError::internal())?;

    room_joined_response(&state, stored).await
}

async fn restore_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let participant_id = authenticated_participant(&state, &headers).await?;
    if let Some(projection) =
        match_runtime::projection_for_participant(&state.database, participant_id).await?
    {
        return Ok(no_store_json(StatusCode::OK, projection));
    }
    let lobby = postgres::load_lobby(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::session_invalid)?;

    Ok(no_store_json(StatusCode::OK, lobby_response(&state, lobby)))
}

async fn select_hero(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SelectHeroRequest>,
) -> Result<Response, ApiError> {
    let hero = HeroId::parse(&request.hero_id)?;
    let participant_id = authenticated_participant(&state, &headers).await?;
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
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
        .map_err(|_| ApiError::internal())?;
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
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
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
        .map_err(|_| ApiError::internal())?;
    let lobby = postgres::load_lobby(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;

    Ok(no_store_json(StatusCode::OK, lobby_response(&state, lobby)))
}

async fn replay_room_join(
    state: &AppState,
    stored: StoredRoomJoin,
    room_code: &str,
    display_name: &str,
    hero: HeroId,
) -> Result<Response, ApiError> {
    if stored.room_code != room_code
        || stored.display_name != display_name
        || stored.hero_id != hero.as_str()
    {
        return Err(ApiError::idempotency_conflict());
    }

    room_joined_response(state, stored).await
}

async fn room_joined_response(
    state: &AppState,
    stored: StoredRoomJoin,
) -> Result<Response, ApiError> {
    let lobby = postgres::load_lobby(&state.database, stored.participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    let mut response = no_store_json(StatusCode::CREATED, lobby_response(state, lobby));
    set_session_cookie(&mut response, &stored.session_token);
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
        content_options: match_runtime::content_options(state),
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
            .and_then(|hero_id| HeroId::parse(hero_id).ok())
            .map(HeroId::summary),
    }
}

pub(crate) fn no_store_json(status: StatusCode, body: impl Serialize) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn set_session_cookie(response: &mut Response, token: &str) {
    let cookie = format!(
        "__Host-session={token}; Path=/; Max-Age=2592000; Secure; HttpOnly; SameSite=Strict"
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("hexadecimal session tokens make a valid cookie"),
    );
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

pub(crate) async fn authenticated_participant(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Uuid, ApiError> {
    let token = session_token(headers)?;
    postgres::participant_for_session(&state.database, token)
        .await?
        .ok_or_else(ApiError::session_invalid)
}

async fn replay_room_creation(
    state: &AppState,
    stored: StoredRoomCreation,
    display_name: &str,
    password: String,
) -> Result<Response, ApiError> {
    let stored_hash = stored.recovery_password_hash.clone();
    let password_matches = tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&stored_hash).map_err(|_| ())?;
        Ok::<_, ()>(
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok(),
        )
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(|()| ApiError::internal())?;

    if stored.display_name != display_name || !password_matches {
        return Err(ApiError::idempotency_conflict());
    }

    room_created_response(state, stored).await
}

async fn room_created_response(
    state: &AppState,
    stored: StoredRoomCreation,
) -> Result<Response, ApiError> {
    let lobby = postgres::load_lobby(&state.database, stored.participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    let mut response = no_store_json(StatusCode::CREATED, lobby_response(state, lobby));
    set_session_cookie(&mut response, &stored.session_token);
    Ok(response)
}

async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|_| ())
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(|()| ApiError::internal())
}

pub(crate) fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::invalid_request(
                ErrorCode::IdempotencyKeyRequired,
                MessageKey::IdempotencyKeyRequired,
            )
        })?;

    if !(8..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(ApiError::invalid_request(
            ErrorCode::InvalidIdempotencyKey,
            MessageKey::InvalidIdempotencyKey,
        ));
    }

    Ok(key.to_owned())
}

fn validate_display_name(display_name: &str) -> Result<&str, ApiError> {
    let normalized = display_name.trim();
    if normalized.is_empty()
        || normalized.chars().count() > 40
        || normalized.chars().any(char::is_control)
    {
        return Err(ApiError::invalid_request(
            ErrorCode::InvalidDisplayName,
            MessageKey::InvalidDisplayName,
        ));
    }

    Ok(normalized)
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.chars().count() > 128 || weak_password(password) {
        return Err(ApiError::weak_password());
    }

    Ok(())
}

fn weak_password(password: &str) -> bool {
    const COMMON_FRAGMENTS: [&str; 7] = [
        "password",
        "qwerty",
        "123456",
        "abcdef",
        "senha",
        "harrypotter",
        "hogwarts",
    ];
    let normalized = password.to_lowercase();

    password.chars().count() < 12
        || distinct_character_count(password, 4) < 4
        || COMMON_FRAGMENTS
            .iter()
            .any(|candidate| normalized.contains(candidate))
        || is_repeated_pattern(password)
        || contains_ascii_sequence(&normalized, 4)
}

fn distinct_character_count(value: &str, limit: usize) -> usize {
    let mut distinct = Vec::with_capacity(limit);
    for character in value.chars() {
        if !distinct.contains(&character) {
            distinct.push(character);
            if distinct.len() == limit {
                break;
            }
        }
    }
    distinct.len()
}

fn is_repeated_pattern(value: &str) -> bool {
    let characters: Vec<char> = value.chars().collect();
    (1..=characters.len() / 2).any(|pattern_length| {
        characters.len().is_multiple_of(pattern_length)
            && characters
                .iter()
                .enumerate()
                .all(|(index, character)| *character == characters[index % pattern_length])
    })
}

fn contains_ascii_sequence(value: &str, minimum_length: usize) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(minimum_length).any(|window| {
        window
            .windows(2)
            .all(|pair| pair[1].is_ascii_alphanumeric() && pair[1] == pair[0].wrapping_add(1))
            || window
                .windows(2)
                .all(|pair| pair[1].is_ascii_alphanumeric() && pair[0] == pair[1].wrapping_add(1))
    })
}

fn random_room_code() -> Result<String, ApiError> {
    let mut random = [0_u8; ROOM_CODE_LENGTH];
    getrandom::fill(&mut random).map_err(|_| ApiError::internal())?;
    Ok(random
        .into_iter()
        .map(|byte| ROOM_CODE_ALPHABET[usize::from(byte) % ROOM_CODE_ALPHABET.len()] as char)
        .collect())
}

fn random_hex_token() -> Result<String, ApiError> {
    let mut random = [0_u8; SESSION_BYTES];
    getrandom::fill(&mut random).map_err(|_| ApiError::internal())?;
    let mut token = String::with_capacity(SESSION_BYTES * 2);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}").expect("writing to a string cannot fail");
    }
    Ok(token)
}
