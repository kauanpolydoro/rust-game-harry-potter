use super::{ChaChaEffectRoller, codec, game_one_tests::prepared_adventure_with_seed};
use game_domain::{
    CardAcquisitionDestination, EffectEntityKind, EffectResource, EffectTargetBinding, EffectZone,
    GameCommand, GameCommandInput, GameEngine, GameIntentDecision, GameIntentInput, GameStatus,
    InitialGameState, PlayerIntent, ValidatedGameRules, apply_game_event, decide_game_command,
    legal_game_intentions,
};
use std::collections::BTreeMap;

fn choose(
    state: &InitialGameState,
    rules: &ValidatedGameRules,
    order: &[String],
) -> Option<GameCommand> {
    let legal = legal_game_intentions(state, state.active_position(), rules.effect_rules());
    if let Some(card) = legal.playable_cards.first() {
        return Some(GameCommand::PlayCard {
            card_id: card.card_id.clone(),
            targets: card
                .target_slots
                .iter()
                .map(|slot| EffectTargetBinding {
                    selector_id: slot.selector_id.clone(),
                    target_ids: select_targets(state, &slot.target_ids, slot.min, ""),
                })
                .collect(),
        });
    }
    if let Some(attack) = legal.attack_targets.first() {
        return Some(GameCommand::AssignAttack {
            villain_id: attack.villain_id.clone(),
            amount: attack.max_amount,
        });
    }
    legal
        .acquisitions
        .iter()
        .max_by_key(|card| {
            let entity = state
                .effect_world()
                .entities_in(EffectZone::Market)
                .iter()
                .find(|entity| entity.id() == card.card_id)
                .unwrap();
            order.len()
                - order
                    .iter()
                    .position(|id| Some(id.as_str()) == entity.catalog_id())
                    .unwrap_or(order.len())
        })
        .map(|card| GameCommand::AcquireCard {
            card_id: card.card_id.clone(),
            destination: if card
                .destinations
                .contains(&CardAcquisitionDestination::DrawPile)
            {
                CardAcquisitionDestination::DrawPile
            } else {
                CardAcquisitionDestination::DiscardPile
            },
        })
}

fn select_targets(
    state: &InitialGameState,
    options: &[String],
    min: u16,
    cause: &str,
) -> Vec<String> {
    if state.adventure_id() == "adventure:004"
        && state.players().len() == 4
        && [
            "rule:g1-starter-004",
            "rule:g1-starter-007",
            "rule:g1-starter-010",
            "rule:g1-starter-013",
        ]
        .contains(&cause)
        && state
            .effect_world()
            .hero_resource(state.active_position(), EffectResource::Health)
            .is_some_and(|health| health <= 7)
    {
        return options.get(1).cloned().into_iter().collect();
    }
    if state.adventure_id() == "adventure:004" && cause == "rule:g4-hogwarts-card-036-effect" {
        return options
            .get(if state.players().len() == 2 { 3 } else { 1 })
            .cloned()
            .into_iter()
            .collect();
    }
    let mut targets = options.to_vec();
    if options.iter().all(|id| id.starts_with("hero:")) {
        targets.sort_by_key(|id| {
            let hero = state
                .effect_world()
                .entities_in(EffectZone::Heroes)
                .iter()
                .find(|hero| hero.id() == id)
                .unwrap();
            if ["rule:g3-hero-002-ability", "rule:g3-hero-005-ability"].contains(&cause) {
                u16::from(hero.owner_position() != Some(state.active_position()))
            } else if cause == "rule:g4-dark-arts-015-effect" {
                10 - hero.resource(EffectResource::Health)
            } else {
                hero.resource(EffectResource::Health)
            }
        });
    }
    targets.into_iter().take(usize::from(min)).collect()
}

pub(super) fn next_decision(
    state: &InitialGameState,
    rules: &ValidatedGameRules,
    order: &[String],
    seed: u8,
) -> GameIntentDecision {
    let actor = state
        .pending_choice()
        .map_or(state.active_position(), |choice| {
            choice.responsible_position
        });
    let mut roller = ChaChaEffectRoller::new(&[seed; 32], state.prng_counter())
        .ok()
        .expect("seed");
    let intent = state
        .pending_choice()
        .map(|choice| PlayerIntent::ResolveChoice {
            choice_id: choice.id.clone(),
            selected_options: select_targets(state, &choice.options, choice.min, &choice.cause),
        });
    if intent.is_none()
        && let Some(command) = choose(state, rules, order)
    {
        let decision = decide_game_command(GameCommandInput {
            state,
            actor_position: actor,
            expected_state_version: state.state_version(),
            command,
            effect_rules: rules.effect_rules(),
            die_roller: &mut roller,
        })
        .expect("legal card action");
        GameIntentDecision {
            state: decision.state,
            event: decision.event,
        }
    } else {
        GameEngine::new(rules)
            .decide(
                GameIntentInput {
                    state,
                    actor_position: actor,
                    expected_state_version: state.state_version(),
                    intent: intent.unwrap_or(PlayerIntent::EndHeroActions),
                },
                &mut roller,
            )
            .expect("legal free intent")
    }
}

fn inventory(state: &InitialGameState) -> BTreeMap<String, (Option<String>, EffectEntityKind)> {
    state
        .effect_world()
        .entities()
        .map(|(_, entity)| {
            (
                entity.id().to_owned(),
                (entity.catalog_id().map(str::to_owned), entity.kind()),
            )
        })
        .collect()
}

fn assert_invariants(
    state: &InitialGameState,
    expected: &BTreeMap<String, (Option<String>, EffectEntityKind)>,
) {
    assert_eq!(
        &inventory(state),
        expected,
        "every physical instance remains in exactly one zone"
    );
    let heroes = state.effect_world().entities_in(EffectZone::Heroes);
    assert_eq!(heroes.len(), state.players().len());
    for (zone, entity) in state.effect_world().entities() {
        for (resource, limit) in entity.resource_limits() {
            assert!(entity.resource(*resource) <= *limit);
        }
        if let Some(owner) = entity.owner_position() {
            assert!(
                state
                    .players()
                    .iter()
                    .any(|player| player.position() == owner)
            );
        }
        if matches!(
            entity.kind(),
            EffectEntityKind::Hero | EffectEntityKind::Villain
        ) {
            let turn = entity.turn_state().expect("persistent turn state");
            assert!(turn.suppressed_by.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(
                turn.healed_positions
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
            );
            if let Some(limit) = turn.attack_limit {
                assert!(turn.attack_assigned <= u16::from(limit));
            }
        }
        if zone == EffectZone::Heroes {
            assert_eq!(entity.kind(), EffectEntityKind::Hero);
        }
    }
}

#[test]
fn game_three_seeded_command_sequences_preserve_inventory_resources_choices_and_replay() {
    assert_seeded_scenarios(
        crate::game_three_manifest,
        "adventure:003",
        include_str!("../../tests/fixtures/game-three/purchase-priority.json"),
    );
}

#[test]
fn game_four_seeded_command_sequences_preserve_inventory_resources_choices_and_replay() {
    assert_seeded_scenarios(
        crate::game_four_manifest,
        "adventure:004",
        include_str!("../../tests/fixtures/game-four/purchase-priority.json"),
    );
}

fn assert_seeded_scenarios(
    manifest: fn() -> game_content::ContentManifest,
    adventure: &str,
    policies: &str,
) {
    let policies: BTreeMap<String, Vec<String>> =
        serde_json::from_str(policies).expect("reviewed policies");
    let mut transitions = 0;
    for count in [2, 3, 4] {
        for seed in 0..12 {
            let (mut state, participants, rules) =
                prepared_adventure_with_seed(count, manifest(), adventure, seed);
            let expected = inventory(&state);
            let mut persisted = codec::persisted_snapshot(&state, &participants);
            for _ in 0..600 {
                assert_invariants(&state, &expected);
                if state.status() != GameStatus::InProgress {
                    break;
                }
                let decision = next_decision(&state, &rules, &policies[&count.to_string()], seed);
                assert_eq!(
                    apply_game_event(&state, &decision.event).expect("independent replay"),
                    decision.state
                );
                let repeated = next_decision(&state, &rules, &policies[&count.to_string()], seed);
                assert_eq!(
                    repeated.event, decision.event,
                    "same state, command and entropy"
                );
                assert_eq!(repeated.state, decision.state);
                state = decision.state;
                transitions += 1;
                if state.pending_choice().is_some() || transitions % 17 == 0 {
                    persisted = codec::persisted_after_decision(&persisted, &state);
                    let bytes = serde_json::to_string(&persisted).expect("canonical snapshot");
                    let decoded = codec::decode_persisted_snapshot(&bytes).ok().unwrap_or_else(|| {
                        panic!("current codec: {adventure}, {count} Heroes, seed {seed}, turn {}, sequence {}", state.turn(), state.sequence());
                    });
                    let restored = codec::command_domain_state(&decoded).ok().expect("restore");
                    assert_eq!(
                        serde_json::to_string(&codec::persisted_after_decision(
                            &persisted, &restored
                        ))
                        .expect("canonical restored state"),
                        bytes
                    );
                    if state.status() == GameStatus::InProgress {
                        let live =
                            next_decision(&state, &rules, &policies[&count.to_string()], seed);
                        let resumed =
                            next_decision(&restored, &rules, &policies[&count.to_string()], seed);
                        assert_eq!(
                            live.event, resumed.event,
                            "restoration preserves the next official event"
                        );
                        assert_eq!(
                            serde_json::to_string(&codec::persisted_after_decision(
                                &persisted,
                                &live.state
                            ))
                            .expect("live"),
                            serde_json::to_string(&codec::persisted_after_decision(
                                &persisted,
                                &resumed.state
                            ))
                            .expect("resumed")
                        );
                    }
                    state = restored;
                }
            }
            assert_ne!(
                state.status(),
                GameStatus::InProgress,
                "bounded scenario seed={seed}, players={count}"
            );
            for player in state.players() {
                assert!(
                    GameEngine::new(&rules)
                        .legal_intent_types(&state, player.position())
                        .is_empty()
                );
                let legal = legal_game_intentions(&state, player.position(), rules.effect_rules());
                assert!(
                    legal.playable_cards.is_empty()
                        && legal.attack_targets.is_empty()
                        && legal.acquisitions.is_empty()
                );
            }
        }
    }
    assert!(
        transitions >= 2000,
        "exercise many complete transitions: {transitions}"
    );
}
