use super::*;
use game_domain::{EffectCardType, EffectEligibility};

#[test]
fn lucius_heals_only_actual_damage_when_control_is_added() {
    let entities = vec![
        starter_card(
            "control",
            "control",
            1,
            "rule:control",
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("lucius", "lucius", "rule:lucius", 6).with_max_health(7),
            EffectZone::ActiveVillains,
        ),
        EffectEntityPlacement::new(
            EffectEntity::location("location", "location", "rule:location", 4, 1),
            EffectZone::ActiveLocation,
        ),
    ];
    let rules = vec![
        EffectRule {
            id: "rule:control".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: single_target_selector(
                    None,
                    EffectZone::ActiveLocation,
                    EffectTargetOwner::Any,
                ),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Control,
                    amount: 2,
                },
            },
        },
        EffectRule {
            id: "rule:lucius".into(),
            trigger: EffectTrigger::Villains,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Reaction {
                trigger: game_domain::EffectReactionTrigger::ControlAdded,
                effect: Box::new(EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::ActiveVillains,
                        EffectTargetOwner::Any,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Health,
                        amount: 1,
                    },
                }),
            },
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let decision = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "control".into(),
            targets: vec![],
        },
        &rules,
    )
    .expect("add two control");
    assert_eq!(
        decision
            .state
            .effect_world()
            .entity("lucius")
            .expect("Lucius")
            .1
            .resource(EffectResource::Health),
        7
    );
    assert_eq!(
        apply_game_event(&state, &decision.event).expect("replay"),
        decision.state
    );
}

#[test]
fn tom_riddle_counts_allies_before_discards_and_preserves_the_remaining_choices() {
    let entities = vec![
        starter_card("ally-one", "ally", 1, "rule:ally", EffectZone::HeroHand),
        starter_card("ally-two", "ally", 1, "rule:ally", EffectZone::HeroHand),
        starter_card("tom", "tom", 1, "rule:tom", EffectZone::HeroHand),
    ];
    let mut target = single_target_selector(None, EffectZone::HeroHand, EffectTargetOwner::Actor);
    target.min = 0;
    target.max = 32;
    target.eligibility.push(EffectEligibility::CardType {
        card_type: EffectCardType::Ally,
    });
    let discard = EffectDefinition::Apply {
        target: single_target_selector(None, EffectZone::HeroHand, EffectTargetOwner::Actor),
        operation: EffectOperation::Discard,
    };
    let rules = vec![
        EffectRule {
            id: "rule:ally".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::CardType {
                card_type: EffectCardType::Ally,
            },
        },
        EffectRule {
            id: "rule:tom".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::ForEachTarget {
                target,
                effect: Box::new(EffectDefinition::Choice {
                    audience: EffectChoiceAudience::Actor,
                    options: vec![
                        discard,
                        EffectDefinition::Apply {
                            target: single_target_selector(
                                None,
                                EffectZone::Heroes,
                                EffectTargetOwner::Actor,
                            ),
                            operation: EffectOperation::ModifyResource {
                                resource: EffectResource::Health,
                                amount: -2,
                            },
                        },
                    ],
                }),
            },
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let mut state = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "tom".into(),
            targets: vec![],
        },
        &rules,
    )
    .expect("count allies")
    .state;
    for selected in ["option:1", "ally-one", "option:1"] {
        let choice = state.pending_choice().expect("pending counted effect");
        let next = decide(
            &state,
            state.state_version(),
            GameCommand::ResolveChoice {
                choice_id: choice.id.clone(),
                selected_options: vec![selected.into()],
            },
            &rules,
        )
        .expect("resolve counted effect");
        assert_eq!(
            apply_game_event(&state, &next.event).expect("replay"),
            next.state
        );
        state = next.state;
    }
    assert!(state.pending_choice().is_none());
    assert!(
        state
            .effect_world()
            .cards_in_zone(1, EffectZone::HeroHand)
            .is_empty()
    );
    assert_eq!(
        state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(10)
    );
}

fn copying_ally_rules() -> Vec<EffectRule> {
    let actor = single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor);
    let gain = |resource, amount| EffectDefinition::Apply {
        target: actor.clone(),
        operation: EffectOperation::ModifyResource { resource, amount },
    };
    let mut ally = single_target_selector(
        Some("ally"),
        EffectZone::HeroPlayArea,
        EffectTargetOwner::Actor,
    );
    ally.eligibility.push(EffectEligibility::CardType {
        card_type: EffectCardType::Ally,
    });
    vec![
        EffectRule {
            id: "rule:ally".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::CardType {
                        card_type: EffectCardType::Ally,
                    },
                    EffectDefinition::Choice {
                        audience: EffectChoiceAudience::Actor,
                        options: vec![
                            gain(EffectResource::Attack, 1),
                            gain(EffectResource::Health, 2),
                        ],
                    },
                    EffectDefinition::Reaction {
                        trigger: game_domain::EffectReactionTrigger::OwnerDefeatsVillain,
                        effect: Box::new(gain(EffectResource::Influence, 2)),
                    },
                ],
            },
        },
        EffectRule {
            id: "rule:potion".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::CardType {
                        card_type: EffectCardType::Item,
                    },
                    EffectDefinition::Apply {
                        target: ally,
                        operation: EffectOperation::CopyPlayedAlly,
                    },
                ],
            },
        },
    ]
}

#[test]
fn polyjuice_copies_an_allys_choice_and_reward_without_playing_another_ally() {
    let entities = vec![
        starter_card("ally", "ally", 1, "rule:ally", EffectZone::HeroPlayArea),
        starter_card("potion", "potion", 1, "rule:potion", EffectZone::HeroHand),
        EffectEntityPlacement::new(
            EffectEntity::villain("villain", "villain", "rule:villain", 1),
            EffectZone::ActiveVillains,
        ),
    ];
    let rules = copying_ally_rules();
    let state = advance_to_hero_action(&entities, &rules);
    let copied = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "potion".into(),
            targets: vec![EffectTargetBinding {
                selector_id: "ally".into(),
                target_ids: vec!["ally".into()],
            }],
        },
        &rules,
    )
    .expect("copy ally");
    assert_eq!(
        apply_game_event(&state, &copied.event).expect("copy replay"),
        copied.state
    );
    let choice = copied.state.pending_choice().expect("copied ally choice");
    let chosen = decide(
        &copied.state,
        copied.state.state_version(),
        GameCommand::ResolveChoice {
            choice_id: choice.id.clone(),
            selected_options: vec!["option:1".into()],
        },
        &rules,
    )
    .expect("choose copied attack");
    let defeated = decide(
        &chosen.state,
        chosen.state.state_version(),
        GameCommand::AssignAttack {
            villain_id: "villain".into(),
            amount: 1,
        },
        &rules,
    )
    .expect("defeat villain");
    assert_eq!(
        defeated
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(4),
        "both original and copied rewards trigger"
    );
    assert_eq!(
        apply_game_event(&chosen.state, &defeated.event).expect("reward replay"),
        defeated.state
    );
}

#[test]
fn basilisk_blocks_extra_draws_and_releases_them_before_its_defeat_reward() {
    let entities = vec![
        starter_card("draw", "draw", 1, "rule:draw", EffectZone::HeroHand),
        starter_card("attack", "attack", 1, "rule:attack", EffectZone::HeroHand),
        starter_card("top", "top", 1, "rule:item", EffectZone::HeroDrawPile),
        EffectEntityPlacement::new(
            EffectEntity::villain("basilisk", "basilisk", "rule:basilisk", 2)
                .with_reward_rule("rule:reward"),
            EffectZone::ActiveVillains,
        ),
    ];
    let actor = single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor);
    let rules = vec![
        EffectRule {
            id: "rule:draw".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: actor.clone(),
                operation: EffectOperation::Draw { amount: 1 },
            },
        },
        EffectRule {
            id: "rule:attack".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: actor.clone(),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Attack,
                    amount: 2,
                },
            },
        },
        EffectRule {
            id: "rule:basilisk".into(),
            trigger: EffectTrigger::Villains,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::PreventExtraDrawing,
        },
        EffectRule {
            id: "rule:reward".into(),
            trigger: EffectTrigger::VillainReward,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: actor,
                operation: EffectOperation::Draw { amount: 1 },
            },
        },
    ];
    let mut state = advance_to_hero_action(&entities, &rules);
    for card in ["draw", "attack"] {
        state = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: card.into(),
                targets: vec![],
            },
            &rules,
        )
        .expect("play card")
        .state;
    }
    assert_eq!(
        state.effect_world().entity_zone("top"),
        Some(EffectZone::HeroDrawPile)
    );
    let decision = decide(
        &state,
        state.state_version(),
        GameCommand::AssignAttack {
            villain_id: "basilisk".into(),
            amount: 2,
        },
        &rules,
    )
    .expect("defeat Basilisk");
    assert_eq!(
        decision.state.effect_world().entity_zone("top"),
        Some(EffectZone::HeroHand)
    );
    assert_eq!(
        apply_game_event(&state, &decision.event).expect("replay"),
        decision.state
    );
}

#[test]
fn retrieving_an_ally_offers_only_the_owners_allies_and_replays_the_move() {
    let entities = vec![
        starter_card(
            "retrieve",
            "retrieve",
            1,
            "rule:retrieve",
            EffectZone::HeroHand,
        ),
        starter_card("ally", "ally", 1, "rule:ally", EffectZone::HeroDiscardPile),
        starter_card("item", "item", 1, "rule:item", EffectZone::HeroDiscardPile),
        starter_card(
            "other-ally",
            "ally",
            2,
            "rule:ally",
            EffectZone::HeroDiscardPile,
        ),
    ];
    let mut target = single_target_selector(
        Some("ally"),
        EffectZone::HeroDiscardPile,
        EffectTargetOwner::Actor,
    );
    target.eligibility.push(EffectEligibility::CardType {
        card_type: EffectCardType::Ally,
    });
    let rules = vec![
        EffectRule {
            id: "rule:retrieve".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target,
                operation: EffectOperation::Move {
                    to: EffectZone::HeroHand,
                },
            },
        },
        EffectRule {
            id: "rule:ally".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::CardType {
                card_type: EffectCardType::Ally,
            },
        },
        EffectRule {
            id: "rule:item".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::CardType {
                card_type: EffectCardType::Item,
            },
        },
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let legal = legal_game_intentions(&state, 1, &rules);
    assert_eq!(
        legal.playable_cards[0].target_slots[0].target_ids,
        vec!["ally"]
    );
    let decision = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "retrieve".into(),
            targets: vec![EffectTargetBinding {
                selector_id: "ally".into(),
                target_ids: vec!["ally".into()],
            }],
        },
        &rules,
    )
    .expect("retrieve ally");
    assert_eq!(
        decision.state.effect_world().entity_zone("ally"),
        Some(EffectZone::HeroHand)
    );
    assert_eq!(
        decision.state.effect_world().entity_zone("item"),
        Some(EffectZone::HeroDiscardPile)
    );
    assert_eq!(
        apply_game_event(&state, &decision.event).expect("replay"),
        decision.state
    );
}

#[test]
fn chamber_reveals_both_dark_arts_even_when_the_first_requires_a_choice() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location("chamber", "location:005", "rule:location", 5, 2),
            EffectZone::ActiveLocation,
        ),
        EffectEntityPlacement::new(
            EffectEntity::new("dark:first", None).with_effect_rule("rule:choice"),
            EffectZone::DarkArtsDeck,
        ),
        EffectEntityPlacement::new(
            EffectEntity::new("dark:second", None).with_effect_rule("rule:damage"),
            EffectZone::DarkArtsDeck,
        ),
    ];
    let mut choice = resource_rule(
        "rule:choice",
        None,
        EffectTargetOwner::Actor,
        EffectResource::Health,
        -1,
    );
    choice.effect = EffectDefinition::Choice {
        audience: EffectChoiceAudience::Actor,
        options: vec![choice.effect, EffectDefinition::NoOp],
    };
    let rules = ValidatedGameRules::new(vec![
        EffectRule {
            id: "rule:reveal".into(),
            trigger: EffectTrigger::DarkArts,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::RevealDarkArts,
        },
        choice,
        resource_rule(
            "rule:damage",
            None,
            EffectTargetOwner::Actor,
            EffectResource::Health,
            -2,
        ),
    ])
    .expect("rules");
    let players = participants();
    let engine = GameEngine::new(&rules);
    let state = engine
        .start(
            StartGameInput {
                actor_role: ParticipantRole::Host,
                participants: &players,
                content: content(&entities),
            },
            &mut ScriptedRoller::empty(),
        )
        .expect("opening choice");
    assert_eq!(
        entity_ids_in(&state, EffectZone::DarkArtsDiscard),
        vec!["dark:first"]
    );
    let choice = state.pending_choice().expect("first card choice");
    let resumed = engine
        .decide(
            GameIntentInput {
                state: &state,
                actor_position: 1,
                expected_state_version: state.state_version(),
                intent: PlayerIntent::ResolveChoice {
                    choice_id: choice.id.clone(),
                    selected_options: vec!["option:1".into()],
                },
            },
            &mut ScriptedRoller::empty(),
        )
        .expect("resume");
    assert_eq!(
        entity_ids_in(&resumed.state, EffectZone::DarkArtsDiscard),
        vec!["dark:first", "dark:second"]
    );
    assert_eq!(
        resumed
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(7)
    );
    assert_eq!(resumed.state.phase(), GamePhase::HeroActions);
    assert_eq!(
        apply_game_event(&state, &resumed.event).expect("replay"),
        resumed.state
    );
}
