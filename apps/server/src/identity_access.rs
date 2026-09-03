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
use game_domain::HeroId;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::{Uuid, Variant};

use crate::{
    AppState,
    http_support::{ApiError, idempotency_key, no_store_json},
    match_runtime,
    session::authenticated_participant,
};

mod postgres;

const ROOM_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const ROOM_CODE_LENGTH: usize = 8;

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
    let stored = postgres::persist_room_creation(
        &mut transaction,
        room_id,
        participant_id,
        guest_session_id,
        display_name,
        &password_hash,
        &session_token,
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
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("identity access application operation", error))?;

    room_joined_response(&state, stored, &idempotency_key).await
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
    let session_max_age =
        postgres::ensure_room_join_session_token(&state.database, idempotency_key, &session_token)
            .await?;
    let lobby = postgres::load_lobby(&state.database, stored.participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    let mut response = no_store_json(StatusCode::CREATED, lobby_response(state, lobby));
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
    .map_err(|error| ApiError::internal_with("identity access application operation", error))?
    .map_err(|()| ApiError::internal())?;

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
    let session_max_age = postgres::ensure_room_creation_session_token(
        &state.database,
        idempotency_key,
        &session_token,
    )
    .await?;
    let lobby = postgres::load_lobby(&state.database, stored.participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    let mut response = no_store_json(StatusCode::CREATED, lobby_response(state, lobby));
    set_session_cookie(&mut response, &session_token, session_max_age);
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
    .map_err(|error| ApiError::internal_with("identity access application operation", error))?
    .map_err(|()| ApiError::internal())
}

fn validate_display_name(display_name: &str) -> Result<&str, ApiError> {
    let normalized = display_name.trim();
    if normalized.is_empty()
        || normalized.chars().count() > 40
        || normalized.chars().any(char::is_control)
    {
        return Err(ApiError::invalid_display_name());
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

fn parse_hero(value: &str) -> Result<HeroId, ApiError> {
    value.parse().map_err(|_| ApiError::invalid_hero())
}

const fn hero_summary(hero: HeroId) -> HeroSummary {
    HeroSummary {
        id: hero.as_str(),
        name: hero.name(),
    }
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
    getrandom::fill(&mut random)
        .map_err(|error| ApiError::internal_with("identity access application operation", error))?;
    Ok(random
        .into_iter()
        .map(|byte| ROOM_CODE_ALPHABET[usize::from(byte) % ROOM_CODE_ALPHABET.len()] as char)
        .collect())
}
