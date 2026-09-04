use std::fmt::Display;

use axum::{
    Json,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

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
    #[serde(rename = "INVALID_COMMAND_ID")]
    InvalidCommandId,
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
    #[serde(rename = "RECOVERY_FAILED")]
    RecoveryFailed,
    #[serde(rename = "RECOVERY_CONFIRMATION_FAILED")]
    RecoveryConfirmationFailed,
    #[serde(rename = "PROTECTION_CONFIRMATION_REQUIRED")]
    ProtectionConfirmationRequired,
    #[serde(rename = "NOT_ROOM_HOST")]
    NotRoomHost,
    #[serde(rename = "ROOM_PARTICIPANT_NOT_FOUND")]
    RoomParticipantNotFound,
    #[serde(rename = "HOST_ASSISTANCE_RISK_NOT_ACKNOWLEDGED")]
    HostAssistanceRiskNotAcknowledged,
    #[serde(rename = "RECOVERY_ASSISTANCE_NOT_REQUIRED")]
    RecoveryAssistanceNotRequired,
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
    #[serde(rename = "STALE_STATE_VERSION")]
    StaleStateVersion,
    #[serde(rename = "GAME_ACTION_NOT_ALLOWED")]
    GameActionNotAllowed,
    #[serde(rename = "CHOICE_NOT_ASSIGNED")]
    ChoiceNotAssigned,
    #[serde(rename = "GAME_EXPIRED")]
    GameExpired,
    #[serde(rename = "COMMAND_NOT_FOUND")]
    CommandNotFound,
    #[serde(rename = "ORIGIN_NOT_ALLOWED")]
    OriginNotAllowed,
    #[serde(rename = "UPGRADE_REQUIRED")]
    UpgradeRequired,
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
    #[serde(rename = "game.command_id.invalid")]
    InvalidCommandId,
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
    #[serde(rename = "participant.recovery.failed")]
    RecoveryFailed,
    #[serde(rename = "room.recovery_password.confirmation_failed")]
    RecoveryConfirmationFailed,
    #[serde(rename = "access.protection.confirmation_required")]
    ProtectionConfirmationRequired,
    #[serde(rename = "room.host.required")]
    NotRoomHost,
    #[serde(rename = "room.participant.not_found")]
    RoomParticipantNotFound,
    #[serde(rename = "participant.recovery.host_assistance_risk_not_acknowledged")]
    HostAssistanceRiskNotAcknowledged,
    #[serde(rename = "participant.recovery.assistance_not_required")]
    RecoveryAssistanceNotRequired,
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
    #[serde(rename = "game.state.stale")]
    StaleStateVersion,
    #[serde(rename = "game.action.not_allowed")]
    GameActionNotAllowed,
    #[serde(rename = "game.choice.not_assigned")]
    ChoiceNotAssigned,
    #[serde(rename = "game.expired")]
    GameExpired,
    #[serde(rename = "game.command.not_found")]
    CommandNotFound,
    #[serde(rename = "realtime.origin.not_allowed")]
    OriginNotAllowed,
    #[serde(rename = "realtime.upgrade.required")]
    UpgradeRequired,
    #[serde(rename = "internal.error")]
    Internal,
}

impl ApiError {
    const fn new(
        status: StatusCode,
        code: ErrorCode,
        category: ErrorCategory,
        retry: RetryPolicy,
        message_key: MessageKey,
    ) -> Self {
        Self {
            status,
            code,
            category,
            retry,
            message_key,
        }
    }

    pub(crate) const fn idempotency_key_required() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::IdempotencyKeyRequired,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::IdempotencyKeyRequired,
        )
    }

    pub(crate) const fn invalid_idempotency_key() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidIdempotencyKey,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::InvalidIdempotencyKey,
        )
    }

    pub(crate) const fn invalid_command_id() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidCommandId,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::InvalidCommandId,
        )
    }

    pub(crate) const fn invalid_display_name() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidDisplayName,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::InvalidDisplayName,
        )
    }

    pub(crate) const fn weak_password() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::WeakRecoveryPassword,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::WeakRecoveryPassword,
        )
    }

    pub(crate) const fn idempotency_conflict() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::IdempotencyKeyReused,
            ErrorCategory::Conflict,
            RetryPolicy::WithNewIdempotencyKey,
            MessageKey::IdempotencyKeyReused,
        )
    }

    pub(crate) const fn room_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::RoomNotFound,
            ErrorCategory::NotFound,
            RetryPolicy::AfterCorrection,
            MessageKey::RoomNotFound,
        )
    }

    pub(crate) const fn room_unavailable() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::RoomUnavailable,
            ErrorCategory::NotFound,
            RetryPolicy::AfterCorrection,
            MessageKey::RoomUnavailable,
        )
    }

    pub(crate) const fn room_full() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::RoomFull,
            ErrorCategory::Conflict,
            RetryPolicy::AfterCorrection,
            MessageKey::RoomFull,
        )
    }

    pub(crate) const fn invalid_hero() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidHero,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::InvalidHero,
        )
    }

    pub(crate) const fn hero_unavailable() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::HeroUnavailable,
            ErrorCategory::Conflict,
            RetryPolicy::AfterCorrection,
            MessageKey::HeroUnavailable,
        )
    }

    pub(crate) const fn session_invalid() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::SessionInvalid,
            ErrorCategory::Authentication,
            RetryPolicy::AfterCorrection,
            MessageKey::SessionInvalid,
        )
    }

    pub(crate) const fn recovery_failed() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::RecoveryFailed,
            ErrorCategory::Authentication,
            RetryPolicy::AfterCorrection,
            MessageKey::RecoveryFailed,
        )
    }

    pub(crate) const fn recovery_confirmation_failed() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::RecoveryConfirmationFailed,
            ErrorCategory::Authentication,
            RetryPolicy::AfterCorrection,
            MessageKey::RecoveryConfirmationFailed,
        )
    }

    pub(crate) const fn protection_confirmation_required() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ProtectionConfirmationRequired,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::ProtectionConfirmationRequired,
        )
    }

    pub(crate) const fn recovery_unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal,
            ErrorCategory::Internal,
            RetryPolicy::SafeToRetry,
            MessageKey::Internal,
        )
    }

    pub(crate) const fn not_room_host() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ErrorCode::NotRoomHost,
            ErrorCategory::Authorization,
            RetryPolicy::AfterCorrection,
            MessageKey::NotRoomHost,
        )
    }

    pub(crate) const fn room_participant_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::RoomParticipantNotFound,
            ErrorCategory::NotFound,
            RetryPolicy::AfterCorrection,
            MessageKey::RoomParticipantNotFound,
        )
    }

    pub(crate) const fn host_assistance_risk_not_acknowledged() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::HostAssistanceRiskNotAcknowledged,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::HostAssistanceRiskNotAcknowledged,
        )
    }

    pub(crate) const fn recovery_assistance_not_required() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::RecoveryAssistanceNotRequired,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::RecoveryAssistanceNotRequired,
        )
    }

    pub(crate) const fn invalid_participant_count() -> Self {
        Self::conflict(
            ErrorCode::RoomParticipantCountInvalid,
            MessageKey::RoomParticipantCountInvalid,
        )
    }

    pub(crate) const fn invalid_positions() -> Self {
        Self::conflict(
            ErrorCode::RoomPositionsInvalid,
            MessageKey::RoomPositionsInvalid,
        )
    }

    pub(crate) const fn invalid_participant_heroes() -> Self {
        Self::conflict(
            ErrorCode::ParticipantHeroesInvalid,
            MessageKey::ParticipantHeroesInvalid,
        )
    }

    pub(crate) const fn participants_not_ready() -> Self {
        Self::conflict(
            ErrorCode::ParticipantsNotReady,
            MessageKey::ParticipantsNotReady,
        )
    }

    pub(crate) const fn content_not_playable() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ContentNotPlayable,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::ContentNotPlayable,
        )
    }

    pub(crate) const fn room_sealed() -> Self {
        Self::conflict(ErrorCode::RoomSealed, MessageKey::RoomSealed)
    }

    pub(crate) const fn stale_state_version() -> Self {
        Self::conflict(ErrorCode::StaleStateVersion, MessageKey::StaleStateVersion)
    }

    pub(crate) const fn game_action_not_allowed() -> Self {
        Self::conflict(
            ErrorCode::GameActionNotAllowed,
            MessageKey::GameActionNotAllowed,
        )
    }

    pub(crate) const fn choice_not_assigned() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ErrorCode::ChoiceNotAssigned,
            ErrorCategory::Authorization,
            RetryPolicy::AfterCorrection,
            MessageKey::ChoiceNotAssigned,
        )
    }

    pub(crate) const fn game_expired() -> Self {
        Self::new(
            StatusCode::GONE,
            ErrorCode::GameExpired,
            ErrorCategory::Conflict,
            RetryPolicy::AfterCorrection,
            MessageKey::GameExpired,
        )
    }

    pub(crate) const fn command_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::CommandNotFound,
            ErrorCategory::NotFound,
            RetryPolicy::AfterCorrection,
            MessageKey::CommandNotFound,
        )
    }

    pub(crate) const fn origin_not_allowed() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ErrorCode::OriginNotAllowed,
            ErrorCategory::Authorization,
            RetryPolicy::AfterCorrection,
            MessageKey::OriginNotAllowed,
        )
    }

    pub(crate) const fn upgrade_required() -> Self {
        Self::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            ErrorCategory::Validation,
            RetryPolicy::AfterCorrection,
            MessageKey::UpgradeRequired,
        )
    }

    pub(crate) const fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            ErrorCategory::Internal,
            RetryPolicy::SafeToRetry,
            MessageKey::Internal,
        )
    }

    pub(crate) fn internal_with(operation: &'static str, error: impl Display) -> Self {
        tracing::error!(operation, error = %error, "internal operation failed");
        Self::internal()
    }

    const fn conflict(code: ErrorCode, message_key: MessageKey) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            code,
            ErrorCategory::Conflict,
            RetryPolicy::AfterCorrection,
            message_key,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let correlation_id = crate::current_correlation_id();
        let game_expired = matches!(self.code, ErrorCode::GameExpired);
        let mut response = (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    category: self.category,
                    retry: self.retry,
                    message_key: self.message_key,
                    details: ErrorDetails::default(),
                    correlation_id: correlation_id.to_string(),
                },
            }),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if game_expired {
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_static(
                    "__Host-session=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict",
                ),
            );
        }
        if let Ok(value) = HeaderValue::from_str(&correlation_id.to_string()) {
            response.headers_mut().insert("x-correlation-id", value);
        }
        response
    }
}

pub(crate) fn no_store_json(status: StatusCode, body: impl Serialize) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::idempotency_key_required)?;

    if !(8..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(ApiError::invalid_idempotency_key());
    }

    Ok(key.to_owned())
}
