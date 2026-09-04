use std::str::FromStr;

use sqlx::{
    AssertSqlSafe, FromRow, PgConnection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

const LEGACY_PASSWORD_HASH: &str = "$argon2id$legacy-room-password-hash";

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, Debug)]
struct LegacyFixture {
    active_credential: Uuid,
    consumed_credential: Uuid,
    host_guest_session: Uuid,
    consumed_guest_session: Uuid,
    recovery_attempt: Uuid,
}

#[derive(Clone, Copy, Debug)]
struct LegacyActors {
    room: Uuid,
    host_identity: Uuid,
    guest_identity: Uuid,
    host_participant: Uuid,
    guest_participant: Uuid,
}

#[derive(Debug, FromRow, PartialEq, Eq)]
struct MigratedCredential {
    position: i16,
    status: String,
    recovery_password_hash: String,
    recovery_epoch: i64,
    password_generation: i64,
    recovery_generation: i64,
    recovery_attempt_id: Option<Uuid>,
    consumed_by_guest_session_id: Option<Uuid>,
    consumed_at_present: bool,
    superseded_at_present: bool,
}

#[derive(Debug)]
struct UpgradeObservation {
    fixture: LegacyFixture,
    pre_upgrade_applied_version: i64,
    pre_upgrade_schema_version: String,
    room_generations: (i64, i64, i64),
    participant_generations: Vec<(i16, i64)>,
    credentials: Vec<MigratedCredential>,
    consumed_device_session_status: String,
    rejected_recovery_session_error: Option<(String, String)>,
    superseded_lifecycle: (String, bool, bool, bool, bool),
    rejected_lifecycle_constraint: Option<String>,
    applied_version: i64,
    schema_version: String,
}

#[derive(Debug)]
struct MigratedState {
    room_generations: (i64, i64, i64),
    participant_generations: Vec<(i16, i64)>,
    credentials: Vec<MigratedCredential>,
    consumed_device_session_status: String,
}

#[tokio::test]
async fn migration_0016_upgrades_existing_recovery_credentials() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    let schema_name = format!("recovery_migration_{}", Uuid::new_v4().simple());
    sqlx::query(AssertSqlSafe(format!(r#"CREATE SCHEMA "{schema_name}""#)))
        .execute(&admin)
        .await
        .expect("the isolated migration schema must be created");

    let test_result: TestResult<UpgradeObservation> = async {
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

    let observation = test_result.expect("the recovery credential upgrade must succeed");
    assert_eq!(observation.pre_upgrade_applied_version, 15);
    assert_eq!(observation.pre_upgrade_schema_version, "15");
    assert_eq!(observation.room_generations, (1, 1, 0));
    assert_eq!(observation.participant_generations, vec![(1, 1), (2, 1)]);
    assert_eq!(observation.consumed_device_session_status, "revoked");
    assert_eq!(
        observation.rejected_recovery_session_error,
        Some((
            "23514".to_owned(),
            "recovery credential must create an active session for its participant".to_owned(),
        ))
    );
    assert_eq!(
        observation.credentials,
        vec![
            MigratedCredential {
                position: 1,
                status: "active".to_owned(),
                recovery_password_hash: LEGACY_PASSWORD_HASH.to_owned(),
                recovery_epoch: 1,
                password_generation: 1,
                recovery_generation: 1,
                recovery_attempt_id: None,
                consumed_by_guest_session_id: None,
                consumed_at_present: false,
                superseded_at_present: false,
            },
            MigratedCredential {
                position: 2,
                status: "consumed".to_owned(),
                recovery_password_hash: LEGACY_PASSWORD_HASH.to_owned(),
                recovery_epoch: 1,
                password_generation: 1,
                recovery_generation: 1,
                recovery_attempt_id: Some(observation.fixture.recovery_attempt),
                consumed_by_guest_session_id: Some(observation.fixture.consumed_guest_session),
                consumed_at_present: true,
                superseded_at_present: false,
            },
        ]
    );
    assert_eq!(
        observation.superseded_lifecycle,
        ("superseded".to_owned(), true, true, true, true)
    );
    assert_eq!(
        observation.rejected_lifecycle_constraint.as_deref(),
        Some("recovery_credentials_lifecycle_consistent")
    );
    assert_eq!(observation.applied_version, 18);
    assert_eq!(observation.schema_version, "18");
}

async fn exercise_upgrade(database: &PgPool) -> TestResult<UpgradeObservation> {
    MIGRATOR.run_to(15, database).await?;
    let pre_upgrade_applied_version =
        sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(database)
            .await?;
    let pre_upgrade_schema_version = sqlx::query_scalar::<_, String>(
        "SELECT value FROM application_metadata WHERE key = 'schema_version'",
    )
    .fetch_one(database)
    .await?;
    let fixture = seed_legacy_recovery_state(database).await?;

    MIGRATOR.run(database).await?;

    observe_upgrade(
        database,
        fixture,
        pre_upgrade_applied_version,
        pre_upgrade_schema_version,
    )
    .await
}

async fn observe_upgrade(
    database: &PgPool,
    fixture: LegacyFixture,
    pre_upgrade_applied_version: i64,
    pre_upgrade_schema_version: String,
) -> TestResult<UpgradeObservation> {
    let state = read_migrated_state(database, fixture).await?;
    let rejected_recovery_session_error =
        reject_invalid_recovery_session(database, fixture).await?;
    let (superseded_lifecycle, rejected_lifecycle_constraint) =
        observe_lifecycle_constraints(database, fixture).await?;
    let applied_version = sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(database)
        .await?;
    let schema_version = sqlx::query_scalar::<_, String>(
        "SELECT value FROM application_metadata WHERE key = 'schema_version'",
    )
    .fetch_one(database)
    .await?;

    Ok(UpgradeObservation {
        fixture,
        pre_upgrade_applied_version,
        pre_upgrade_schema_version,
        room_generations: state.room_generations,
        participant_generations: state.participant_generations,
        credentials: state.credentials,
        consumed_device_session_status: state.consumed_device_session_status,
        rejected_recovery_session_error,
        superseded_lifecycle,
        rejected_lifecycle_constraint,
        applied_version,
        schema_version,
    })
}

async fn read_migrated_state(
    database: &PgPool,
    fixture: LegacyFixture,
) -> Result<MigratedState, sqlx::Error> {
    let room_generations = sqlx::query_as::<_, (i64, i64, i64)>(
        r"
        SELECT password_generation, recovery_epoch, security_event_sequence
        FROM rooms
        WHERE code = 'TEST2345'
        ",
    )
    .fetch_one(database)
    .await?;
    let participant_generations = sqlx::query_as::<_, (i16, i64)>(
        r"
        SELECT position, recovery_generation
        FROM participants
        ORDER BY position
        ",
    )
    .fetch_all(database)
    .await?;
    let credentials = sqlx::query_as::<_, MigratedCredential>(
        r"
        SELECT
            participants.position,
            credentials.status,
            credentials.recovery_password_hash,
            credentials.recovery_epoch,
            credentials.password_generation,
            credentials.recovery_generation,
            credentials.recovery_attempt_id,
            credentials.consumed_by_guest_session_id,
            credentials.consumed_at IS NOT NULL AS consumed_at_present,
            credentials.superseded_at IS NOT NULL AS superseded_at_present
        FROM recovery_credentials AS credentials
        JOIN participants ON participants.id = credentials.participant_id
        ORDER BY participants.position
        ",
    )
    .fetch_all(database)
    .await?;
    let consumed_device_session_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM device_sessions WHERE guest_session_id = $1",
    )
    .bind(fixture.consumed_guest_session)
    .fetch_one(database)
    .await?;

    Ok(MigratedState {
        room_generations,
        participant_generations,
        credentials,
        consumed_device_session_status,
    })
}

async fn reject_invalid_recovery_session(
    database: &PgPool,
    fixture: LegacyFixture,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let mut transaction = database.begin().await?;
    sqlx::query(
        r"
        UPDATE recovery_credentials
        SET
            status = 'consumed',
            recovery_attempt_id = $2,
            consumed_by_guest_session_id = $3,
            consumed_at = clock_timestamp()
        WHERE id = $1
        ",
    )
    .bind(fixture.active_credential)
    .bind(Uuid::new_v4())
    .bind(fixture.host_guest_session)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r"
        UPDATE device_sessions
        SET status = 'revoked'
        WHERE guest_session_id = $1
        ",
    )
    .bind(fixture.host_guest_session)
    .execute(&mut *transaction)
    .await?;
    Ok(transaction.commit().await.err().and_then(|error| {
        error.as_database_error().and_then(|database_error| {
            Some((
                database_error.code()?.into_owned(),
                database_error.message().to_owned(),
            ))
        })
    }))
}

async fn observe_lifecycle_constraints(
    database: &PgPool,
    fixture: LegacyFixture,
) -> Result<((String, bool, bool, bool, bool), Option<String>), sqlx::Error> {
    let superseded_lifecycle = sqlx::query_as::<_, (String, bool, bool, bool, bool)>(
        r"
        UPDATE recovery_credentials
        SET status = 'superseded', superseded_at = clock_timestamp()
        WHERE id = $1
        RETURNING
            status,
            recovery_attempt_id IS NULL,
            consumed_by_guest_session_id IS NULL,
            consumed_at IS NULL,
            superseded_at IS NOT NULL
        ",
    )
    .bind(fixture.active_credential)
    .fetch_one(database)
    .await?;
    let rejected_lifecycle_constraint = sqlx::query(
        r"
        UPDATE recovery_credentials
        SET superseded_at = clock_timestamp()
        WHERE id = $1
        ",
    )
    .bind(fixture.consumed_credential)
    .execute(database)
    .await
    .err()
    .and_then(|error| {
        error
            .as_database_error()
            .and_then(|database_error| database_error.constraint().map(str::to_owned))
    });

    Ok((superseded_lifecycle, rejected_lifecycle_constraint))
}

async fn seed_legacy_recovery_state(database: &PgPool) -> Result<LegacyFixture, sqlx::Error> {
    let actors = LegacyActors {
        room: Uuid::new_v4(),
        host_identity: Uuid::new_v4(),
        guest_identity: Uuid::new_v4(),
        host_participant: Uuid::new_v4(),
        guest_participant: Uuid::new_v4(),
    };
    let fixture = LegacyFixture {
        active_credential: Uuid::new_v4(),
        consumed_credential: Uuid::new_v4(),
        host_guest_session: Uuid::new_v4(),
        consumed_guest_session: Uuid::new_v4(),
        recovery_attempt: Uuid::new_v4(),
    };
    let mut transaction = database.begin().await?;

    seed_legacy_actors(&mut transaction, actors).await?;
    seed_legacy_sessions(&mut transaction, actors, fixture).await?;
    seed_legacy_credentials(&mut transaction, actors, fixture).await?;
    transaction.commit().await?;

    sqlx::query("UPDATE device_sessions SET status = 'revoked' WHERE guest_session_id = $1")
        .bind(fixture.consumed_guest_session)
        .execute(database)
        .await?;

    Ok(fixture)
}

async fn seed_legacy_actors(
    connection: &mut PgConnection,
    actors: LegacyActors,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1), ($2)")
        .bind(actors.host_identity)
        .bind(actors.guest_identity)
        .execute(&mut *connection)
        .await?;
    sqlx::query(
        r"
        INSERT INTO rooms (id, code, host_participant_id, recovery_password_hash)
        VALUES ($1, 'TEST2345', $2, $3)
        ",
    )
    .bind(actors.room)
    .bind(actors.host_participant)
    .bind(LEGACY_PASSWORD_HASH)
    .execute(&mut *connection)
    .await?;
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
        VALUES
            ($1, $3, $4, 'Minerva', 'host', 1, NULL),
            ($2, $3, $5, 'Luna', 'guest', 2, 'hermione')
        ",
    )
    .bind(actors.host_participant)
    .bind(actors.guest_participant)
    .bind(actors.room)
    .bind(actors.host_identity)
    .bind(actors.guest_identity)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn seed_legacy_sessions(
    connection: &mut PgConnection,
    actors: LegacyActors,
    fixture: LegacyFixture,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token_digest)
        VALUES
            ($1, $3, $5),
            ($2, $4, $6)
        ",
    )
    .bind(fixture.host_guest_session)
    .bind(fixture.consumed_guest_session)
    .bind(actors.host_identity)
    .bind(actors.guest_identity)
    .bind(format!("sha256:{}", "a".repeat(64)))
    .bind(format!("sha256:{}", "b".repeat(64)))
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        r"
        INSERT INTO device_sessions (id, guest_session_id, participant_id)
        VALUES
            ($1, $3, $5),
            ($2, $4, $6)
        ",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(fixture.host_guest_session)
    .bind(fixture.consumed_guest_session)
    .bind(actors.host_participant)
    .bind(actors.guest_participant)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn seed_legacy_credentials(
    connection: &mut PgConnection,
    actors: LegacyActors,
    fixture: LegacyFixture,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r"
        INSERT INTO recovery_credentials (id, participant_id, token_hmac)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(fixture.active_credential)
    .bind(actors.host_participant)
    .bind(format!("hmac-sha256:{}", "c".repeat(64)))
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        r"
        INSERT INTO recovery_credentials (
            id,
            participant_id,
            token_hmac,
            status,
            recovery_attempt_id,
            consumed_by_guest_session_id,
            consumed_at
        )
        VALUES ($1, $2, $3, 'consumed', $4, $5, clock_timestamp())
        ",
    )
    .bind(fixture.consumed_credential)
    .bind(actors.guest_participant)
    .bind(format!("hmac-sha256:{}", "d".repeat(64)))
    .bind(fixture.recovery_attempt)
    .bind(fixture.consumed_guest_session)
    .execute(&mut *connection)
    .await?;

    Ok(())
}
