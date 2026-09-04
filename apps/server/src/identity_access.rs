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
    participant_id: Uuid,
    guest_identity_id: Uuid,
    recovery_password_hash: String,
    status: String,
    recovery_attempt_id: Option<Uuid>,
    session_max_age_seconds: Option<i64>,
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

async fn recover_participation(
    State(state): State<AppState>,
    Json(request): Json<RecoverParticipationRequest>,
) -> Result<Response, ApiError> {
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
    let Some(candidate) = candidate
        .filter(|_| token_is_well_formed && recovery_attempt_id.is_some() && password_matches)
    else {
        return Err(ApiError::recovery_failed());
    };
    let recovery_attempt_id = recovery_attempt_id.expect("the candidate requires a valid attempt");
    drop(password_check_permit);

    let mut response = recovered_participation_response(&state, candidate.participant_id).await?;

    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|error| ApiError::internal_with("participant recovery transaction", error))?;
    let locked =
        postgres::lock_recovery_candidate(&mut transaction, &token_hmac, recovery_attempt_id)
            .await?
            .filter(|locked| {
                locked.credential_id == candidate.credential_id
                    && locked.participant_id == candidate.participant_id
                    && locked.guest_identity_id == candidate.guest_identity_id
                    && locked.recovery_password_hash == candidate.recovery_password_hash
            })
            .ok_or_else(ApiError::recovery_failed)?;

    let session_token = state.recovered_session_token(&request.token, locked.participant_id);
    let session_max_age = if locked.status == "consumed"
        && locked.recovery_attempt_id == Some(recovery_attempt_id)
    {
        locked
            .session_max_age_seconds
            .ok_or_else(ApiError::recovery_failed)?
    } else {
        if locked.status != "active"
            || postgres::active_session_count(&mut transaction, locked.participant_id).await? >= 2
        {
            return Err(ApiError::recovery_failed());
        }
        postgres::consume_recovery_credential(
            &mut transaction,
            &locked,
            Uuid::new_v4(),
            Uuid::new_v4(),
            recovery_attempt_id,
            &session_token,
        )
        .await?
    };
    transaction
        .commit()
        .await
        .map_err(|error| ApiError::internal_with("participant recovery transaction", error))?;

    set_session_cookie(&mut response, &session_token, session_max_age);
    Ok(response)
}

async fn recovered_participation_response(
    state: &AppState,
    participant_id: Uuid,
) -> Result<Response, ApiError> {
    if let Some(projection) =
        match_runtime::projection_for_participant(&state.database, participant_id).await?
    {
        return Ok(no_store_json(StatusCode::OK, projection));
    }

    let lobby = postgres::load_lobby(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(StatusCode::OK, lobby_response(state, lobby)))
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
