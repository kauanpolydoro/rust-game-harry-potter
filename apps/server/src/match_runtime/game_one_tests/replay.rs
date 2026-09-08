use std::{collections::BTreeMap, path::PathBuf};

use game_domain::{
    EffectResource, EffectZone, GameCommandInput, GameEngine, GameIntentDecision, GameIntentInput,
    GameStatus, InitialGameState, ValidatedGameRules, apply_game_event, decide_game_command,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::super::{ChaChaEffectRoller, ExecuteGameCommandRequest, codec};
use super::{prepared_adventure, prepared_adventure_with_seed, prepared_game};

#[derive(Deserialize, Serialize)]
struct Scenario {
    players: usize,
    seed_byte: u8,
    outcome: String,
    #[serde(default)]
    opening_digest: String,
    #[serde(default)]
    checkpoints: BTreeMap<String, Value>,
    commands: Vec<Step>,
}

#[derive(Deserialize, Serialize)]
struct Step {
    request: Value,
    #[serde(default)]
    event_digest: String,
    #[serde(default)]
    snapshot_digest: String,
}

#[test]
fn game_one_browser_transcripts_replay_exact_events_and_snapshot_goldens() {
    replay_adventure("one");
}

#[test]
fn game_two_browser_transcripts_replay_exact_events_and_snapshot_goldens() {
    replay_adventure("two");
}

#[test]
fn game_three_browser_transcripts_replay_exact_events_and_snapshot_goldens() {
    replay_adventure("three");
}

#[test]
fn game_four_browser_transcripts_replay_exact_events_and_snapshot_goldens() {
    replay_adventure("four");
}

fn replay_adventure(game: &str) {
    let update = std::env::var_os(format!("UPDATE_GAME_{}_GOLDENS", game.to_uppercase())).is_some();
    for count in [2, 3, 4] {
        for outcome in ["lost", "won"] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/game-{game}/{count}-{outcome}.json"));
            let bytes = std::fs::read(&path).expect("committed browser transcript");
            let mut scenario: Scenario = serde_json::from_slice(&bytes).expect("scenario");
            assert_eq!(
                (scenario.players, scenario.outcome.as_str()),
                (count, outcome)
            );
            if game == "four" && outcome == "won" {
                let seeds: BTreeMap<String, u8> = serde_json::from_str(include_str!(
                    "../../../tests/fixtures/game-four/scenario-seeds.json"
                ))
                .expect("seeds");
                assert_eq!(scenario.seed_byte, seeds[&count.to_string()]);
            } else {
                assert_eq!(scenario.seed_byte, 7);
            }
            replay_scenario(&mut scenario, update, game);
            if update {
                std::fs::write(
                    path,
                    format!(
                        "{}\n",
                        serde_json::to_string_pretty(&scenario).expect("golden JSON")
                    ),
                )
                .expect("write reviewed golden");
            }
        }
    }
}

type EventEncoder = fn(
    game_domain::GameEvent,
) -> Result<(u16, &'static str, String), crate::http_support::ApiError>;

fn event_encoder(game: &str) -> EventEncoder {
    match game {
        "four" => codec::persisted_game_four_event,
        "three" => codec::persisted_game_three_event,
        "two" => codec::persisted_game_two_event,
        _ => codec::persisted_event,
    }
}

fn prepare_scenario(
    count: usize,
    game: &str,
    seed: u8,
) -> (
    InitialGameState,
    Vec<super::super::StoredRoomParticipant>,
    ValidatedGameRules,
) {
    match game {
        "one" => prepared_game(count),
        "two" => prepared_adventure(count, crate::game_two_manifest(), "adventure:002"),
        "three" => prepared_adventure(count, crate::game_three_manifest(), "adventure:003"),
        "four" => {
            prepared_adventure_with_seed(count, crate::game_four_manifest(), "adventure:004", seed)
        }
        _ => panic!("unknown adventure"),
    }
}

fn replay_scenario(scenario: &mut Scenario, update: bool, game: &str) {
    let (mut state, participants, rules) =
        prepare_scenario(scenario.players, game, scenario.seed_byte);
    let mut persisted = codec::persisted_snapshot(&state, &participants);
    let opening = serde_json::to_string(&persisted).expect("opening");
    check_digest(
        &mut scenario.opening_digest,
        &opening,
        update,
        "preparation",
    );
    let mut checkpoints = BTreeMap::new();
    checkpoints.insert("preparation".to_owned(), checkpoint(&state, &opening));
    for step in &mut scenario.commands {
        let mut input = step.request.clone();
        input["command_id"] = json!(uuid::Uuid::nil().to_string());
        let request: ExecuteGameCommandRequest =
            serde_json::from_value(input).expect("HTTP command contract");
        let before = serde_json::to_string(&persisted).expect("previous snapshot");
        let restored = codec::command_domain_state(
            &codec::decode_persisted_snapshot(&before)
                .ok()
                .expect("decode"),
        )
        .ok()
        .expect("restore");
        let decision = decide(&state, &request, &rules, scenario.seed_byte);
        let repeated = decide(&restored, &request, &rules, scenario.seed_byte);
        assert_eq!(
            repeated.event, decision.event,
            "restoring preserves the official event"
        );
        assert_eq!(
            serde_json::to_string(&codec::persisted_after_decision(
                &persisted,
                &repeated.state
            ))
            .expect("restored decision"),
            serde_json::to_string(&codec::persisted_after_decision(
                &persisted,
                &decision.state
            ))
            .expect("live decision"),
            "restoring preserves canonical state bytes, including the order within each owned pile"
        );
        assert_eq!(
            apply_game_event(&state, &decision.event).expect("event replay"),
            decision.state
        );
        let encode = event_encoder(game);
        let (_, _, event) = encode(decision.event).ok().expect("canonical event");
        let (_, _, repeated_event) = encode(repeated.event).ok().expect("repeated event");
        assert_eq!(
            event.as_bytes(),
            repeated_event.as_bytes(),
            "exact event bytes"
        );
        codec::decode_persisted_event(&event)
            .ok()
            .expect("current event codec");
        check_digest(&mut step.event_digest, &event, update, "event");
        state = decision.state;
        persisted = codec::persisted_after_decision(&persisted, &state);
        let snapshot = serde_json::to_string(&persisted).expect("canonical snapshot");
        check_digest(&mut step.snapshot_digest, &snapshot, update, "snapshot");
        capture_checkpoints(&mut checkpoints, &state, &snapshot);
    }
    assert_eq!(
        state.status(),
        if scenario.outcome == "won" {
            GameStatus::Won
        } else {
            GameStatus::Lost
        }
    );
    for name in [
        "preparation",
        "end_turn",
        "dark_arts",
        "villains",
        "hero_actions",
        "terminal",
    ] {
        assert!(checkpoints.contains_key(name), "missing {name} checkpoint");
    }
    if scenario.outcome == "lost" {
        assert!(checkpoints.contains_key("stun"));
        assert!(checkpoints.contains_key("location_advance"));
    }
    if update {
        scenario.checkpoints = checkpoints;
    } else {
        assert_eq!(scenario.checkpoints, checkpoints, "named scenario goldens");
    }
}

fn decide(
    state: &InitialGameState,
    request: &ExecuteGameCommandRequest,
    rules: &ValidatedGameRules,
    seed: u8,
) -> GameIntentDecision {
    let actor_position = state
        .pending_choice()
        .map_or(state.active_position(), |choice| {
            choice.responsible_position
        });
    let mut random = ChaChaEffectRoller::new(&[seed; 32], state.prng_counter())
        .ok()
        .expect("seed and counter");
    if let Some(intent) = request.player_intent() {
        GameEngine::new(rules)
            .decide(
                GameIntentInput {
                    state,
                    actor_position,
                    expected_state_version: request.expected_state_version(),
                    intent,
                },
                &mut random,
            )
            .expect("recorded player intent")
    } else {
        let decision = decide_game_command(GameCommandInput {
            state,
            actor_position,
            expected_state_version: request.expected_state_version(),
            command: request.game_command().expect("hero action"),
            effect_rules: rules.effect_rules(),
            die_roller: &mut random,
        })
        .expect("recorded hero action");
        GameIntentDecision {
            state: decision.state,
            event: decision.event,
        }
    }
}

fn check_digest(expected: &mut String, serialized: &str, update: bool, label: &str) {
    let digest = format!("blake3:{}", blake3::hash(serialized.as_bytes()).to_hex());
    if update {
        *expected = digest;
    } else {
        assert_eq!(*expected, digest, "{label} golden");
    }
}

fn capture_checkpoints(
    checkpoints: &mut BTreeMap<String, Value>,
    state: &InitialGameState,
    snapshot: &str,
) {
    checkpoints
        .entry(codec::game_phase_name(state.phase()).to_owned())
        .or_insert_with(|| checkpoint(state, snapshot));
    for step in state.last_turn_steps() {
        let name = codec::game_phase_name(step.phase()).to_owned();
        checkpoints
            .entry(name)
            .or_insert_with(|| checkpoint(state, snapshot));
    }
    if state
        .effect_world()
        .entities_in(EffectZone::Heroes)
        .iter()
        .any(|hero| hero.resource(EffectResource::Health) == 0)
    {
        checkpoints
            .entry("stun".to_owned())
            .or_insert_with(|| checkpoint(state, snapshot));
    }
    if !state
        .effect_world()
        .entities_in(EffectZone::LocationDiscard)
        .is_empty()
    {
        checkpoints
            .entry("location_advance".to_owned())
            .or_insert_with(|| checkpoint(state, snapshot));
    }
    if state.status() != GameStatus::InProgress {
        checkpoints.insert("terminal".to_owned(), checkpoint(state, snapshot));
    }
}

fn checkpoint(state: &InitialGameState, snapshot: &str) -> Value {
    let value: Value = serde_json::from_str(snapshot).expect("snapshot JSON");
    let mut result = json!({
        "sequence": state.sequence(), "status": value["status"], "turn": value["turn"],
        "prng": value["prng"], "steps": value["last_turn_steps"], "decision_point": value["decision_point"],
        "heroes_and_locations": value["effects"]["entities"].as_array().expect("entities").iter().filter(|entity| {
            ["heroes", "active_location", "location_deck", "location_discard"].contains(&entity["zone"].as_str().unwrap_or_default())
        }).collect::<Vec<_>>(),
    });
    if state.sequence() == 0 {
        result["preparation_samples"] = value["preparation_samples"].clone();
        result["prepared_piles"] = json!(value["effects"]["entities"].as_array().expect("entities").iter().map(|entity| {
            json!({"id":entity["id"], "zone":entity["zone"], "owner_position":entity["owner_position"]})
        }).collect::<Vec<_>>());
    }
    if state.snapshot_version() == game_domain::GAME_FOUR_SNAPSHOT_VERSION {
        result["house_die_rolls"] = value["house_die_rolls"].clone();
    }
    result
}
