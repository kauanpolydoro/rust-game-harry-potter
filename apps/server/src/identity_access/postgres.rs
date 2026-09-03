use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{ApiError, StoredRoomCreation, random_hex_token, random_room_code};

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

    let room_code = insert_room(transaction, room_id, participant_id, password_hash).await?;

    sqlx::query(
        r"
        INSERT INTO participants (id, room_id, guest_identity_id, display_name, role)
        VALUES ($1, $2, $3, $4, 'host')
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
        room_code,
        room_status: "open".to_owned(),
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
            rooms.code AS room_code,
            rooms.status AS room_status,
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
