use super::*;
use harry_potter_server::lifecycle::{
    FileLedger, LifecycleWorker, PurgeError, PurgeProof, RestoreError, TombstoneLedger,
    reconcile_restore,
};
use sqlx::{AssertSqlSafe, postgres::PgConnectOptions};
use std::str::FromStr;
use uuid::Uuid;

struct Fixture {
    schema: String,
    database: PgPool,
    source: AppState,
    target_database: PgPool,
}
impl Fixture {
    async fn new() -> Self {
        Self::at_version(None).await
    }
    async fn at_version(version: Option<i64>) -> Self {
        let admin = database().await;
        let schema = format!("restore_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .unwrap();
        let password = Uuid::new_v4().simple().to_string();
        sqlx::query(AssertSqlSafe(format!(
            "CREATE ROLE {schema} LOGIN SUPERUSER PASSWORD '{password}'"
        )))
        .execute(&admin)
        .await
        .unwrap();
        admin.close().await;
        let options = PgConnectOptions::from_str(&std::env::var("TEST_DATABASE_URL").unwrap())
            .unwrap()
            .options([("search_path", schema.clone())]);
        let database = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(options.clone())
            .await
            .unwrap();
        let source = AppState::with_content_manifests(database.clone(), vec![playable_manifest()])
            .with_session_token_key([31; 32])
            .with_deployment_epoch(Uuid::new_v4());
        if let Some(version) = version {
            static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
            MIGRATOR.run_to(version, &database).await.unwrap();
        } else {
            initialize(&source).await.unwrap();
        }
        let target_database = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(options.username(&schema).password(&password))
            .await
            .unwrap();
        Self {
            schema,
            database,
            source,
            target_database,
        }
    }
    fn target(&self) -> AppState {
        AppState::with_content_manifests(self.target_database.clone(), vec![playable_manifest()])
            .with_session_token_key([32; 32])
            .with_deployment_epoch(Uuid::new_v4())
    }
    async fn cleanup(self) {
        self.source.begin_shutdown();
        drop(self.source);
        self.database.close().await;
        self.target_database.close().await;
        let admin = database().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .unwrap();
        sqlx::query(AssertSqlSafe(format!("DROP ROLE {}", self.schema)))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
    }
}

async fn status(state: &AppState, path: &str) -> StatusCode {
    build_router(state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_restored_database_cannot_start_with_a_new_deployment_before_reconciliation() {
    let fixture = Fixture::new().await;
    assert_eq!(
        status(&fixture.source, "/health/ready").await,
        StatusCode::OK
    );
    let target = fixture.target();
    assert!(initialize(&target).await.is_err());
    target.mark_started(); // An in-memory flag cannot bypass the persisted restore gate.
    assert_eq!(
        status(&target, "/health/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        status(&target, "/api/session").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    target.begin_shutdown();
    drop(target);
    fixture.cleanup().await;
}

#[tokio::test]
async fn reconciliation_revokes_old_access_and_preserves_live_game_history_and_deadlines() {
    let fixture = Fixture::new().await;
    let room = ready_room_in_app(
        build_router(fixture.source.clone()),
        fixture.database.clone(),
        fixture.source.clone(),
        playable_manifest(),
    )
    .await;
    start_ready_game(&room, "restore-live").await;
    accepted_request(&room, command_request(&room.host_cookie, Uuid::new_v4(), 1)).await;
    let before: (String, String, String) = sqlx::query_as(
        "SELECT snapshot::TEXT, last_game_action_at::TEXT, expires_at::TEXT FROM games",
    )
    .fetch_one(&fixture.database)
    .await
    .unwrap();
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    LifecycleWorker::new(fixture.database.clone(), ledger.clone(), [7; 32])
        .tick()
        .await
        .unwrap();
    let point = database_now_ms(&fixture.database).await;
    fixture.source.begin_shutdown();
    let target = fixture.target();
    let grants = reconcile_restore(&target, &ledger, [7; 32], point)
        .await
        .unwrap();
    assert_eq!(
        grants.len(),
        2,
        "both original participants receive fresh access"
    );
    initialize(&target).await.unwrap();
    assert_eq!(status(&target, "/health/ready").await, StatusCode::OK);
    let app = build_router(target.clone());
    for cookie in [&room.host_cookie, &room.guest_cookie] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/session")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let command = app
            .clone()
            .oneshot(command_request(cookie, Uuid::new_v4(), 2))
            .await
            .unwrap();
        assert_eq!(command.status(), StatusCode::UNAUTHORIZED);
    }
    let recovery = app
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": room.host_recovery_token,
                "recovery_password": "a long uncommon passphrase",
                "recovery_attempt_id": Uuid::new_v4().to_string()
            }),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(recovery.status(), StatusCode::UNAUTHORIZED);
    let after: (String, String, String) = sqlx::query_as(
        "SELECT snapshot::TEXT, last_game_action_at::TEXT, expires_at::TEXT FROM games",
    )
    .fetch_one(&fixture.database)
    .await
    .unwrap();
    assert_eq!(after, before);
    let epochs: (i64, i64, i64) = sqlx::query_as(
        "SELECT recovery_epoch, password_generation, (SELECT min(recovery_generation) FROM participants) FROM rooms"
    ).fetch_one(&fixture.database).await.unwrap();
    assert_eq!(epochs, (2, 2, 2));
    reconcile_restore(&target, &ledger, [7; 32], point)
        .await
        .unwrap();
    let epoch: i64 = sqlx::query_scalar("SELECT recovery_epoch FROM rooms")
        .fetch_one(&fixture.database)
        .await
        .unwrap();
    assert_eq!(epoch, 2, "retry must not rotate generations again");
    let grant = grants.iter().find(|grant| grant.position == 2).unwrap();
    resume_with_fresh_access(&target, grant).await;
    target.begin_shutdown();
    drop(target);
    drop(room);
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}

async fn database_now_ms(database: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000)::BIGINT")
        .fetch_one(database)
        .await
        .unwrap()
}

#[tokio::test]
async fn tombstones_override_future_deadlines_and_expired_games_are_purged_after_restore() {
    use harry_potter_server::lifecycle::{PurgeProof, TombstoneLedger};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let fixture = Fixture::new().await;
    let mut rooms = Vec::new();
    for _ in 0..2 {
        let room = ready_room_in_app(
            build_router(fixture.source.clone()),
            fixture.database.clone(),
            fixture.source.clone(),
            playable_manifest(),
        )
        .await;
        start_ready_game(&room, "restore-deleted").await;
        rooms.push(room);
    }
    fixture.source.begin_shutdown();
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    let worker = LifecycleWorker::new(fixture.database.clone(), ledger.clone(), [7; 32]);
    worker.tick().await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "SELECT games.id FROM games JOIN rooms ON rooms.id = games.room_id WHERE rooms.code = $1",
    )
    .bind(&rooms[0].room_code)
    .fetch_one(&fixture.database)
    .await
    .unwrap();
    let now = database_now_ms(&fixture.database).await;
    let mut hmac = <Hmac<Sha256> as KeyInit>::new_from_slice(&[7; 32]).unwrap();
    for value in [b"hogwarts-tombstone-v1".as_slice(), id.as_bytes()] {
        hmac.update(&(value.len() as u64).to_be_bytes());
        hmac.update(value);
    }
    let key = hmac
        .finalize()
        .into_bytes()
        .iter()
        .fold(String::new(), |mut output, byte| {
            write!(&mut output, "{byte:02x}").unwrap();
            output
        });
    ledger
        .record(
            &key,
            &PurgeProof {
                version: 1,
                expires_at_ms: now,
                detected_at_ms: now,
                completed_at_ms: None,
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '8 days', expires_at = clock_timestamp() - INTERVAL '1 second' WHERE room_id = (SELECT id FROM rooms WHERE code = $1)")
        .bind(&rooms[1].room_code).execute(&fixture.database).await.unwrap();
    let target = fixture.target();
    reconcile_restore(&target, &ledger, [7; 32], now)
        .await
        .unwrap();
    initialize(&target).await.unwrap();
    let gated: i64 = sqlx::query_scalar("SELECT count(*) FROM games WHERE access_expired_at IS NOT NULL AND expires_at <= clock_timestamp()")
        .fetch_one(&fixture.database).await.unwrap();
    assert_eq!(gated, 2);
    for room in &rooms {
        let response = build_router(target.clone())
            .oneshot(command_request(&room.host_cookie, Uuid::new_v4(), 1))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    for _ in 0..8 {
        worker.tick().await.unwrap();
    }
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM games")
        .fetch_one(&fixture.database)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    assert_eq!(worker.metrics().await.unwrap().pending, 0);
    target.begin_shutdown();
    drop(target);
    drop(rooms);
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn invalid_window_credentials_or_ledger_never_open_the_target() {
    use harry_potter_server::lifecycle::RestoreError;
    let fixture = Fixture::new().await;
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    LifecycleWorker::new(fixture.database.clone(), ledger.clone(), [7; 32])
        .tick()
        .await
        .unwrap();
    let now = database_now_ms(&fixture.database).await;
    let target = fixture.target();
    for point in [now - 604_800_001, now + 60_000] {
        assert_eq!(
            reconcile_restore(&target, &ledger, [7; 32], point).await,
            Err(RestoreError::Window)
        );
    }
    let reused_key = fixture.target().with_session_token_key([31; 32]);
    assert_eq!(
        reconcile_restore(&reused_key, &ledger, [7; 32], now).await,
        Err(RestoreError::Deployment)
    );
    assert_eq!(
        reconcile_restore(&target, &ledger, [8; 32], now).await,
        Err(RestoreError::Ledger)
    );
    let empty_directory = directory.join("wrong-ledger");
    std::fs::create_dir(&empty_directory).unwrap();
    assert_eq!(
        reconcile_restore(&target, &FileLedger::new(empty_directory), [7; 32], now).await,
        Err(RestoreError::Ledger)
    );
    assert_eq!(
        status(&target, "/health/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        status(&fixture.source, "/health/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    reconcile_restore(&target, &ledger, [7; 32], now)
        .await
        .unwrap();
    initialize(&target).await.unwrap();
    assert_eq!(status(&target, "/health/ready").await, StatusCode::OK);
    assert_eq!(
        status(&fixture.source, "/api/session").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    target.begin_shutdown();
    drop(target);
    drop(reused_key);
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn restore_rejects_reusing_the_source_database_login() {
    use harry_potter_server::lifecycle::RestoreError;
    let fixture = Fixture::new().await;
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    LifecycleWorker::new(fixture.database.clone(), ledger.clone(), [7; 32])
        .tick()
        .await
        .unwrap();
    let target = AppState::new(fixture.database.clone())
        .with_session_token_key([32; 32])
        .with_deployment_epoch(Uuid::new_v4());
    assert_eq!(
        reconcile_restore(
            &target,
            &ledger,
            [7; 32],
            database_now_ms(&fixture.database).await
        )
        .await,
        Err(RestoreError::Deployment)
    );
    drop(target);
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn a_recent_claim_cannot_disguise_an_old_backup_watermark() {
    use harry_potter_server::lifecycle::RestoreError;
    let fixture = Fixture::new().await;
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    LifecycleWorker::new(fixture.database.clone(), ledger.clone(), [7; 32])
        .tick()
        .await
        .unwrap();
    sqlx::query(
        "UPDATE runtime_deployment SET checkpoint_at = clock_timestamp() - INTERVAL '8 days'",
    )
    .execute(&fixture.database)
    .await
    .unwrap();
    let target = fixture.target();
    assert_eq!(
        reconcile_restore(
            &target,
            &ledger,
            [7; 32],
            database_now_ms(&fixture.database).await
        )
        .await,
        Err(RestoreError::Window)
    );
    drop(target);
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}

async fn resume_with_fresh_access(
    target: &AppState,
    grant: &harry_potter_server::lifecycle::RestoredAccess,
) {
    let recovered = build_router(target.clone())
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": grant.recovery_token,
                "recovery_password": grant.recovery_password,
                "recovery_attempt_id": Uuid::new_v4().to_string()
            }),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(recovered.status(), StatusCode::OK);
    let cookie = session_cookie(&recovered);
    let resumed = build_router(target.clone())
        .oneshot(command_request(&cookie, Uuid::new_v4(), 2))
        .await
        .unwrap();
    assert_eq!(
        resumed.status(),
        StatusCode::OK,
        "fresh access resumes the surviving game"
    );
}

#[tokio::test]
async fn explicit_source_adoption_upgrades_v22_without_revoking_existing_participation() {
    assert_source_adoption_preserves_access(22).await;
}

#[tokio::test]
async fn explicit_source_adoption_upgrades_v23_without_revoking_existing_participation() {
    assert_source_adoption_preserves_access(23).await;
}

async fn assert_source_adoption_preserves_access(version: i64) {
    use harry_potter_server::lifecycle::adopt_existing_source;
    let fixture = Fixture::at_version(Some(version)).await;
    let mut tx = fixture.database.begin().await.unwrap();
    let identity = Uuid::new_v4();
    let room = Uuid::new_v4();
    let participant = Uuid::new_v4();
    let session = Uuid::new_v4();
    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(identity)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO rooms (id, code, host_participant_id, recovery_password_hash) VALUES ($1, 'SAFE2345', $2, '$argon2id$legacy-fixture')")
        .bind(room).bind(participant).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO participants (id, room_id, guest_identity_id, display_name, role, position) VALUES ($1, $2, $3, 'Legacy fixture host', 'host', 1)")
        .bind(participant).bind(room).bind(identity).execute(&mut *tx).await.unwrap();
    let token = "a".repeat(64);
    sqlx::query("INSERT INTO guest_sessions (id, guest_identity_id, token_digest) VALUES ($1, $2, 'sha256:' || encode(sha256(convert_to($3, 'UTF8')), 'hex'))")
        .bind(session).bind(identity).bind(&token).execute(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO device_sessions (id, guest_session_id, participant_id) VALUES ($1, $2, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(session)
    .bind(participant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(
        initialize(&fixture.source).await.is_err(),
        "upgrade requires explicit source adoption"
    );
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    adopt_existing_source(&fixture.source, &ledger, [7; 32])
        .await
        .unwrap();
    initialize(&fixture.source).await.unwrap();
    let response = build_router(fixture.source.clone())
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, format!("__Host-session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let epoch: i64 = sqlx::query_scalar("SELECT recovery_epoch FROM rooms")
        .fetch_one(&fixture.database)
        .await
        .unwrap();
    assert_eq!(epoch, 1);
    assert!(
        adopt_existing_source(&fixture.target(), &ledger, [7; 32])
            .await
            .is_err()
    );
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}

struct PausedLedger {
    ledger: FileLedger,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl TombstoneLedger for PausedLedger {
    async fn lookup(&self, key: &str) -> Result<Option<PurgeProof>, PurgeError> {
        self.ledger.lookup(key).await
    }
    async fn record(&self, key: &str, proof: &PurgeProof) -> Result<(), PurgeError> {
        self.entered.notify_one();
        self.release.notified().await;
        self.ledger.record(key, proof).await
    }
}

#[tokio::test]
async fn cancellation_keeps_traffic_closed_and_releases_the_coordinator_for_retry() {
    let fixture = Fixture::new().await;
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    LifecycleWorker::new(fixture.database.clone(), ledger.clone(), [7; 32])
        .tick()
        .await
        .unwrap();
    let paused = std::sync::Arc::new(PausedLedger {
        ledger: ledger.clone(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let point = database_now_ms(&fixture.database).await;
    let target = fixture.target();
    target.mark_started();
    let coordinator = tokio::spawn({
        let target = target.clone();
        let paused = paused.clone();
        async move { reconcile_restore(&target, paused.as_ref(), [7; 32], point).await }
    });
    tokio::time::timeout(Duration::from_secs(10), paused.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        status(&target, "/health/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        status(&fixture.source, "/api/session").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        reconcile_restore(&target, &ledger, [7; 32], point).await,
        Err(RestoreError::Busy)
    );
    coordinator.abort();
    assert!(coordinator.await.unwrap_err().is_cancelled());
    assert_eq!(
        status(&target, "/health/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    reconcile_restore(&target, &ledger, [7; 32], point)
        .await
        .unwrap();
    initialize(&target).await.unwrap();
    assert_eq!(status(&target, "/health/ready").await, StatusCode::OK);
    target.begin_shutdown();
    drop(target);
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn a_game_expiring_during_reconciliation_is_gated_before_readiness() {
    let fixture = Fixture::new().await;
    let room = ready_room_in_app(
        build_router(fixture.source.clone()),
        fixture.database.clone(),
        fixture.source.clone(),
        playable_manifest(),
    )
    .await;
    start_ready_game(&room, "restore-mid-expiration").await;
    fixture.source.begin_shutdown();
    let directory = std::env::temp_dir().join(&fixture.schema);
    let ledger = FileLedger::new(directory.clone());
    LifecycleWorker::new(fixture.database.clone(), ledger.clone(), [7; 32])
        .tick()
        .await
        .unwrap();
    let deadline: i64 = sqlx::query_scalar("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '7 days' + INTERVAL '5 seconds', expires_at = clock_timestamp() + INTERVAL '5 seconds' RETURNING (extract(epoch FROM expires_at) * 1000)::BIGINT")
        .fetch_one(&fixture.database).await.unwrap();
    let paused = std::sync::Arc::new(PausedLedger {
        ledger,
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let target = fixture.target();
    let point = database_now_ms(&fixture.database).await;
    let coordinator = tokio::spawn({
        let target = target.clone();
        let paused = paused.clone();
        async move { reconcile_restore(&target, paused.as_ref(), [7; 32], point).await }
    });
    tokio::time::timeout(Duration::from_secs(10), paused.entered.notified())
        .await
        .unwrap();
    assert!(
        database_now_ms(&fixture.database).await < deadline,
        "reconciliation must reach the ledger while the game is live"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut poll = tokio::time::interval(Duration::from_millis(20));
        while database_now_ms(&fixture.database).await <= deadline {
            poll.tick().await;
        }
    })
    .await
    .unwrap();
    paused.release.notify_one();
    let access = coordinator.await.unwrap().unwrap();
    assert!(
        access.is_empty(),
        "a deadline crossed during reconciliation must not issue access"
    );
    let expired: bool = sqlx::query_scalar("SELECT access_expired_at IS NOT NULL FROM games")
        .fetch_one(&fixture.database)
        .await
        .unwrap();
    assert!(
        expired,
        "expiration must be durable before startup can open readiness"
    );
    initialize(&target).await.unwrap();
    assert_eq!(status(&target, "/health/ready").await, StatusCode::OK);
    target.begin_shutdown();
    drop(target);
    drop(room);
    fixture.cleanup().await;
    std::fs::remove_dir_all(directory).unwrap();
}
