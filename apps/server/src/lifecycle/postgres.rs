use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{LifecycleMetrics, PurgeError, PurgeProof};

type Tx<'a> = Transaction<'a, Postgres>;

#[derive(sqlx::FromRow)]
pub(super) struct Job {
    pub game_id: Uuid,
    room_id: Uuid,
    pub stage: String,
    expires_at_ms: i64,
    detected_at_ms: i64,
    completed_at_ms: Option<i64>,
    participant_ids: Vec<Uuid>,
    identity_ids: Vec<Uuid>,
    session_ids: Vec<Uuid>,
}
impl Job {
    pub fn proof(&self) -> PurgeProof {
        PurgeProof {
            version: 1,
            expires_at_ms: self.expires_at_ms,
            detected_at_ms: self.detected_at_ms,
            completed_at_ms: self.completed_at_ms,
        }
    }
    fn identifiers(&self) -> Vec<Uuid> {
        let mut ids = vec![self.game_id, self.room_id];
        ids.extend(&self.participant_ids);
        ids.extend(&self.identity_ids);
        ids.extend(&self.session_ids);
        ids
    }
}

pub(super) async fn enqueue(database: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = database.begin().await?;
    configure_transaction(&mut tx).await?;
    let games = sqlx::query_scalar::<_, Uuid>(
        r"
        SELECT id FROM games
        WHERE expires_at <= statement_timestamp()
          AND NOT EXISTS (SELECT 1 FROM lifecycle_purge_jobs WHERE game_id = games.id)
        ORDER BY expires_at LIMIT 100 FOR UPDATE SKIP LOCKED
    ",
    )
    .fetch_all(&mut *tx)
    .await?;
    for id in games {
        sqlx::query("SELECT expire_game_access($1)")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r"
            INSERT INTO lifecycle_purge_jobs (game_id, room_id, expires_at, detected_at)
            SELECT id, room_id, expires_at, access_expired_at FROM games
            WHERE id = $1 AND access_expired_at IS NOT NULL
            ON CONFLICT (game_id) DO NOTHING
        ",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM lifecycle_purge_observations WHERE observed_at < clock_timestamp() - INTERVAL '14 days'")
        .execute(&mut *tx).await?;
    tx.commit().await
}

pub(super) async fn claim(tx: &mut Tx<'_>) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as(
        r"
        SELECT game_id, room_id, stage, participant_ids, identity_ids, session_ids,
            (extract(epoch FROM expires_at) * 1000)::BIGINT AS expires_at_ms,
            (extract(epoch FROM detected_at) * 1000)::BIGINT AS detected_at_ms,
            (extract(epoch FROM verified_at) * 1000)::BIGINT AS completed_at_ms
        FROM lifecycle_purge_jobs WHERE next_attempt_at <= statement_timestamp()
        ORDER BY next_attempt_at, game_id LIMIT 1 FOR UPDATE SKIP LOCKED
    ",
    )
    .fetch_optional(&mut **tx)
    .await
}

pub(super) async fn advance(tx: &mut Tx<'_>, id: Uuid, stage: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE lifecycle_purge_jobs SET stage = $2 WHERE game_id = $1")
        .bind(id)
        .bind(stage)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn retry_later(
    tx: &mut Tx<'_>,
    id: Uuid,
    error: PurgeError,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE lifecycle_purge_jobs SET failures = failures + 1, last_failure = $2, next_attempt_at = clock_timestamp() + INTERVAL '30 seconds' WHERE game_id = $1")
        .bind(id).bind(match error { PurgeError::Database => "database", PurgeError::Ledger => "ledger", PurgeError::Orphans => "orphans", PurgeError::Timeout => "timeout" }).execute(&mut **tx).await?;
    Ok(())
}

async fn lock_roots(tx: &mut Tx<'_>, job: &Job) -> Result<(), sqlx::Error> {
    // Identity/recovery operations use room -> game -> participants as well.
    sqlx::query("SELECT id FROM rooms WHERE id = $1 FOR UPDATE")
        .bind(job.room_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT id FROM games WHERE id = $1 FOR UPDATE")
        .bind(job.game_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn detach(tx: &mut Tx<'_>, job: &Job) -> Result<(), sqlx::Error> {
    lock_roots(tx, job).await?;
    // Capture all generations, including consumed recovery and replaced sessions.
    // Keep shared identity candidates too; ownership is resolved at deletion time.
    sqlx::query(r"
        UPDATE lifecycle_purge_jobs SET
            participant_ids = ARRAY(SELECT id FROM participants WHERE room_id = $2),
            identity_ids = ARRAY(
                SELECT DISTINCT p.guest_identity_id FROM participants p WHERE p.room_id = $2
            ),
            session_ids = ARRAY(SELECT guest_session_id FROM device_sessions JOIN participants ON participants.id = device_sessions.participant_id WHERE participants.room_id = $2),
            stage = 'purge'
        WHERE game_id = $1
    ").bind(job.game_id).bind(job.room_id).execute(&mut **tx).await?;
    Ok(())
}

// This is the operational storage inventory, in FK-safe deletion order.
// Global, non-identifying tables are explicitly separated below.
const ROOM_TABLES: &[&str] = &[
    "device_session_revocation_requests",
    "participant_protection_requests",
    "room_protection_requests",
    "recovery_password_rotation_requests",
    "recovery_credential_regeneration_requests",
    "identity_security_event_recipients",
    "identity_security_events",
    "room_creation_requests",
    "room_join_requests",
    "game_start_requests",
];
const GAME_TABLES: &[&str] = &[
    "game_realtime_connections",
    "game_command_receipts",
    "game_events",
    "game_state_anchors",
];
const IDENTITY_TABLES: &[&str] = &[
    "recovery_credentials",
    "device_sessions",
    "participants",
    "guest_sessions",
    "guest_identities",
    "games",
    "rooms",
];
const GLOBAL_TABLES: &[&str] = &[
    "_sqlx_migrations",
    "application_metadata",
    "runtime_deployment",
    "content_manifests",
    "lifecycle_purge_jobs",
    "lifecycle_purge_observations",
];

pub(super) async fn purge(tx: &mut Tx<'_>, job: &Job) -> Result<(), PurgeError> {
    lock_roots(tx, job).await?;
    // Two jobs may share identities. Serialize ownership checks before either
    // removes its participation so the final owner also removes the identity.
    sqlx::query("SELECT id FROM guest_identities WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(&job.identity_ids)
        .execute(&mut **tx)
        .await?;
    // Reject an unregistered store before deleting anything; verify repeats this.
    inventory(tx).await?;
    for table in ROOM_TABLES {
        sqlx::query(AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE room_id = $1"
        )))
        .bind(job.room_id)
        .execute(&mut **tx)
        .await?;
    }
    for table in GAME_TABLES {
        sqlx::query(AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE game_id = $1"
        )))
        .bind(job.game_id)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query("DELETE FROM recovery_credentials WHERE participant_id = ANY($1)")
        .bind(&job.participant_ids)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM device_sessions WHERE participant_id = ANY($1)")
        .bind(&job.participant_ids)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM games WHERE id = $1")
        .bind(job.game_id)
        .execute(&mut **tx)
        .await?;
    // Both sides of the deferred host FK are removed in the same transaction.
    sqlx::query("DELETE FROM participants WHERE room_id = $1")
        .bind(job.room_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM rooms WHERE id = $1")
        .bind(job.room_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM guest_sessions WHERE id = ANY($1) OR (guest_identity_id = ANY($2) AND NOT EXISTS (SELECT 1 FROM participants WHERE participants.guest_identity_id = guest_sessions.guest_identity_id))")
        .bind(&job.session_ids)
        .bind(&job.identity_ids)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM guest_identities WHERE id = ANY($1) AND NOT EXISTS (SELECT 1 FROM participants WHERE guest_identity_id = guest_identities.id)")
        .bind(&job.identity_ids)
        .execute(&mut **tx)
        .await?;
    advance(tx, job.game_id, "verify").await?;
    Ok(())
}

pub(super) async fn inventory(tx: &mut Tx<'_>) -> Result<Vec<String>, PurgeError> {
    let tables: Vec<String> = sqlx::query_scalar(
        r"
        SELECT c.relname::TEXT FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = current_schema() AND c.relkind IN ('r', 'p', 'm', 'f')
        ORDER BY c.relname
    ",
    )
    .fetch_all(&mut **tx)
    .await?;
    for table in &tables {
        if !ROOM_TABLES.contains(&table.as_str())
            && !GAME_TABLES.contains(&table.as_str())
            && !IDENTITY_TABLES.contains(&table.as_str())
            && !GLOBAL_TABLES.contains(&table.as_str())
        {
            return Err(PurgeError::Orphans);
        }
    }
    Ok(tables)
}

pub(super) async fn verify_inventory(tx: &mut Tx<'_>, job: &Job) -> Result<(), PurgeError> {
    let shared: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT guest_identity_id FROM participants WHERE guest_identity_id = ANY($1) AND room_id <> $2")
        .bind(&job.identity_ids).bind(job.room_id).fetch_all(&mut **tx).await?;
    let mut ids = job.identifiers();
    ids.retain(|id| !shared.contains(id));
    for table in inventory(tx).await? {
        if GLOBAL_TABLES.contains(&table.as_str()) {
            continue;
        }
        // All legitimate writes to this cohort require its deleted roots/FKs.
        // Do not lock unrelated games while checking detached orphan rows.
        let columns: Vec<String> = sqlx::query_scalar(
            r"
            SELECT column_name::TEXT FROM information_schema.columns
            WHERE table_schema = current_schema() AND table_name = $1 AND udt_name = 'uuid'
        ",
        )
        .bind(&table)
        .fetch_all(&mut **tx)
        .await?;
        for column in columns {
            // Identifiers come from PostgreSQL, still quote them defensively.
            let column = column.replace('"', "\"\"");
            let exists: bool = sqlx::query_scalar(AssertSqlSafe(format!(
                "SELECT EXISTS(SELECT 1 FROM {table} WHERE \"{column}\" = ANY($1))"
            )))
            .bind(&ids)
            .fetch_one(&mut **tx)
            .await?;
            if exists {
                return Err(PurgeError::Orphans);
            }
        }
    }
    Ok(())
}

pub(super) async fn verify(tx: &mut Tx<'_>, job: &Job) -> Result<(), PurgeError> {
    verify_inventory(tx, job).await?;
    sqlx::query("UPDATE lifecycle_purge_jobs SET stage = 'complete', verified_at = clock_timestamp() WHERE game_id = $1")
        .bind(job.game_id).execute(&mut **tx).await?;
    Ok(())
}

pub(super) async fn complete(tx: &mut Tx<'_>, job: &Job) -> Result<(), sqlx::Error> {
    sqlx::query(
        r"
        INSERT INTO lifecycle_purge_observations (detection_seconds, purge_seconds, failed_attempts)
        SELECT greatest(0, extract(epoch FROM detected_at - expires_at)),
               greatest(0, extract(epoch FROM clock_timestamp() - expires_at)), failures
        FROM lifecycle_purge_jobs WHERE game_id = $1
    ",
    )
    .bind(job.game_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM lifecycle_purge_jobs WHERE game_id = $1")
        .bind(job.game_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn metrics(database: &PgPool) -> Result<LifecycleMetrics, sqlx::Error> {
    sqlx::query_as(r"
        WITH due AS (SELECT greatest(0, extract(epoch FROM clock_timestamp() - expires_at))::FLOAT8 AS age,
            greatest(0, extract(epoch FROM coalesce(access_expired_at, clock_timestamp()) - expires_at))::FLOAT8 AS detection,
            access_expired_at IS NULL AS undetected
            FROM games WHERE expires_at <= clock_timestamp()
                AND NOT EXISTS (SELECT 1 FROM lifecycle_purge_jobs WHERE game_id = games.id)),
        pending AS (SELECT greatest(0, extract(epoch FROM clock_timestamp() - expires_at))::FLOAT8 AS age,
            greatest(0, extract(epoch FROM detected_at - expires_at))::FLOAT8 AS detection, failures FROM lifecycle_purge_jobs),
        observations AS (SELECT * FROM lifecycle_purge_observations WHERE observed_at >= clock_timestamp() - INTERVAL '14 days'),
        detections AS (SELECT detection_seconds AS seconds FROM observations UNION ALL SELECT detection FROM pending UNION ALL SELECT detection FROM due)
        SELECT (SELECT count(*) FROM pending) + (SELECT count(*) FROM due) AS pending,
            (SELECT count(*) FROM observations) AS completed,
            (SELECT coalesce(sum(failures), 0)::BIGINT FROM pending) + (SELECT coalesce(sum(failed_attempts), 0)::BIGINT FROM observations) AS failed_attempts,
            (SELECT coalesce(percentile_cont(0.95) WITHIN GROUP (ORDER BY seconds), 0)::FLOAT8 FROM detections) AS detection_p95_seconds,
            (SELECT coalesce(percentile_cont(0.95) WITHIN GROUP (ORDER BY seconds), 0)::FLOAT8 FROM (
                SELECT purge_seconds AS seconds FROM observations UNION ALL SELECT age FROM pending UNION ALL SELECT age FROM due
            ) purge_ages) AS purge_p95_seconds,
            (SELECT coalesce(max(purge_seconds), 0)::FLOAT8 FROM observations) AS purge_max_seconds,
            greatest((SELECT coalesce(max(age), 0) FROM pending), (SELECT coalesce(max(age), 0) FROM due)) AS oldest_pending_seconds,
            (SELECT count(*) FROM due WHERE undetected) AS undetected,
            (SELECT count(*) FROM pending WHERE age >= 86400) + (SELECT count(*) FROM due WHERE age >= 86400) AS overdue,
            (SELECT count(*) FROM lifecycle_purge_jobs WHERE last_failure = 'orphans') AS orphan_jobs,
            (SELECT coalesce(jsonb_object_agg(stage, count), '{}'::JSONB)
                FROM (SELECT stage, count(*) FROM lifecycle_purge_jobs GROUP BY stage) counts) AS stages
    ").fetch_one(database).await
}

pub(super) async fn configure_transaction(tx: &mut Tx<'_>) -> Result<(), sqlx::Error> {
    for setting in [
        "SET LOCAL statement_timeout = '4s'",
        "SET LOCAL lock_timeout = '2s'",
    ] {
        sqlx::query(setting).execute(&mut **tx).await?;
    }
    Ok(())
}
