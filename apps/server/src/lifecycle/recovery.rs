//! Isolated restore reconciliation and the persistent deployment traffic gate.
mod postgres;

use super::{PurgeError, PurgeProof, TombstoneLedger};
use crate::AppState;
use std::{
    error::Error,
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

/// Private, freshly issued recovery material for one surviving participation.
/// Deliver only after verifying the recipient outside the restored credentials.
#[derive(Eq, PartialEq, serde::Serialize)]
pub struct RestoredAccess {
    pub room_code: String,
    pub participant_id: String,
    pub position: i16,
    pub recovery_password: String,
    pub recovery_token: String,
}
impl fmt::Debug for RestoredAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RestoredAccess([redacted])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreError {
    Database,
    Deployment,
    Ledger,
    Clock,
    Window,
    Busy,
}
impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Database => "restore database reconciliation failed",
            Self::Deployment => "restore requires a bound source and fresh deployment credentials",
            Self::Ledger => "restore external tombstone evidence unavailable",
            Self::Clock => "restore database clock is inconsistent with current time",
            Self::Window => "restore point is outside the seven-day window",
            Self::Busy => "another restore coordinator is running",
        })
    }
}
impl Error for RestoreError {}
impl From<sqlx::Error> for RestoreError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}

/// Reconciles a target isolated from clients, source processes and lifecycle workers.
/// The target must use a fresh epoch and session key from outside the backup.
/// Successful retries of the same restore point do not rotate access again.
///
/// # Errors
/// Fails closed on unavailable evidence, clock skew, an old restore point or
/// reused credentials. A failed reconciliation leaves the persisted gate closed.
pub async fn reconcile_restore<L: TombstoneLedger>(
    state: &AppState,
    ledger: &L,
    key: [u8; 32],
    restored_from_ms: i64,
) -> Result<Vec<RestoredAccess>, RestoreError> {
    crate::MIGRATOR
        .run(&state.migration_database)
        .await
        .map_err(|_| RestoreError::Database)?;
    let epoch = state.deployment_epoch.ok_or(RestoreError::Deployment)?;
    let mut tx = state.database.begin().await?;
    postgres::lock_coordinator(&mut tx).await?;
    let binding = postgres::binding(&mut tx)
        .await?
        .ok_or(RestoreError::Deployment)?;
    if binding.epoch == epoch {
        return if binding.ready
            && binding.credential_fingerprint == credential_fingerprint(state)
            && binding.restored_from_ms == Some(restored_from_ms)
        {
            restored_access(state, &mut tx, false).await
        } else {
            Err(RestoreError::Deployment)
        };
    }
    if binding.credential_fingerprint == credential_fingerprint(state)
        || binding.database_login == postgres::current_login(&mut tx).await?
    {
        return Err(RestoreError::Deployment);
    }
    let now = checked_clock(&mut tx, restored_from_ms).await?;
    // This separate commit survives rollback, process loss and uncertain ACKs.
    postgres::close_gate(&state.database).await?;
    if binding.ledger_key_fingerprint.as_deref() != Some(&ledger_fingerprint(&key))
        || ledger
            .lookup(&witness_key(&key, binding.epoch))
            .await
            .map_err(|_| RestoreError::Ledger)?
            != Some(witness_proof())
    {
        return Err(RestoreError::Ledger);
    }
    super::postgres::inventory(&mut tx)
        .await
        .map_err(|_| RestoreError::Database)?;
    let games = postgres::lock_games(&mut tx).await?;
    for &game_id in &games {
        let opaque = crate::encode_hex(&crate::hmac_sha256(
            &key,
            &[b"hogwarts-tombstone-v1", game_id.as_bytes()],
        ));
        let proof = ledger
            .lookup(&opaque)
            .await
            .map_err(|_| RestoreError::Ledger)?;
        if let Some(proof) = proof {
            if proof.version != 1
                || proof.completed_at_ms.is_some()
                || proof.expires_at_ms < 0
                || proof.detected_at_ms < proof.expires_at_ms
                || proof.detected_at_ms > now
            {
                return Err(RestoreError::Ledger);
            }
            postgres::apply_tombstone(&mut tx, game_id, &proof).await?;
        }
        postgres::expire_game(&mut tx, game_id).await?;
    }
    postgres::revoke_access(&mut tx).await?;
    restored_access(state, &mut tx, true).await?;
    // Preserve the new scope witness before it can enter the next backup.
    ledger
        .record(&witness_key(&key, epoch), &witness_proof())
        .await
        .map_err(|_| RestoreError::Ledger)?;
    checked_clock(&mut tx, restored_from_ms).await?;
    for game_id in games {
        postgres::expire_game(&mut tx, game_id).await?;
    }
    let access = restored_access(state, &mut tx, false).await?;
    postgres::open_gate(&mut tx, state, restored_from_ms).await?;
    tx.commit().await?;
    Ok(access)
}

async fn checked_clock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    point: i64,
) -> Result<i64, RestoreError> {
    let before = wall_clock_ms()?;
    let now = postgres::clock_ms(tx).await?;
    let after = wall_clock_ms()?;
    if now < before - 5000 || now > after + 5000 || !postgres::timestamps_valid(tx).await? {
        return Err(RestoreError::Clock);
    }
    let watermark = postgres::backup_watermark(tx).await?;
    if watermark > now {
        return Err(RestoreError::Clock);
    }
    if point < now - 7 * 24 * 60 * 60 * 1000
        || point > now
        || point < 0
        || watermark < now - 7 * 24 * 60 * 60 * 1000
        || point < watermark
        || point - watermark > 300_000
    {
        return Err(RestoreError::Window);
    }
    Ok(now)
}
fn wall_clock_ms() -> Result<i64, RestoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .ok_or(RestoreError::Clock)
}

pub(crate) async fn register_deployment(state: &AppState) -> Result<(), PurgeError> {
    postgres::register_deployment(state).await
}
pub(crate) async fn deployment_ready(state: &AppState) -> Result<bool, PurgeError> {
    postgres::deployment_ready(state).await
}
fn credential_fingerprint(state: &AppState) -> String {
    crate::encode_hex(&crate::hmac_sha256(
        state.session_token_key.as_ref(),
        &[b"hogwarts-deployment-credential-v1"],
    ))
}
fn ledger_fingerprint(key: &[u8; 32]) -> String {
    crate::encode_hex(&crate::hmac_sha256(key, &[b"hogwarts-ledger-key-v1"]))
}
fn witness_key(key: &[u8; 32], epoch: uuid::Uuid) -> String {
    crate::encode_hex(&crate::hmac_sha256(
        key,
        &[b"hogwarts-ledger-witness-v1", epoch.as_bytes()],
    ))
}
fn witness_proof() -> PurgeProof {
    PurgeProof {
        version: 1,
        expires_at_ms: 0,
        detected_at_ms: 0,
        completed_at_ms: None,
    }
}

pub(super) async fn record_ledger_witness<L: TombstoneLedger>(
    database: &sqlx::PgPool,
    ledger: &L,
    key: &[u8; 32],
) -> Result<(), PurgeError> {
    if let Some(epoch) = postgres::bind_ledger(database, &ledger_fingerprint(key)).await? {
        ledger
            .record(&witness_key(key, epoch), &witness_proof())
            .await?;
    }
    Ok(())
}

async fn restored_access(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    issue: bool,
) -> Result<Vec<RestoredAccess>, RestoreError> {
    let participants = postgres::surviving_participants(tx).await?;
    let epoch = state
        .deployment_epoch
        .ok_or(RestoreError::Deployment)?
        .to_string();
    let mut access = Vec::with_capacity(participants.len());
    let mut previous_room = None;
    for participant in participants {
        let password =
            state.idempotent_session_token("restore-room-password", &epoch, participant.room_id);
        let token =
            state.idempotent_recovery_token("restore-participation", &epoch, participant.id);
        if issue {
            if previous_room != Some(participant.room_id) {
                let hash = crate::identity_access::hash_password(state, password.clone())
                    .await
                    .map_err(|_| RestoreError::Database)?;
                postgres::rotate_password(tx, participant.room_id, &hash).await?;
                previous_room = Some(participant.room_id);
            }
            postgres::issue_credential(tx, participant.id, &state.recovery_token_hmac(&token))
                .await?;
        } else if !postgres::credential_current(
            tx,
            participant.id,
            &state.recovery_token_hmac(&token),
        )
        .await?
        {
            continue;
        }
        access.push(RestoredAccess {
            room_code: participant.room_code,
            participant_id: participant.id.to_string(),
            position: participant.position,
            recovery_password: password,
            recovery_token: token,
        });
    }
    Ok(access)
}

/// Explicitly adopts the original pre-restore database during a schema upgrade.
/// This operator-only step must never be used to approve a restored copy.
///
/// # Errors
/// Rejects an existing incompatible binding or unavailable external ledger.
pub async fn adopt_existing_source<L: TombstoneLedger>(
    state: &AppState,
    ledger: &L,
    key: [u8; 32],
) -> Result<(), RestoreError> {
    crate::MIGRATOR
        .run(&state.migration_database)
        .await
        .map_err(|_| RestoreError::Database)?;
    let epoch = state.deployment_epoch.ok_or(RestoreError::Deployment)?;
    let mut tx = state.database.begin().await?;
    postgres::lock_coordinator(&mut tx).await?;
    if let Some(binding) = postgres::binding(&mut tx).await? {
        if binding.epoch != epoch
            || binding.credential_fingerprint != credential_fingerprint(state)
            || binding.restored_from_ms.is_some()
            || !binding.ready
            || binding
                .ledger_key_fingerprint
                .as_deref()
                .is_some_and(|value| value != ledger_fingerprint(&key))
        {
            return Err(RestoreError::Deployment);
        }
    } else {
        postgres::insert_source_binding(&mut tx, state).await?;
    }
    ledger
        .record(&witness_key(&key, epoch), &witness_proof())
        .await
        .map_err(|_| RestoreError::Ledger)?;
    postgres::set_source_ledger(&mut tx, &ledger_fingerprint(&key)).await?;
    tx.commit().await?;
    Ok(())
}
