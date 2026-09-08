use super::{codec, game_one_tests::prepared_adventure};
use game_domain::EffectZone;

#[test]
fn game_four_prepares_the_cumulative_inventory_and_preserves_hero_identities() {
    for count in 2..=4 {
        let (state, participants, _) =
            prepared_adventure(count, crate::game_four_manifest(), "adventure:004");
        assert_eq!(state.snapshot_version(), 8);
        assert_eq!(state.preparation_samples().len(), 115 + 9 * count);
        assert_eq!(
            state.effect_world().entities_in(EffectZone::Market).len(),
            6
        );
        assert_eq!(
            state
                .effect_world()
                .entities_in(EffectZone::ActiveVillains)
                .len(),
            2
        );
        assert_eq!(
            state.effect_world().entities_in(EffectZone::ActiveLocation)[0].catalog_id(),
            Some("location:009")
        );
        let snapshot = codec::persisted_snapshot(&state, &participants);
        let encoded = serde_json::to_string(&snapshot).expect("snapshot");
        let decoded = codec::decode_persisted_snapshot(&encoded)
            .ok()
            .expect("supported snapshot");
        assert_eq!(
            codec::command_domain_state(&decoded).ok().expect("restore"),
            state
        );
    }
}

#[test]
fn an_opening_house_roll_records_its_purpose_counter_and_result_in_the_snapshot() {
    let (state, participants, _) = (0..64)
        .find_map(|seed| {
            let prepared = super::game_one_tests::prepared_adventure_with_seed(
                2,
                crate::game_four_manifest(),
                "adventure:004",
                seed,
            );
            prepared
                .0
                .last_effects()
                .iter()
                .any(|effect| matches!(effect, game_domain::EffectOutcome::DieRolled { .. }))
                .then_some(prepared)
        })
        .expect("a deterministic seed reveals Heir of Slytherin");
    let roll = state.house_die_rolls().last().expect("persisted roll");
    assert_eq!(roll.purpose, "rule:g4-dark-arts-014-effect");
    assert_eq!(roll.counter, state.preparation_samples().len() as u64);
    assert_eq!(roll.die, game_domain::EffectDie::SlytherinV1);
    assert_eq!(roll.sides, 6);
    assert!((1..=6).contains(&roll.result));
    let snapshot = codec::persisted_snapshot(&state, &participants);
    let encoded = serde_json::to_string(&snapshot).expect("snapshot");
    let decoded = codec::decode_persisted_snapshot(&encoded)
        .ok()
        .expect("decode");
    assert_eq!(
        codec::command_domain_state(&decoded).ok().expect("restore"),
        state
    );
}

#[test]
fn each_event_roll_keeps_its_absolute_counter_even_with_an_intervening_shuffle_sample() {
    use game_domain::{EffectDie, EffectOutcome, EffectStop, GameEvent};
    let event = GameEvent::CardPlayed {
        sequence: 1,
        state_version: 2,
        turn: 1,
        actor_position: 1,
        card_id: "fixture:card".into(),
        targets: vec![],
        stop: EffectStop::Stable,
        prng_counter: 147,
        effects: vec![
            EffectOutcome::DieRolled {
                rule_id: "rule:fixture-first".into(),
                die: EffectDie::HufflepuffV1,
                result: 6,
            },
            EffectOutcome::RandomSampled {
                rule_id: "rule:fixture-draw".into(),
                upper_exclusive: 4,
                result: 2,
            },
            EffectOutcome::DieRolled {
                rule_id: "rule:fixture-second".into(),
                die: EffectDie::RavenclawV1,
                result: 3,
            },
        ],
    };
    let (version, _, encoded) = codec::persisted_game_four_event(event)
        .ok()
        .expect("encode event 9");
    assert_eq!(version, 9);
    let document: serde_json::Value = serde_json::from_str(&encoded).expect("JSON");
    assert_eq!(
        document["house_die_rolls"],
        serde_json::json!([
            {"purpose":"rule:fixture-first","counter":144,"die":"hufflepuff_v1","sides":6,"result":6},
            {"purpose":"rule:fixture-second","counter":146,"die":"ravenclaw_v1","sides":6,"result":3},
        ])
    );
    for mutation in ["counter", "result", "purpose", "missing", "legacy"] {
        let mut forged = document.clone();
        match mutation {
            "counter" => forged["house_die_rolls"][0]["counter"] = serde_json::json!(145),
            "result" => forged["house_die_rolls"][0]["result"] = serde_json::json!(1),
            "purpose" => {
                forged["house_die_rolls"][0]["purpose"] = serde_json::json!("rule:wrong-source");
            }
            "missing" => {
                forged.as_object_mut().unwrap().remove("house_die_rolls");
            }
            "legacy" => forged["event_version"] = serde_json::json!(8),
            _ => unreachable!(),
        }
        assert!(
            codec::decode_persisted_event(&forged.to_string()).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn restoring_and_resuming_the_heirs_discard_choices_never_rolls_the_die_again() {
    struct NoEntropy;
    impl game_domain::EffectRoller for NoEntropy {
        fn roll(&mut self, _: game_domain::EffectDie) -> Option<u8> {
            panic!("persisted die must not be rolled again")
        }
        fn sample_below(&mut self, _: u32) -> Option<u32> {
            panic!("these discard continuations require no new entropy")
        }
    }
    let (mut state, participants, rules) = (0..96)
        .find_map(|seed| {
            let prepared = super::game_one_tests::prepared_adventure_with_seed(
                2,
                crate::game_four_manifest(),
                "adventure:004",
                seed,
            );
            prepared
                .0
                .house_die_rolls()
                .last()
                .is_some_and(|roll| roll.result == 3)
                .then_some(prepared)
        })
        .expect("Heir of Slytherin's discard face");
    let original_rolls = state.house_die_rolls().to_vec();
    let counter = state.prng_counter();
    let mut snapshot = codec::persisted_snapshot(&state, &participants);
    let mut choices = 0;
    while let Some(choice) = state.pending_choice() {
        choices += 1;
        assert!(choices < 20, "finite choices");
        let intent = game_domain::PlayerIntent::ResolveChoice {
            choice_id: choice.id.clone(),
            selected_options: choice
                .options
                .iter()
                .take(usize::from(choice.min))
                .cloned()
                .collect(),
        };
        let actor = choice.responsible_position;
        let restored =
            codec::decode_persisted_snapshot(&serde_json::to_string(&snapshot).expect("snapshot"))
                .ok()
                .expect("reload");
        state = codec::command_domain_state(&restored)
            .ok()
            .expect("restore domain");
        let decision = game_domain::GameEngine::new(&rules)
            .decide(
                game_domain::GameIntentInput {
                    state: &state,
                    actor_position: actor,
                    expected_state_version: state.state_version(),
                    intent,
                },
                &mut NoEntropy,
            )
            .expect("resume persisted choice");
        assert_eq!(
            game_domain::apply_game_event(&state, &decision.event).expect("replay"),
            decision.state
        );
        assert_eq!(decision.state.house_die_rolls(), original_rolls);
        assert_eq!(decision.state.prng_counter(), counter);
        codec::persisted_game_four_event(decision.event)
            .ok()
            .expect("encode continuation");
        snapshot = codec::persisted_after_decision(&restored, &decision.state);
        state = decision.state;
    }
    assert!(choices >= 2, "each Hero owns a discard decision");
    let restored = codec::command_domain_state(&snapshot)
        .ok()
        .expect("final restore");
    assert_eq!(restored, state);
}

#[test]
fn graveyard_discards_pause_end_turn_and_resume_after_snapshot_restore() {
    use game_domain::{GameEngine, GameIntentInput, GamePhase, PlayerIntent, apply_game_event};
    let (state, snapshot, rules) = graveyard_scenario();
    let mut roller = super::ChaChaEffectRoller::new(&[19; 32], state.prng_counter())
        .ok()
        .expect("entropy");
    let mut decision = GameEngine::new(&rules)
        .decide(
            GameIntentInput {
                state: &state,
                actor_position: state.active_position(),
                expected_state_version: state.state_version(),
                intent: PlayerIntent::EndHeroActions,
            },
            &mut roller,
        )
        .expect("end turn");
    assert_eq!(decision.state.phase(), GamePhase::EndTurn);
    assert_eq!(decision.state.turn(), state.turn());
    assert_eq!(
        apply_game_event(&state, &decision.event).expect("replay end turn"),
        decision.state
    );
    codec::persisted_game_four_event(decision.event.clone())
        .ok()
        .expect("encode end turn");
    let mut discarded_by_hermione = false;
    while decision.state.phase() == GamePhase::EndTurn {
        let saved = codec::persisted_after_decision(&snapshot, &decision.state);
        let restored =
            codec::decode_persisted_snapshot(&serde_json::to_string(&saved).expect("JSON"))
                .ok()
                .expect("reload pending end turn");
        let current = codec::command_domain_state(&restored)
            .ok()
            .expect("restore pending end turn");
        let choice = current.pending_choice().expect("Graveyard choice");
        assert_eq!(choice.cause, "rule:g4-location-011-effect");
        discarded_by_hermione |= choice.responsible_position == 2;
        decision = GameEngine::new(&rules)
            .decide(
                GameIntentInput {
                    state: &current,
                    actor_position: choice.responsible_position,
                    expected_state_version: current.state_version(),
                    intent: PlayerIntent::ResolveChoice {
                        choice_id: choice.id.clone(),
                        selected_options: choice
                            .options
                            .iter()
                            .take(usize::from(choice.min))
                            .cloned()
                            .collect(),
                    },
                },
                &mut roller,
            )
            .expect("resume Graveyard");
        assert_eq!(
            apply_game_event(&current, &decision.event).expect("replay continuation"),
            decision.state
        );
        codec::persisted_game_four_event(decision.event.clone())
            .ok()
            .expect("encode continuation");
    }
    assert!(discarded_by_hermione);
    assert_eq!(decision.state.turn(), state.turn() + 1);
    assert_eq!(decision.state.active_position(), 2);
}

#[test]
fn an_opening_death_eater_reacts_only_to_villains_revealed_after_it() {
    use game_domain::{EffectOutcome, EffectResource};
    for death_eater_index in [0, 1] {
        let (state, _, _) = (0..128)
            .find_map(|seed| {
                let prepared = super::game_one_tests::prepared_adventure_with_seed(
                    2,
                    crate::game_four_manifest(),
                    "adventure:004",
                    seed,
                );
                let villains = prepared
                    .0
                    .effect_world()
                    .entities_in(EffectZone::ActiveVillains);
                let morsmordre_revealed = prepared
                    .0
                    .effect_world()
                    .entities_in(EffectZone::DarkArtsDiscard)
                    .iter()
                    .any(|card| card.catalog_id() == Some("dark-arts:016"));
                (!morsmordre_revealed
                    && villains[death_eater_index].catalog_id() == Some("villain:010"))
                .then_some(prepared)
            })
            .expect("ordered initial Death Eater revelation");
        let damage = state.last_effects().iter().filter(|effect| matches!(effect,
            EffectOutcome::ResourceChanged { rule_id, resource: EffectResource::Health, before, after, .. }
                if rule_id == "rule:g4-villain-010-effect" && before > after
        )).count();
        assert_eq!(
            damage,
            if death_eater_index == 0 { 2 } else { 0 },
            "initial revelation order"
        );
    }
}

#[test]
fn every_house_face_executes_its_published_golden_and_survives_replay() {
    use game_domain::{
        EffectDie, EffectResource, GameCommand, GameCommandInput, GameEngine, GameIntentInput,
        PlayerIntent, apply_game_event, decide_game_command,
    };
    let (state, snapshot, rules, card_id) = house_die_scenario();
    let play = decide_game_command(GameCommandInput {
        state: &state,
        actor_position: 1,
        expected_state_version: state.state_version(),
        command: GameCommand::PlayCard {
            card_id,
            targets: vec![],
        },
        effect_rules: rules.effect_rules(),
        die_roller: &mut Face(EffectDie::GryffindorV1, 1, 0),
    })
    .expect("choose a House");
    let choice = play.state.pending_choice().expect("four Houses");
    assert_eq!(choice.options.len(), 4);
    for (house_index, (die, outcomes)) in house_faces().into_iter().enumerate() {
        for (face_index, outcome) in outcomes.into_iter().enumerate() {
            let face = u8::try_from(face_index + 1).expect("face");
            let mut roller = Face(die, face, 0);
            let decision = GameEngine::new(&rules)
                .decide(
                    GameIntentInput {
                        state: &play.state,
                        actor_position: 1,
                        expected_state_version: play.state.state_version(),
                        intent: PlayerIntent::ResolveChoice {
                            choice_id: choice.id.clone(),
                            selected_options: vec![choice.options[house_index].clone()],
                        },
                    },
                    &mut roller,
                )
                .expect("roll chosen House");
            assert_eq!(roller.2, 1);
            assert!(decision.state.pending_choice().is_none());
            for hero in decision
                .state
                .effect_world()
                .entities_in(EffectZone::Heroes)
            {
                let owner = hero.owner_position().expect("Hero owner");
                let hand_count = |state: &game_domain::InitialGameState| {
                    state
                        .effect_world()
                        .entities_in(EffectZone::HeroHand)
                        .iter()
                        .filter(|card| card.owner_position() == Some(owner))
                        .count()
                };
                assert_eq!(
                    (
                        hero.resource(EffectResource::Health),
                        hero.resource(EffectResource::Attack),
                        hero.resource(EffectResource::Influence),
                        hand_count(&decision.state) - hand_count(&play.state)
                    ),
                    (
                        if outcome == "health" { 8 } else { 7 },
                        u16::from(outcome == "attack"),
                        u16::from(outcome == "influence"),
                        usize::from(outcome == "draw")
                    ),
                    "{die:?}, face {face}, Hero {owner}"
                );
            }
            let roll = decision.state.house_die_rolls().last().expect("audit");
            assert_eq!(
                (roll.die, roll.counter, roll.sides, roll.result),
                (die, play.state.prng_counter(), 6, face)
            );
            assert_eq!(
                apply_game_event(&play.state, &decision.event).expect("replay"),
                decision.state
            );
            codec::persisted_game_four_event(decision.event)
                .ok()
                .expect("persist event");
            let persisted = codec::persisted_after_decision(&snapshot, &decision.state);
            let restored = codec::command_domain_state(&persisted)
                .ok()
                .expect("restore face");
            // Persistence orders each owner's pile; unrelated owners share no stack order.
            assert_eq!(
                serde_json::to_value(codec::persisted_after_decision(&persisted, &restored))
                    .expect("restored facts"),
                serde_json::to_value(&persisted).expect("committed facts")
            );
            assert_eq!(restored.house_die_rolls(), decision.state.house_die_rolls());
        }
    }
}

#[test]
fn game_four_reviewed_purchase_policy_reaches_victory_with_each_player_count() {
    let policies: std::collections::BTreeMap<String, Vec<String>> = serde_json::from_str(
        include_str!("../../tests/fixtures/game-four/purchase-priority.json"),
    )
    .expect("policies");
    let seeds: std::collections::BTreeMap<String, u8> = serde_json::from_str(include_str!(
        "../../tests/fixtures/game-four/scenario-seeds.json"
    ))
    .expect("seeds");
    let mut outcomes = Vec::new();
    for count in [2, 3, 4] {
        let seed = seeds[&count.to_string()];
        let (mut state, _, rules) = super::game_one_tests::prepared_adventure_with_seed(
            count,
            crate::game_four_manifest(),
            "adventure:004",
            seed,
        );
        let mut commands = 0;
        while state.status() == game_domain::GameStatus::InProgress && commands < 600 {
            let decision = super::game_three_scenarios::next_decision(
                &state,
                &rules,
                &policies[&count.to_string()],
                seed,
            );
            assert_eq!(
                game_domain::apply_game_event(&state, &decision.event).expect("replay"),
                decision.state
            );
            state = decision.state;
            commands += 1;
        }
        outcomes.push((count, state.status(), commands, state.turn()));
    }
    assert!(
        outcomes
            .iter()
            .all(|(_, status, _, _)| *status == game_domain::GameStatus::Won),
        "{outcomes:?}"
    );
}

pub(super) fn graveyard_scenario() -> (
    game_domain::InitialGameState,
    super::PersistedSnapshot,
    game_domain::ValidatedGameRules,
) {
    let (state, participants, rules) = (0..16)
        .find_map(|seed| {
            let prepared = super::game_one_tests::prepared_adventure_with_seed(
                3,
                crate::game_four_manifest(),
                "adventure:004",
                seed,
            );
            (prepared.0.pending_choice().is_none()).then_some(prepared)
        })
        .expect("opening without choices");
    let mut snapshot = codec::persisted_snapshot(&state, &participants);
    for entity in &mut snapshot.effects.entities {
        match entity.catalog_id.as_deref() {
            Some("location:009") => {
                entity.zone = "location_discard".into();
                entity.zone_index = Some(0);
            }
            Some("location:010") => {
                entity.zone = "active_location".into();
                entity.zone_index = Some(0);
                entity.resources.insert("control".into(), 6);
            }
            Some("location:011") => {
                entity.zone_index = Some(0);
            }
            _ => (),
        }
    }
    // Give the non-active Hero an Ally in hand, preserving the physical inventory.
    let ally = snapshot
        .effects
        .entities
        .iter()
        .position(|entity| {
            entity.owner_position == Some(2) && entity.catalog_id.as_deref() == Some("starter:010")
        })
        .expect("Hermione's Ally");
    if snapshot.effects.entities[ally].zone != "hero_hand" {
        let replacement = snapshot
            .effects
            .entities
            .iter()
            .position(|entity| entity.owner_position == Some(2) && entity.zone == "hero_hand")
            .expect("hand");
        let (zone, index) = (
            snapshot.effects.entities[ally].zone.clone(),
            snapshot.effects.entities[ally].zone_index,
        );
        snapshot.effects.entities[ally].zone = "hero_hand".into();
        snapshot.effects.entities[ally].zone_index =
            snapshot.effects.entities[replacement].zone_index;
        snapshot.effects.entities[replacement].zone = zone;
        snapshot.effects.entities[replacement].zone_index = index;
    }
    let second_ally = snapshot
        .effects
        .entities
        .iter_mut()
        .find(|entity| entity.catalog_id.as_deref() == Some("hogwarts-card:035"))
        .expect("Fleur");
    second_ally.owner_position = Some(2);
    second_ally.zone = "hero_hand".into();
    second_ally.zone_index = Some(5);
    // Ron's two Allies force a second independent choice after Hermione's discard.
    for entity in &mut snapshot.effects.entities {
        if matches!(
            entity.catalog_id.as_deref(),
            Some("hogwarts-card:033" | "hogwarts-card:034")
        ) {
            entity.owner_position = Some(3);
            entity.zone = "hero_hand".into();
            entity.zone_index = Some(6);
        }
    }
    // Zone indices are a dense ordering within each pile.
    let mut order = std::collections::BTreeMap::new();
    snapshot
        .effects
        .entities
        .sort_by_key(|entity| entity.zone_index);
    for entity in &mut snapshot.effects.entities {
        if entity.zone_index.is_some() {
            let next = order
                .entry((entity.zone.clone(), entity.owner_position))
                .or_insert(0_u16);
            entity.zone_index = Some(*next);
            *next += 1;
        }
    }
    let state = codec::command_domain_state(&snapshot)
        .ok()
        .expect("valid scenario");
    (state, snapshot, rules)
}

struct Face(game_domain::EffectDie, u8, usize);
impl game_domain::EffectRoller for Face {
    fn roll(&mut self, die: game_domain::EffectDie) -> Option<u8> {
        assert_eq!(die, self.0);
        self.2 += 1;
        Some(self.1)
    }
    fn sample_below(&mut self, _: u32) -> Option<u32> {
        panic!("the prepared draw piles contain cards")
    }
}

#[test]
fn all_six_heir_faces_resolve_the_adverse_effects_and_retain_the_roll_through_choices() {
    use game_domain::{EffectDie, EffectResource, GameEngine, GameIntentInput, PlayerIntent};
    let (state, snapshot, rules) = heir_scenario();
    for face in 1..=6 {
        let mut roller = Face(EffectDie::SlytherinV1, face, 0);
        let mut decision = GameEngine::new(&rules)
            .decide(
                GameIntentInput {
                    state: &state,
                    actor_position: 1,
                    expected_state_version: state.state_version(),
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut roller,
            )
            .expect("reveal Heir through the turn engine");
        assert_eq!(roller.2, 1);
        let roll = decision
            .state
            .house_die_rolls()
            .last()
            .expect("roll")
            .clone();
        assert_eq!(roll.result, face);
        assert_eq!(roll.purpose, "rule:g4-dark-arts-014-effect");
        assert_eq!(
            game_domain::apply_game_event(&state, &decision.event).expect("replay"),
            decision.state
        );
        let mut discards = 0;
        while let Some(choice) = decision.state.pending_choice() {
            assert_eq!(face, 3, "only the discard face creates choices");
            assert_eq!(choice.cause, "rule:g4-dark-arts-014-effect");
            let saved = codec::persisted_after_decision(&snapshot, &decision.state);
            let current = codec::command_domain_state(&saved)
                .ok()
                .expect("restore adverse face");
            decision = GameEngine::new(&rules)
                .decide(
                    GameIntentInput {
                        state: &current,
                        actor_position: choice.responsible_position,
                        expected_state_version: current.state_version(),
                        intent: PlayerIntent::ResolveChoice {
                            choice_id: choice.id.clone(),
                            selected_options: choice
                                .options
                                .iter()
                                .take(usize::from(choice.min))
                                .cloned()
                                .collect(),
                        },
                    },
                    &mut roller,
                )
                .expect("continue without rerolling");
            assert_eq!(
                game_domain::apply_game_event(&current, &decision.event).expect("replay choice"),
                decision.state
            );
            discards += 1;
        }
        assert_eq!(discards, if face == 3 { 2 } else { 0 });
        assert_eq!(roller.2, 1);
        assert_eq!(decision.state.house_die_rolls().last(), Some(&roll));
        for hero in decision
            .state
            .effect_world()
            .entities_in(EffectZone::Heroes)
        {
            assert_eq!(
                hero.resource(EffectResource::Health),
                if face >= 4 { 6 } else { 7 }
            );
        }
        for villain in decision
            .state
            .effect_world()
            .entities_in(EffectZone::ActiveVillains)
        {
            assert_eq!(
                villain.resource(EffectResource::Health),
                if face == 2 { 2 } else { 1 }
            );
        }
        let control = |state: &game_domain::InitialGameState| {
            state.effect_world().entities_in(EffectZone::ActiveLocation)[0]
                .resource(EffectResource::Control)
        };
        assert_eq!(
            control(&decision.state) - control(&state),
            u16::from(face == 1)
        );
        codec::persisted_game_four_event(decision.event)
            .ok()
            .expect("valid persisted final event");
    }
}

fn heir_scenario() -> (
    game_domain::InitialGameState,
    super::PersistedSnapshot,
    game_domain::ValidatedGameRules,
) {
    let (_, mut snapshot, rules, _) = house_die_scenario();
    let heir = snapshot
        .effects
        .entities
        .iter()
        .position(|entity| {
            entity.zone == "dark_arts_deck" && entity.catalog_id.as_deref() == Some("dark-arts:014")
        })
        .expect("Heir in deck");
    let top = snapshot
        .effects
        .entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity.zone == "dark_arts_deck")
        .min_by_key(|(_, entity)| entity.zone_index)
        .map(|(index, _)| index)
        .expect("deck top");
    let index = snapshot.effects.entities[heir].zone_index;
    snapshot.effects.entities[heir].zone_index = snapshot.effects.entities[top].zone_index;
    snapshot.effects.entities[top].zone_index = index;
    for entity in &mut snapshot.effects.entities {
        if entity.zone == "active_villains" {
            entity.resources.insert("health".into(), 1);
        }
    }
    let state = codec::command_domain_state(&snapshot)
        .ok()
        .expect("ordered Dark Arts scenario");
    (state, snapshot, rules)
}

fn house_die_scenario() -> (
    game_domain::InitialGameState,
    super::PersistedSnapshot,
    game_domain::ValidatedGameRules,
    String,
) {
    let (state, participants, rules) = (0..16)
        .find_map(|seed| {
            let prepared = super::game_one_tests::prepared_adventure_with_seed(
                2,
                crate::game_four_manifest(),
                "adventure:004",
                seed,
            );
            (prepared.0.pending_choice().is_none()).then_some(prepared)
        })
        .expect("stable opening");
    let mut snapshot = codec::persisted_snapshot(&state, &participants);
    let card = snapshot
        .effects
        .entities
        .iter_mut()
        .find(|entity| entity.catalog_id.as_deref() == Some("hogwarts-card:036"))
        .expect("Hogwarts: A History");
    let card_id = card.id.clone();
    card.zone = "hero_hand".into();
    card.zone_index = Some(99);
    card.owner_position = Some(1);
    let mut order = std::collections::BTreeMap::new();
    snapshot
        .effects
        .entities
        .sort_by_key(|entity| entity.zone_index);
    for entity in &mut snapshot.effects.entities {
        if entity.kind.as_deref() == Some("hero") {
            entity.resources.insert("health".into(), 7);
            entity.resources.insert("attack".into(), 0);
            entity.resources.insert("influence".into(), 0);
        }
        if entity.kind.as_deref() == Some("villain") {
            entity
                .turn_state
                .as_mut()
                .expect("Villain state")
                .suppressed_by = vec![1];
        }
        if entity.zone_index.is_some() {
            let next = order
                .entry((entity.zone.clone(), entity.owner_position))
                .or_insert(0_u16);
            entity.zone_index = Some(*next);
            *next += 1;
        }
    }
    let state = codec::command_domain_state(&snapshot)
        .ok()
        .expect("valid face scenario");
    (state, snapshot, rules, card_id)
}

fn house_faces() -> [(game_domain::EffectDie, [&'static str; 6]); 4] {
    use game_domain::EffectDie;
    // Published ordered faces, independently recorded in game-four-rules-v1.md.
    [
        (
            EffectDie::GryffindorV1,
            [
                "influence",
                "influence",
                "influence",
                "health",
                "draw",
                "attack",
            ],
        ),
        (
            EffectDie::HufflepuffV1,
            ["influence", "health", "health", "health", "draw", "attack"],
        ),
        (
            EffectDie::RavenclawV1,
            ["influence", "health", "draw", "draw", "draw", "attack"],
        ),
        (
            EffectDie::SlytherinV1,
            ["influence", "health", "draw", "attack", "attack", "attack"],
        ),
    ]
}
