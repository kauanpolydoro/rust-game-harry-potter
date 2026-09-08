use super::game_one::{play_game_one, ready_adventure_room};
use super::*;

#[tokio::test]
async fn game_two_starts_through_http_with_its_own_persisted_inventory() {
    let (room, _) = ready_adventure_room(2, harry_potter_server::game_two_manifest()).await;
    let projection = start_ready_game(&room, "game-two-start").await;
    assert_eq!(projection["snapshot"]["snapshot_version"], 6);
    assert_eq!(
        projection["snapshot"]["versions"]["content"],
        "game-two-en-v1"
    );
    let snapshot = current_game_snapshot(&room).await;
    assert_eq!(
        snapshot["preparation_samples"]
            .as_array()
            .expect("samples")
            .len(),
        80
    );
    assert_eq!(snapshot["adventure_id"], "adventure:002");
    for mutation in ["legacy_codec", "preparation", "dangling_copy"] {
        let mut forged = snapshot.clone();
        match mutation {
            "legacy_codec" => forged["snapshot_version"] = json!(5),
            "preparation" => forged["preparation_samples"][0]["result"] = json!(44),
            "dangling_copy" => {
                let card = forged["effects"]["entities"]
                    .as_array_mut()
                    .expect("entities")
                    .iter_mut()
                    .find(|entity| entity["zone"] == "hero_hand")
                    .expect("card");
                card["zone"] = json!("hero_play_area");
                card["copied_ally_id"] = json!("missing:ally");
            }
            _ => unreachable!(),
        }
        let valid: bool = sqlx::query_scalar("SELECT valid_game_snapshot_v6($1)")
            .bind(&forged)
            .fetch_one(&room.database)
            .await
            .expect("SQL validation");
        assert!(!valid, "database rejects {mutation}");
    }
}

#[tokio::test]
async fn game_two_with_two_three_and_four_players_reaches_both_outcomes_through_http() {
    for count in [2, 3, 4] {
        for seek_victory in [false, true] {
            let (room, cookies) =
                ready_adventure_room(count, harry_potter_server::game_two_manifest()).await;
            let projection = play_game_one(&room, &cookies, seek_victory).await;
            assert_eq!(
                projection["game"]["status"],
                if seek_victory { "won" } else { "lost" },
                "{count} players, victory={seek_victory}"
            );
            if count == 4 && seek_victory {
                assert_repeated_petrification_is_persisted(&room).await;
            }
        }
    }
}

async fn assert_repeated_petrification_is_persisted(room: &ReadyRoom) {
    let repeated: bool = sqlx::query_scalar(
        r"
        SELECT EXISTS (
            SELECT event.sequence, effect ->> 'target_id'
            FROM game_events AS event
            JOIN games AS game ON game.id = event.game_id
            JOIN rooms AS room ON room.id = game.room_id
            CROSS JOIN LATERAL jsonb_array_elements(event.payload -> 'steps') AS step
            CROSS JOIN LATERAL jsonb_array_elements(step -> 'effects') AS effect
            WHERE room.code = $1 AND event.event_version = 7
                AND event.event_type = 'turn_completed' AND effect ->> 'type' = 'drawing_blocked'
            GROUP BY event.sequence, effect ->> 'target_id' HAVING COUNT(*) > 1
        )
    ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("persisted double revelation");
    assert!(
        repeated,
        "the scenario must persist both Petrifications in one Turn"
    );
}
