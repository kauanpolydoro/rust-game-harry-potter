use super::game_one::{play_game_one, ready_adventure_room};
use super::*;

#[tokio::test]
async fn game_three_starts_through_http_with_its_own_persisted_inventory() {
    let (room, _) = ready_adventure_room(2, harry_potter_server::game_three_manifest()).await;
    let projection = start_ready_game(&room, "game-three-start").await;
    assert_eq!(projection["snapshot"]["snapshot_version"], 7);
    assert_eq!(
        projection["snapshot"]["versions"]["content"],
        "game-three-en-v1"
    );
    let snapshot = current_game_snapshot(&room).await;
    assert_eq!(
        snapshot["preparation_samples"]
            .as_array()
            .expect("samples")
            .len(),
        102
    );
    assert_eq!(snapshot["adventure_id"], "adventure:003");
    for mutation in [
        "legacy_codec",
        "preparation",
        "dangling_copy",
        "hero_identity",
        "missing_turn_state",
        "unknown_caster",
    ] {
        let mut forged = snapshot.clone();
        match mutation {
            "legacy_codec" => forged["snapshot_version"] = json!(5),
            "preparation" => forged["preparation_samples"][0]["result"] = json!(60),
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
            "hero_identity" => {
                let hero = forged["effects"]["entities"]
                    .as_array_mut()
                    .expect("entities")
                    .iter_mut()
                    .find(|entity| entity["kind"] == "hero" && entity["owner_position"] == 2)
                    .expect("Hermione");
                hero["catalog_id"] = json!("hero:002");
                hero["effect_rule_id"] = json!("rule:g3-hero-002-ability");
            }
            "missing_turn_state" | "unknown_caster" => {
                let villain = forged["effects"]["entities"]
                    .as_array_mut()
                    .expect("entities")
                    .iter_mut()
                    .find(|entity| entity["kind"] == "villain")
                    .expect("Villain");
                if mutation == "missing_turn_state" {
                    villain
                        .as_object_mut()
                        .expect("entity")
                        .remove("turn_state");
                } else {
                    villain["turn_state"]["suppressed_by"] = json!([4]);
                }
            }
            _ => unreachable!(),
        }
        let valid: bool = sqlx::query_scalar("SELECT valid_game_snapshot_v7($1)")
            .bind(&forged)
            .fetch_one(&room.database)
            .await
            .expect("SQL validation");
        assert!(!valid, "database rejects {mutation}");
    }
}

#[tokio::test]
async fn game_three_database_checks_previous_turn_state_and_actual_revealed_top() {
    let (room, _) = ready_adventure_room(2, harry_potter_server::game_three_manifest()).await;
    start_ready_game(&room, "game-three-event-validation").await;
    let snapshot = current_game_snapshot(&room).await;
    let world = &snapshot["effects"]["entities"];
    let hero = world
        .as_array()
        .expect("entities")
        .iter()
        .find(|entity| entity["kind"] == "hero")
        .expect("Hero");
    let mut after = hero["turn_state"].clone();
    after["ability_used"] = json!(true);
    let mut effect = json!({"type":"turn_state_changed", "rule_id":"rule:g3-hero-002-ability", "target_id":hero["id"], "before":hero["turn_state"], "after":after});
    let valid: Option<Value> = sqlx::query_scalar("SELECT apply_turn_state_v8($1, $2)")
        .bind(world)
        .bind(&effect)
        .fetch_one(&room.database)
        .await
        .expect("valid transition");
    assert!(valid.is_some());
    effect["before"]["attack_assigned"] = json!(1);
    let invalid: Option<Value> = sqlx::query_scalar("SELECT apply_turn_state_v8($1, $2)")
        .bind(world)
        .bind(&effect)
        .fetch_one(&room.database)
        .await
        .expect("forged transition");
    assert!(invalid.is_none());
    let top = world
        .as_array()
        .expect("entities")
        .iter()
        .filter(|entity| entity["zone"] == "hero_draw_pile" && entity["owner_position"] == 1)
        .max_by_key(|entity| entity["zone_index"].as_u64())
        .expect("draw pile");
    let mut reveal = json!({"type":"top_card_revealed", "rule_id":"rule:g3-villain-008-effect", "card_id":top["id"], "owner_position":1});
    let valid: Option<Value> = sqlx::query_scalar("SELECT validate_revealed_top_v8($1, $2)")
        .bind(world)
        .bind(&reveal)
        .fetch_one(&room.database)
        .await
        .expect("valid reveal");
    assert_eq!(valid.as_ref(), Some(world));
    reveal["owner_position"] = json!(2);
    let invalid: Option<Value> = sqlx::query_scalar("SELECT validate_revealed_top_v8($1, $2)")
        .bind(world)
        .bind(&reveal)
        .fetch_one(&room.database)
        .await
        .expect("forged reveal");
    assert!(invalid.is_none());
}

#[tokio::test]
async fn game_three_with_two_three_and_four_players_reaches_both_outcomes_through_http() {
    for count in [2, 3, 4] {
        for seek_victory in [false, true] {
            let (room, cookies) =
                ready_adventure_room(count, harry_potter_server::game_three_manifest()).await;
            let projection = play_game_one(&room, &cookies, seek_victory).await;
            assert_eq!(
                projection["game"]["status"],
                if seek_victory { "won" } else { "lost" },
                "{count} players, victory={seek_victory}"
            );
        }
    }
}

pub(super) fn scenario_targets(
    projection: &Value,
    options: &[Value],
    count: usize,
    cause: &str,
) -> Vec<Value> {
    let mut options = options.to_vec();
    if projection["snapshot"]["versions"]["content"] == "game-three-en-v1"
        && options
            .iter()
            .all(|option| option.as_str().is_some_and(|id| id.starts_with("hero:")))
    {
        options.sort_by_key(|option| {
            let position = option
                .as_str()
                .expect("Hero ID")
                .trim_start_matches("hero:")
                .parse::<u64>()
                .expect("position");
            if ["rule:g3-hero-002-ability", "rule:g3-hero-005-ability"].contains(&cause) {
                u64::from(projection["turn"]["active_position"] != position)
            } else {
                projection["participants"]
                    .as_array()
                    .expect("participants")
                    .iter()
                    .find(|hero| hero["position"] == position)
                    .expect("Hero")["resources"]["health"]
                    .as_u64()
                    .expect("health")
            }
        });
    }
    options.into_iter().take(count).collect()
}
