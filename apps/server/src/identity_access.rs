use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;

mod postgres;

const ROOM_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const ROOM_CODE_LENGTH: usize = 8;
const SESSION_BYTES: usize = 32;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/rooms", post(create_room))
        .route("/api/rooms/{room_code}", get(find_room))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRoomRequest {
    display_name: String,
    recovery_password: String,
}

#[derive(Serialize)]
struct CreateRoomResponse {
    room: RoomSummary,
    participant: ParticipantSummary,
}

#[derive(Serialize)]
struct FindRoomResponse {
    room: RoomSummary,
}

#[derive(Serialize)]
struct RoomSummary {
    code: String,
    status: String,
}

#[derive(Serialize)]
struct ParticipantSummary {
    display_name: String,
    role: &'static str,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    category: &'static str,
    retry: &'static str,
    message_key: &'static str,
    details: ErrorDetails,
    correlation_id: String,
}

#[derive(Default, Serialize)]
struct ErrorDetails {}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    category: &'static str,
    retry: &'static str,
    message_key: &'static str,
}

#[derive(FromRow)]
struct StoredRoomCreation {
    room_code: String,
    room_status: String,
    display_name: String,
    recovery_password_hash: String,
    session_token: String,
}

impl ApiError {
    fn invalid_request(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            category: "validation",
            retry: "after_correction",
            message_key,
        }
    }

    fn weak_password() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "WEAK_RECOVERY_PASSWORD",
            category: "validation",
            retry: "after_correction",
            message_key: "room.recovery_password.weak",
        }
    }

    fn idempotency_conflict() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "IDEMPOTENCY_KEY_REUSED",
            category: "conflict",
            retry: "with_new_idempotency_key",
            message_key: "request.idempotency_key.reused",
        }
    }

    fn room_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "ROOM_NOT_FOUND",
            category: "not_found",
            retry: "after_correction",
            message_key: "room.not_found",
        }
    }

    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            category: "internal",
            retry: "safe_to_retry",
            message_key: "internal.error",
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
        return replay_room_creation(stored, display_name, request.recovery_password).await;
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
        return replay_room_creation(stored, display_name, request.recovery_password).await;
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

    Ok(room_created_response(stored))
}

async fn find_room(
    State(state): State<AppState>,
    Path(room_code): Path<String>,
) -> Result<Response, ApiError> {
    let normalized_code = room_code.to_ascii_uppercase();
    let room = postgres::find_open_room(&state.database, &normalized_code)
        .await?
        .ok_or_else(ApiError::room_not_found)?;

    let mut response = Json(FindRoomResponse {
        room: RoomSummary {
            code: room.0,
            status: room.1,
        },
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn replay_room_creation(
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

    Ok(room_created_response(stored))
}

fn room_created_response(stored: StoredRoomCreation) -> Response {
    let cookie = format!(
        "__Host-session={}; Path=/; Max-Age=2592000; Secure; HttpOnly; SameSite=Strict",
        stored.session_token
    );
    let mut response = (
        StatusCode::CREATED,
        Json(CreateRoomResponse {
            room: RoomSummary {
                code: stored.room_code,
                status: stored.room_status,
            },
            participant: ParticipantSummary {
                display_name: stored.display_name,
                role: "host",
            },
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("hexadecimal session tokens make a valid cookie"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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

fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::invalid_request(
                "IDEMPOTENCY_KEY_REQUIRED",
                "request.idempotency_key.required",
            )
        })?;

    if !(8..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(ApiError::invalid_request(
            "INVALID_IDEMPOTENCY_KEY",
            "request.idempotency_key.invalid",
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
            "INVALID_DISPLAY_NAME",
            "participant.display_name.invalid",
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
    const COMMON_PASSWORDS: [&str; 5] = [
        "password",
        "password123",
        "123456789012",
        "qwertyuiop12",
        "harrypotter",
    ];

    password.chars().count() < 12
        || distinct_character_count(password, 4) < 4
        || COMMON_PASSWORDS
            .iter()
            .any(|candidate| password.eq_ignore_ascii_case(candidate))
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
