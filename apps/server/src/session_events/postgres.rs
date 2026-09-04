use sqlx::PgPool;
use uuid::Uuid;

use super::{SessionSecurityEvent, StoredSecurityPosition};
use crate::http_support::ApiError;

pub(super) async fn load_security_position(
    database: &PgPool,
    participant_id: Uuid,
) -> Result<Option<StoredSecurityPosition>, ApiError> {
    sqlx::query_as::<_, StoredSecurityPosition>(
        r"
        SELECT
            rooms.id AS room_id,
            rooms.security_event_sequence AS cursor
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        LEFT JOIN games ON games.room_id = rooms.id
        WHERE participants.id = $1
          AND rooms.status <> 'cancelled'
          AND (games.id IS NULL OR (games.access_expired_at IS NULL AND games.expires_at > clock_timestamp()))
        ",
    )
    .bind(participant_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("load security event position", error))
}

pub(super) async fn load_security_events(
    database: &PgPool,
    room_id: Uuid,
    participant_id: Uuid,
    cursor: i64,
    upper_cursor: i64,
    limit: i64,
) -> Result<Vec<SessionSecurityEvent>, ApiError> {
    sqlx::query_as::<_, SessionSecurityEvent>(
        r"
        SELECT
            1::SMALLINT AS event_version,
            events.sequence AS cursor,
            events.event_type,
            actors.position AS actor_position,
            targets.position AS target_position,
            events.delivery,
            events.password_generation,
            events.recovery_generation,
            CASE
                WHEN events.session_slot IS NULL THEN NULL
                ELSE 'Sessão ' || events.session_slot::TEXT
            END AS session_label,
            events.revoked_session_count AS revoked_sessions,
            events.recovery_epoch,
            events.current_session_preserved,
            replace(
                to_char(events.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z' AS occurred_at
        FROM identity_security_event_recipients AS recipients
        JOIN identity_security_events AS events
          ON events.room_id = recipients.room_id
         AND events.sequence = recipients.security_event_sequence
        JOIN participants AS actors ON actors.id = events.actor_participant_id
        LEFT JOIN participants AS targets ON targets.id = events.target_participant_id
        WHERE recipients.room_id = $1
          AND recipients.participant_id = $2
          AND events.sequence > $3
          AND events.sequence <= $4
        ORDER BY events.sequence
        LIMIT $5
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .bind(cursor)
    .bind(upper_cursor)
    .bind(limit)
    .fetch_all(database)
    .await
    .map_err(|error| ApiError::internal_with("load security events", error))
}
