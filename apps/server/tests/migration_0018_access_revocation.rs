use std::str::FromStr;

use sqlx::{AssertSqlSafe, PgPool, postgres::PgConnectOptions, postgres::PgPoolOptions};
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn migration_0018_preserves_existing_events_and_enforces_revocation_shapes() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    let schema_name = format!("access_revocation_migration_{}", Uuid::new_v4().simple());
    sqlx::query(AssertSqlSafe(format!(r#"CREATE SCHEMA "{schema_name}""#)))
        .execute(&admin)
        .await
        .expect("the isolated migration schema must be created");

    let test_result: TestResult<()> = async {
        let connect_options = PgConnectOptions::from_str(&database_url)?
            .options([("search_path", format!("{schema_name},public"))]);
        let database = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(connect_options)
            .await?;
        let result = exercise_upgrade(&database).await;
        database.close().await;
        result
    }
    .await;

    let cleanup_result = sqlx::query(AssertSqlSafe(format!(
        r#"DROP SCHEMA IF EXISTS "{schema_name}" CASCADE"#
    )))
    .execute(&admin)
    .await;
    admin.close().await;
    cleanup_result.expect("the isolated migration schema must be removed");
    test_result.expect("the access revocation upgrade must succeed");
}

async fn exercise_upgrade(database: &PgPool) -> TestResult<()> {
    MIGRATOR.run_to(17, database).await?;
    let (room_id, participant_id) = seed_legacy_security_event(database).await?;
    MIGRATOR.run_to(18, database).await?;

    assert_legacy_event_preserved(database, room_id).await?;
    assert_access_revocation_event_shapes(database, room_id, participant_id).await?;
    assert_access_revocation_receipt_shapes(database, room_id, participant_id).await?;
    assert_access_revocation_schema(database).await?;
    Ok(())
}

async fn seed_legacy_security_event(database: &PgPool) -> TestResult<(Uuid, Uuid)> {
    let room_id = Uuid::new_v4();
    let participant_id = Uuid::new_v4();
    let identity_id = Uuid::new_v4();
    let mut transaction = database.begin().await?;
    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(identity_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        r"
        INSERT INTO rooms (id, code, host_participant_id, recovery_password_hash)
        VALUES ($1, 'SAFE2345', $2, '$argon2id$legacy-room-password-hash')
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r"
        INSERT INTO participants (
            id,
            room_id,
            guest_identity_id,
            display_name,
            role,
            position
        )
        VALUES ($1, $2, $3, 'Minerva', 'host', 1)
        ",
    )
    .bind(participant_id)
    .bind(room_id)
    .bind(identity_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r"
        INSERT INTO identity_security_events (
            room_id,
            sequence,
            event_type,
            actor_participant_id,
            password_generation
        )
        VALUES ($1, 1, 'recovery_password_rotated', $2, 2)
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok((room_id, participant_id))
}

async fn assert_legacy_event_preserved(database: &PgPool, room_id: Uuid) -> TestResult<()> {
    let preserved = sqlx::query_as::<_, (String, Option<i16>, Option<i64>, Option<i64>)>(
        r"
        SELECT event_type, session_slot, revoked_session_count, recovery_epoch
        FROM identity_security_events
        WHERE room_id = $1 AND sequence = 1
        ",
    )
    .bind(room_id)
    .fetch_one(database)
    .await?;
    assert_eq!(
        preserved,
        ("recovery_password_rotated".to_owned(), None, None, None)
    );
    Ok(())
}

async fn assert_access_revocation_event_shapes(
    database: &PgPool,
    room_id: Uuid,
    participant_id: Uuid,
) -> TestResult<()> {
    sqlx::query(
        r"
        INSERT INTO identity_security_events (
            room_id,
            sequence,
            event_type,
            actor_participant_id,
            target_participant_id,
            session_slot
        )
        VALUES ($1, 2, 'session_revoked', $2, $2, 1)
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .execute(database)
    .await?;

    sqlx::query(
        r"
        INSERT INTO identity_security_events (
            room_id,
            sequence,
            event_type,
            actor_participant_id,
            target_participant_id,
            recovery_generation,
            revoked_session_count
        )
        VALUES ($1, 3, 'participant_protected', $2, $2, 2, 1)
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .execute(database)
    .await?;
    sqlx::query(
        r"
        INSERT INTO identity_security_events (
            room_id,
            sequence,
            event_type,
            actor_participant_id,
            password_generation,
            recovery_epoch,
            revoked_session_count,
            current_session_preserved
        )
        VALUES ($1, 4, 'room_protected', $2, 2, 2, 0, TRUE)
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .execute(database)
    .await?;

    let invalid = sqlx::query(
        r"
        INSERT INTO identity_security_events (
            room_id,
            sequence,
            event_type,
            actor_participant_id,
            target_participant_id
        )
        VALUES ($1, 5, 'session_revoked', $2, $2)
        ",
    )
    .bind(room_id)
    .bind(participant_id)
    .execute(database)
    .await
    .expect_err("session revocation without a safe slot must be rejected");
    assert_eq!(
        invalid
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("identity_security_events_shape")
    );
    Ok(())
}

async fn assert_access_revocation_receipt_shapes(
    database: &PgPool,
    room_id: Uuid,
    participant_id: Uuid,
) -> TestResult<()> {
    let guest_session_id = Uuid::new_v4();
    sqlx::query(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token_digest)
        SELECT $1, guest_identity_id, 'sha256:' || repeat('a', 64)
        FROM participants
        WHERE id = $2
        ",
    )
    .bind(guest_session_id)
    .bind(participant_id)
    .execute(database)
    .await?;

    let partial_participant_receipt = sqlx::query(
        r"
        INSERT INTO participant_protection_requests (
            idempotency_key,
            room_id,
            actor_participant_id,
            actor_guest_session_id,
            request_fingerprint,
            revoked_session_count,
            security_event_sequence,
            completed_at
        )
        VALUES ($1, $2, $3, $4, $5, 1, 3, clock_timestamp())
        ",
    )
    .bind("participant-partial-receipt")
    .bind(room_id)
    .bind(participant_id)
    .bind(guest_session_id)
    .bind(format!("hmac-sha256:{}", "b".repeat(64)))
    .execute(database)
    .await
    .expect_err("a completed participant receipt with a null generation must be rejected");
    assert_eq!(
        partial_participant_receipt
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("participant_protection_completion_consistent")
    );

    let partial_room_receipt = sqlx::query(
        r"
        INSERT INTO room_protection_requests (
            idempotency_key,
            room_id,
            actor_participant_id,
            actor_guest_session_id,
            request_fingerprint,
            recovery_epoch,
            revoked_session_count,
            current_session_preserved,
            security_event_sequence,
            completed_at
        )
        VALUES ($1, $2, $3, $4, $5, 2, 0, TRUE, 4, clock_timestamp())
        ",
    )
    .bind("room-partial-receipt")
    .bind(room_id)
    .bind(participant_id)
    .bind(guest_session_id)
    .bind(format!("hmac-sha256:{}", "c".repeat(64)))
    .execute(database)
    .await
    .expect_err("a completed room receipt with a null password generation must be rejected");
    assert_eq!(
        partial_room_receipt
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("room_protection_completion_consistent")
    );
    Ok(())
}

async fn assert_access_revocation_schema(database: &PgPool) -> TestResult<()> {
    let receipt_tables = sqlx::query_scalar::<_, i64>(
        r"
        SELECT COUNT(*)
        FROM information_schema.tables
        WHERE table_schema = current_schema()
          AND table_name IN (
              'device_session_revocation_requests',
              'participant_protection_requests',
              'room_protection_requests'
          )
        ",
    )
    .fetch_one(database)
    .await?;
    assert_eq!(receipt_tables, 3);
    let schema_version = sqlx::query_scalar::<_, String>(
        "SELECT value FROM application_metadata WHERE key = 'schema_version'",
    )
    .fetch_one(database)
    .await?;
    assert_eq!(schema_version, "18");
    Ok(())
}
