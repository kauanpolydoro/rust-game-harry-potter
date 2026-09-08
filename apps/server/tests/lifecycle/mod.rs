use super::*;
use harry_potter_server::lifecycle::{FileLedger, LifecycleWorker};
use sqlx::{AssertSqlSafe, postgres::PgConnectOptions};
use std::{path::PathBuf, str::FromStr};
use uuid::Uuid;

struct Fixture {
    room: ReadyRoom,
    schema: String,
    ledger_dir: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let admin = database().await;
        let schema = format!("purge_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let options = PgConnectOptions::from_str(&std::env::var("TEST_DATABASE_URL").unwrap())
            .unwrap()
            .options([("search_path", schema.clone())]);
        let database = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .unwrap();
        let manifest = playable_manifest();
        let state = AppState::with_content_manifests(database.clone(), vec![manifest.clone()]);
        initialize(&state).await.unwrap();
        let room = ready_room_in_app(build_router(state.clone()), database, state, manifest).await;
        start_ready_game(&room, "purge").await;
        accepted_request(&room, command_request(&room.host_cookie, Uuid::new_v4(), 1)).await;
        let ledger_dir = std::env::temp_dir().join(&schema);
        Self {
            room,
            schema,
            ledger_dir,
        }
    }

    fn worker(&self) -> LifecycleWorker<FileLedger> {
        LifecycleWorker::new(
            self.room.database.clone(),
            FileLedger::new(self.ledger_dir.clone()),
            [7; 32],
        )
    }

    async fn expire(&self) {
        sqlx::query("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '8 days', expires_at = clock_timestamp() - INTERVAL '1 second' WHERE room_id = (SELECT id FROM rooms WHERE code = $1)")
            .bind(&self.room.room_code).execute(&self.room.database).await.unwrap();
    }

    async fn cleanup(self) {
        self.room.state.begin_shutdown();
        drop(self.room);
        let admin = database().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .unwrap();
        admin.close().await;
        if self.ledger_dir.exists() {
            std::fs::remove_dir_all(self.ledger_dir).unwrap();
        }
    }
}

#[tokio::test]
async fn purge_removes_operational_data_and_leaves_only_opaque_external_proof() {
    let fixture = Fixture::new().await;
    let worker = fixture.worker();
    assert_eq!(
        worker.tick().await.unwrap(),
        0,
        "live games must remain intact"
    );
    fixture.expire().await;
    for _ in 0..8 {
        worker.tick().await.unwrap();
    }
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM games")
        .fetch_one(&fixture.room.database)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    for table in [
        "rooms",
        "participants",
        "guest_sessions",
        "device_sessions",
        "guest_identities",
        "recovery_credentials",
        "game_events",
        "game_command_receipts",
        "game_state_anchors",
        "room_creation_requests",
        "room_join_requests",
        "game_start_requests",
        "lifecycle_purge_jobs",
    ] {
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&fixture.room.database)
            .await
            .unwrap();
        assert_eq!(count, 0, "operational copy remains in {table}");
    }
    let response = fixture
        .room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, &fixture.room.host_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let proofs = std::fs::read_dir(&fixture.ledger_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        proofs.len(),
        2,
        "tombstone and completion are durable outside PostgreSQL"
    );
    for proof in proofs {
        let bytes = std::fs::read_to_string(proof.path()).unwrap();
        assert!(!bytes.contains(&fixture.room.room_code));
        assert!(!bytes.contains("Minerva"));
        assert!(!bytes.contains(&fixture.room.host_recovery_token));
    }
    let metrics = worker.metrics().await.unwrap();
    assert_eq!(metrics.pending, 0);
    assert_eq!(metrics.completed, 1);
    assert!(metrics.detection_p95_seconds < 300.0);
    assert!(metrics.purge_max_seconds < 86400.0);
    fixture.cleanup().await;
}

use harry_potter_server::lifecycle::{PurgeError, PurgeProof, TombstoneLedger};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Clone)]
struct InterruptedLedger {
    file: FileLedger,
    lose_ack: Arc<AtomicBool>,
    pause_completion: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl InterruptedLedger {
    fn new(fixture: &Fixture) -> Self {
        Self {
            file: FileLedger::new(fixture.ledger_dir.clone()),
            lose_ack: Arc::new(AtomicBool::new(false)),
            pause_completion: Arc::new(AtomicBool::new(false)),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }
}
impl TombstoneLedger for InterruptedLedger {
    async fn record(&self, key: &str, proof: &PurgeProof) -> Result<(), PurgeError> {
        if proof.completed_at_ms.is_some() && self.pause_completion.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.file.record(key, proof).await?;
        if self.lose_ack.load(Ordering::SeqCst) {
            return Err(PurgeError::Ledger);
        }
        Ok(())
    }
}

async fn retry_now(fixture: &Fixture) {
    sqlx::query("UPDATE lifecycle_purge_jobs SET next_attempt_at = clock_timestamp()")
        .execute(&fixture.room.database)
        .await
        .unwrap();
}

#[tokio::test]
async fn lost_ledger_ack_keeps_the_root_and_a_fresh_worker_resumes_idempotently() {
    let fixture = Fixture::new().await;
    fixture.expire().await;
    let ledger = InterruptedLedger::new(&fixture);
    ledger.lose_ack.store(true, Ordering::SeqCst);
    let worker = LifecycleWorker::new(fixture.room.database.clone(), ledger.clone(), [7; 32]);
    assert_eq!(worker.tick().await, Err(PurgeError::Ledger));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM games")
            .fetch_one(&fixture.room.database)
            .await
            .unwrap(),
        1
    );
    assert_eq!(std::fs::read_dir(&fixture.ledger_dir).unwrap().count(), 1);
    assert_eq!(worker.metrics().await.unwrap().completed, 0);
    drop(worker);
    ledger.lose_ack.store(false, Ordering::SeqCst);
    retry_now(&fixture).await;
    let restarted = LifecycleWorker::new(fixture.room.database.clone(), ledger, [7; 32]);
    restarted.tick().await.unwrap();
    restarted.tick().await.unwrap();
    assert_eq!(restarted.metrics().await.unwrap().completed, 1);
    assert_eq!(std::fs::read_dir(&fixture.ledger_dir).unwrap().count(), 2);
    fixture.cleanup().await;
}

#[tokio::test]
async fn verifier_blocks_completion_for_an_orphan_after_root_removal_and_process_loss() {
    let fixture = Fixture::new().await;
    let identity: Uuid = sqlx::query_scalar("SELECT id FROM guest_identities LIMIT 1")
        .fetch_one(&fixture.room.database)
        .await
        .unwrap();
    fixture.expire().await;
    let ledger = InterruptedLedger::new(&fixture);
    ledger.pause_completion.store(true, Ordering::SeqCst);
    let worker = LifecycleWorker::new(fixture.room.database.clone(), ledger.clone(), [7; 32]);
    let process = tokio::spawn(async move { worker.tick().await });
    tokio::time::timeout(Duration::from_secs(10), ledger.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM games")
            .fetch_one(&fixture.room.database)
            .await
            .unwrap(),
        0
    );
    process.abort();
    assert!(process.await.unwrap_err().is_cancelled());
    // Emulate a detached legacy copy surviving outside the root's FK graph.
    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(identity)
        .execute(&fixture.room.database)
        .await
        .unwrap();
    let restarted = fixture.worker();
    assert_eq!(restarted.tick().await, Err(PurgeError::Orphans));
    assert_eq!(restarted.metrics().await.unwrap().completed, 0);
    assert_eq!(std::fs::read_dir(&fixture.ledger_dir).unwrap().count(), 1);
    sqlx::query("DELETE FROM guest_identities WHERE id = $1")
        .bind(identity)
        .execute(&fixture.room.database)
        .await
        .unwrap();
    retry_now(&fixture).await;
    restarted.tick().await.unwrap();
    assert_eq!(restarted.metrics().await.unwrap().completed, 1);
    fixture.cleanup().await;
}

#[tokio::test]
async fn an_unregistered_blob_or_telemetry_store_prevents_false_completion() {
    let fixture = Fixture::new().await;
    sqlx::query("CREATE TABLE forgotten_blobs (owner UUID, private_bytes BYTEA)")
        .execute(&fixture.room.database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO forgotten_blobs SELECT id, decode('abcdef', 'hex') FROM games")
        .execute(&fixture.room.database)
        .await
        .unwrap();
    fixture.expire().await;
    let worker = fixture.worker();
    assert_eq!(worker.tick().await, Err(PurgeError::Orphans));
    assert_eq!(worker.metrics().await.unwrap().completed, 0);
    sqlx::query("DROP TABLE forgotten_blobs")
        .execute(&fixture.room.database)
        .await
        .unwrap();
    retry_now(&fixture).await;
    worker.tick().await.unwrap();
    assert_eq!(worker.metrics().await.unwrap().completed, 1);
    fixture.cleanup().await;
}

#[tokio::test]
async fn concurrent_workers_skip_locked_jobs_and_complete_exactly_once() {
    let fixture = Fixture::new().await;
    fixture.expire().await;
    let ledger = InterruptedLedger::new(&fixture);
    ledger.pause_completion.store(true, Ordering::SeqCst);
    let worker = LifecycleWorker::new(fixture.room.database.clone(), ledger.clone(), [7; 32]);
    let process = tokio::spawn(async move { worker.tick().await });
    tokio::time::timeout(Duration::from_secs(10), ledger.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), fixture.worker().tick())
            .await
            .unwrap()
            .unwrap(),
        0
    );
    ledger.release.notify_one();
    process.await.unwrap().unwrap();
    assert_eq!(fixture.worker().metrics().await.unwrap().completed, 1);
    fixture.cleanup().await;
}

#[tokio::test]
async fn metrics_include_stalled_and_undetected_games_instead_of_only_successes() {
    let fixture = Fixture::new().await;
    // Stop the API's expiration scanner so the overdue undetected queue is visible.
    fixture.room.state.begin_shutdown();
    tokio::time::sleep(Duration::from_millis(20)).await;
    sqlx::query("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '9 days', expires_at = clock_timestamp() - INTERVAL '25 hours'")
        .execute(&fixture.room.database).await.unwrap();
    let worker = fixture.worker();
    let metrics = worker.metrics().await.unwrap();
    assert_eq!(metrics.undetected, 1);
    assert_eq!(metrics.overdue, 1);
    assert!(metrics.detection_p95_seconds >= 90000.0);
    let ledger = InterruptedLedger::new(&fixture);
    ledger.lose_ack.store(true, Ordering::SeqCst);
    let worker = LifecycleWorker::new(fixture.room.database.clone(), ledger.clone(), [7; 32]);
    assert_eq!(worker.tick().await, Err(PurgeError::Ledger));
    let metrics = worker.metrics().await.unwrap();
    assert_eq!(metrics.pending, 1);
    assert_eq!(metrics.overdue, 1);
    assert_eq!(metrics.failed_attempts, 1);
    assert!(metrics.oldest_pending_seconds >= 90000.0);
    ledger.lose_ack.store(false, Ordering::SeqCst);
    retry_now(&fixture).await;
    worker.tick().await.unwrap();
    let metrics = worker.metrics().await.unwrap();
    assert_eq!(metrics.completed, 1);
    assert!(metrics.purge_max_seconds >= 90000.0);
    fixture.cleanup().await;
}

#[tokio::test]
async fn metrics_keep_api_detected_games_visible_while_the_purge_worker_is_down() {
    let fixture = Fixture::new().await;
    sqlx::query("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '9 days', expires_at = clock_timestamp() - INTERVAL '25 hours'")
        .execute(&fixture.room.database).await.unwrap();
    sqlx::query("SELECT expire_game_access(id) FROM games")
        .execute(&fixture.room.database)
        .await
        .unwrap();
    let metrics = fixture.worker().metrics().await.unwrap();
    assert_eq!(metrics.undetected, 0);
    assert_eq!(metrics.pending, 1);
    assert_eq!(metrics.overdue, 1);
    assert!(metrics.oldest_pending_seconds >= 90000.0);
    assert!(metrics.detection_p95_seconds >= 90000.0);
    fixture.cleanup().await;
}

#[tokio::test]
async fn purge_removes_consumed_credentials_security_history_and_open_connections() {
    let fixture = Fixture::new().await;
    let room = &fixture.room;
    populate_security_history(room).await;
    let tables = [
        "device_session_revocation_requests",
        "participant_protection_requests",
        "room_protection_requests",
        "recovery_password_rotation_requests",
        "recovery_credential_regeneration_requests",
        "identity_security_events",
        "identity_security_event_recipients",
        "recovery_credentials",
    ];
    for table in tables {
        assert!(
            sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&room.database)
                .await
                .unwrap()
                > 0,
            "fixture must populate {table}"
        );
    }
    let (address, server) = start_network_server(room.app.clone()).await;
    let mut socket = connect_current_game(address, &room.host_cookie).await;
    fixture.expire().await;
    let mut locked = room.database.begin().await.unwrap();
    sqlx::query("SELECT id FROM games FOR UPDATE")
        .execute(&mut *locked)
        .await
        .unwrap();
    assert_eq!(
        fixture.worker().tick().await.unwrap(),
        0,
        "a locked game is skipped until a later batch"
    );
    locked.commit().await.unwrap();
    let close = tokio::time::timeout(Duration::from_secs(5), socket.read_close_code())
        .await
        .unwrap();
    assert!(
        matches!(close, 4001 | 1008),
        "expired or already purged sessions must close"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        let worker = fixture.worker();
        while worker.metrics().await.unwrap().completed != 1 {
            worker.tick().await.unwrap();
        }
    })
    .await
    .expect("later batches must complete the purge after the root is unlocked");
    for table in tables.into_iter().chain([
        "game_realtime_connections",
        "guest_sessions",
        "guest_identities",
    ]) {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&room.database)
                .await
                .unwrap(),
            0,
            "copy remains in {table}"
        );
    }
    server.abort();
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "100-game lifecycle SLO profile; run with make check-lifecycle-profile"]
async fn lifecycle_profile_measures_detection_and_purge_for_one_hundred_games() {
    let fixture = Fixture::new().await;
    for batch in 0..25 {
        let mut tasks = JoinSet::new();
        for offset in 0..4 {
            if batch * 4 + offset == 0 {
                continue;
            }
            let room = &fixture.room;
            // Match the reconnect profile: distinct clients share the production
            // state and its admission limits while preparing independent games.
            let peer = std::net::SocketAddr::from((
                [192, 0, 2, u8::try_from(batch * 4 + offset + 1).unwrap()],
                12345,
            ));
            let (app, database, state, manifest) = (
                room.app
                    .clone()
                    .layer(axum::Extension(axum::extract::ConnectInfo(peer))),
                room.database.clone(),
                room.state.clone(),
                room.manifest.clone(),
            );
            tasks.spawn(async move {
                let room = ready_room_in_app(app, database, state, manifest).await;
                start_ready_game(&room, "purge-profile").await;
                accepted_request(&room, command_request(&room.host_cookie, Uuid::new_v4(), 1))
                    .await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    }
    sqlx::query("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '8 days', expires_at = clock_timestamp() - INTERVAL '1 second'")
        .execute(&fixture.room.database).await.unwrap();
    let first = fixture.worker();
    let second = fixture.worker();
    for _ in 0..20 {
        let (a, b) = tokio::join!(first.tick(), second.tick());
        a.unwrap();
        b.unwrap();
        if first.metrics().await.unwrap().completed == 100 {
            break;
        }
    }
    let metrics = first.metrics().await.unwrap();
    eprintln!(
        "lifecycle profile: {}",
        serde_json::to_string(&metrics).unwrap()
    );
    assert_eq!(metrics.pending, 0);
    assert_eq!(metrics.completed, 100);
    assert!(metrics.detection_p95_seconds < 300.0);
    assert!(metrics.purge_p95_seconds < 3600.0);
    assert!(metrics.purge_max_seconds < 86400.0);
    fixture.cleanup().await;
}

#[tokio::test]
async fn waiting_for_external_completion_does_not_block_a_command_in_another_game() {
    let fixture = Fixture::new().await;
    let other = ready_room_in_app(
        fixture.room.app.clone(),
        fixture.room.database.clone(),
        fixture.room.state.clone(),
        fixture.room.manifest.clone(),
    )
    .await;
    start_ready_game(&other, "unrelated-live-game").await;
    fixture.expire().await;
    let ledger = InterruptedLedger::new(&fixture);
    ledger.pause_completion.store(true, Ordering::SeqCst);
    let worker = LifecycleWorker::new(fixture.room.database.clone(), ledger.clone(), [7; 32]);
    let process = tokio::spawn(async move { worker.tick().await });
    tokio::time::timeout(Duration::from_secs(10), ledger.entered.notified())
        .await
        .unwrap();
    let command = tokio::time::timeout(
        Duration::from_secs(1),
        other
            .app
            .clone()
            .oneshot(command_request(&other.host_cookie, Uuid::new_v4(), 1)),
    )
    .await;
    ledger.release.notify_one();
    process.await.unwrap().unwrap();
    assert_eq!(
        command
            .expect("an unrelated game must not wait for the ledger")
            .unwrap()
            .status(),
        StatusCode::OK
    );
    fixture.cleanup().await;
}

#[tokio::test]
async fn pending_purge_age_breaches_the_one_hour_percentile_before_the_day_deadline() {
    let fixture = Fixture::new().await;
    sqlx::query("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '8 days', expires_at = clock_timestamp() - INTERVAL '2 hours', access_expired_at = clock_timestamp() - INTERVAL '2 hours' + INTERVAL '1 second'").execute(&fixture.room.database).await.unwrap();
    let metrics = fixture.worker().metrics().await.unwrap();
    assert!(metrics.detection_p95_seconds < 300.0);
    assert_eq!(metrics.overdue, 0);
    assert!(metrics.purge_p95_seconds >= 7200.0);
    fixture.cleanup().await;
}

#[derive(Clone)]
struct RendezvousLedger {
    file: FileLedger,
    writes: Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>>,
    before_purge: Arc<Barrier>,
}
impl TombstoneLedger for RendezvousLedger {
    async fn record(&self, key: &str, proof: &PurgeProof) -> Result<(), PurgeError> {
        let count = {
            let mut writes = self.writes.lock().unwrap();
            let count = writes.entry(key.to_owned()).or_default();
            *count += 1;
            *count
        };
        if count == 2 {
            self.before_purge.wait().await;
        }
        self.file.record(key, proof).await
    }
}

#[tokio::test]
async fn concurrent_purges_remove_the_last_copy_of_a_shared_identity() {
    let fixture = Fixture::new().await;
    let other = ready_room_in_app(
        fixture.room.app.clone(),
        fixture.room.database.clone(),
        fixture.room.state.clone(),
        fixture.room.manifest.clone(),
    )
    .await;
    start_ready_game(&other, "shared-identity").await;
    let shared: Uuid = sqlx::query_scalar("SELECT guest_identity_id FROM participants JOIN rooms ON rooms.id = participants.room_id WHERE rooms.code = $1 AND participants.role = 'host'")
        .bind(&fixture.room.room_code).fetch_one(&fixture.room.database).await.unwrap();
    let replaced: Uuid = sqlx::query_scalar("SELECT guest_identity_id FROM participants JOIN rooms ON rooms.id = participants.room_id WHERE rooms.code = $1 AND participants.role = 'host'")
        .bind(&other.room_code).fetch_one(&fixture.room.database).await.unwrap();
    // A legacy identity can be shared according to the persisted schema.
    sqlx::query("UPDATE participants SET guest_identity_id = $1 WHERE guest_identity_id = $2")
        .bind(shared)
        .bind(replaced)
        .execute(&fixture.room.database)
        .await
        .unwrap();
    sqlx::query("UPDATE guest_sessions SET guest_identity_id = $1 WHERE guest_identity_id = $2")
        .bind(shared)
        .bind(replaced)
        .execute(&fixture.room.database)
        .await
        .unwrap();
    sqlx::query("DELETE FROM guest_identities WHERE id = $1")
        .bind(replaced)
        .execute(&fixture.room.database)
        .await
        .unwrap();
    sqlx::query("UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '8 days', expires_at = clock_timestamp() - INTERVAL '1 second'").execute(&fixture.room.database).await.unwrap();
    let uncertain = InterruptedLedger::new(&fixture);
    uncertain.lose_ack.store(true, Ordering::SeqCst);
    assert_eq!(
        LifecycleWorker::new(fixture.room.database.clone(), uncertain, [7; 32])
            .tick()
            .await,
        Err(PurgeError::Ledger)
    );
    retry_now(&fixture).await;
    let ledger = RendezvousLedger {
        file: FileLedger::new(fixture.ledger_dir.clone()),
        writes: Arc::default(),
        before_purge: Arc::new(Barrier::new(2)),
    };
    let first = LifecycleWorker::new(fixture.room.database.clone(), ledger.clone(), [7; 32]);
    let second = LifecycleWorker::new(fixture.room.database.clone(), ledger, [7; 32]);
    let (a, b) = tokio::join!(first.tick(), second.tick());
    a.unwrap();
    b.unwrap();
    assert_eq!(first.metrics().await.unwrap().completed, 2);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM guest_identities")
            .fetch_one(&fixture.room.database)
            .await
            .unwrap(),
        0
    );
    fixture.cleanup().await;
}

#[tokio::test]
async fn slow_sql_is_cancelled_in_postgres_and_the_job_can_resume() {
    let fixture = Fixture::new().await;
    sqlx::raw_sql("CREATE FUNCTION slow_purge() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(12); RETURN OLD; END $$; CREATE TRIGGER slow_purge BEFORE DELETE ON game_events FOR EACH ROW EXECUTE FUNCTION slow_purge();")
        .execute(&fixture.room.database).await.unwrap();
    fixture.expire().await;
    let started = Instant::now();
    assert_eq!(fixture.worker().tick().await, Err(PurgeError::Database));
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "server-side work must be cancelled too"
    );
    sqlx::query("DROP TRIGGER slow_purge ON game_events")
        .execute(&fixture.room.database)
        .await
        .unwrap();
    retry_now(&fixture).await;
    fixture.worker().tick().await.unwrap();
    assert_eq!(fixture.worker().metrics().await.unwrap().completed, 1);
    fixture.cleanup().await;
}

async fn populate_security_history(room: &ReadyRoom) {
    let recovered = room.app.clone().oneshot(json_request("POST", "/api/session/recover", &json!({
        "recovery_token": room.host_recovery_token, "recovery_password": "a long uncommon passphrase",
        "recovery_attempt_id": Uuid::new_v4().to_string()
    }), None, None)).await.unwrap();
    assert_eq!(recovered.status(), StatusCode::OK);
    let sessions = response_json(
        room.app
            .clone()
            .oneshot(list_device_sessions_request(&room.host_cookie))
            .await
            .unwrap(),
    )
    .await;
    let second_id = sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["current"] == false)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        room.app
            .clone()
            .oneshot(revoke_device_session_request(
                &room.host_cookie,
                second_id,
                &unique_key("purge-revoke")
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for (method, path, body, cookie) in [
        (
            "PUT",
            "/api/session/recovery-password",
            json!({"current_recovery_password": "a long uncommon passphrase", "new_recovery_password": "a newer uncommon recovery phrase"}),
            &room.host_cookie,
        ),
        (
            "POST",
            "/api/session/recovery-credential",
            json!({}),
            &room.guest_cookie,
        ),
        (
            "PUT",
            "/api/session/protection",
            json!({"protection_confirmed": true}),
            &room.guest_cookie,
        ),
        (
            "PUT",
            "/api/rooms/current/protection",
            json!({"current_recovery_password": "a newer uncommon recovery phrase", "new_recovery_password": "the final uncommon recovery phrase", "preserve_current_session": true, "protection_confirmed": true}),
            &room.host_cookie,
        ),
    ] {
        let response = room
            .app
            .clone()
            .oneshot(json_request(
                method,
                path,
                &body,
                Some(cookie),
                Some(&unique_key("purge-security")),
            ))
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(
            status,
            StatusCode::OK,
            "{path}: {}",
            response_json(response).await
        );
    }
}
