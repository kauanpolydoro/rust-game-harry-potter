use std::collections::VecDeque;

use game_domain::{
    ContentSelection, EffectChoiceAudience, EffectCondition, EffectDefinition, EffectEntity,
    EffectEntityKind, EffectEntityPlacement, EffectOperation, EffectResource, EffectRoller,
    EffectRule, EffectSelector, EffectTargetBinding, EffectTargetOwner, EffectTrigger, EffectZone,
    GameCommand, GameCommandDecision, GameCommandError, GameCommandInput, HeroId, InitialGameState,
    LobbyParticipant, ParticipantRole, StartGameInput, apply_game_event, decide_game_command,
    initialize_game, legal_game_intentions,
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
        cost: vec![],
        effect: EffectDefinition::Apply {
            target: single_target_selector(selector_id, EffectZone::Heroes, owner),
            operation: EffectOperation::ModifyResource { resource, amount },
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
fn playing_a_conditional_card_accepts_all_announced_targets_and_uses_only_the_selected_branch() {
    let entities = vec![starter_card(
        "instance:conditional-spell",
        "starter:conditional-spell",
        1,
        "rule:conditional-spell",
        EffectZone::HeroHand,
    )];
    let rules = vec![EffectRule {
        id: "rule:conditional-spell".to_owned(),
        trigger: EffectTrigger::Manual,
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
    }];
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
fn assigning_attack_spends_the_hero_resource_and_leaves_a_zero_health_villain_active() {
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
    assert_eq!(zone, EffectZone::ActiveVillains);
    assert_eq!(villain.resource(EffectResource::Health), 0);
    assert_eq!(
        apply_game_event(&after_card, &decision.event)
            .expect("the attack event should replay exactly"),
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
