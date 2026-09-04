use std::collections::VecDeque;

use game_domain::{
    ContentSelection, EffectChoiceAudience, EffectCondition, EffectDefinition, EffectEntity,
    EffectEntityKind, EffectEntityPlacement, EffectOperation, EffectResource, EffectRoller,
    EffectRule, EffectSelector, EffectTargetBinding, EffectTargetOwner, EffectTrigger, EffectZone,
    GameCommand, GameCommandDecision, GameCommandError, GameCommandInput, GameEngine,
    GameIntentInput, GamePhase, GameStatus, HeroId, InitialGameState, LobbyParticipant,
    ParticipantRole, PendingEffectChoiceKind, PlayerIntent, StartGameInput, ValidatedGameRules,
    apply_game_event, decide_game_command, initialize_game, legal_game_intentions,
};

struct ScriptedRoller {
    rolls: VecDeque<u8>,
}

impl ScriptedRoller {
    fn empty() -> Self {
        Self {
            rolls: VecDeque::new(),
        }
    }
}

impl EffectRoller for ScriptedRoller {
    fn roll(&mut self, _die: game_domain::EffectDie) -> Option<u8> {
        self.rolls.pop_front()
    }

    fn sample_below(&mut self, upper_bound: u32) -> Option<u32> {
        (upper_bound > 0).then_some(0)
    }
}

fn participants() -> Vec<LobbyParticipant> {
    vec![
        LobbyParticipant {
            role: ParticipantRole::Host,
            position: 1,
            hero: Some(HeroId::Harry),
            ready: true,
        },
        LobbyParticipant {
            role: ParticipantRole::Guest,
            position: 2,
            hero: Some(HeroId::Hermione),
            ready: true,
        },
    ]
}

fn content(entities: &[EffectEntityPlacement]) -> ContentSelection<'_> {
    ContentSelection {
        adventure_id: "adventure:001",
        content_version: "fixture-v1",
        ruleset_version: "fixture-rules-v1",
        manifest_digest: "blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        manifest_version: 1,
        playable: true,
        initial_entities: entities,
    }
}

fn starter_card(
    id: &str,
    catalog_id: &str,
    owner_position: u8,
    effect_rule_id: &str,
    zone: EffectZone,
) -> EffectEntityPlacement {
    EffectEntityPlacement::new(
        EffectEntity::card(
            id,
            catalog_id,
            EffectEntityKind::StarterCard,
            Some(owner_position),
            effect_rule_id,
            None,
        ),
        zone,
    )
}

fn hogwarts_card(
    id: &str,
    catalog_id: &str,
    influence_cost: u16,
    zone: EffectZone,
) -> EffectEntityPlacement {
    EffectEntityPlacement::new(
        EffectEntity::card(
            id,
            catalog_id,
            EffectEntityKind::HogwartsCard,
            None,
            "rule:noop",
            Some(influence_cost),
        ),
        zone,
    )
}

fn active_villain(id: &str, catalog_id: &str, health: u16) -> EffectEntityPlacement {
    EffectEntityPlacement::new(
        EffectEntity::villain(id, catalog_id, "rule:villain", health),
        EffectZone::ActiveVillains,
    )
}

fn single_target_selector(
    id: Option<&str>,
    zone: EffectZone,
    owner: EffectTargetOwner,
) -> EffectSelector {
    EffectSelector {
        id: id.map(str::to_owned),
        zone,
        owner,
        min: 1,
        max: 1,
        eligibility: vec![],
    }
}

fn resource_rule(
    id: &str,
    selector_id: Option<&str>,
    owner: EffectTargetOwner,
    resource: EffectResource,
    amount: i16,
) -> EffectRule {
    EffectRule {
        id: id.to_owned(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: single_target_selector(selector_id, EffectZone::Heroes, owner),
            operation: EffectOperation::ModifyResource { resource, amount },
        },
    }
}

fn health_change(amount: i16) -> EffectDefinition {
    EffectDefinition::Apply {
        target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
        operation: EffectOperation::ModifyResource {
            resource: EffectResource::Health,
            amount,
        },
    }
}

fn decide(
    state: &InitialGameState,
    expected_state_version: u64,
    command: GameCommand,
    rules: &[EffectRule],
) -> Result<GameCommandDecision, GameCommandError> {
    let mut roller = ScriptedRoller::empty();
    decide_game_command(GameCommandInput {
        state,
        actor_position: 1,
        expected_state_version,
        command,
        effect_rules: rules,
        die_roller: &mut roller,
    })
}

fn entity_ids_in(state: &InitialGameState, zone: EffectZone) -> Vec<&str> {
    state
        .effect_world()
        .entities_in(zone)
        .iter()
        .map(EffectEntity::id)
        .collect()
}

fn target_binding(selector_id: &str, target_id: &str) -> EffectTargetBinding {
    EffectTargetBinding {
        selector_id: selector_id.to_owned(),
        target_ids: vec![target_id.to_owned()],
    }
}

fn advance_to_hero_action(
    entities: &[EffectEntityPlacement],
    rules: &[EffectRule],
) -> game_domain::InitialGameState {
    let participants = participants();
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(entities),
    })
    .expect("the synthetic validated game should start");
    decide(&initial, 1, GameCommand::CompleteDarkArts, rules)
        .expect("the active player should reach hero actions")
        .state
}

#[test]
fn playing_an_owned_card_moves_only_that_instance_and_resolves_its_bound_target() {
    let entities = vec![
        starter_card(
            "instance:owned-spell",
            "starter:synthetic-spell",
            1,
            "rule:synthetic-spell",
            EffectZone::HeroHand,
        ),
        starter_card(
            "instance:other-spell",
            "starter:synthetic-spell",
            1,
            "rule:synthetic-spell",
            EffectZone::HeroHand,
        ),
    ];
    let rules = vec![resource_rule(
        "rule:synthetic-spell",
        Some("target:ally"),
        EffectTargetOwner::Any,
        EffectResource::Influence,
        2,
    )];
    let lobby = participants();
    let dark_arts_state = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &lobby,
        content: content(&entities),
    })
    .expect("the synthetic setup should initialize");
    assert!(
        decide(
            &dark_arts_state,
            1,
            GameCommand::PlayCard {
                card_id: "instance:owned-spell".to_owned(),
                targets: vec![target_binding("target:ally", "hero:1")],
            },
            &rules,
        )
        .is_err()
    );
    let state = advance_to_hero_action(&entities, &rules);
    let mut before_ids = state.effect_world().entity_ids().collect::<Vec<_>>();
    before_ids.sort_unstable();
    let decision = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:owned-spell".to_owned(),
            targets: vec![target_binding("target:ally", "hero:2")],
        },
        &rules,
    )
    .expect("an owned eligible card and target should be accepted");

    assert_eq!(
        decision
            .state
            .effect_world()
            .entity_zone("instance:owned-spell"),
        Some(EffectZone::HeroPlayArea)
    );
    assert_eq!(
        decision
            .state
            .effect_world()
            .entity_zone("instance:other-spell"),
        Some(EffectZone::HeroHand)
    );
    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(2, EffectResource::Influence),
        Some(2)
    );
    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::HeroHand),
        vec!["instance:other-spell"]
    );
    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::HeroPlayArea),
        vec!["instance:owned-spell"]
    );
    let mut after_ids = decision
        .state
        .effect_world()
        .entity_ids()
        .collect::<Vec<_>>();
    after_ids.sort_unstable();
    assert_eq!(after_ids, before_ids);
    assert_eq!(
        apply_game_event(&state, &decision.event)
            .expect("the official event should replay the card action"),
        decision.state
    );
}

#[test]
fn playing_a_conditional_card_uses_only_the_selected_branch() {
    let entities = vec![starter_card(
        "instance:conditional-spell",
        "starter:conditional-spell",
        1,
        "rule:conditional-spell",
        EffectZone::HeroHand,
    )];
    let rules = vec![conditional_card_rule()];
    let state = advance_to_hero_action(&entities, &rules);
    let intentions = legal_game_intentions(&state, 1, &rules);
    let target_slots = &intentions.playable_cards[0].target_slots;

    assert_eq!(
        target_slots
            .iter()
            .map(|slot| slot.selector_id.as_str())
            .collect::<Vec<_>>(),
        vec!["target:eligible-branch", "target:ineligible-branch"]
    );

    for invalid_targets in [
        vec![
            target_binding("target:eligible-branch", "hero:2"),
            target_binding("target:ineligible-branch", "hero:1"),
            target_binding("target:extra", "hero:1"),
        ],
        vec![
            target_binding("target:eligible-branch", "hero:2"),
            target_binding("target:eligible-branch", "hero:1"),
        ],
    ] {
        assert_eq!(
            decide(
                &state,
                2,
                GameCommand::PlayCard {
                    card_id: "instance:conditional-spell".to_owned(),
                    targets: invalid_targets,
                },
                &rules,
            ),
            Err(GameCommandError::CommandNotLegal)
        );
    }

    let decision = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:conditional-spell".to_owned(),
            targets: vec![
                target_binding("target:eligible-branch", "hero:2"),
                target_binding("target:ineligible-branch", "hero:1"),
            ],
        },
        &rules,
    )
    .expect("all targets announced by the legal intention should be accepted atomically");

    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(2, EffectResource::Influence),
        Some(2)
    );
    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Attack),
        Some(0)
    );
}

#[test]
fn hero_health_cannot_exceed_its_maximum() {
    let entities = vec![starter_card(
        "instance:healing-spell",
        "starter:healing-spell",
        1,
        "rule:healing-spell",
        EffectZone::HeroHand,
    )];
    let rules = vec![resource_rule(
        "rule:healing-spell",
        None,
        EffectTargetOwner::Actor,
        EffectResource::Health,
        5,
    )];
    let state = advance_to_hero_action(&entities, &rules);

    let decision = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:healing-spell".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("healing at maximum health should remain a legal no-op");

    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(10)
    );
    assert_eq!(
        apply_game_event(&state, &decision.event)
            .expect("the capped health event should replay exactly"),
        decision.state
    );
}

#[test]
fn a_played_card_can_stun_its_owner_and_replay_the_consequences() {
    let entities = vec![
        starter_card(
            "card:stun",
            "starter:stun",
            1,
            "rule:stun",
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::location("location:one", "catalog:location", "rule:location", 3, 1),
            EffectZone::ActiveLocation,
        ),
    ];
    let rules = vec![resource_rule(
        "rule:stun",
        None,
        EffectTargetOwner::Actor,
        EffectResource::Health,
        -10,
    )];
    let state = advance_to_hero_action(&entities, &rules);
    let decision = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "card:stun".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("card damage must commit its stun consequences");
    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(0)
    );
    assert_eq!(
        decision
            .state
            .effect_world()
            .entity("location:one")
            .expect("location")
            .1
            .resource(EffectResource::Control),
        1
    );
    assert_eq!(
        apply_game_event(&state, &decision.event).expect("stun replay"),
        decision.state
    );
}

#[test]
fn paying_the_last_health_stuns_and_resumes_the_card_after_discard() {
    let entities = vec![
        starter_card(
            "card:cost",
            "starter:cost",
            1,
            "rule:cost",
            EffectZone::HeroHand,
        ),
        starter_card(
            "card:a",
            "starter:a",
            1,
            "rule:unused",
            EffectZone::HeroHand,
        ),
        starter_card(
            "card:b",
            "starter:b",
            1,
            "rule:unused",
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::location("location:cost", "catalog:location", "rule:location", 3, 1),
            EffectZone::ActiveLocation,
        ),
    ];
    let mut rule = resource_rule(
        "rule:cost",
        None,
        EffectTargetOwner::Actor,
        EffectResource::Influence,
        2,
    );
    rule.cost.push(game_domain::EffectResourceCost {
        resource: EffectResource::Health,
        amount: 10,
    });
    let rules = vec![rule];
    let state = advance_to_hero_action(&entities, &rules);
    let paid = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "card:cost".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("health cost");
    let pending = paid
        .state
        .pending_choice()
        .expect("paying the last health must open stun discard");
    assert_eq!(pending.kind, PendingEffectChoiceKind::StunDiscard);
    assert_eq!(
        paid.state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(0)
    );
    let resolved = decide(
        &paid.state,
        3,
        GameCommand::ResolveChoice {
            choice_id: pending.id.clone(),
            selected_options: vec![pending.options[0].clone()],
        },
        &rules,
    )
    .expect("resume card after stun");
    assert_eq!(
        resolved
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(0)
    );
    assert_eq!(
        resolved
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(2)
    );
    assert_eq!(
        resolved
            .state
            .effect_world()
            .entity("location:cost")
            .expect("location")
            .1
            .resource(EffectResource::Control),
        1
    );
    assert_eq!(
        apply_game_event(&paid.state, &resolved.event).expect("cost choice replay"),
        resolved.state
    );
}

#[test]
fn card_damage_defeats_the_last_villain_and_resolves_its_reward_before_victory() {
    let entities = vec![
        starter_card(
            "card:damage",
            "starter:damage",
            1,
            "rule:damage",
            EffectZone::HeroHand,
        ),
        reward_villain("villain:damage", 1, "rule:damage-reward"),
    ];
    let rules = vec![
        EffectRule {
            id: "rule:damage".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: single_target_selector(
                    Some("target:shared"),
                    EffectZone::ActiveVillains,
                    EffectTargetOwner::Any,
                ),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Health,
                    amount: -1,
                },
            },
        },
        EffectRule {
            id: "rule:damage-reward".to_owned(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::Apply {
                        target: single_target_selector(
                            Some("target:shared"),
                            EffectZone::Heroes,
                            EffectTargetOwner::Actor,
                        ),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Health,
                            amount: 1,
                        },
                    },
                    EffectDefinition::Choice {
                        audience: EffectChoiceAudience::Actor,
                        options: vec![health_change(1), health_change(2)],
                    },
                ],
            },
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let defeated = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "card:damage".to_owned(),
            targets: vec![target_binding("target:shared", "villain:damage")],
        },
        &rules,
    )
    .expect("card damage");
    assert_eq!(
        defeated.state.effect_world().entity_zone("villain:damage"),
        Some(EffectZone::VillainDiscard)
    );
    let pending = defeated
        .state
        .pending_choice()
        .expect("reward must precede victory");
    let won = decide(
        &defeated.state,
        3,
        GameCommand::ResolveChoice {
            choice_id: pending.id.clone(),
            selected_options: vec![pending.options[0].clone()],
        },
        &rules,
    )
    .expect("reward resolution");
    assert_eq!(won.state.status(), GameStatus::Won);
    assert_eq!(
        apply_game_event(&defeated.state, &won.event).expect("reward replay"),
        won.state
    );
}

#[test]
fn choosing_card_damage_finishes_victory_after_the_villain_reward() {
    let entities = vec![
        starter_card(
            "card:choice-damage",
            "starter:choice-damage",
            1,
            "rule:choice-damage",
            EffectZone::HeroHand,
        ),
        reward_villain("villain:choice-damage", 1, "rule:choice-reward"),
    ];
    let rules = vec![
        EffectRule {
            id: "rule:choice-damage".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Choice {
                audience: EffectChoiceAudience::Actor,
                options: vec![
                    EffectDefinition::Apply {
                        target: single_target_selector(
                            None,
                            EffectZone::ActiveVillains,
                            EffectTargetOwner::Any,
                        ),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Health,
                            amount: -1,
                        },
                    },
                    health_change(1),
                ],
            },
        },
        EffectRule {
            id: "rule:choice-reward".to_owned(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: health_change(1),
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let played = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "card:choice-damage".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("card choice");
    let pending = played.state.pending_choice().expect("damage choice");
    let won = decide(
        &played.state,
        3,
        GameCommand::ResolveChoice {
            choice_id: pending.id.clone(),
            selected_options: vec![pending.options[0].clone()],
        },
        &rules,
    )
    .expect("chosen damage and reward");
    assert_eq!(won.state.status(), GameStatus::Won);
    assert_eq!(
        apply_game_event(&played.state, &won.event).expect("card choice replay"),
        won.state
    );
}

#[test]
fn terminal_effect_in_the_next_turn_keeps_the_new_actor_and_replays() {
    let state = advance_to_hero_action(&[], &[]);
    let validated = ValidatedGameRules::new(vec![EffectRule {
        id: "rule:terminal-next-turn".to_owned(),
        trigger: EffectTrigger::DarkArts,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Terminal {
            outcome: game_domain::EffectGameOutcome::Lost,
        },
    }])
    .expect("terminal rule");
    let decision = GameEngine::new(&validated)
        .decide(
            GameIntentInput {
                state: &state,
                actor_position: 1,
                expected_state_version: 2,
                intent: PlayerIntent::EndHeroActions,
            },
            &mut ScriptedRoller::empty(),
        )
        .expect("next turn terminal");
    assert_eq!(decision.state.status(), GameStatus::Lost);
    assert_eq!(decision.state.active_position(), 2);
    assert_eq!(decision.state.turn(), 2);
    assert_eq!(
        apply_game_event(&state, &decision.event).expect("next turn terminal replay"),
        decision.state
    );
}

#[test]
fn control_cannot_exceed_the_active_location_limit() {
    let entities = vec![EffectEntityPlacement::new(
        EffectEntity::location(
            "instance:location-one",
            "location:one",
            "rule:location-one",
            2,
            1,
        ),
        EffectZone::ActiveLocation,
    )];
    let rules = vec![EffectRule {
        id: "rule:add-control".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: EffectSelector {
                id: None,
                zone: EffectZone::ActiveLocation,
                owner: EffectTargetOwner::Any,
                min: 1,
                max: 1,
                eligibility: vec![],
            },
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Control,
                amount: 5,
            },
        },
    }];

    let state = advance_to_hero_action(&entities, &rules);
    let (_, location) = state
        .effect_world()
        .entity("instance:location-one")
        .expect("the active location should remain in the world");

    assert_eq!(location.resource(EffectResource::Control), 2);
    assert_eq!(location.resource_limit(EffectResource::Control), Some(2));
}

#[test]
fn ending_the_turn_advances_a_controlled_location_in_ruleset_order() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location(
                "instance:location-one",
                "location:one",
                "rule:location-one",
                2,
                1,
            ),
            EffectZone::ActiveLocation,
        ),
        EffectEntityPlacement::new(
            EffectEntity::location(
                "instance:location-two",
                "location:two",
                "rule:location-two",
                3,
                2,
            ),
            EffectZone::LocationDeck,
        ),
    ];
    let rules = vec![EffectRule {
        id: "rule:add-control".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: EffectSelector {
                id: None,
                zone: EffectZone::ActiveLocation,
                owner: EffectTargetOwner::Any,
                min: 1,
                max: 1,
                eligibility: vec![],
            },
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Control,
                amount: 2,
            },
        },
    }];
    let state = advance_to_hero_action(&entities, &rules);
    let validated = ValidatedGameRules::new(rules).expect("the fixture rules should be valid");
    let mut roller = ScriptedRoller::empty();

    let decision = GameEngine::new(&validated)
        .decide(
            GameIntentInput {
                state: &state,
                actor_position: 1,
                expected_state_version: 2,
                intent: PlayerIntent::EndHeroActions,
            },
            &mut roller,
        )
        .expect("ending the turn should advance the controlled location");

    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::LocationDiscard),
        vec!["instance:location-one"]
    );
    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::ActiveLocation),
        vec!["instance:location-two"]
    );
    assert!(entity_ids_in(&decision.state, EffectZone::LocationDeck).is_empty());
    assert_eq!(
        apply_game_event(&state, &decision.event)
            .expect("the location advancement should replay exactly"),
        decision.state
    );
}

#[test]
fn controlling_the_final_location_ends_the_turn_in_defeat() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location(
                "instance:final-location",
                "location:final",
                "rule:final-location",
                1,
                1,
            ),
            EffectZone::ActiveLocation,
        ),
        active_villain("instance:remaining-villain", "villain:remaining", 2),
    ];
    let rules = vec![EffectRule {
        id: "rule:add-final-control".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: EffectSelector {
                id: None,
                zone: EffectZone::ActiveLocation,
                owner: EffectTargetOwner::Any,
                min: 1,
                max: 1,
                eligibility: vec![],
            },
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Control,
                amount: 1,
            },
        },
    }];
    let state = advance_to_hero_action(&entities, &rules);
    let validated = ValidatedGameRules::new(rules).expect("the fixture rules should be valid");
    let mut roller = ScriptedRoller::empty();

    let decision = GameEngine::new(&validated)
        .decide(
            GameIntentInput {
                state: &state,
                actor_position: 1,
                expected_state_version: 2,
                intent: PlayerIntent::EndHeroActions,
            },
            &mut roller,
        )
        .expect("controlling the final location should commit the defeat");

    assert_eq!(decision.state.status(), GameStatus::Lost);
    assert_eq!(decision.state.phase(), GamePhase::EndTurn);
    assert_eq!(decision.state.turn(), 1);
    assert!(decision.state.decision_point().is_none());
    assert_eq!(
        apply_game_event(&state, &decision.event).expect("the terminal turn should replay exactly"),
        decision.state
    );
}

#[test]
fn reaching_zero_health_stuns_without_eliminating_the_hero() {
    let entities = vec![EffectEntityPlacement::new(
        EffectEntity::location("instance:location", "location:one", "rule:location", 2, 1),
        EffectZone::ActiveLocation,
    )];
    let rules = vec![EffectRule {
        id: "rule:stun".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Sequence {
            effects: vec![
                EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Attack,
                        amount: 2,
                    },
                },
                EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Influence,
                        amount: 3,
                    },
                },
                EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Health,
                        amount: -10,
                    },
                },
            ],
        },
    }];
    let participants = participants();
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(&entities),
    })
    .expect("the fixture game should start");

    let decision = decide(&initial, 1, GameCommand::CompleteDarkArts, &rules)
        .expect("the stun consequences should resolve without eliminating the hero");

    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(0)
    );
    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Attack),
        Some(0)
    );
    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(0)
    );
    let (_, location) = decision
        .state
        .effect_world()
        .entity("instance:location")
        .expect("the current location should remain active");
    assert_eq!(location.resource(EffectResource::Control), 1);
    assert_eq!(decision.state.status(), GameStatus::InProgress);
    assert_eq!(decision.state.active_position(), 1);
    assert_eq!(
        apply_game_event(&initial, &decision.event)
            .expect("the stun consequences should replay exactly"),
        decision.state
    );
}

#[test]
fn stunned_hero_cannot_gain_or_lose_more_health_during_the_same_turn() {
    let entities = vec![EffectEntityPlacement::new(
        EffectEntity::location("instance:location", "location:one", "rule:location", 3, 1),
        EffectZone::ActiveLocation,
    )];
    let rules = vec![EffectRule {
        id: "rule:stun-then-modify-health".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Sequence {
            effects: vec![health_change(-10), health_change(5), health_change(-2)],
        },
    }];
    let participants = participants();
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(&entities),
    })
    .expect("the fixture game should start");

    let decision = decide(&initial, 1, GameCommand::CompleteDarkArts, &rules)
        .expect("health changes after stun should resolve as no-ops");

    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(0)
    );
    let (_, location) = decision
        .state
        .effect_world()
        .entity("instance:location")
        .expect("the current location should remain active");
    assert_eq!(location.resource(EffectResource::Control), 1);
}

#[test]
fn stun_control_on_the_final_location_is_resolved_at_the_end_of_turn() {
    let entities = vec![EffectEntityPlacement::new(
        EffectEntity::location(
            "instance:final-location",
            "location:final",
            "rule:location",
            1,
            1,
        ),
        EffectZone::ActiveLocation,
    )];
    let rules = vec![EffectRule {
        id: "rule:stun".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Health,
                amount: -10,
            },
        },
    }];
    let participants = participants();
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(&entities),
    })
    .expect("the fixture game should start");

    let stunned = decide(&initial, 1, GameCommand::CompleteDarkArts, &rules)
        .expect("the stun should finish its effect window");

    assert_eq!(stunned.state.status(), GameStatus::InProgress);
    assert_eq!(stunned.state.phase(), GamePhase::HeroActions);
    let validated = ValidatedGameRules::new(rules).expect("the fixture rules should be valid");
    let mut roller = ScriptedRoller::empty();
    let decision = GameEngine::new(&validated)
        .decide(
            GameIntentInput {
                state: &stunned.state,
                actor_position: 1,
                expected_state_version: 2,
                intent: PlayerIntent::EndHeroActions,
            },
            &mut roller,
        )
        .expect("the end-turn location check should commit defeat");

    assert_eq!(decision.state.status(), GameStatus::Lost);
    assert_eq!(decision.state.phase(), GamePhase::EndTurn);
    assert_eq!(
        apply_game_event(&stunned.state, &decision.event)
            .expect("the end-turn defeat should replay exactly"),
        decision.state
    );
}

#[test]
fn stunned_hero_chooses_half_their_hand_before_the_window_continues() {
    let mut entities = vec![EffectEntityPlacement::new(
        EffectEntity::location("instance:location", "location:one", "rule:location", 3, 1),
        EffectZone::ActiveLocation,
    )];
    entities.extend((1..=5).map(|index| {
        starter_card(
            &format!("instance:hand-{index}"),
            &format!("starter:hand-{index}"),
            1,
            "rule:unused",
            EffectZone::HeroHand,
        )
    }));
    let rules = vec![EffectRule {
        id: "rule:stun-with-hand".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Health,
                amount: -10,
            },
        },
    }];
    let participants = participants();
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(&entities),
    })
    .expect("the fixture game should start");

    let stunned = decide(&initial, 1, GameCommand::CompleteDarkArts, &rules)
        .expect("stunning with a hand should pause for its discard choice");
    let pending = stunned
        .state
        .pending_choice()
        .expect("the stunned hero should own a pending discard choice");

    assert_eq!(pending.kind, PendingEffectChoiceKind::StunDiscard);
    assert_eq!(pending.responsible_position, 1);
    assert_eq!(pending.min, 2);
    assert_eq!(pending.max, 2);
    assert_eq!(
        pending.options,
        vec![
            "instance:hand-1",
            "instance:hand-2",
            "instance:hand-3",
            "instance:hand-4",
            "instance:hand-5",
        ]
    );
    let (_, location_before_choice) = stunned
        .state
        .effect_world()
        .entity("instance:location")
        .expect("the current location should remain active");
    assert_eq!(location_before_choice.resource(EffectResource::Control), 0);
    assert_eq!(
        apply_game_event(&initial, &stunned.event)
            .expect("the pending stun choice should replay exactly"),
        stunned.state
    );

    let selected = pending.options[..2].to_vec();
    let resolved = decide(
        &stunned.state,
        2,
        GameCommand::ResolveChoice {
            choice_id: pending.id.clone(),
            selected_options: selected,
        },
        &rules,
    )
    .expect("the responsible hero should resolve the stun discard");

    assert!(resolved.state.pending_choice().is_none());
    assert_eq!(
        entity_ids_in(&resolved.state, EffectZone::HeroDiscardPile),
        vec!["instance:hand-1", "instance:hand-2"]
    );
    assert_eq!(
        entity_ids_in(&resolved.state, EffectZone::HeroHand),
        vec!["instance:hand-3", "instance:hand-4", "instance:hand-5"]
    );
    let (_, location) = resolved
        .state
        .effect_world()
        .entity("instance:location")
        .expect("the current location should remain active");
    assert_eq!(location.resource(EffectResource::Control), 1);
    assert_eq!(
        apply_game_event(&stunned.state, &resolved.event)
            .expect("the resolved stun choice should replay exactly"),
        resolved.state
    );
}

#[test]
fn resolving_a_stun_choice_defers_location_resolution_until_end_turn() {
    let mut entities = vec![EffectEntityPlacement::new(
        EffectEntity::location(
            "instance:final-location",
            "location:final",
            "rule:location",
            1,
            1,
        ),
        EffectZone::ActiveLocation,
    )];
    entities.extend((1..=2).map(|index| {
        starter_card(
            &format!("instance:hand-{index}"),
            &format!("starter:hand-{index}"),
            1,
            "rule:unused",
            EffectZone::HeroHand,
        )
    }));
    let rules = vec![EffectRule {
        id: "rule:stun-with-hand".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Health,
                amount: -10,
            },
        },
    }];
    let participants = participants();
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(&entities),
    })
    .expect("the fixture game should start");
    let stunned = decide(&initial, 1, GameCommand::CompleteDarkArts, &rules)
        .expect("the stun should pause for its mandatory discard");
    let pending = stunned
        .state
        .pending_choice()
        .expect("the stun discard should be pending");

    let decision = decide(
        &stunned.state,
        2,
        GameCommand::ResolveChoice {
            choice_id: pending.id.clone(),
            selected_options: vec![pending.options[0].clone()],
        },
        &rules,
    )
    .expect("finishing the stun should close the choice window");

    assert_eq!(decision.state.status(), GameStatus::InProgress);
    assert_eq!(decision.state.phase(), GamePhase::HeroActions);
    assert!(decision.state.pending_choice().is_none());
    assert_eq!(
        apply_game_event(&stunned.state, &decision.event)
            .expect("the terminal choice resolution should replay exactly"),
        decision.state
    );
}

#[test]
fn stunned_heroes_recover_at_the_end_of_the_active_heroes_turn() {
    let entities = vec![EffectEntityPlacement::new(
        EffectEntity::location("instance:location", "location:one", "rule:location", 3, 1),
        EffectZone::ActiveLocation,
    )];
    let rules = vec![EffectRule {
        id: "rule:stun".to_owned(),
        trigger: EffectTrigger::DarkArtsCompleted,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Health,
                amount: -10,
            },
        },
    }];
    let participants = participants();
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(&entities),
    })
    .expect("the fixture game should start");
    let stunned = decide(&initial, 1, GameCommand::CompleteDarkArts, &rules)
        .expect("the hero should become stunned");
    let validated = ValidatedGameRules::new(rules).expect("the fixture rules should be valid");
    let mut roller = ScriptedRoller::empty();

    let recovered = GameEngine::new(&validated)
        .decide(
            GameIntentInput {
                state: &stunned.state,
                actor_position: 1,
                expected_state_version: 2,
                intent: PlayerIntent::EndHeroActions,
            },
            &mut roller,
        )
        .expect("ending the turn should recover every stunned hero");

    assert_eq!(
        recovered
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(10)
    );
    assert_eq!(
        apply_game_event(&stunned.state, &recovered.event)
            .expect("the recovery should replay exactly"),
        recovered.state
    );
}

fn conditional_card_rule() -> EffectRule {
    EffectRule {
        id: "rule:conditional-spell".to_owned(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Condition {
            condition: EffectCondition::HasEligibleTarget {
                target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
            },
            then: Box::new(EffectDefinition::Apply {
                target: single_target_selector(
                    Some("target:eligible-branch"),
                    EffectZone::Heroes,
                    EffectTargetOwner::Any,
                ),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Influence,
                    amount: 2,
                },
            }),
            otherwise: Some(Box::new(EffectDefinition::Apply {
                target: single_target_selector(
                    Some("target:ineligible-branch"),
                    EffectZone::Heroes,
                    EffectTargetOwner::Any,
                ),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Attack,
                    amount: 3,
                },
            })),
        },
    }
}

#[test]
fn playing_a_card_can_pause_for_and_resume_an_effect_choice() {
    let entities = vec![starter_card(
        "instance:choice-spell",
        "starter:choice-spell",
        1,
        "rule:choice-spell",
        EffectZone::HeroHand,
    )];
    let rules = vec![EffectRule {
        id: "rule:choice-spell".to_owned(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Choice {
            audience: EffectChoiceAudience::Actor,
            options: vec![
                EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Attack,
                        amount: 1,
                    },
                },
                EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Influence,
                        amount: 1,
                    },
                },
            ],
        },
    }];
    let state = advance_to_hero_action(&entities, &rules);
    let played = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:choice-spell".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("the card should commit before its effect choice is answered");
    let pending = played
        .state
        .pending_choice()
        .expect("the card effect choice should remain pending");

    assert_eq!(
        played
            .state
            .effect_world()
            .entity_zone("instance:choice-spell"),
        Some(EffectZone::HeroPlayArea)
    );
    assert_eq!(
        apply_game_event(&state, &played.event)
            .expect("the played card event should replay its pending choice"),
        played.state
    );

    let selected_option = pending.options[1].clone();
    let mut roller = ScriptedRoller::empty();
    let resolved = decide_game_command(GameCommandInput {
        state: &played.state,
        actor_position: pending.responsible_position,
        expected_state_version: 3,
        command: GameCommand::ResolveChoice {
            choice_id: pending.id.clone(),
            selected_options: vec![selected_option],
        },
        effect_rules: &rules,
        die_roller: &mut roller,
    })
    .expect("the responsible participant should resume the card effect");

    assert!(resolved.state.pending_choice().is_none());
    assert_eq!(
        resolved
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(1)
    );
    assert_eq!(
        apply_game_event(&played.state, &resolved.event)
            .expect("the choice event should replay after the card event"),
        resolved.state
    );
}

#[test]
fn playable_card_targets_are_planned_after_the_card_enters_the_play_area() {
    let entities = vec![starter_card(
        "instance:self-discard",
        "starter:self-discard",
        1,
        "rule:self-discard",
        EffectZone::HeroHand,
    )];
    let rules = vec![EffectRule {
        id: "rule:self-discard".to_owned(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: single_target_selector(
                Some("target:played-card"),
                EffectZone::HeroPlayArea,
                EffectTargetOwner::Actor,
            ),
            operation: EffectOperation::Move {
                to: EffectZone::HeroDiscardPile,
            },
        },
    }];
    let state = advance_to_hero_action(&entities, &rules);
    let intentions = legal_game_intentions(&state, 1, &rules);

    assert_eq!(intentions.playable_cards.len(), 1);
    assert_eq!(intentions.playable_cards[0].target_slots.len(), 1);
    assert_eq!(
        intentions.playable_cards[0].target_slots[0].target_ids,
        vec!["instance:self-discard"]
    );

    let decision = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:self-discard".to_owned(),
            targets: vec![target_binding(
                "target:played-card",
                "instance:self-discard",
            )],
        },
        &rules,
    )
    .expect("the target announced after moving the card should execute successfully");

    assert_eq!(
        decision
            .state
            .effect_world()
            .entity_zone("instance:self-discard"),
        Some(EffectZone::HeroDiscardPile)
    );
}

#[test]
fn assigning_attack_spends_the_hero_resource_and_discards_a_defeated_villain() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::card(
                "instance:attack-card",
                "starter:synthetic-attack",
                EffectEntityKind::StarterCard,
                Some(1),
                "rule:synthetic-attack",
                None,
            ),
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain(
                "instance:villain",
                "villain:synthetic",
                "rule:synthetic-villain",
                2,
            ),
            EffectZone::ActiveVillains,
        ),
    ];
    let rules = vec![EffectRule {
        id: "rule:synthetic-attack".to_owned(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: EffectSelector {
                id: None,
                zone: EffectZone::Heroes,
                owner: EffectTargetOwner::Actor,
                min: 1,
                max: 1,
                eligibility: vec![],
            },
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Attack,
                amount: 2,
            },
        },
    }];
    let state = advance_to_hero_action(&entities, &rules);
    let mut roller = ScriptedRoller::empty();
    let after_card = decide_game_command(GameCommandInput {
        state: &state,
        actor_position: 1,
        expected_state_version: 2,
        command: GameCommand::PlayCard {
            card_id: "instance:attack-card".to_owned(),
            targets: vec![],
        },
        effect_rules: &rules,
        die_roller: &mut roller,
    })
    .expect("the attack card should resolve")
    .state;
    let mut roller = ScriptedRoller::empty();

    let decision = decide_game_command(GameCommandInput {
        state: &after_card,
        actor_position: 1,
        expected_state_version: 3,
        command: GameCommand::AssignAttack {
            villain_id: "instance:villain".to_owned(),
            amount: 2,
        },
        effect_rules: &rules,
        die_roller: &mut roller,
    })
    .expect("available attack should be assignable to an active villain");

    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Attack),
        Some(0)
    );
    let (zone, villain) = decision
        .state
        .effect_world()
        .entity("instance:villain")
        .expect("the villain instance must remain in the world");
    assert_eq!(zone, EffectZone::VillainDiscard);
    assert_eq!(villain.resource(EffectResource::Health), 0);
    assert_eq!(
        apply_game_event(&after_card, &decision.event)
            .expect("the attack event should replay exactly"),
        decision.state
    );
}

#[test]
fn defeating_a_villain_resolves_its_reward_in_the_same_window() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::card(
                "instance:attack-card",
                "starter:synthetic-attack",
                EffectEntityKind::StarterCard,
                Some(1),
                "rule:synthetic-attack",
                None,
            ),
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain(
                "instance:villain",
                "villain:synthetic",
                "rule:synthetic-villain",
                2,
            )
            .with_reward_rule("rule:synthetic-reward"),
            EffectZone::ActiveVillains,
        ),
    ];
    let rules = vec![
        EffectRule {
            id: "rule:synthetic-attack".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Attack,
                    amount: 2,
                },
            },
        },
        EffectRule {
            id: "rule:synthetic-reward".to_owned(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Influence,
                    amount: 1,
                },
            },
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let after_card = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:attack-card".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("the attack card should resolve")
    .state;

    let decision = decide(
        &after_card,
        3,
        GameCommand::AssignAttack {
            villain_id: "instance:villain".to_owned(),
            amount: 2,
        },
        &rules,
    )
    .expect("defeating the villain should resolve its reward atomically");

    assert_eq!(
        decision
            .state
            .effect_world()
            .entity_zone("instance:villain"),
        Some(EffectZone::VillainDiscard)
    );
    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(1)
    );
    assert_eq!(
        apply_game_event(&after_card, &decision.event)
            .expect("the villain reward window should replay exactly"),
        decision.state
    );
}

fn reward_villain(id: &str, health: u16, reward: &str) -> EffectEntityPlacement {
    EffectEntityPlacement::new(
        EffectEntity::villain(id, "villain:final", "rule:final-villain", health)
            .with_reward_rule(reward),
        EffectZone::ActiveVillains,
    )
}

#[test]
fn final_villain_reward_choice_resolves_before_victory_closes_the_window() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location(
                "instance:safe-location",
                "location:safe",
                "rule:safe-location",
                2,
                1,
            ),
            EffectZone::ActiveLocation,
        ),
        starter_card(
            "instance:attack-card",
            "starter:synthetic-attack",
            1,
            "rule:synthetic-attack",
            EffectZone::HeroHand,
        ),
        reward_villain("instance:final-villain", 1, "rule:choose-reward"),
    ];
    let rules = vec![
        resource_rule(
            "rule:synthetic-attack",
            None,
            EffectTargetOwner::Actor,
            EffectResource::Attack,
            1,
        ),
        EffectRule {
            id: "rule:choose-reward".to_owned(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Choice {
                audience: EffectChoiceAudience::Actor,
                options: vec![EffectDefinition::NoOp, EffectDefinition::NoOp],
            },
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let after_card = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:attack-card".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("the attack card should resolve")
    .state;
    let reward = decide(
        &after_card,
        3,
        GameCommand::AssignAttack {
            villain_id: "instance:final-villain".to_owned(),
            amount: 1,
        },
        &rules,
    )
    .expect("the defeated villain should open its reward choice");
    let pending = reward
        .state
        .pending_choice()
        .expect("victory must wait for the reward choice");
    assert_eq!(reward.state.status(), GameStatus::InProgress);
    assert_eq!(
        reward
            .state
            .effect_world()
            .entity_zone("instance:final-villain"),
        Some(EffectZone::VillainDiscard)
    );

    let validated =
        ValidatedGameRules::new(rules).expect("the reward fixture rules should be valid");
    let mut roller = ScriptedRoller::empty();
    let victory = GameEngine::new(&validated)
        .decide(
            GameIntentInput {
                state: &reward.state,
                actor_position: 1,
                expected_state_version: 4,
                intent: PlayerIntent::ResolveChoice {
                    choice_id: pending.id.clone(),
                    selected_options: vec![pending.options[0].clone()],
                },
            },
            &mut roller,
        )
        .expect("the reward choice should resolve before victory");

    assert_eq!(victory.state.status(), GameStatus::Won);
    assert!(victory.state.pending_choice().is_none());
    assert!(victory.state.decision_point().is_none());
    assert_eq!(
        apply_game_event(&reward.state, &victory.event)
            .expect("the terminal reward choice should replay exactly"),
        victory.state
    );
}

#[test]
fn defeating_the_final_villain_ends_the_game_in_victory() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location(
                "instance:safe-location",
                "location:safe",
                "rule:safe-location",
                2,
                1,
            ),
            EffectZone::ActiveLocation,
        ),
        EffectEntityPlacement::new(
            EffectEntity::card(
                "instance:attack-card",
                "starter:synthetic-attack",
                EffectEntityKind::StarterCard,
                Some(1),
                "rule:synthetic-attack",
                None,
            ),
            EffectZone::HeroHand,
        ),
        reward_villain("instance:final-villain", 1, "rule:final-reward"),
    ];
    let rules = vec![
        resource_rule(
            "rule:synthetic-attack",
            None,
            EffectTargetOwner::Actor,
            EffectResource::Attack,
            1,
        ),
        EffectRule {
            id: "rule:final-reward".to_owned(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::NoOp,
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let after_card = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:attack-card".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("the attack card should resolve")
    .state;

    let decision = decide(
        &after_card,
        3,
        GameCommand::AssignAttack {
            villain_id: "instance:final-villain".to_owned(),
            amount: 1,
        },
        &rules,
    )
    .expect("defeating the final villain should commit victory");

    assert_eq!(decision.state.status(), GameStatus::Won);
    assert!(decision.state.decision_point().is_none());
    assert_eq!(
        apply_game_event(&after_card, &decision.event)
            .expect("the victory event should replay exactly"),
        decision.state
    );
}

#[test]
fn defeat_precedes_victory_when_both_are_true_in_the_same_window() {
    assert_last_location_attack(1, EffectDefinition::NoOp, GameStatus::Lost);
}

#[test]
fn nonlethal_attack_does_not_resolve_the_last_location_before_end_turn() {
    assert_last_location_attack(2, EffectDefinition::NoOp, GameStatus::InProgress);
}

#[test]
fn structural_defeat_overrides_explicit_victory_in_the_reward() {
    assert_last_location_attack(
        1,
        EffectDefinition::Terminal {
            outcome: game_domain::EffectGameOutcome::Won,
        },
        GameStatus::Lost,
    );
}

#[test]
fn villain_reward_can_stun_a_hero_in_the_same_window() {
    assert_last_location_attack(1, health_change(-10), GameStatus::Lost);
}

fn assert_last_location_attack(health: u16, reward: EffectDefinition, expected_status: GameStatus) {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location(
                "instance:final-location",
                "location:final",
                "rule:final-location",
                1,
                1,
            ),
            EffectZone::ActiveLocation,
        ),
        starter_card(
            "instance:last-stand",
            "starter:last-stand",
            1,
            "rule:last-stand",
            EffectZone::HeroHand,
        ),
        reward_villain("instance:final-villain", health, "rule:final-reward"),
    ];
    let rules = vec![
        EffectRule {
            id: "rule:last-stand".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::Apply {
                        target: single_target_selector(
                            None,
                            EffectZone::Heroes,
                            EffectTargetOwner::Actor,
                        ),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Attack,
                            amount: 1,
                        },
                    },
                    EffectDefinition::Apply {
                        target: single_target_selector(
                            None,
                            EffectZone::ActiveLocation,
                            EffectTargetOwner::Any,
                        ),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Control,
                            amount: 1,
                        },
                    },
                ],
            },
        },
        EffectRule {
            id: "rule:final-reward".to_owned(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: reward,
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let after_card = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:last-stand".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("the last stand card should resolve")
    .state;

    let decision = decide(
        &after_card,
        3,
        GameCommand::AssignAttack {
            villain_id: "instance:final-villain".to_owned(),
            amount: 1,
        },
        &rules,
    )
    .expect("the simultaneous terminal window should commit");

    assert_eq!(decision.state.status(), expected_status);
    assert_eq!(
        decision.state.decision_point().is_none(),
        expected_status == GameStatus::Lost
    );
    assert_eq!(
        apply_game_event(&after_card, &decision.event)
            .expect("the loss-precedence event should replay exactly"),
        decision.state
    );
}

#[test]
fn ending_the_turn_refills_defeated_villains_in_ruleset_order() {
    let entities = vec![
        starter_card(
            "instance:attack-card",
            "starter:attack-card",
            1,
            "rule:attack-card",
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain(
                "instance:first-villain",
                "villain:first",
                "rule:first-villain",
                1,
            )
            .with_reward_rule("rule:first-reward"),
            EffectZone::ActiveVillains,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain(
                "instance:second-villain",
                "villain:second",
                "rule:second-villain",
                2,
            )
            .with_reward_rule("rule:second-reward"),
            EffectZone::VillainDeck,
        ),
    ];
    let rules = vec![
        resource_rule(
            "rule:attack-card",
            None,
            EffectTargetOwner::Actor,
            EffectResource::Attack,
            1,
        ),
        EffectRule {
            id: "rule:first-reward".to_owned(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::NoOp,
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let after_card = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:attack-card".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("the attack card should resolve")
    .state;
    let after_villain = decide(
        &after_card,
        3,
        GameCommand::AssignAttack {
            villain_id: "instance:first-villain".to_owned(),
            amount: 1,
        },
        &rules,
    )
    .expect("the first villain should be defeated")
    .state;
    let validated = ValidatedGameRules::new(rules).expect("the fixture rules should be valid");
    let mut roller = ScriptedRoller::empty();

    let decision = GameEngine::new(&validated)
        .decide(
            GameIntentInput {
                state: &after_villain,
                actor_position: 1,
                expected_state_version: 4,
                intent: PlayerIntent::EndHeroActions,
            },
            &mut roller,
        )
        .expect("ending the turn should refill the villain slot");

    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::ActiveVillains),
        vec!["instance:second-villain"]
    );
    assert!(entity_ids_in(&decision.state, EffectZone::VillainDeck).is_empty());
    assert_eq!(
        apply_game_event(&after_villain, &decision.event)
            .expect("the villain refill should replay exactly"),
        decision.state
    );
}

#[test]
fn acquiring_a_payable_card_charges_influence_moves_ownership_and_refills_in_order() {
    let entities = vec![
        starter_card(
            "instance:influence-card",
            "starter:synthetic-influence",
            1,
            "rule:synthetic-influence",
            EffectZone::HeroHand,
        ),
        hogwarts_card(
            "instance:market-first",
            "hogwarts:market-first",
            3,
            EffectZone::Market,
        ),
        hogwarts_card(
            "instance:market-second",
            "hogwarts:market-second",
            2,
            EffectZone::Market,
        ),
        hogwarts_card(
            "instance:deck-first",
            "hogwarts:deck-first",
            1,
            EffectZone::HogwartsDeck,
        ),
        hogwarts_card(
            "instance:deck-second",
            "hogwarts:deck-second",
            4,
            EffectZone::HogwartsDeck,
        ),
    ];
    let rules = vec![resource_rule(
        "rule:synthetic-influence",
        None,
        EffectTargetOwner::Actor,
        EffectResource::Influence,
        4,
    )];
    let state = advance_to_hero_action(&entities, &rules);
    let after_card = decide(
        &state,
        2,
        GameCommand::PlayCard {
            card_id: "instance:influence-card".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("the influence card should resolve")
    .state;

    let decision = decide(
        &after_card,
        3,
        GameCommand::AcquireCard {
            card_id: "instance:market-first".to_owned(),
        },
        &rules,
    )
    .expect("a payable market card should be acquired atomically");

    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(1)
    );
    let (_, acquired) = decision
        .state
        .effect_world()
        .entity("instance:market-first")
        .expect("the acquired instance must still exist");
    assert_eq!(acquired.owner_position(), Some(1));
    assert_eq!(
        decision
            .state
            .effect_world()
            .entity_zone("instance:market-first"),
        Some(EffectZone::HeroDiscardPile)
    );
    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::Market),
        vec!["instance:market-second", "instance:deck-first"]
    );
    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::HogwartsDeck),
        vec!["instance:deck-second"]
    );
    assert_eq!(
        apply_game_event(&after_card, &decision.event)
            .expect("the acquisition event should replay exactly"),
        decision.state
    );
}

#[test]
fn legal_intentions_and_execution_reject_the_same_wrong_owner_target_and_resources() {
    let entities = vec![
        starter_card(
            "instance:owned",
            "starter:owned",
            1,
            "rule:targeted",
            EffectZone::HeroHand,
        ),
        starter_card(
            "instance:not-owned",
            "starter:not-owned",
            2,
            "rule:targeted",
            EffectZone::HeroHand,
        ),
        hogwarts_card(
            "instance:too-expensive",
            "hogwarts:too-expensive",
            1,
            EffectZone::Market,
        ),
        active_villain("instance:healthy-villain", "villain:healthy", 2),
    ];
    let rules = vec![resource_rule(
        "rule:targeted",
        Some("target:hero"),
        EffectTargetOwner::Any,
        EffectResource::Influence,
        1,
    )];
    let state = advance_to_hero_action(&entities, &rules);
    let intentions = legal_game_intentions(&state, 1, &rules);

    assert_eq!(
        intentions
            .playable_cards
            .iter()
            .map(|card| card.card_id.as_str())
            .collect::<Vec<_>>(),
        vec!["instance:owned"]
    );
    assert_eq!(intentions.playable_cards[0].target_slots.len(), 1);
    assert_eq!(
        intentions.playable_cards[0].target_slots[0].target_ids,
        vec!["hero:1", "hero:2"]
    );
    assert!(intentions.attack_targets.is_empty());
    assert!(intentions.acquisitions.is_empty());

    for command in [
        GameCommand::PlayCard {
            card_id: "instance:not-owned".to_owned(),
            targets: vec![target_binding("target:hero", "hero:1")],
        },
        GameCommand::PlayCard {
            card_id: "instance:owned".to_owned(),
            targets: vec![],
        },
        GameCommand::PlayCard {
            card_id: "instance:owned".to_owned(),
            targets: vec![target_binding("target:hero", "hero:missing")],
        },
        GameCommand::AssignAttack {
            villain_id: "instance:healthy-villain".to_owned(),
            amount: 1,
        },
        GameCommand::AcquireCard {
            card_id: "instance:too-expensive".to_owned(),
        },
    ] {
        assert!(decide(&state, 2, command, &rules).is_err());
    }
}

#[test]
fn initial_instances_preserve_stack_order_and_cannot_exist_in_two_zones() {
    let ordered = vec![
        EffectEntityPlacement::new(
            EffectEntity::card(
                "instance:first",
                "starter:first",
                EffectEntityKind::StarterCard,
                Some(1),
                "rule:first",
                None,
            ),
            EffectZone::HeroDrawPile,
        ),
        EffectEntityPlacement::new(
            EffectEntity::card(
                "instance:second",
                "starter:second",
                EffectEntityKind::StarterCard,
                Some(1),
                "rule:second",
                None,
            ),
            EffectZone::HeroDrawPile,
        ),
    ];
    let participants = participants();
    let state = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants,
        content: content(&ordered),
    })
    .expect("valid instances should initialize");

    assert_eq!(
        state
            .effect_world()
            .entities_in(EffectZone::HeroDrawPile)
            .iter()
            .map(EffectEntity::id)
            .collect::<Vec<_>>(),
        vec!["instance:first", "instance:second"]
    );

    let duplicated = vec![
        ordered[0].clone(),
        EffectEntityPlacement::new(
            EffectEntity::card(
                "instance:first",
                "starter:first",
                EffectEntityKind::StarterCard,
                Some(1),
                "rule:first",
                None,
            ),
            EffectZone::HeroDiscardPile,
        ),
    ];
    assert_eq!(
        initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: content(&duplicated),
        }),
        Err(game_domain::StartGameError::InvalidInitialEntities)
    );
}
