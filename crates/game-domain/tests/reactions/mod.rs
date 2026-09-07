use super::*;
use game_domain::EffectReactionTrigger;

#[test]
fn acquisition_on_top_requires_a_matching_permission_played_by_the_buyer() {
    use game_domain::{CardAcquisitionDestination, EffectCardType};
    for category in [
        EffectCardType::Ally,
        EffectCardType::Item,
        EffectCardType::Spell,
    ] {
        let entities = vec![
            starter_card(
                "permission",
                "permission",
                1,
                "rule:permission",
                EffectZone::HeroHand,
            ),
            starter_card(
                "other-permission",
                "permission",
                2,
                "rule:permission",
                EffectZone::HeroPlayArea,
            ),
            starter_card("coins", "coins", 1, "rule:coins", EffectZone::HeroHand),
            hogwarts_card("market", "market", 2, EffectZone::Market),
        ];
        let rules = acquisition_rules(category);
        let state = advance_to_hero_action(&entities, &rules);
        let state = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: "coins".to_owned(),
                targets: vec![],
            },
            &rules,
        )
        .expect("generate influence")
        .state;
        let acquire = GameCommand::AcquireCard {
            card_id: "market".to_owned(),
            destination: CardAcquisitionDestination::DrawPile,
        };
        assert!(
            decide(&state, state.state_version(), acquire.clone(), &rules).is_err(),
            "another hero's permission is insufficient"
        );
        let played = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: "permission".to_owned(),
                targets: vec![],
            },
            &rules,
        )
        .expect("play acquisition permission")
        .state;
        let offer = legal_game_intentions(&played, 1, &rules)
            .acquisitions
            .remove(0);
        assert_eq!(
            offer.destinations,
            vec![
                CardAcquisitionDestination::DiscardPile,
                CardAcquisitionDestination::DrawPile
            ]
        );
        let acquired =
            decide(&played, played.state_version(), acquire, &rules).expect("choose deck top");
        assert_eq!(
            acquired.state.effect_world().entity_zone("market"),
            Some(EffectZone::HeroDrawPile)
        );
        assert_eq!(
            acquired
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Influence),
            Some(0)
        );
        assert_eq!(
            apply_game_event(&played, &acquired.event).expect("top-deck replay"),
            acquired.state
        );
    }
}

#[test]
fn beans_count_the_owners_allies_played_before_and_after_the_beans_once_each() {
    use game_domain::EffectCardType;
    let entities = vec![
        starter_card("ally-before", "ally", 1, "rule:ally", EffectZone::HeroHand),
        starter_card("ally-after", "ally", 1, "rule:ally", EffectZone::HeroHand),
        starter_card(
            "other-ally",
            "ally",
            2,
            "rule:ally",
            EffectZone::HeroPlayArea,
        ),
        starter_card("beans", "beans", 1, "rule:beans", EffectZone::HeroHand),
    ];
    let rules = vec![
        EffectRule {
            id: "rule:ally".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::CardType {
                card_type: EffectCardType::Ally,
            },
        },
        EffectRule {
            id: "rule:beans".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::CardType {
                        card_type: EffectCardType::Item,
                    },
                    EffectDefinition::Apply {
                        target: single_target_selector(
                            None,
                            EffectZone::Heroes,
                            EffectTargetOwner::Actor,
                        ),
                        operation: EffectOperation::GainAttackPerAllyPlayed { amount: 1 },
                    },
                    reaction(
                        EffectReactionTrigger::OwnerPlaysAlly,
                        EffectResource::Attack,
                        1,
                    ),
                ],
            },
        },
    ];
    let mut state = advance_to_hero_action(&entities, &rules);
    for (card, attack) in [("ally-before", 0), ("beans", 1), ("ally-after", 2)] {
        let played = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: card.to_owned(),
                targets: vec![],
            },
            &rules,
        )
        .expect("play ally or beans");
        assert_eq!(
            played
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Attack),
            Some(attack)
        );
        assert_eq!(
            apply_game_event(&state, &played.event).expect("ally replay"),
            played.state
        );
        state = played.state;
    }
}

#[test]
fn cloak_caps_each_dark_arts_loss_only_while_in_its_owners_hand() {
    for zone in [
        EffectZone::HeroHand,
        EffectZone::HeroPlayArea,
        EffectZone::HeroDiscardPile,
    ] {
        let entities = vec![
            starter_card("cloak", "cloak", 1, "rule:cloak", zone),
            EffectEntityPlacement::new(
                EffectEntity::new("dark", None)
                    .with_kind(EffectEntityKind::DarkArts)
                    .with_catalog_id("dark:test")
                    .with_effect_rule("rule:damage"),
                EffectZone::DarkArtsDeck,
            ),
        ];
        let mut loss = resource_rule(
            "rule:damage",
            None,
            EffectTargetOwner::Any,
            EffectResource::Health,
            -3,
        );
        let EffectDefinition::Apply { target, .. } = &mut loss.effect else {
            unreachable!()
        };
        target.max = 4;
        let rules = ValidatedGameRules::new(vec![
            EffectRule {
                id: "rule:cloak".to_owned(),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::HandDamageLimit { maximum: 1 },
            },
            EffectRule {
                id: "rule:reveal".to_owned(),
                trigger: EffectTrigger::DarkArts,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::RevealDarkArts,
            },
            EffectRule {
                effect: EffectDefinition::Sequence {
                    effects: vec![loss.effect.clone(), loss.effect],
                },
                ..loss
            },
        ])
        .expect("valid protection rules");
        let state = GameEngine::new(&rules)
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants(),
                    content: content(&entities),
                },
                &mut ScriptedRoller::empty(),
            )
            .expect("resolve actual dark card");
        assert_eq!(
            state
                .effect_world()
                .hero_resource(1, EffectResource::Health),
            Some(if zone == EffectZone::HeroHand { 8 } else { 4 })
        );
        assert_eq!(
            state
                .effect_world()
                .hero_resource(2, EffectResource::Health),
            Some(4)
        );
    }
}

fn reaction(
    trigger: EffectReactionTrigger,
    resource: EffectResource,
    amount: i16,
) -> EffectDefinition {
    EffectDefinition::Reaction {
        trigger,
        effect: Box::new(EffectDefinition::Apply {
            target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
            operation: EffectOperation::ModifyResource { resource, amount },
        }),
    }
}

#[test]
fn played_broom_rewards_each_defeat_and_olivers_choice_delays_final_victory() {
    let entities = vec![
        starter_card("broom", "broom", 1, "rule:broom", EffectZone::HeroHand),
        starter_card(
            "unused-broom",
            "broom",
            1,
            "rule:broom",
            EffectZone::HeroHand,
        ),
        starter_card(
            "oliver",
            "oliver",
            1,
            "rule:oliver",
            EffectZone::HeroPlayArea,
        ),
        active_villain("villain:last", "villain:last", 1),
    ];
    let rules = broom_and_oliver_rules();
    let state = advance_to_hero_action(&entities, &rules);
    let played = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "broom".to_owned(),
            targets: vec![],
        },
        &rules,
    )
    .expect("play broom")
    .state;
    assert_eq!(
        played
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(0)
    );
    let defeated = decide(
        &played,
        played.state_version(),
        GameCommand::AssignAttack {
            villain_id: "villain:last".to_owned(),
            amount: 1,
        },
        &rules,
    )
    .expect("defeat villain and trigger played cards");
    assert_eq!(
        apply_game_event(&played, &defeated.event).expect("defeat replay"),
        defeated.state
    );
    assert_eq!(defeated.state.status(), GameStatus::InProgress);
    let choice = defeated
        .state
        .pending_choice()
        .expect("Oliver healing target");
    assert_eq!(choice.cause, "rule:oliver");
    let selected = decide(
        &defeated.state,
        defeated.state.state_version(),
        GameCommand::ResolveChoice {
            choice_id: choice.id.clone(),
            selected_options: vec!["hero:2".to_owned()],
        },
        &rules,
    )
    .expect("resolve final passive choice");
    assert_eq!(
        selected
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Influence),
        Some(1)
    );
    assert_eq!(selected.state.status(), GameStatus::Won);
    assert_eq!(
        apply_game_event(&defeated.state, &selected.event).expect("choice replay"),
        selected.state
    );
}

#[test]
fn active_draco_reacts_to_each_actual_control_added_before_the_next_phase() {
    for initial_control in [0, 3, 4] {
        let entities = vec![
            EffectEntityPlacement::new(
                EffectEntity::location("location:test", "location:test", "rule:location", 4, 1)
                    .with_resource(EffectResource::Control, initial_control),
                EffectZone::ActiveLocation,
            ),
            EffectEntityPlacement::new(
                EffectEntity::villain("villain:draco", "villain:draco", "rule:draco", 6),
                EffectZone::ActiveVillains,
            ),
        ];
        let rules = ValidatedGameRules::new(vec![
            EffectRule {
                id: "rule:control".to_owned(),
                trigger: EffectTrigger::DarkArts,
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
                id: "rule:draco".to_owned(),
                trigger: EffectTrigger::Villains,
                order: 0,
                cost: vec![],
                effect: reaction(
                    EffectReactionTrigger::ControlAdded,
                    EffectResource::Health,
                    -2,
                ),
            },
        ])
        .expect("valid reactions");
        let state = GameEngine::new(&rules)
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants(),
                    content: content(&entities),
                },
                &mut ScriptedRoller::empty(),
            )
            .expect("reactive opening turn");
        let added = (4 - initial_control).min(2);
        assert_eq!(
            state
                .effect_world()
                .hero_resource(1, EffectResource::Health),
            Some(10 - added * 2)
        );
        assert_eq!(
            state
                .effect_world()
                .hero_resource(2, EffectResource::Health),
            Some(10)
        );
    }
}

#[test]
fn forced_discard_reactions_resume_for_the_discarder_and_control_damage_hits_the_active_hero() {
    let mut entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location("location:test", "location:test", "rule:location", 4, 1),
            EffectZone::ActiveLocation,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("villain:crabbe", "villain:crabbe", "rule:crabbe", 5),
            EffectZone::ActiveVillains,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("villain:draco", "villain:draco", "rule:draco", 6),
            EffectZone::ActiveVillains,
        ),
    ];
    for (id, owner, rule) in [
        ("coin-one", 1, "rule:coin"),
        ("coin-two", 1, "rule:coin"),
        ("remembrall", 2, "rule:remembrall"),
        ("coin-three", 2, "rule:coin"),
    ] {
        entities.push(starter_card(id, id, owner, rule, EffectZone::HeroHand));
    }
    let rules = forced_discard_rules();
    let engine = GameEngine::new(&rules);
    let mut state = engine
        .start(
            StartGameInput {
                actor_role: ParticipantRole::Host,
                participants: &participants(),
                content: content(&entities),
            },
            &mut ScriptedRoller::empty(),
        )
        .expect("opening choice");
    for selected in [None, Some("coin-one"), None, Some("remembrall")] {
        let pending = state.pending_choice().expect("pending selection");
        let selected = selected.map_or_else(|| pending.options[0].clone(), str::to_owned);
        let decision = engine
            .decide(
                GameIntentInput {
                    state: &state,
                    actor_position: pending.responsible_position,
                    expected_state_version: state.state_version(),
                    intent: PlayerIntent::ResolveChoice {
                        choice_id: pending.id.clone(),
                        selected_options: vec![selected],
                    },
                },
                &mut ScriptedRoller::empty(),
            )
            .expect("resume reactions");
        assert_eq!(
            apply_game_event(&state, &decision.event).expect("replay reaction choice"),
            decision.state
        );
        state = decision.state;
    }
    assert_eq!(
        state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(7)
    );
    assert_eq!(
        state
            .effect_world()
            .hero_resource(2, EffectResource::Health),
        Some(9)
    );
    assert_eq!(
        state
            .effect_world()
            .hero_resource(2, EffectResource::Influence),
        Some(2)
    );
    assert_eq!(state.phase(), GamePhase::HeroActions);
}

fn broom_and_oliver_rules() -> Vec<EffectRule> {
    vec![
        EffectRule {
            id: "rule:broom".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    resource_rule(
                        "unused",
                        None,
                        EffectTargetOwner::Actor,
                        EffectResource::Attack,
                        1,
                    )
                    .effect,
                    reaction(
                        EffectReactionTrigger::OwnerDefeatsVillain,
                        EffectResource::Influence,
                        1,
                    ),
                ],
            },
        },
        EffectRule {
            id: "rule:oliver".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 1,
            cost: vec![],
            effect: EffectDefinition::Reaction {
                trigger: EffectReactionTrigger::OwnerDefeatsVillain,
                effect: Box::new(
                    resource_rule(
                        "unused",
                        None,
                        EffectTargetOwner::Any,
                        EffectResource::Health,
                        2,
                    )
                    .effect,
                ),
            },
        },
    ]
}

fn forced_discard_rules() -> ValidatedGameRules {
    ValidatedGameRules::new(vec![
        EffectRule {
            id: "rule:dark".to_owned(),
            trigger: EffectTrigger::DarkArts,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Choice {
                audience: EffectChoiceAudience::EachHero,
                options: vec![
                    EffectDefinition::Apply {
                        target: single_target_selector(
                            None,
                            EffectZone::HeroHand,
                            EffectTargetOwner::Actor,
                        ),
                        operation: EffectOperation::Discard,
                    },
                    EffectDefinition::NoOp,
                ],
            },
        },
        EffectRule {
            id: "rule:crabbe".to_owned(),
            trigger: EffectTrigger::Villains,
            order: 0,
            cost: vec![],
            effect: reaction(
                EffectReactionTrigger::HeroForcedDiscard,
                EffectResource::Health,
                -1,
            ),
        },
        EffectRule {
            id: "rule:draco".to_owned(),
            trigger: EffectTrigger::Villains,
            order: 1,
            cost: vec![],
            effect: reaction(
                EffectReactionTrigger::ControlAdded,
                EffectResource::Health,
                -2,
            ),
        },
        EffectRule {
            id: "rule:remembrall".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    reaction(
                        EffectReactionTrigger::SelfForcedDiscard,
                        EffectResource::Influence,
                        2,
                    ),
                    EffectDefinition::Reaction {
                        trigger: EffectReactionTrigger::SelfForcedDiscard,
                        effect: Box::new(EffectDefinition::Apply {
                            target: single_target_selector(
                                None,
                                EffectZone::ActiveLocation,
                                EffectTargetOwner::Any,
                            ),
                            operation: EffectOperation::ModifyResource {
                                resource: EffectResource::Control,
                                amount: 1,
                            },
                        }),
                    },
                ],
            },
        },
    ])
    .expect("valid reactions")
}

fn acquisition_rules(category: game_domain::EffectCardType) -> Vec<EffectRule> {
    vec![
        resource_rule(
            "rule:coins",
            None,
            EffectTargetOwner::Actor,
            EffectResource::Influence,
            2,
        ),
        EffectRule {
            id: "rule:permission".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::TopDeckAcquisition {
                card_type: category,
            },
        },
        EffectRule {
            id: "rule:noop".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::CardType {
                card_type: category,
            },
        },
    ]
}
