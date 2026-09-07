use super::{PurgeError, credential_fingerprint};
use crate::AppState;

pub(super) async fn register_deployment(state: &AppState) -> Result<(), PurgeError> {
    if let Some(epoch) = state.deployment_epoch {
        sqlx::query(
            r"
            INSERT INTO runtime_deployment (epoch, credential_fingerprint, database_login)
            SELECT $1, $2, session_user WHERE NOT EXISTS (SELECT 1 FROM rooms)
            ON CONFLICT (singleton) DO NOTHING
        ",
        )
        .bind(epoch)
        .bind(credential_fingerprint(state))
        .execute(&state.database)
        .await?;
    }
    if deployment_ready(state).await? {
        Ok(())
    } else {
        Err(PurgeError::Database)
    }
}

pub(super) async fn deployment_ready(state: &AppState) -> Result<bool, PurgeError> {
    let binding: Option<(uuid::Uuid, String, bool)> = sqlx::query_as(
        "SELECT epoch, credential_fingerprint, ready FROM runtime_deployment WHERE singleton",
    )
    .fetch_optional(&state.database)
    .await?;
    Ok(match (state.deployment_epoch, binding) {
        (None, None) => true,
        (Some(expected), Some((epoch, fingerprint, ready))) => {
            ready && epoch == expected && fingerprint == credential_fingerprint(state)
        }
        _ => false,
    })
}

use super::RestoreError;
use crate::lifecycle::PurgeProof;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
type Tx<'a> = Transaction<'a, Postgres>;

#[derive(sqlx::FromRow)]
pub(super) struct Binding {
    pub epoch: Uuid,
    pub database_login: String,
    pub credential_fingerprint: String,
    pub ledger_key_fingerprint: Option<String>,
    pub ready: bool,
    pub restored_from_ms: Option<i64>,
}

pub(super) async fn binding(tx: &mut Tx<'_>) -> Result<Option<Binding>, sqlx::Error> {
    sqlx::query_as("SELECT epoch, database_login, credential_fingerprint, ledger_key_fingerprint, ready, restored_from_ms FROM runtime_deployment WHERE singleton")
        .fetch_optional(&mut **tx).await
}

pub(super) async fn lock_coordinator(tx: &mut Tx<'_>) -> Result<(), RestoreError> {
    let acquired: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtext(current_schema()), 330033)")
            .fetch_one(&mut **tx)
            .await?;
    if acquired {
        Ok(())
    } else {
        Err(RestoreError::Busy)
    }
}

pub(super) async fn close_gate(database: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE runtime_deployment SET ready = FALSE WHERE singleton")
        .execute(database)
        .await?;
    Ok(())
}

pub(super) async fn clock_ms(tx: &mut Tx<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000)::BIGINT")
        .fetch_one(&mut **tx)
        .await
}

pub(super) async fn timestamps_valid(tx: &mut Tx<'_>) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r"
        SELECT NOT EXISTS (SELECT 1 FROM games WHERE last_game_action_at > clock_timestamp()
            OR created_at > clock_timestamp() OR access_expired_at > clock_timestamp())
        AND NOT EXISTS (SELECT 1 FROM game_events WHERE created_at > clock_timestamp())
    ",
    )
    .fetch_one(&mut **tx)
    .await
}

pub(super) async fn lock_games(tx: &mut Tx<'_>) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query("SELECT id FROM rooms ORDER BY id FOR UPDATE")
        .execute(&mut **tx)
        .await?;
    sqlx::query_scalar("SELECT id FROM games ORDER BY id FOR UPDATE")
        .fetch_all(&mut **tx)
        .await
}

pub(super) async fn apply_tombstone(
    tx: &mut Tx<'_>,
    id: Uuid,
    proof: &PurgeProof,
) -> Result<(), sqlx::Error> {
    // Reuse original proof timestamps so the idempotent purge cannot conflict with
    // the evidence which predates this restore, even in the immutable file ledger.
    sqlx::query(
        r"
        UPDATE games SET expires_at = to_timestamp($2::DOUBLE PRECISION / 1000),
            access_expired_at = to_timestamp($3::DOUBLE PRECISION / 1000)
        WHERE id = $1
    ",
    )
    .bind(id)
    .bind(proof.expires_at_ms)
    .bind(proof.detected_at_ms)
    .execute(&mut **tx)
    .await?;
    sqlx::query(r"
        INSERT INTO lifecycle_purge_jobs (game_id, room_id, expires_at, detected_at)
        SELECT id, room_id, expires_at, access_expired_at FROM games WHERE id = $1
        ON CONFLICT (game_id) DO UPDATE SET expires_at = EXCLUDED.expires_at, detected_at = EXCLUDED.detected_at
    ").bind(id).execute(&mut **tx).await?;
    Ok(())
}
pub(super) async fn expire_game(tx: &mut Tx<'_>, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT expire_game_access($1)")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn revoke_access(tx: &mut Tx<'_>) -> Result<(), sqlx::Error> {
    // Consumed credentials are also removed: their retry receipts are old grants.
    sqlx::query("DELETE FROM recovery_credentials")
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE device_sessions SET status = 'revoked' WHERE status = 'active'")
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT pg_notify('hogwarts_session_revoked', id::TEXT) FROM guest_sessions")
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE guest_sessions SET token_digest = 'sha256:' || encode(sha256(convert_to(gen_random_uuid()::TEXT, 'UTF8')), 'hex')")
        .execute(&mut **tx).await?;
    sqlx::query("UPDATE rooms SET recovery_epoch = recovery_epoch + 1, password_generation = password_generation + 1")
        .execute(&mut **tx).await?;
    sqlx::query("UPDATE participants SET recovery_generation = recovery_generation + 1")
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM game_realtime_connections")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn open_gate(
    tx: &mut Tx<'_>,
    state: &AppState,
    point: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r"
        UPDATE runtime_deployment SET epoch = $1, credential_fingerprint = $2, database_login = session_user,
            ready = TRUE, restored_from_ms = $3, reconciled_at = clock_timestamp() WHERE singleton
    ",
    )
    .bind(state.deployment_epoch)
    .bind(credential_fingerprint(state))
    .bind(point)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn bind_ledger(
    database: &PgPool,
    fingerprint: &str,
) -> Result<Option<Uuid>, PurgeError> {
    let binding: Option<(Uuid, bool)> = sqlx::query_as(
        r"
        UPDATE runtime_deployment SET ledger_key_fingerprint = COALESCE(ledger_key_fingerprint, $1), checkpoint_at = clock_timestamp()
        WHERE singleton AND ready
        RETURNING epoch, ledger_key_fingerprint = $1
    ",
    )
    .bind(fingerprint)
    .fetch_optional(database)
    .await?;
    match binding {
        Some((epoch, true)) => Ok(Some(epoch)),
        Some((_, false)) => Err(PurgeError::Ledger),
        None => Ok(None),
    }
}

pub(super) async fn current_login(tx: &mut Tx<'_>) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT session_user::TEXT")
        .fetch_one(&mut **tx)
        .await
}

pub(super) async fn backup_watermark(tx: &mut Tx<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT (extract(epoch FROM checkpoint_at) * 1000)::BIGINT FROM runtime_deployment WHERE singleton")
        .fetch_one(&mut **tx).await
}

#[derive(sqlx::FromRow)]
pub(super) struct Survivor {
    pub id: Uuid,
    pub room_id: Uuid,
    pub room_code: String,
    pub position: i16,
}
pub(super) async fn surviving_participants(tx: &mut Tx<'_>) -> Result<Vec<Survivor>, sqlx::Error> {
    sqlx::query_as(
        r"
        SELECT participants.id, participants.room_id, rooms.code AS room_code, participants.position
        FROM participants JOIN rooms ON rooms.id = participants.room_id
        LEFT JOIN games ON games.room_id = rooms.id
        WHERE rooms.status <> 'cancelled' AND (games.id IS NULL OR
            (games.access_expired_at IS NULL AND games.expires_at > clock_timestamp()))
        ORDER BY participants.room_id, participants.position
    ",
    )
    .fetch_all(&mut **tx)
    .await
}
pub(super) async fn rotate_password(
    tx: &mut Tx<'_>,
    room: Uuid,
    hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE rooms SET recovery_password_hash = $2 WHERE id = $1")
        .bind(room)
        .bind(hash)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub(super) async fn issue_credential(
    tx: &mut Tx<'_>,
    participant: Uuid,
    hmac: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO recovery_credentials (id, participant_id, token_hmac) VALUES ($1, $2, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(participant)
    .bind(hmac)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
pub(super) async fn credential_current(
    tx: &mut Tx<'_>,
    participant: Uuid,
    hmac: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM recovery_credentials WHERE participant_id = $1 AND token_hmac = $2 AND status = 'active')")
        .bind(participant).bind(hmac).fetch_one(&mut **tx).await
}

pub(super) async fn insert_source_binding(
    tx: &mut Tx<'_>,
    state: &AppState,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO runtime_deployment (epoch, credential_fingerprint, database_login) VALUES ($1, $2, session_user)")
        .bind(state.deployment_epoch).bind(credential_fingerprint(state)).execute(&mut **tx).await?;
    Ok(())
}
pub(super) async fn set_source_ledger(
    tx: &mut Tx<'_>,
    fingerprint: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE runtime_deployment SET ledger_key_fingerprint = $1, checkpoint_at = clock_timestamp() WHERE singleton")
        .bind(fingerprint).execute(&mut **tx).await?;
    Ok(())
}
