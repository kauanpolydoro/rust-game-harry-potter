use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ApiError, StoredLobby, StoredParticipant, StoredRoomCreation, StoredRoomJoin, random_hex_token,
    random_room_code,
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
    .map_err(|_| ApiError::internal())
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
    .map_err(|_| ApiError::internal())
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
    .map_err(|_| ApiError::internal())
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
    .map_err(|_| ApiError::internal())
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
    .map_err(|_| ApiError::internal())
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
            .map_err(|_| ApiError::internal())?;

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
    .map_err(|_| ApiError::internal())
}

pub(super) async fn lock_participant_open_room(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
) -> Result<Option<Uuid>, ApiError> {
    sqlx::query_scalar::<_, Uuid>(
        r"
        SELECT id
        FROM rooms
        WHERE id = (SELECT room_id FROM participants WHERE id = $1)
          AND status = 'open'
        FOR UPDATE
        ",
    )
    .bind(participant_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())
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
    .map_err(|_| ApiError::internal())
}

pub(super) async fn update_participant_hero(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
    hero_id: &str,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE participants SET hero_id = $2 WHERE id = $1")
        .bind(participant_id)
        .bind(hero_id)
        .execute(&mut **transaction)
        .await
        .map(|_| ())
        .map_err(|_| ApiError::internal())
}

pub(super) async fn persist_room_join(
    transaction: &mut Transaction<'_, Postgres>,
    room_join: NewRoomJoin<'_>,
) -> Result<StoredRoomJoin, ApiError> {
    let guest_identity_id = Uuid::new_v4();
    let device_session_id = Uuid::new_v4();
    let session_token = random_hex_token()?;

    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(guest_identity_id)
        .execute(&mut **transaction)
        .await
        .map_err(|_| ApiError::internal())?;

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
    .map_err(|_| ApiError::internal())?;

    sqlx::query(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(room_join.guest_session_id)
    .bind(guest_identity_id)
    .bind(&session_token)
    .execute(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())?;

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
    .map_err(|_| ApiError::internal())?;

    Ok(StoredRoomJoin {
        participant_id: room_join.participant_id,
        room_code: room_join.room_code.to_owned(),
        display_name: room_join.display_name.to_owned(),
        hero_id: room_join.hero_id.to_owned(),
        session_token,
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
            participants.hero_id,
            guest_sessions.token AS session_token
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
    .map_err(|_| ApiError::internal())
}

pub(super) async fn participant_for_session(
    database: &PgPool,
    session_token: &str,
) -> Result<Option<Uuid>, ApiError> {
    sqlx::query_scalar::<_, Uuid>(
        r"
        SELECT device_sessions.participant_id
        FROM guest_sessions
        JOIN device_sessions ON device_sessions.guest_session_id = guest_sessions.id
        WHERE guest_sessions.token = $1
          AND guest_sessions.expires_at > clock_timestamp()
          AND device_sessions.status = 'active'
        ",
    )
    .bind(session_token)
    .fetch_optional(database)
    .await
    .map_err(|_| ApiError::internal())
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
    .map_err(|_| ApiError::internal())?;
    let Some((room_code, room_status)) = room else {
        return Ok(None);
    };

    let participants = sqlx::query_as::<_, StoredParticipant>(
        r"
        SELECT id, display_name, role, position, hero_id
        FROM participants
        WHERE room_id = (SELECT room_id FROM participants WHERE id = $1)
        ORDER BY position
        ",
    )
    .bind(current_participant_id)
    .fetch_all(database)
    .await
    .map_err(|_| ApiError::internal())?;

    Ok(Some(StoredLobby {
        room_code,
        room_status,
        current_participant_id,
        participants,
    }))
}

pub(super) async fn persist_room_creation(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    participant_id: Uuid,
    guest_session_id: Uuid,
    display_name: &str,
    password_hash: &str,
) -> Result<StoredRoomCreation, ApiError> {
    let guest_identity_id = Uuid::new_v4();
    let device_session_id = Uuid::new_v4();
    let session_token = random_hex_token()?;

    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(guest_identity_id)
        .execute(&mut **transaction)
        .await
        .map_err(|_| ApiError::internal())?;

    insert_room(transaction, room_id, participant_id, password_hash).await?;

    sqlx::query(
        r"
        INSERT INTO participants (id, room_id, guest_identity_id, display_name, role, position)
        VALUES ($1, $2, $3, $4, 'host', 1)
        ",
    )
    .bind(participant_id)
    .bind(room_id)
    .bind(guest_identity_id)
    .bind(display_name)
    .execute(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())?;

    sqlx::query(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(guest_session_id)
    .bind(guest_identity_id)
    .bind(&session_token)
    .execute(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())?;

    sqlx::query(
        r"
        INSERT INTO device_sessions (id, guest_session_id, participant_id)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(device_session_id)
    .bind(guest_session_id)
    .bind(participant_id)
    .execute(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())?;

    Ok(StoredRoomCreation {
        participant_id,
        display_name: display_name.to_owned(),
        recovery_password_hash: password_hash.to_owned(),
        session_token,
    })
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
        .map_err(|_| ApiError::internal())?;

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
            rooms.recovery_password_hash,
            guest_sessions.token AS session_token
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
    .map_err(|_| ApiError::internal())
}
