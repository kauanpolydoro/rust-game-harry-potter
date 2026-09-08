use super::*;

#[test]
fn barty_blocks_control_removal_until_defeated_then_his_reward_removes_control() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location("location", "location", "rule:location", 6, 1)
                .with_resource(EffectResource::Control, 3),
            EffectZone::ActiveLocation,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("barty", "barty", "rule:barty", 1)
                .with_reward_rule("rule:reward"),
            EffectZone::ActiveVillains,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("remaining", "remaining", "rule:idle", 7),
            EffectZone::VillainDeck,
        ),
        starter_card("spell", "spell", 1, "rule:spell", EffectZone::HeroHand),
    ];
    let control = |amount| EffectDefinition::Apply {
        target: single_target_selector(None, EffectZone::ActiveLocation, EffectTargetOwner::Any),
        operation: EffectOperation::ModifyResource {
            resource: EffectResource::Control,
            amount,
        },
    };
    let rules = vec![
        rule(
            "rule:barty",
            EffectTrigger::Villains,
            EffectDefinition::PreventControlRemoval,
        ),
        rule("rule:idle", EffectTrigger::Villains, EffectDefinition::NoOp),
        rule("rule:reward", EffectTrigger::VillainReward, control(-2)),
        rule(
            "rule:spell",
            EffectTrigger::Manual,
            EffectDefinition::Sequence {
                effects: vec![
                    control(-1),
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
                ],
            },
        ),
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let play = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "spell".into(),
            targets: vec![],
        },
        &rules,
    )
    .expect("play spell");
    assert_eq!(
        play.state
            .effect_world()
            .entities_in(EffectZone::ActiveLocation)[0]
            .resource(EffectResource::Control),
        3
    );
    assert_eq!(
        apply_game_event(&state, &play.event).expect("replay blocked removal"),
        play.state
    );
    let defeat = decide(
        &play.state,
        play.state.state_version(),
        GameCommand::AssignAttack {
            villain_id: "barty".into(),
            amount: 1,
        },
        &rules,
    )
    .expect("defeat Barty");
    assert_eq!(
        defeat
            .state
            .effect_world()
            .entities_in(EffectZone::ActiveLocation)[0]
            .resource(EffectResource::Control),
        1
    );
    assert_eq!(
        apply_game_event(&play.state, &defeat.event).expect("replay reward"),
        defeat.state
    );
}

#[test]
fn an_extra_dark_arts_card_resolves_after_its_source_and_replays_the_whole_chain() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::location("location", "location", "rule:location", 6, 1),
            EffectZone::ActiveLocation,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("villain", "villain", "rule:idle", 7),
            EffectZone::ActiveVillains,
        ),
        EffectEntityPlacement::new(
            EffectEntity::card(
                "extra",
                "extra",
                EffectEntityKind::DarkArts,
                None,
                "rule:extra",
                None,
            ),
            EffectZone::DarkArtsDeck,
        ),
        EffectEntityPlacement::new(
            EffectEntity::card(
                "damage",
                "damage",
                EffectEntityKind::DarkArts,
                None,
                "rule:damage",
                None,
            ),
            EffectZone::DarkArtsDeck,
        ),
    ];
    let rules = vec![
        EffectRule {
            id: "rule:reveal".into(),
            trigger: EffectTrigger::DarkArtsCompleted,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::RevealDarkArts,
        },
        EffectRule {
            id: "rule:extra".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![health_change(-1), EffectDefinition::RevealExtraDarkArts],
            },
        },
        EffectRule {
            id: "rule:damage".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: health_change(-2),
        },
    ];
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &participants(),
        content: content(&entities),
    })
    .expect("fixture");
    let decision =
        decide(&initial, 1, GameCommand::CompleteDarkArts, &rules).expect("additional revelation");
    assert_eq!(
        decision
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(7)
    );
    assert_eq!(
        entity_ids_in(&decision.state, EffectZone::DarkArtsDiscard),
        ["extra", "damage"]
    );
    assert_eq!(
        apply_game_event(&initial, &decision.event).expect("replay chain"),
        decision.state
    );
}

#[test]
fn pensieve_gives_influence_to_both_selected_heroes_even_when_drawing_is_blocked() {
    for blocked in [false, true] {
        let entities = vec![
            starter_card(
                "pensieve",
                "pensieve",
                1,
                "rule:pensieve",
                EffectZone::HeroHand,
            ),
            starter_card("one", "card", 1, "rule:card", EffectZone::HeroDrawPile),
            starter_card("two", "card", 2, "rule:card", EffectZone::HeroDrawPile),
            EffectEntityPlacement::new(
                EffectEntity::villain("villain", "villain", "rule:block", 7),
                EffectZone::ActiveVillains,
            ),
        ];
        let mut target =
            single_target_selector(Some("heroes"), EffectZone::Heroes, EffectTargetOwner::Any);
        target.min = 2;
        target.max = 2;
        let rules = vec![
            EffectRule {
                id: "rule:pensieve".into(),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::Apply {
                    target,
                    operation: EffectOperation::GainInfluenceAndDraw {
                        influence: 1,
                        cards: 1,
                    },
                },
            },
            EffectRule {
                id: "rule:block".into(),
                trigger: EffectTrigger::Villains,
                order: 0,
                cost: vec![],
                effect: if blocked {
                    EffectDefinition::PreventExtraDrawing
                } else {
                    EffectDefinition::NoOp
                },
            },
        ];
        let initial = advance_to_hero_action(&entities, &rules);
        let decision = decide(
            &initial,
            initial.state_version(),
            GameCommand::PlayCard {
                card_id: "pensieve".into(),
                targets: vec![EffectTargetBinding {
                    selector_id: "heroes".into(),
                    target_ids: vec!["hero:1".into(), "hero:2".into()],
                }],
            },
            &rules,
        )
        .expect("play Pensieve");
        for position in [1, 2] {
            assert_eq!(
                decision
                    .state
                    .effect_world()
                    .hero_resource(position, EffectResource::Influence),
                Some(1)
            );
        }
        assert_eq!(
            entity_ids_in(&decision.state, EffectZone::HeroHand),
            if blocked { vec![] } else { vec!["one", "two"] }
        );
        assert_eq!(
            apply_game_event(&initial, &decision.event).expect("replay Pensieve"),
            decision.state
        );
    }
}

#[test]
fn fleur_heals_once_when_another_ally_is_played_before_or_after_her() {
    for fleur_first in [false, true] {
        let entities = vec![
            EffectEntityPlacement::new(
                EffectEntity::hero(1).with_resource(EffectResource::Health, 4),
                EffectZone::Heroes,
            ),
            EffectEntityPlacement::new(
                EffectEntity::card(
                    "fleur",
                    "fleur",
                    EffectEntityKind::HogwartsCard,
                    Some(1),
                    "rule:fleur",
                    Some(4),
                )
                .with_turn_state(Some(game_domain::EffectTurnState::default())),
                EffectZone::HeroHand,
            ),
            starter_card("one", "ally", 1, "rule:ally", EffectZone::HeroHand),
            starter_card("two", "ally", 1, "rule:ally", EffectZone::HeroHand),
        ];
        let ally = EffectDefinition::CardType {
            card_type: game_domain::EffectCardType::Ally,
        };
        let rules = vec![
            EffectRule {
                id: "rule:fleur".into(),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::Sequence {
                    effects: vec![ally.clone(), EffectDefinition::OtherAllyBonus { health: 2 }],
                },
            },
            EffectRule {
                id: "rule:ally".into(),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: ally,
            },
        ];
        let mut state = advance_to_hero_action(&entities, &rules);
        for id in if fleur_first {
            ["fleur", "one", "two"]
        } else {
            ["one", "fleur", "two"]
        } {
            let decision = decide(
                &state,
                state.state_version(),
                GameCommand::PlayCard {
                    card_id: id.into(),
                    targets: vec![],
                },
                &rules,
            )
            .expect("play ally");
            assert_eq!(
                apply_game_event(&state, &decision.event).expect("replay bonus"),
                decision.state
            );
            state = decision.state;
        }
        assert_eq!(
            state
                .effect_world()
                .hero_resource(1, EffectResource::Health),
            Some(6)
        );
    }
}

#[test]
fn death_eater_reacts_to_morsmordre_once_and_petrificus_suppresses_the_reaction() {
    for suppressed in [false, true] {
        let entities = vec![
            EffectEntityPlacement::new(
                EffectEntity::location("location", "location", "rule:location", 6, 1),
                EffectZone::ActiveLocation,
            ),
            EffectEntityPlacement::new(
                EffectEntity::villain("death-eater", "villain:010", "rule:death-eater", 7)
                    .with_turn_state(Some(game_domain::EffectTurnState {
                        suppressed_by: if suppressed { vec![1] } else { vec![] },
                        ..Default::default()
                    })),
                EffectZone::ActiveVillains,
            ),
            EffectEntityPlacement::new(
                EffectEntity::card(
                    "morsmordre",
                    "dark-arts:016",
                    EffectEntityKind::DarkArts,
                    None,
                    "rule:morsmordre",
                    None,
                ),
                EffectZone::DarkArtsDeck,
            ),
        ];
        let mut target = single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Any);
        target.min = 0;
        target.max = 4;
        let damage = EffectDefinition::Apply {
            target,
            operation: EffectOperation::ModifyResource {
                resource: EffectResource::Health,
                amount: -1,
            },
        };
        let rules = vec![
            EffectRule {
                id: "rule:reveal".into(),
                trigger: EffectTrigger::DarkArtsCompleted,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::RevealDarkArts,
            },
            EffectRule {
                id: "rule:morsmordre".into(),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: damage.clone(),
            },
            EffectRule {
                id: "rule:death-eater".into(),
                trigger: EffectTrigger::Villains,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::Reaction {
                    trigger: game_domain::EffectReactionTrigger::MorsmordreRevealedV1,
                    effect: Box::new(damage),
                },
            },
        ];
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants(),
            content: content(&entities),
        })
        .expect("fixture");
        let decision =
            decide(&initial, 1, GameCommand::CompleteDarkArts, &rules).expect("reveal Morsmordre");
        for position in [1, 2] {
            assert_eq!(
                decision
                    .state
                    .effect_world()
                    .hero_resource(position, EffectResource::Health),
                Some(if suppressed { 9 } else { 8 })
            );
        }
        assert_eq!(
            apply_game_event(&initial, &decision.event).expect("replay reaction"),
            decision.state
        );
    }
}

#[test]
fn copying_fleur_has_an_independent_bonus_and_does_not_count_as_playing_an_ally() {
    let mut entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::hero(1).with_resource(EffectResource::Health, 2),
            EffectZone::Heroes,
        ),
        starter_card("one", "ally", 1, "rule:ally", EffectZone::HeroHand),
        starter_card("two", "ally", 1, "rule:ally", EffectZone::HeroHand),
    ];
    entities.extend([("fleur", 4), ("potion", 3)].map(|(id, cost)| {
        EffectEntityPlacement::new(
            EffectEntity::card(
                id,
                id,
                EffectEntityKind::HogwartsCard,
                Some(1),
                format!("rule:{id}"),
                Some(cost),
            )
            .with_turn_state(Some(game_domain::EffectTurnState::default())),
            EffectZone::HeroHand,
        )
    }));
    let ally = EffectDefinition::CardType {
        card_type: game_domain::EffectCardType::Ally,
    };
    let mut target = single_target_selector(
        Some("ally"),
        EffectZone::HeroPlayArea,
        EffectTargetOwner::Actor,
    );
    target
        .eligibility
        .push(game_domain::EffectEligibility::CardType {
            card_type: game_domain::EffectCardType::Ally,
        });
    let rules = vec![
        EffectRule {
            id: "rule:fleur".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![ally.clone(), EffectDefinition::OtherAllyBonus { health: 2 }],
            },
        },
        EffectRule {
            id: "rule:ally".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: ally,
        },
        EffectRule {
            id: "rule:potion".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::CardType {
                        card_type: game_domain::EffectCardType::Item,
                    },
                    EffectDefinition::Apply {
                        target,
                        operation: EffectOperation::CopyPlayedAlly,
                    },
                ],
            },
        },
    ];
    let mut state = advance_to_hero_action(&entities, &rules);
    for (id, health) in [("fleur", 2), ("potion", 4), ("one", 6), ("two", 6)] {
        let decision = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: id.into(),
                targets: if id == "potion" {
                    vec![target_binding("ally", "fleur")]
                } else {
                    vec![]
                },
            },
            &rules,
        )
        .expect("play card");
        assert_eq!(
            decision
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Health),
            Some(health),
            "after {id}"
        );
        assert_eq!(
            apply_game_event(&state, &decision.event).expect("replay independent bonuses"),
            decision.state
        );
        state = decision.state;
    }
}

fn rule(id: &str, trigger: EffectTrigger, effect: EffectDefinition) -> EffectRule {
    EffectRule {
        id: id.into(),
        trigger,
        order: 0,
        cost: vec![],
        effect,
    }
}
