use super::game_one::ready_adventure_room;
use super::*;

#[tokio::test]
async fn game_four_starts_through_http_and_sql_rejects_forged_randomness_history() {
    let (room, _) = ready_adventure_room(2, harry_potter_server::game_four_manifest()).await;
    let projection = start_ready_game(&room, "game-four-start").await;
    assert_eq!(projection["snapshot"]["snapshot_version"], 8);
    assert_eq!(
        projection["snapshot"]["versions"]["content"],
        "game-four-en-v1"
    );
    let snapshot = current_game_snapshot(&room).await;
    assert_eq!(
        snapshot["preparation_samples"]
            .as_array()
            .expect("samples")
            .len(),
        133
    );
    assert_eq!(snapshot["adventure_id"], "adventure:004");
    assert!(snapshot["house_die_rolls"].is_array());
    for mutation in [
        "missing",
        "legacy_codec",
        "fake_roll",
        "missing_fleur_state",
    ] {
        let mut forged = snapshot.clone();
        match mutation {
            "missing" => { forged.as_object_mut().expect("snapshot").remove("house_die_rolls"); },
            "legacy_codec" => forged["snapshot_version"] = json!(7),
            "fake_roll" => forged["house_die_rolls"].as_array_mut().expect("history").push(json!({"purpose":"rule:forged", "counter":0, "die":"gryffindor_v1", "sides":6, "result":1})),
            "missing_fleur_state" => { forged["effects"]["entities"].as_array_mut().expect("entities").iter_mut().find(|entity| entity["catalog_id"] == "hogwarts-card:035").expect("Fleur").as_object_mut().expect("entity").remove("turn_state"); },
            _ => unreachable!(),
        }
        let valid: bool = sqlx::query_scalar("SELECT valid_game_snapshot_v8($1)")
            .bind(&forged)
            .fetch_one(&room.database)
            .await
            .expect("SQL validation");
        assert!(!valid, "database rejects {mutation}");
    }
}

#[tokio::test]
async fn game_four_with_two_three_and_four_heroes_persists_each_turn_until_defeat() {
    for count in [2, 3, 4] {
        let (room, cookies) =
            ready_adventure_room(count, harry_potter_server::game_four_manifest()).await;
        let projection = super::game_one::play_game_one(&room, &cookies, false).await;
        assert_eq!(projection["game"]["status"], "lost", "{count} Heroes");
        let snapshot = current_game_snapshot(&room).await;
        assert!(
            !snapshot["house_die_rolls"]
                .as_array()
                .expect("history")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn a_house_roll_survives_a_lost_response_restart_retry_and_websocket_reconnect() {
    let (room, cookies) = ready_adventure_room(2, harry_potter_server::game_four_manifest()).await;
    let (command, cookie, snapshot, before) = commit_until_house_discard(&room, &cookies).await;
    assert_sql_roll_evidence(&room).await;
    let restarted_state =
        AppState::with_content_manifests(room.database.clone(), vec![room.manifest.clone()])
            .with_game_seed_source(|| panic!("a restarted game must retain its original seed"));
    initialize(&restarted_state).await.expect("restart");
    let restarted = build_router(restarted_state);
    let request = || {
        json_request(
            "POST",
            "/api/games/current/commands",
            &command,
            Some(&cookie),
            None,
        )
    };
    let response = restarted
        .clone()
        .oneshot(request())
        .await
        .expect("retry after restart");
    assert_eq!(response.status(), StatusCode::OK);
    let retry = response_json(response).await;
    let repeated = restarted
        .clone()
        .oneshot(request())
        .await
        .expect("identical retry");
    assert_eq!(repeated.status(), StatusCode::OK);
    assert_eq!(response_json(repeated).await["receipt"], retry["receipt"]);
    assert_eq!(
        retry["projection"]["snapshot"]["sequence"],
        snapshot["sequence"]
    );
    assert_eq!(current_game_snapshot(&room).await, snapshot);

    let (address, server) = start_network_server(restarted).await;
    let (status, _, mut socket) = websocket_handshake(
        address,
        &realtime_path(&before),
        Some(&cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v2"),
    )
    .await;
    assert_eq!(status, 101);
    let batch = tokio::time::timeout(Duration::from_secs(5), socket.read_text())
        .await
        .expect("replayed event");
    let batch: Value = serde_json::from_str(&batch).expect("event batch");
    assert_eq!(batch["type"], "events");
    assert_eq!(batch["events"].as_array().expect("events").len(), 1);
    assert_eq!(batch["events"][0]["event_version"], 6);
    assert!(
        batch["events"][0]["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .flat_map(|step| step["effects"].as_array().expect("effects"))
            .any(|effect| effect["type"] == "die_rolled"
                && effect["die"] == "d6"
                && effect["result"] == 3)
    );
    drop(socket);
    let (status, _, mut socket) = websocket_handshake(
        address,
        "/api/games/current/events",
        Some(&cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v2"),
    )
    .await;
    assert_eq!(status, 101);
    let restored = tokio::time::timeout(Duration::from_secs(5), socket.read_text())
        .await
        .expect("restored snapshot");
    let restored: Value = serde_json::from_str(&restored).expect("snapshot message");
    assert_eq!(restored["type"], "snapshot");
    assert_eq!(
        restored["projection"]["choice"],
        retry["projection"]["choice"]
    );
    assert_eq!(
        restored["projection"]["snapshot"]["digest"],
        retry["projection"]["snapshot"]["digest"]
    );
    assert_eq!(current_game_snapshot(&room).await, snapshot);
    server.abort();
}

async fn assert_sql_roll_evidence(room: &ReadyRoom) {
    let payload: Value = sqlx::query_scalar("SELECT events.payload FROM game_events AS events JOIN games ON games.id = events.game_id JOIN rooms ON rooms.id = games.room_id WHERE rooms.code = $1 ORDER BY events.sequence DESC LIMIT 1")
        .bind(&room.room_code).fetch_one(&room.database).await.expect("committed event");
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT valid_event_house_die_rolls_v9($1)")
            .bind(&payload)
            .fetch_one(&room.database)
            .await
            .expect("SQL audit")
    );
    for (field, value) in [
        ("purpose", json!("rule:forged")),
        ("counter", json!(0)),
        ("result", json!(1)),
        ("sides", json!(8)),
    ] {
        let mut forged = payload.clone();
        forged["house_die_rolls"][0][field] = value;
        assert!(
            !sqlx::query_scalar::<_, bool>("SELECT valid_event_house_die_rolls_v9($1)")
                .bind(&forged)
                .fetch_one(&room.database)
                .await
                .expect("SQL rejects forged evidence"),
            "{field}"
        );
    }
}

async fn commit_until_house_discard(
    room: &ReadyRoom,
    cookies: &[String],
) -> (Value, String, Value, Value) {
    let mut projection = start_ready_game(room, "game-four-reconnect").await;
    for _ in 0..40 {
        let position = if projection["choice"]["status"] == "pending" {
            &projection["choice"]["responsible_position"]
        } else {
            &projection["turn"]["active_position"]
        };
        let cookie =
            &cookies[usize::try_from(position.as_u64().expect("position")).expect("position") - 1];
        let request = if projection["choice"]["status"] == "pending" {
            json!({"type":"resolve_choice", "choice_id":projection["choice"]["id"], "selected_options":projection["choice"]["options"].as_array().expect("options").iter().take(usize::try_from(projection["choice"]["min"].as_u64().expect("min")).expect("min")).cloned().collect::<Vec<_>>()})
        } else {
            json!({"type":"end_hero_actions"})
        };
        let command = json!({"command_id":uuid::Uuid::new_v4().to_string(), "expected_state_version":projection["snapshot"]["state_version"], "type":request["type"], "choice_id":request.get("choice_id"), "selected_options":request.get("selected_options")});
        let mut command = command;
        if request["type"] == "end_hero_actions" {
            command.as_object_mut().unwrap().remove("choice_id");
            command.as_object_mut().unwrap().remove("selected_options");
        }
        let response = room
            .app
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/games/current/commands",
                &command,
                Some(cookie),
                None,
            ))
            .await
            .expect("command");
        assert_eq!(response.status(), StatusCode::OK);
        // Simulate a response lost after the server commits the command.
        drop(response);
        let snapshot = current_game_snapshot(room).await;
        let has_discard_roll = snapshot["house_die_rolls"]
            .as_array()
            .expect("history")
            .last()
            .is_some_and(|roll| roll["result"] == 3);
        if has_discard_roll
            && snapshot["effects"]["choice"]["cause"] == "rule:g4-dark-arts-014-effect"
        {
            return (command, cookie.clone(), snapshot, projection);
        }
        let response = room
            .app
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
        assert_eq!(response.status(), StatusCode::OK);
        projection = response_json(response).await;
    }
    panic!("expected a persisted Heir discard face");
}

#[tokio::test]
async fn two_heroes_win_game_four_through_http() {
    assert_http_victory(2).await;
}

#[tokio::test]
async fn three_heroes_win_game_four_through_http() {
    assert_http_victory(3).await;
}

#[tokio::test]
async fn four_heroes_win_game_four_through_http() {
    assert_http_victory(4).await;
}

async fn assert_http_victory(count: usize) {
    fn seed(count: usize) -> [u8; 32] {
        let seeds: std::collections::BTreeMap<String, u8> =
            serde_json::from_str(include_str!("../fixtures/game-four/scenario-seeds.json"))
                .expect("seeds");
        [seeds[&count.to_string()]; 32]
    }
    let source: fn() -> Result<[u8; 32], getrandom::Error> = match count {
        2 => || Ok(seed(2)),
        3 => || Ok(seed(3)),
        4 => || Ok(seed(4)),
        _ => unreachable!("supported participant count"),
    };
    let (room, cookies) = super::game_one::ready_adventure_room_with_seed_source(
        count,
        harry_potter_server::game_four_manifest(),
        source,
    )
    .await;
    let projection = super::game_one::play_game_one(&room, &cookies, true).await;
    assert_eq!(projection["game"]["status"], "won", "{count} Heroes");
    assert!(
        !current_game_snapshot(&room).await["house_die_rolls"]
            .as_array()
            .expect("die history")
            .is_empty()
    );
}
