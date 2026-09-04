use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ApiError, StoredLobby, StoredParticipant, StoredRecoveryCandidate,
    StoredRecoveryCredentialRegeneration, StoredRecoveryCredentialSecurityEvent,
    StoredRecoveryParticipant, StoredRecoveryPasswordAuthority, StoredRecoveryPasswordRotation,
    StoredRecoveryRoom, StoredRoomCreation, StoredRoomJoin, StoredSecurityEvent, random_room_code,
};

pub(super) struct NewRoomJoin<'a> {
    pub(super) room_id: Uuid,
    pub(super) participant_id: Uuid,
    pub(super) guest_session_id: Uuid,
    pub(super) room_code: &'a str,
    pub(super) display_name: &'a str,
    pub(super) hero_id: &'a str,
    pub(super) position: i16,
}

pub(super) struct NewRoomCreation<'a> {
    pub(super) room_id: Uuid,
    pub(super) participant_id: Uuid,
    pub(super) guest_session_id: Uuid,
    pub(super) display_name: &'a str,
    pub(super) password_hash: &'a str,
}

pub(super) async fn claim_room_creation(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    room_id: Uuid,
    participant_id: Uuid,
    guest_session_id: Uuid,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO room_creation_requests (
            idempotency_key,
            room_id,
            participant_id,
            guest_session_id
        )
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (idempotency_key) DO NOTHING
        RETURNING idempotency_key
        ",
    )
    .bind(idempotency_key)
    .bind(room_id)
    .bind(participant_id)
    .bind(guest_session_id)
    .fetch_optional(&mut **transaction)
    .await
    .map(|claim| claim.is_some())
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn find_open_room(
    database: &PgPool,
    room_code: &str,
) -> Result<Option<(String, String)>, ApiError> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT code, status FROM rooms WHERE code = $1 AND status = 'open'",
    )
    .bind(room_code)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn selected_room_heroes(
    database: &PgPool,
    room_code: &str,
) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        r"
        SELECT participants.hero_id
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
          AND rooms.status = 'open'
          AND participants.hero_id IS NOT NULL
        ORDER BY participants.position
        ",
    )
    .bind(room_code)
    .fetch_all(database)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn load_recovery_password_authority(
    database: &PgPool,
    participant_id: Uuid,
) -> Result<Option<StoredRecoveryPasswordAuthority>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryPasswordAuthority>(
        r"
        SELECT
            rooms.id AS room_id,
            participants.id AS participant_id,
            participants.role,
            rooms.recovery_password_hash,
            rooms.password_generation
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        LEFT JOIN games ON games.room_id = rooms.id
        WHERE participants.id = $1
          AND rooms.status <> 'cancelled'
          AND (games.id IS NULL OR games.expires_at > clock_timestamp())
        ",
    )
    .bind(participant_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("load recovery password authority", error))
}

pub(super) async fn lock_recovery_password_authority(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
) -> Result<Option<StoredRecoveryPasswordAuthority>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryPasswordAuthority>(
        r"
        SELECT
            rooms.id AS room_id,
            participants.id AS participant_id,
            participants.role,
            rooms.recovery_password_hash,
            rooms.password_generation
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        LEFT JOIN games ON games.room_id = rooms.id
        WHERE participants.id = $1
          AND rooms.status <> 'cancelled'
          AND (games.id IS NULL OR games.expires_at > clock_timestamp())
        FOR UPDATE OF rooms
        ",
    )
    .bind(participant_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("lock recovery password authority", error))
}

pub(super) async fn load_recovery_password_rotation(
    database: &PgPool,
    idempotency_key: &str,
) -> Result<Option<StoredRecoveryPasswordRotation>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryPasswordRotation>(RECOVERY_PASSWORD_ROTATION_SELECT)
        .bind(idempotency_key)
        .fetch_optional(database)
        .await
        .map_err(|error| ApiError::internal_with("load recovery password rotation", error))
}

pub(super) async fn load_recovery_password_rotation_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
) -> Result<Option<StoredRecoveryPasswordRotation>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryPasswordRotation>(RECOVERY_PASSWORD_ROTATION_SELECT)
        .bind(idempotency_key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| ApiError::internal_with("load recovery password rotation", error))
}

const RECOVERY_PASSWORD_ROTATION_SELECT: &str = r"
    SELECT
        requests.actor_participant_id,
        requests.request_fingerprint,
        requests.password_generation,
        events.sequence,
        participants.position AS actor_position,
        replace(
            to_char(events.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
            ' ',
            'T'
        ) || 'Z' AS occurred_at
    FROM recovery_password_rotation_requests AS requests
    JOIN identity_security_events AS events
      ON events.room_id = requests.room_id
     AND events.sequence = requests.security_event_sequence
    JOIN participants ON participants.id = requests.actor_participant_id
    WHERE requests.idempotency_key = $1
      AND requests.completed_at IS NOT NULL
";

pub(super) async fn claim_recovery_password_rotation(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    room_id: Uuid,
    actor_participant_id: Uuid,
    request_fingerprint: &str,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO recovery_password_rotation_requests (
            idempotency_key,
            room_id,
            actor_participant_id,
            request_fingerprint
        )
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (idempotency_key) DO NOTHING
        RETURNING idempotency_key
        ",
    )
    .bind(idempotency_key)
    .bind(room_id)
    .bind(actor_participant_id)
    .bind(request_fingerprint)
    .fetch_optional(&mut **transaction)
    .await
    .map(|claim| claim.is_some())
    .map_err(|error| ApiError::internal_with("claim recovery password rotation", error))
}

pub(super) async fn complete_recovery_password_rotation(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    event: &StoredSecurityEvent,
) -> Result<(), ApiError> {
    let completed = sqlx::query(
        r"
        UPDATE recovery_password_rotation_requests
        SET
            password_generation = $2,
            security_event_sequence = $3,
            completed_at = clock_timestamp()
        WHERE idempotency_key = $1
          AND completed_at IS NULL
        ",
    )
    .bind(idempotency_key)
    .bind(event.password_generation)
    .bind(event.sequence)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("complete recovery password rotation", error))?;
    if completed.rows_affected() != 1 {
        return Err(ApiError::internal());
    }
    Ok(())
}

pub(super) async fn rotate_recovery_password(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    actor_participant_id: Uuid,
    new_password_hash: &str,
) -> Result<StoredSecurityEvent, ApiError> {
    sqlx::query_as::<_, StoredSecurityEvent>(
        r"
        WITH rotated AS (
            UPDATE rooms
            SET
                recovery_password_hash = $3,
                password_generation = password_generation + 1,
                security_event_sequence = security_event_sequence + 1
            WHERE id = $1
            RETURNING password_generation, security_event_sequence
        ),
        superseded AS (
            UPDATE recovery_credentials
            SET
                status = 'superseded',
                superseded_at = clock_timestamp()
            FROM participants, rotated
            WHERE participants.room_id = $1
              AND recovery_credentials.participant_id = participants.id
              AND recovery_credentials.status = 'active'
            RETURNING recovery_credentials.id
        ),
        inserted AS (
            INSERT INTO identity_security_events (
                room_id,
                sequence,
                event_type,
                actor_participant_id,
                password_generation
            )
            SELECT
                $1,
                rotated.security_event_sequence,
                'recovery_password_rotated',
                $2,
                rotated.password_generation
            FROM rotated
            RETURNING sequence, actor_participant_id, password_generation, created_at
        ),
        inserted_recipients AS (
            INSERT INTO identity_security_event_recipients (
                room_id,
                security_event_sequence,
                participant_id
            )
            SELECT $1, inserted.sequence, participants.id
            FROM inserted
            JOIN participants ON participants.room_id = $1
            RETURNING participant_id
        )
        SELECT
            inserted.sequence,
            participants.position AS actor_position,
            inserted.password_generation,
            replace(
                to_char(inserted.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z' AS occurred_at
        FROM inserted
        JOIN participants ON participants.id = inserted.actor_participant_id
        WHERE (SELECT COUNT(*) FROM inserted_recipients) >= 1
        ",
    )
    .bind(room_id)
    .bind(actor_participant_id)
    .bind(new_password_hash)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("rotate recovery password", error))
}

pub(super) async fn load_recovery_credential_regeneration(
    database: &PgPool,
    idempotency_key: &str,
) -> Result<Option<StoredRecoveryCredentialRegeneration>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryCredentialRegeneration>(
        RECOVERY_CREDENTIAL_REGENERATION_SELECT,
    )
    .bind(idempotency_key)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("load recovery credential regeneration", error))
}

pub(super) async fn load_recovery_credential_regeneration_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
) -> Result<Option<StoredRecoveryCredentialRegeneration>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryCredentialRegeneration>(
        RECOVERY_CREDENTIAL_REGENERATION_SELECT,
    )
    .bind(idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("load recovery credential regeneration", error))
}

const RECOVERY_CREDENTIAL_REGENERATION_SELECT: &str = r"
    SELECT
        requests.actor_participant_id,
        requests.target_participant_id,
        requests.delivery,
        requests.request_fingerprint,
        requests.recovery_generation,
        events.sequence,
        actors.position AS actor_position,
        targets.display_name AS target_display_name,
        targets.position AS target_position,
        replace(
            to_char(events.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
            ' ',
            'T'
        ) || 'Z' AS occurred_at
    FROM recovery_credential_regeneration_requests AS requests
    JOIN identity_security_events AS events
      ON events.room_id = requests.room_id
     AND events.sequence = requests.security_event_sequence
    JOIN participants AS actors ON actors.id = requests.actor_participant_id
    JOIN participants AS targets ON targets.id = requests.target_participant_id
    WHERE requests.idempotency_key = $1
      AND requests.completed_at IS NOT NULL
";

pub(super) async fn claim_recovery_credential_regeneration(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    room_id: Uuid,
    actor_participant_id: Uuid,
    target_participant_id: Uuid,
    delivery: &str,
    request_fingerprint: &str,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO recovery_credential_regeneration_requests (
            idempotency_key,
            room_id,
            actor_participant_id,
            target_participant_id,
            delivery,
            request_fingerprint
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (idempotency_key) DO NOTHING
        RETURNING idempotency_key
        ",
    )
    .bind(idempotency_key)
    .bind(room_id)
    .bind(actor_participant_id)
    .bind(target_participant_id)
    .bind(delivery)
    .bind(request_fingerprint)
    .fetch_optional(&mut **transaction)
    .await
    .map(|claim| claim.is_some())
    .map_err(|error| ApiError::internal_with("claim recovery credential regeneration", error))
}

pub(super) async fn lock_recovery_participant(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    participant_id: Uuid,
) -> Result<Option<StoredRecoveryParticipant>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryParticipant>(
        r"
        SELECT
            id AS participant_id,
            display_name,
            position,
            recovery_generation
        FROM participants
        WHERE room_id = $1
          AND id = $2
        FOR NO KEY UPDATE
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("lock recovery participant", error))
}

pub(super) async fn lock_recovery_participant_by_position(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    position: i16,
) -> Result<Option<StoredRecoveryParticipant>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryParticipant>(
        r"
        SELECT
            id AS participant_id,
            display_name,
            position,
            recovery_generation
        FROM participants
        WHERE room_id = $1
          AND position = $2
        FOR NO KEY UPDATE
        ",
    )
    .bind(room_id)
    .bind(position)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("lock recovery participant by position", error))
}

pub(super) async fn advance_recovery_generation(
    transaction: &mut Transaction<'_, Postgres>,
    participant: StoredRecoveryParticipant,
) -> Result<StoredRecoveryParticipant, ApiError> {
    sqlx::query_as::<_, StoredRecoveryParticipant>(
        r"
        UPDATE participants
        SET recovery_generation = recovery_generation + 1
        WHERE id = $1
          AND recovery_generation = $2
        RETURNING
            id AS participant_id,
            display_name,
            position,
            recovery_generation
        ",
    )
    .bind(participant.participant_id)
    .bind(participant.recovery_generation)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("advance recovery generation", error))?
    .ok_or_else(ApiError::internal)
}

pub(super) async fn supersede_active_recovery_credentials(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        r"
        UPDATE recovery_credentials
        SET
            status = 'superseded',
            superseded_at = clock_timestamp()
        WHERE participant_id = $1
          AND status = 'active'
        ",
    )
    .bind(participant_id)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| ApiError::internal_with("supersede recovery credentials", error))
}

pub(super) async fn append_recovery_credential_security_event(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    actor_participant_id: Uuid,
    target_participant_id: Uuid,
    delivery: &str,
    recovery_generation: i64,
) -> Result<StoredRecoveryCredentialSecurityEvent, ApiError> {
    sqlx::query_as::<_, StoredRecoveryCredentialSecurityEvent>(
        r"
        WITH advanced_room AS (
            UPDATE rooms
            SET security_event_sequence = security_event_sequence + 1
            WHERE id = $1
            RETURNING security_event_sequence
        ),
        inserted_event AS (
            INSERT INTO identity_security_events (
                room_id,
                sequence,
                event_type,
                actor_participant_id,
                target_participant_id,
                delivery,
                recovery_generation
            )
            SELECT
                $1,
                advanced_room.security_event_sequence,
                'recovery_credential_regenerated',
                $2,
                $3,
                $4,
                $5
            FROM advanced_room
            RETURNING sequence, actor_participant_id, target_participant_id,
                recovery_generation, created_at
        ),
        inserted_recipients AS (
            INSERT INTO identity_security_event_recipients (
                room_id,
                security_event_sequence,
                participant_id
            )
            SELECT $1, inserted_event.sequence, $3
            FROM inserted_event
            UNION
            SELECT $1, inserted_event.sequence, $2
            FROM inserted_event
            WHERE $4 = 'host_assisted'
            RETURNING participant_id
        )
        SELECT
            inserted_event.sequence,
            actors.position AS actor_position,
            targets.position AS target_position,
            inserted_event.recovery_generation,
            replace(
                to_char(inserted_event.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z' AS occurred_at
        FROM inserted_event
        JOIN participants AS actors ON actors.id = inserted_event.actor_participant_id
        JOIN participants AS targets ON targets.id = inserted_event.target_participant_id
        WHERE (SELECT COUNT(*) FROM inserted_recipients) >= 1
        ",
    )
    .bind(room_id)
    .bind(actor_participant_id)
    .bind(target_participant_id)
    .bind(delivery)
    .bind(recovery_generation)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("append recovery credential security event", error))
}

pub(super) async fn complete_recovery_credential_regeneration(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    recovery_generation: i64,
    security_event_sequence: i64,
) -> Result<(), ApiError> {
    let completed = sqlx::query(
        r"
        UPDATE recovery_credential_regeneration_requests
        SET
            recovery_generation = $2,
            security_event_sequence = $3,
            completed_at = clock_timestamp()
        WHERE idempotency_key = $1
          AND completed_at IS NULL
        ",
    )
    .bind(idempotency_key)
    .bind(recovery_generation)
    .bind(security_event_sequence)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("complete recovery credential regeneration", error))?;
    if completed.rows_affected() != 1 {
        return Err(ApiError::internal());
    }
    Ok(())
}

pub(super) async fn lock_open_room(
    transaction: &mut Transaction<'_, Postgres>,
    room_code: &str,
) -> Result<Option<Uuid>, ApiError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM rooms WHERE code = $1 AND status = 'open' FOR UPDATE",
    )
    .bind(room_code)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn claim_room_join(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    room_id: Uuid,
    participant_id: Uuid,
    guest_session_id: Uuid,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO room_join_requests (
            idempotency_key,
            room_id,
            participant_id,
            guest_session_id
        )
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (idempotency_key) DO NOTHING
        RETURNING idempotency_key
        ",
    )
    .bind(idempotency_key)
    .bind(room_id)
    .bind(participant_id)
    .bind(guest_session_id)
    .fetch_optional(&mut **transaction)
    .await
    .map(|claim| claim.is_some())
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn next_room_position(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
) -> Result<i16, ApiError> {
    let participant_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM participants WHERE room_id = $1")
            .bind(room_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|error| {
                ApiError::internal_with("identity access PostgreSQL operation", error)
            })?;

    i16::try_from(participant_count + 1)
        .ok()
        .filter(|position| *position <= 4)
        .ok_or_else(ApiError::room_full)
}

pub(super) async fn room_has_hero(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    hero_id: &str,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM participants WHERE room_id = $1 AND hero_id = $2)",
    )
    .bind(room_id)
    .bind(hero_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn lock_participant_room(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
) -> Result<Option<(Uuid, String, Option<String>)>, ApiError> {
    sqlx::query_as::<_, (Uuid, String, Option<String>)>(
        r"
        SELECT rooms.id, rooms.status, participants.hero_id
        FROM rooms
        JOIN participants ON participants.room_id = rooms.id
        WHERE participants.id = $1
        FOR UPDATE
        ",
    )
    .bind(participant_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn room_hero_owned_by_other(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    participant_id: Uuid,
    hero_id: &str,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, bool>(
        r"
        SELECT EXISTS(
            SELECT 1
            FROM participants
            WHERE room_id = $1
              AND hero_id = $2
              AND id <> $3
        )
        ",
    )
    .bind(room_id)
    .bind(hero_id)
    .bind(participant_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn update_participant_hero(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
    hero_id: &str,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE participants SET hero_id = $2, ready = FALSE WHERE id = $1")
        .bind(participant_id)
        .bind(hero_id)
        .execute(&mut **transaction)
        .await
        .map(|_| ())
        .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn update_participant_readiness(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
    ready: bool,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE participants SET ready = $2 WHERE id = $1")
        .bind(participant_id)
        .bind(ready)
        .execute(&mut **transaction)
        .await
        .map(|_| ())
        .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn persist_room_join(
    transaction: &mut Transaction<'_, Postgres>,
    room_join: NewRoomJoin<'_>,
    session_token: &str,
    recovery_token_hmac: &str,
) -> Result<StoredRoomJoin, ApiError> {
    let guest_identity_id = Uuid::new_v4();
    let device_session_id = Uuid::new_v4();

    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(guest_identity_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    sqlx::query(
        r"
        INSERT INTO participants (
            id,
            room_id,
            guest_identity_id,
            display_name,
            role,
            position,
            hero_id
        )
        VALUES ($1, $2, $3, $4, 'guest', $5, $6)
        ",
    )
    .bind(room_join.participant_id)
    .bind(room_join.room_id)
    .bind(guest_identity_id)
    .bind(room_join.display_name)
    .bind(room_join.position)
    .bind(room_join.hero_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    insert_recovery_credential(transaction, room_join.participant_id, recovery_token_hmac).await?;

    sqlx::query(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token_digest)
        VALUES (
            $1,
            $2,
            'sha256:' || encode(sha256(convert_to($3, 'UTF8')), 'hex')
        )
        ",
    )
    .bind(room_join.guest_session_id)
    .bind(guest_identity_id)
    .bind(session_token)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    sqlx::query(
        r"
        INSERT INTO device_sessions (id, guest_session_id, participant_id)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(device_session_id)
    .bind(room_join.guest_session_id)
    .bind(room_join.participant_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    Ok(StoredRoomJoin {
        participant_id: room_join.participant_id,
        room_code: room_join.room_code.to_owned(),
        display_name: room_join.display_name.to_owned(),
        hero_id: room_join.hero_id.to_owned(),
    })
}

pub(super) async fn load_room_join(
    database: &PgPool,
    idempotency_key: &str,
) -> Result<Option<StoredRoomJoin>, ApiError> {
    sqlx::query_as::<_, StoredRoomJoin>(
        r"
        SELECT
            participants.id AS participant_id,
            rooms.code AS room_code,
            participants.display_name,
            participants.hero_id
        FROM room_join_requests
        JOIN rooms ON rooms.id = room_join_requests.room_id
        JOIN participants ON participants.id = room_join_requests.participant_id
        JOIN guest_sessions ON guest_sessions.id = room_join_requests.guest_session_id
        WHERE room_join_requests.idempotency_key = $1
        ",
    )
    .bind(idempotency_key)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn load_lobby(
    database: &PgPool,
    current_participant_id: Uuid,
) -> Result<Option<StoredLobby>, ApiError> {
    let room = sqlx::query_as::<_, (String, String)>(
        r"
        SELECT rooms.code, rooms.status
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE participants.id = $1
        ",
    )
    .bind(current_participant_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;
    let Some((room_code, room_status)) = room else {
        return Ok(None);
    };

    let participants = sqlx::query_as::<_, StoredParticipant>(
        r"
        SELECT id, display_name, role, position, hero_id, ready
        FROM participants
        WHERE room_id = (SELECT room_id FROM participants WHERE id = $1)
        ORDER BY position
        ",
    )
    .bind(current_participant_id)
    .fetch_all(database)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    Ok(Some(StoredLobby {
        room_code,
        room_status,
        current_participant_id,
        participants,
    }))
}

pub(super) async fn persist_room_creation(
    transaction: &mut Transaction<'_, Postgres>,
    room_creation: NewRoomCreation<'_>,
    session_token: &str,
    recovery_token_hmac: &str,
) -> Result<StoredRoomCreation, ApiError> {
    let guest_identity_id = Uuid::new_v4();
    let device_session_id = Uuid::new_v4();

    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(guest_identity_id)
        .execute(&mut **transaction)
        .await
        .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    insert_room(
        transaction,
        room_creation.room_id,
        room_creation.participant_id,
        room_creation.password_hash,
    )
    .await?;

    sqlx::query(
        r"
        INSERT INTO participants (id, room_id, guest_identity_id, display_name, role, position)
        VALUES ($1, $2, $3, $4, 'host', 1)
        ",
    )
    .bind(room_creation.participant_id)
    .bind(room_creation.room_id)
    .bind(guest_identity_id)
    .bind(room_creation.display_name)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    insert_recovery_credential(
        transaction,
        room_creation.participant_id,
        recovery_token_hmac,
    )
    .await?;

    sqlx::query(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token_digest)
        VALUES (
            $1,
            $2,
            'sha256:' || encode(sha256(convert_to($3, 'UTF8')), 'hex')
        )
        ",
    )
    .bind(room_creation.guest_session_id)
    .bind(guest_identity_id)
    .bind(session_token)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    sqlx::query(
        r"
        INSERT INTO device_sessions (id, guest_session_id, participant_id)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(device_session_id)
    .bind(room_creation.guest_session_id)
    .bind(room_creation.participant_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

    Ok(StoredRoomCreation {
        participant_id: room_creation.participant_id,
        display_name: room_creation.display_name.to_owned(),
        recovery_password_hash: room_creation.password_hash.to_owned(),
    })
}

pub(super) async fn insert_recovery_credential(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
    token_hmac: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        r"
        INSERT INTO recovery_credentials (
            id,
            participant_id,
            token_hmac,
            recovery_password_hash,
            recovery_epoch,
            password_generation,
            recovery_generation
        )
        SELECT
            $1,
            participants.id,
            $3,
            rooms.recovery_password_hash,
            rooms.recovery_epoch,
            rooms.password_generation,
            participants.recovery_generation
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE participants.id = $2
        ",
    )
    .bind(Uuid::new_v4())
    .bind(participant_id)
    .bind(token_hmac)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn load_recovery_candidate(
    database: &PgPool,
    token_hmac: &str,
    recovery_attempt_id: Option<Uuid>,
) -> Result<Option<StoredRecoveryCandidate>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryCandidate>(
        r"
        SELECT
            recovery_credentials.id AS credential_id,
            rooms.id AS room_id,
            participants.id AS participant_id,
            participants.guest_identity_id,
            recovery_credentials.recovery_password_hash,
            recovery_credentials.recovery_epoch,
            recovery_credentials.password_generation,
            recovery_credentials.recovery_generation,
            recovery_credentials.status,
            recovery_credentials.recovery_attempt_id,
            CASE
                WHEN consumed_device_sessions.status = 'active'
                 AND consumed_guest_sessions.expires_at > clock_timestamp()
                THEN GREATEST(
                    0,
                    FLOOR(EXTRACT(EPOCH FROM (
                        consumed_guest_sessions.expires_at - clock_timestamp()
                    )))::BIGINT
                )
            END AS session_max_age_seconds
        FROM recovery_credentials
        JOIN participants ON participants.id = recovery_credentials.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        LEFT JOIN games ON games.room_id = rooms.id
        LEFT JOIN guest_sessions AS consumed_guest_sessions
          ON consumed_guest_sessions.id = recovery_credentials.consumed_by_guest_session_id
        LEFT JOIN device_sessions AS consumed_device_sessions
          ON consumed_device_sessions.guest_session_id = consumed_guest_sessions.id
         AND consumed_device_sessions.participant_id = participants.id
        WHERE recovery_credentials.token_hmac = $1
          AND rooms.status <> 'cancelled'
          AND (games.id IS NULL OR games.expires_at > clock_timestamp())
          AND (
              (
                  recovery_credentials.status = 'active'
                  AND recovery_credentials.recovery_epoch = rooms.recovery_epoch
                  AND recovery_credentials.password_generation = rooms.password_generation
                  AND recovery_credentials.recovery_generation = participants.recovery_generation
              )
              OR (
                  recovery_credentials.status = 'consumed'
                  AND recovery_credentials.recovery_attempt_id = $2
                  AND consumed_device_sessions.status = 'active'
                  AND consumed_guest_sessions.expires_at > clock_timestamp()
              )
          )
        ",
    )
    .bind(token_hmac)
    .bind(recovery_attempt_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("load participant recovery candidate", error))
}

pub(super) async fn lock_recovery_room(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
) -> Result<Option<StoredRecoveryRoom>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryRoom>(
        r"
        SELECT
            rooms.id AS room_id,
            rooms.recovery_password_hash,
            rooms.recovery_epoch,
            rooms.password_generation
        FROM rooms
        LEFT JOIN games ON games.room_id = rooms.id
        WHERE rooms.id = $1
          AND rooms.status <> 'cancelled'
          AND (games.id IS NULL OR games.expires_at > clock_timestamp())
        FOR UPDATE OF rooms
        ",
    )
    .bind(room_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("lock participant recovery room", error))
}

pub(super) async fn lock_recovery_candidate(
    transaction: &mut Transaction<'_, Postgres>,
    token_hmac: &str,
    recovery_attempt_id: Uuid,
) -> Result<Option<StoredRecoveryCandidate>, ApiError> {
    sqlx::query_as::<_, StoredRecoveryCandidate>(
        r"
        SELECT
            recovery_credentials.id AS credential_id,
            rooms.id AS room_id,
            participants.id AS participant_id,
            participants.guest_identity_id,
            recovery_credentials.recovery_password_hash,
            recovery_credentials.recovery_epoch,
            recovery_credentials.password_generation,
            recovery_credentials.recovery_generation,
            recovery_credentials.status,
            recovery_credentials.recovery_attempt_id,
            CASE
                WHEN consumed_device_sessions.status = 'active'
                 AND consumed_guest_sessions.expires_at > clock_timestamp()
                THEN GREATEST(
                    0,
                    FLOOR(EXTRACT(EPOCH FROM (
                        consumed_guest_sessions.expires_at - clock_timestamp()
                    )))::BIGINT
                )
            END AS session_max_age_seconds
        FROM recovery_credentials
        JOIN participants ON participants.id = recovery_credentials.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        LEFT JOIN games ON games.room_id = rooms.id
        LEFT JOIN guest_sessions AS consumed_guest_sessions
          ON consumed_guest_sessions.id = recovery_credentials.consumed_by_guest_session_id
        LEFT JOIN device_sessions AS consumed_device_sessions
          ON consumed_device_sessions.guest_session_id = consumed_guest_sessions.id
         AND consumed_device_sessions.participant_id = participants.id
        WHERE recovery_credentials.token_hmac = $1
          AND rooms.status <> 'cancelled'
          AND (games.id IS NULL OR games.expires_at > clock_timestamp())
          AND (
              (
                  recovery_credentials.status = 'active'
                  AND recovery_credentials.recovery_epoch = rooms.recovery_epoch
                  AND recovery_credentials.password_generation = rooms.password_generation
                  AND recovery_credentials.recovery_generation = participants.recovery_generation
              )
              OR (
                  recovery_credentials.status = 'consumed'
                  AND recovery_credentials.recovery_attempt_id = $2
                  AND consumed_device_sessions.status = 'active'
                  AND consumed_guest_sessions.expires_at > clock_timestamp()
              )
          )
        FOR UPDATE OF recovery_credentials
        ",
    )
    .bind(token_hmac)
    .bind(recovery_attempt_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("lock participant recovery candidate", error))
}

pub(super) async fn active_session_count(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        r"
        SELECT COUNT(*)
        FROM device_sessions
        JOIN guest_sessions ON guest_sessions.id = device_sessions.guest_session_id
        WHERE device_sessions.participant_id = $1
          AND device_sessions.status = 'active'
          AND guest_sessions.expires_at > clock_timestamp()
        ",
    )
    .bind(participant_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("count participant device sessions", error))
}

pub(super) async fn consume_recovery_credential(
    transaction: &mut Transaction<'_, Postgres>,
    candidate: &StoredRecoveryCandidate,
    guest_session_id: Uuid,
    device_session_id: Uuid,
    recovery_attempt_id: Uuid,
    session_token: &str,
) -> Result<i64, ApiError> {
    let max_age_seconds = sqlx::query_scalar(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token_digest)
        VALUES (
            $1,
            $2,
            'sha256:' || encode(sha256(convert_to($3, 'UTF8')), 'hex')
        )
        RETURNING GREATEST(
            0,
            FLOOR(EXTRACT(EPOCH FROM (expires_at - clock_timestamp())))::BIGINT
        )
        ",
    )
    .bind(guest_session_id)
    .bind(candidate.guest_identity_id)
    .bind(session_token)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("create recovered guest session", error))?;

    sqlx::query(
        r"
        INSERT INTO device_sessions (id, guest_session_id, participant_id)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(device_session_id)
    .bind(guest_session_id)
    .bind(candidate.participant_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("create recovered device session", error))?;

    let consumed = sqlx::query(
        r"
        UPDATE recovery_credentials
        SET
            status = 'consumed',
            recovery_attempt_id = $3,
            consumed_by_guest_session_id = $2,
            consumed_at = clock_timestamp()
        WHERE id = $1
          AND status = 'active'
          AND EXISTS (
              SELECT 1
              FROM participants
              JOIN rooms ON rooms.id = participants.room_id
              LEFT JOIN games ON games.room_id = rooms.id
              WHERE participants.id = recovery_credentials.participant_id
                AND rooms.status <> 'cancelled'
                AND (games.id IS NULL OR games.expires_at > clock_timestamp())
          )
          AND 2 >= (
              SELECT COUNT(*)
              FROM device_sessions
              JOIN guest_sessions
                ON guest_sessions.id = device_sessions.guest_session_id
              WHERE device_sessions.participant_id = recovery_credentials.participant_id
                AND device_sessions.status = 'active'
                AND guest_sessions.expires_at > clock_timestamp()
          )
        ",
    )
    .bind(candidate.credential_id)
    .bind(guest_session_id)
    .bind(recovery_attempt_id)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("consume participant recovery credential", error))?;
    if consumed.rows_affected() != 1 {
        return Err(ApiError::recovery_failed());
    }

    Ok(max_age_seconds)
}

async fn insert_room(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    host_participant_id: Uuid,
    password_hash: &str,
) -> Result<String, ApiError> {
    loop {
        let room_code = random_room_code()?;
        let inserted = sqlx::query_scalar::<_, String>(
            r"
            INSERT INTO rooms (id, code, host_participant_id, recovery_password_hash)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (code) DO NOTHING
            RETURNING code
            ",
        )
        .bind(room_id)
        .bind(&room_code)
        .bind(host_participant_id)
        .bind(password_hash)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))?;

        if let Some(code) = inserted {
            return Ok(code);
        }
    }
}

pub(super) async fn load_room_creation(
    database: &PgPool,
    idempotency_key: &str,
) -> Result<Option<StoredRoomCreation>, ApiError> {
    sqlx::query_as::<_, StoredRoomCreation>(
        r"
        SELECT
            participants.id AS participant_id,
            participants.display_name,
            rooms.recovery_password_hash
        FROM room_creation_requests
        JOIN rooms ON rooms.id = room_creation_requests.room_id
        JOIN participants ON participants.id = room_creation_requests.participant_id
        JOIN guest_sessions ON guest_sessions.id = room_creation_requests.guest_session_id
        WHERE room_creation_requests.idempotency_key = $1
        ",
    )
    .bind(idempotency_key)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("identity access PostgreSQL operation", error))
}

pub(super) async fn ensure_room_creation_session_token(
    database: &PgPool,
    idempotency_key: &str,
    session_token: &str,
) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        r"
        UPDATE guest_sessions
        SET token_digest = 'sha256:' || encode(sha256(convert_to($2, 'UTF8')), 'hex')
        FROM room_creation_requests, device_sessions
        WHERE room_creation_requests.idempotency_key = $1
          AND guest_sessions.id = room_creation_requests.guest_session_id
          AND device_sessions.guest_session_id = guest_sessions.id
          AND device_sessions.status = 'active'
          AND guest_sessions.expires_at > clock_timestamp()
        RETURNING GREATEST(
            0,
            FLOOR(EXTRACT(EPOCH FROM (guest_sessions.expires_at - clock_timestamp())))::BIGINT
        )
        ",
    )
    .bind(idempotency_key)
    .bind(session_token)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("ensure room creation session grant", error))?
    .ok_or_else(ApiError::session_invalid)
}

pub(super) async fn ensure_room_join_session_token(
    database: &PgPool,
    idempotency_key: &str,
    session_token: &str,
) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        r"
        UPDATE guest_sessions
        SET token_digest = 'sha256:' || encode(sha256(convert_to($2, 'UTF8')), 'hex')
        FROM room_join_requests, device_sessions
        WHERE room_join_requests.idempotency_key = $1
          AND guest_sessions.id = room_join_requests.guest_session_id
          AND device_sessions.guest_session_id = guest_sessions.id
          AND device_sessions.status = 'active'
          AND guest_sessions.expires_at > clock_timestamp()
        RETURNING GREATEST(
            0,
            FLOOR(EXTRACT(EPOCH FROM (guest_sessions.expires_at - clock_timestamp())))::BIGINT
        )
        ",
    )
    .bind(idempotency_key)
    .bind(session_token)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("ensure room join session grant", error))?
    .ok_or_else(ApiError::session_invalid)
}
