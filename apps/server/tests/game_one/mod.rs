use super::*;

#[tokio::test]
async fn real_game_one_starts_through_http_and_persists_preparation_evidence() {
    let (room, _) = ready_game_one_room(2).await;
    let schema: String =
        sqlx::query_scalar("SELECT value FROM application_metadata WHERE key = 'schema_version'")
            .fetch_one(&room.database)
            .await
            .expect("published schema version");
    assert_eq!(schema, "24");
    let projection = start_ready_game(&room, "game-one").await;
    assert_eq!(projection["snapshot"]["snapshot_version"], 5);
    assert_eq!(projection["choice"]["source_name"], "Flipendo");
    assert_eq!(
        projection["choice"]["instruction"],
        "Você descarta 1 carta da mão."
    );
    assert!(
        projection["table"]["revealed_dark_arts"]["description"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    for card in projection["table"]["hand"].as_array().expect("hand") {
        assert!(
            card["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
    }
    let snapshot = current_game_snapshot(&room).await;
    assert_eq!(
        snapshot["preparation_samples"]
            .as_array()
            .expect("samples")
            .len(),
        58
    );
    let entities = snapshot["effects"]["entities"]
        .as_array()
        .expect("entities");
    assert_eq!(
        entities
            .iter()
            .filter(|entity| entity["zone"] == "market")
            .count(),
        6
    );
    assert_eq!(
        entities
            .iter()
            .filter(|entity| entity["zone"] == "dark_arts_discard")
            .count(),
        1
    );
    let mut forged = snapshot.clone();
    forged["preparation_samples"][0]["result"] = json!(30);
    let valid: bool = sqlx::query_scalar("SELECT valid_game_snapshot_v5($1)")
        .bind(&forged)
        .fetch_one(&room.database)
        .await
        .expect("SQL validation");
    assert!(
        !valid,
        "the database rejects an out-of-range preparation sample"
    );
}

async fn ready_game_one_room(count: usize) -> (ReadyRoom, Vec<String>) {
    ready_adventure_room(count, harry_potter_server::game_one_manifest()).await
}

pub(super) async fn ready_adventure_room(
    count: usize,
    manifest: game_content::ContentManifest,
) -> (ReadyRoom, Vec<String>) {
    let database = database().await;
    let state = AppState::with_content_manifests(database.clone(), vec![manifest.clone()])
        .with_game_seed_source(|| Ok([7; 32]));
    initialize(&state).await.expect("isolated database");
    let app = build_router(state.clone());
    let (room_code, host_cookie, host_recovery_token) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &host_cookie, "harry").await.status(),
        StatusCode::OK
    );
    let (guest_cookie, guest_recovery_token) = join_room(&app, &room_code).await;
    let mut cookies = vec![host_cookie.clone(), guest_cookie.clone()];
    for hero in ["ron", "neville"].into_iter().take(count - 2) {
        let response = app
            .clone()
            .oneshot(json_request(
                "POST",
                &format!("/api/rooms/{room_code}/participants"),
                &json!({"display_name": hero, "hero_id": hero}),
                None,
                Some(&unique_key("game-one-join")),
            ))
            .await
            .expect("additional participant");
        assert_eq!(response.status(), StatusCode::CREATED);
        cookies.push(session_cookie(&response));
    }
    for cookie in &cookies {
        assert_eq!(set_ready(&app, cookie, true).await.status(), StatusCode::OK);
    }
    (
        ReadyRoom {
            app,
            state,
            database,
            room_code,
            host_cookie,
            host_recovery_token,
            guest_cookie,
            guest_recovery_token,
            manifest,
        },
        cookies,
    )
}

#[tokio::test]
async fn real_game_one_resolves_choices_reshuffles_and_locations_until_defeat() {
    for count in [2, 3, 4] {
        let (room, cookies) = ready_game_one_room(count).await;
        let projection = play_game_one(&room, &cookies, false).await;
        assert_eq!(projection["game"]["status"], "lost");
        let snapshot = current_game_snapshot(&room).await;
        assert!(snapshot["prng"]["counter"].as_u64().expect("counter") > 40 + 9 * count as u64);
        assert_eq!(
            snapshot["preparation_samples"]
                .as_array()
                .expect("samples")
                .len(),
            40 + 9 * count
        );
    }
}

#[tokio::test]
async fn real_game_one_players_buy_play_and_defeat_every_villain_through_http() {
    for count in [2, 3, 4] {
        let (room, cookies) = ready_game_one_room(count).await;
        let projection = play_game_one(&room, &cookies, true).await;
        assert_eq!(
            projection["game"]["status"], "won",
            "{count} heroes at turn {}",
            projection["turn"]["number"]
        );
    }
}

pub(super) async fn play_game_one(
    room: &ReadyRoom,
    cookies: &[String],
    seek_victory: bool,
) -> Value {
    let mut projection = start_ready_game(room, "game-one-flow").await;
    for _ in 0..600 {
        if projection["game"]["status"] != "in_progress" {
            return projection;
        }
        let position = if projection["choice"]["status"] == "pending" {
            &projection["choice"]["responsible_position"]
        } else {
            &projection["turn"]["active_position"]
        };
        let cookie = &cookies
            [usize::try_from(position.as_u64().expect("responsible position") - 1).expect("index")];
        // Accepted commands already return the actor's current projection.
        // Fetch another player's private state only when responsibility changes.
        if &projection["participant"]["position"] != position {
            let response = room
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/session")
                        .header(header::COOKIE, cookie)
                        .body(Body::empty())
                        .expect("projection request"),
                )
                .await
                .expect("participant projection");
            assert_eq!(response.status(), StatusCode::OK);
            projection = response_json(response).await;
        }
        let mut command = game_one_command(&projection, seek_victory);
        command["command_id"] = json!(uuid::Uuid::new_v4().to_string());
        command["expected_state_version"] = projection["snapshot"]["state_version"].clone();
        projection = accepted_request(
            room,
            json_request(
                "POST",
                "/api/games/current/commands",
                &command,
                Some(cookie),
                None,
            ),
        )
        .await;
    }
    panic!("Game 1 did not reach a terminal state within the bounded scenario");
}

fn game_one_command(projection: &Value, seek_victory: bool) -> Value {
    let choice = &projection["choice"];
    if choice["status"] == "pending" {
        let count = usize::try_from(choice["min"].as_u64().expect("choice count")).expect("count");
        let options = choice["options"]
            .as_array()
            .expect("options")
            .iter()
            .take(count)
            .cloned()
            .collect::<Vec<_>>();
        return json!({"type":"resolve_choice", "choice_id":choice["id"], "selected_options":options});
    }
    if !seek_victory {
        return json!({"type":"end_hero_actions"});
    }
    let legal = &projection["legal_intentions"];
    if let Some(card) = legal["play_cards"]
        .as_array()
        .expect("play actions")
        .first()
    {
        let targets = card["target_slots"]
            .as_array()
            .expect("slots")
            .iter()
            .map(|slot| {
                let count = usize::try_from(slot["min"].as_u64().expect("min")).expect("count");
                let ids = slot["options"]
                    .as_array()
                    .expect("options")
                    .iter()
                    .take(count)
                    .map(|option| option["target_id"].clone())
                    .collect::<Vec<_>>();
                json!({"selector_id":slot["selector_id"], "target_ids":ids})
            })
            .collect::<Vec<_>>();
        return json!({"type":"play_card", "card_id":card["card_id"], "targets":targets});
    }
    if let Some(attack) = legal["assign_attack"]
        .as_array()
        .expect("attack actions")
        .first()
    {
        return json!({"type":"assign_attack", "villain_id":attack["villain_id"], "amount":attack["max_amount"]});
    }
    if let Some(card) = legal["acquire_cards"]
        .as_array()
        .expect("acquisitions")
        .iter()
        .max_by_key(|card| {
            projection["table"]["market"]
                .as_array()
                .expect("market")
                .iter()
                .find(|item| item["instance_id"] == card["card_id"])
                .map_or(0, |item| {
                    acquisition_priority(&item["catalog_id"], projection)
                })
        })
    {
        let destination = if card["destinations"]
            .as_array()
            .expect("destinations")
            .contains(&json!("draw_pile"))
        {
            "draw_pile"
        } else {
            "discard_pile"
        };
        return json!({"type":"acquire_card", "card_id":card["card_id"], "destination":destination});
    }
    json!({"type":"end_hero_actions"})
}

fn acquisition_priority(catalog: &Value, projection: &Value) -> u8 {
    if projection["snapshot"]["versions"]["content"] == "game-two-en-v1" {
        let policies: Value =
            serde_json::from_str(include_str!("../fixtures/game-two/purchase-priority.json"))
                .expect("scenario policies");
        let count = projection["participants"]
            .as_array()
            .expect("participants")
            .len()
            .to_string();
        let order = policies[&count].as_array().expect("policy");
        return u8::try_from(
            order.len()
                - order
                    .iter()
                    .position(|id| id == catalog)
                    .unwrap_or(order.len()),
        )
        .expect("priority");
    }
    match catalog.as_str().expect("catalog ID") {
        "hogwarts-card:001" => 12,
        "hogwarts-card:005" => 11,
        "hogwarts-card:010" => 10,
        "hogwarts-card:008" => 9,
        "hogwarts-card:002" => 8,
        "hogwarts-card:007" => 7,
        "hogwarts-card:004" => 6,
        "hogwarts-card:006" => 5,
        "hogwarts-card:011" => 4,
        "hogwarts-card:009" => 3,
        "hogwarts-card:013" => 2,
        _ => 1,
    }
}
