use super::*;
use uuid::Uuid;

#[tokio::test]
async fn solicited_delivery_receipts_do_not_consume_the_unsolicited_message_budget() {
    assert_receipt_preserves_budget(Duration::ZERO).await;
}

#[tokio::test]
async fn late_delivery_receipts_remain_recognizable_after_the_measurement_deadline() {
    assert_receipt_preserves_budget(Duration::from_secs(6)).await;
}

async fn assert_receipt_preserves_budget(delay: Duration) {
    let room = ready_room().await;
    let projection = start_ready_game(&room, "observed-receipt-budget").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let (_, _, mut socket) = websocket_handshake(
        address,
        &realtime_path(&projection),
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    socket.read_text().await;
    for index in 0..60_u8 {
        socket.send_frame(9, &[index]).await;
        let (opcode, payload) = socket.read_frame().await;
        assert_eq!((opcode, payload), (10, vec![index]));
    }
    let command = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, Uuid::new_v4(), 1))
        .await
        .unwrap();
    assert_eq!(command.status(), StatusCode::OK);
    socket.read_text().await;
    let (opcode, nonce) = socket.read_frame().await;
    assert_eq!(opcode, 9);
    tokio::time::sleep(delay).await;
    let second = room
        .app
        .clone()
        .oneshot(command_request(&room.guest_cookie, Uuid::new_v4(), 2))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    socket.read_text().await;
    socket.send_frame(10, &nonce).await;
    let third = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, Uuid::new_v4(), 3))
        .await
        .unwrap();
    assert_eq!(third.status(), StatusCode::OK);
    let next: Value = serde_json::from_str(&socket.read_text().await).unwrap();
    assert_eq!(
        next["type"], "events",
        "the solicited receipt must preserve the authorized game connection"
    );
    server.abort();
}

#[tokio::test]
async fn recovery_counts_host_assistance_once_and_the_successor_is_self_service() {
    let logs = log_capture::LogCapture::start();
    let room = ready_room().await;
    let issued = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/rooms/current/participants/2/recovery-credential",
            &json!({"host_assistance_risk_acknowledged": true}),
            Some(&room.host_cookie),
            Some(&Uuid::new_v4().to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(issued.status(), StatusCode::OK);
    let issued = response_json(issued).await;
    let attempt = Uuid::new_v4();
    let recovery = json!({"recovery_token": issued["recovery_token"], "recovery_password": "a long uncommon passphrase", "recovery_attempt_id": attempt.to_string()});
    let recovered = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &recovery,
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(recovered.status(), StatusCode::OK);
    assert_eq!(recovery_outcomes(&logs, &recovered), ["host_assisted"]);
    let recovered = response_json(recovered).await;
    let retry = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &recovery,
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    assert!(
        recovery_outcomes(&logs, &retry).is_empty(),
        "a retry must not inflate completed human recoveries"
    );
    let mut successor = json!({"recovery_token": recovered["recovery_token"], "recovery_password": "a long uncommon passphrase", "recovery_attempt_id": Uuid::new_v4().to_string()});
    let replacement = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &successor,
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(replacement.status(), StatusCode::CONFLICT);
    successor["replace_session_id"] = response_json(replacement).await["sessions"][0]["id"].clone();
    let second = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &successor,
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(recovery_outcomes(&logs, &second), ["self_service"]);
}

fn recovery_outcomes(logs: &log_capture::LogCapture, response: &Response<Body>) -> Vec<String> {
    let correlation = response.headers()["x-correlation-id"].to_str().unwrap();
    logs.text()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|sample| {
            sample["metric"] == "recovery_completions" && sample["correlation_id"] == correlation
        })
        .map(|sample| sample["outcome"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn realtime_distinguishes_synchronization_writes_and_remote_transport_receipt() {
    let logs = log_capture::LogCapture::start();
    let room = ready_room().await;
    let projection = start_ready_game(&room, "observed-realtime").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let (status, _, mut socket) = websocket_handshake(
        address,
        &realtime_path(&projection),
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    socket.read_text().await;
    let response = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, Uuid::new_v4(), 1))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let correlation = response.headers()["x-correlation-id"].to_str().unwrap();
    let event: Value = serde_json::from_str(&socket.read_text().await).unwrap();
    assert_eq!(event["type"], "events");
    let (opcode, nonce) = tokio::time::timeout(Duration::from_secs(2), socket.read_frame())
        .await
        .expect("a committed event must be followed by a bounded receipt probe");
    assert_eq!(opcode, 9);
    assert_eq!(nonce.len(), 16);
    // The client can receive the event and still withhold its transport receipt.
    tokio::time::sleep(Duration::from_millis(60)).await;
    socket.send_frame(10, &nonce).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if logs.text().lines().any(|line| {
                serde_json::from_str::<Value>(line).unwrap()["metric"]
                    == "delivery_round_trip_seconds"
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let output = logs.text();
    let events: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events
            .iter()
            .any(|sample| sample["metric"] == "delivery_round_trip_seconds"
                && sample["value"].as_f64().unwrap() >= 0.06)
    );
    for metric in [
        "socket_write_seconds",
        "realtime_sync_seconds",
        "delivery_events",
    ] {
        assert!(
            events.iter().any(|sample| sample["metric"] == metric),
            "missing {metric}"
        );
    }
    let trace = events
        .iter()
        .find(|sample| {
            sample["correlation_id"] == correlation && sample["metric"] == "commit_seconds"
        })
        .unwrap()["command_trace"]
        .clone();
    assert!(trace.is_string());
    assert!(
        events
            .iter()
            .any(|sample| sample["metric"] == "delivery_events"
                && sample["outcome"] == "success"
                && sample["command_trace"] == trace)
    );
    let tracking: String = sqlx::query_scalar("SHOW track_commit_timestamp")
        .fetch_one(&room.database)
        .await
        .unwrap();
    if tracking == "on" {
        assert!(events.iter().any(|sample| sample["metric"]
            == "commit_to_receipt_upper_bound_seconds"
            && sample["value"].as_f64().unwrap() >= 0.06));
    } else {
        assert!(
            events
                .iter()
                .any(|sample| sample["metric"] == "commit_time_observations"
                    && sample["outcome"] == "unavailable")
        );
    }
    assert!(!output.contains(&room.host_cookie));
    server.abort();
}

#[tokio::test]
async fn persisted_divergence_blocks_the_command_and_emits_a_sanitized_p0() {
    let logs = log_capture::LogCapture::start();
    let room = ready_room().await;
    start_ready_game(&room, "observed-divergence").await;
    // Simulate damaged storage using a privilege restricted to the test database.
    // The bypass applies only to this transaction, never to other connections.
    let mut corruption = room.database.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *corruption)
        .await
        .unwrap();
    sqlx::query("UPDATE games SET state_digest = 'blake3:0000000000000000000000000000000000000000000000000000000000000000' WHERE room_id = (SELECT id FROM rooms WHERE code = $1)")
        .bind(&room.room_code).execute(&mut *corruption).await.unwrap();
    corruption.commit().await.unwrap();
    let response = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, Uuid::new_v4(), 1))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        logs.text()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|event| event["metric"] == "p0_state_divergence")
    );
}

#[tokio::test]
async fn a_structurally_corrupt_persisted_snapshot_is_an_integrity_incident() {
    let logs = log_capture::LogCapture::start();
    let room = ready_room().await;
    start_ready_game(&room, "observed-structural-divergence").await;
    let mut corruption = room.database.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *corruption)
        .await
        .unwrap();
    sqlx::query("UPDATE games SET snapshot = snapshot - 'versions' WHERE room_id = (SELECT id FROM rooms WHERE code = $1)").bind(&room.room_code).execute(&mut *corruption).await.unwrap();
    corruption.commit().await.unwrap();
    let response = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, Uuid::new_v4(), 1))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let correlation = response.headers()["x-correlation-id"].to_str().unwrap();
    assert!(logs.text().lines().any(|line| {
        let sample: Value = serde_json::from_str(line).unwrap();
        sample["correlation_id"] == correlation && sample["metric"] == "p0_state_divergence"
    }));
}

#[tokio::test]
async fn recovery_measures_argon_queue_and_execution_without_exporting_credentials() {
    let logs = log_capture::LogCapture::start();
    let room = ready_room().await;
    let response = room
        .app
        .clone()
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
    let correlation = response.headers()["x-correlation-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response_json(response).await
    );
    let output = logs.text();
    assert!(!output.contains(&room.host_recovery_token));
    assert!(!output.contains("a long uncommon passphrase"));
    let events: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for metric in [
        "queue_wait_seconds",
        "password_seconds",
        "pool_wait_seconds",
        "lock_seconds",
        "commit_seconds",
    ] {
        assert!(
            events.iter().any(|event| event["metric"] == metric
                && event["correlation_id"] == correlation
                && event["operation"] == "recovery"
                && event["outcome"] == "success"),
            "missing {metric} in recovery"
        );
    }
}

#[tokio::test]
async fn accepted_command_exports_distinct_phases_without_private_dimensions() {
    let logs = log_capture::LogCapture::start();
    let room = ready_room().await;
    start_ready_game(&room, "observed-command").await;
    let command_id = Uuid::new_v4();
    let response = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, command_id, 1))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let correlation = response.headers()["x-correlation-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let projection = response_json(response).await;
    assert_eq!(projection["projection"]["snapshot"]["state_version"], 2);
    let output = logs.text();
    assert!(!output.contains(&room.host_cookie));
    assert!(!output.contains(&command_id.to_string()));
    let events: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|event| event["correlation_id"] == correlation)
        .collect();
    for metric in [
        "pool_wait_seconds",
        "lock_seconds",
        "rule_seconds",
        "sql_seconds",
        "commit_seconds",
        "response_prepare_seconds",
    ] {
        let sample = events
            .iter()
            .find(|event| {
                event["metric"] == metric
                    && event["operation"] == "command"
                    && event["outcome"] == "success"
            })
            .unwrap_or_else(|| panic!("missing successful {metric}"));
        assert!(sample["value"].as_f64().unwrap() >= 0.0);
    }
    assert!(
        !events.iter().any(|event| event["metric"]
            .as_str()
            .is_some_and(|name| name.starts_with("p0_"))),
        "a normal command must not produce an integrity incident"
    );
}
