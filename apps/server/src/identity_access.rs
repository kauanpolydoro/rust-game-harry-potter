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
    code: ErrorCode,
    category: ErrorCategory,
    retry: RetryPolicy,
    message_key: MessageKey,
    details: ErrorDetails,
    correlation_id: String,
}

#[derive(Default, Serialize)]
struct ErrorDetails {}

struct ApiError {
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
    #[serde(rename = "INTERNAL_ERROR")]
    Internal,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ErrorCategory {
    Validation,
    Conflict,
    NotFound,
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
    #[serde(rename = "internal.error")]
    Internal,
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

    fn idempotency_conflict() -> Self {
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

    fn internal() -> Self {
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
