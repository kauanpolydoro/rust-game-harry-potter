use super::*;

#[test]
fn a_top_deck_discard_triggers_the_frogs_harmful_discard_bonus_but_voluntary_discard_does_not() {
    for harmful in [false, true] {
        let entities = vec![
            starter_card("action", "action", 1, "rule:action", EffectZone::HeroHand),
            EffectEntityPlacement::new(
                EffectEntity::card(
                    "frog",
                    "frog",
                    EffectEntityKind::HogwartsCard,
                    Some(1),
                    "rule:frog",
                    Some(2),
                ),
                if harmful {
                    EffectZone::HeroDrawPile
                } else {
                    EffectZone::HeroHand
                },
            ),
            EffectEntityPlacement::new(
                EffectEntity::villain("peter", "peter", "rule:action", 7),
                EffectZone::ActiveVillains,
            ),
        ];
        let rules = vec![
            EffectRule {
                id: "rule:action".into(),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: if harmful {
                    EffectDefinition::RevealTopCard {
                        minimum_cost: 1,
                        effect: Box::new(EffectDefinition::NoOp),
                    }
                } else {
                    EffectDefinition::Apply {
                        target: single_target_selector(
                            None,
                            EffectZone::HeroHand,
                            EffectTargetOwner::Actor,
                        ),
                        operation: EffectOperation::DiscardVoluntarily,
                    }
                },
            },
            EffectRule {
                id: "rule:frog".into(),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::Reaction {
                    trigger: game_domain::EffectReactionTrigger::SelfHarmfulDiscard,
                    effect: Box::new(EffectDefinition::Apply {
                        target: single_target_selector(
                            None,
                            EffectZone::Heroes,
                            EffectTargetOwner::Actor,
                        ),
                        operation: EffectOperation::GainInfluenceAndHealth {
                            influence: 1,
                            health: 1,
                        },
                    }),
                },
            },
        ];
        let state = advance_to_hero_action(&entities, &rules);
        let result = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: "action".into(),
                targets: vec![],
            },
            &rules,
        )
        .unwrap_or_else(|error| panic!("discard a frog harmful={harmful}: {error:?}"));
        assert_eq!(
            result
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Influence),
            Some(u16::from(harmful))
        );
        assert_eq!(
            apply_game_event(&state, &result.event).expect("replay discard reaction"),
            result.state
        );
    }
}

#[test]
fn petrificus_suppresses_villains_until_before_the_casters_next_dark_arts_phase() {
    let entities = vec![
        starter_card(
            "petrificus",
            "petrificus",
            1,
            "rule:petrificus",
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("villain", "villain", "rule:villain", 8)
                .with_turn_state(Some(game_domain::EffectTurnState::default())),
            EffectZone::ActiveVillains,
        ),
    ];
    let rules = petrificus_rules();
    let validated = ValidatedGameRules::new(rules.clone()).expect("rules");
    let mut state = GameEngine::new(&validated)
        .start(
            StartGameInput {
                actor_role: ParticipantRole::Host,
                participants: &participants(),
                content: content(&entities),
            },
            &mut ScriptedRoller::empty(),
        )
        .expect("opening turn");
    assert_eq!(
        state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(8)
    );
    let played = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "petrificus".into(),
            targets: vec![target_binding("villain", "villain")],
        },
        &rules,
    )
    .expect("suppress villain");
    assert_eq!(
        apply_game_event(&state, &played.event).expect("replay spell"),
        played.state
    );
    state = played.state;
    for actor in [1, 2] {
        let result = GameEngine::new(&validated)
            .decide(
                GameIntentInput {
                    state: &state,
                    actor_position: actor,
                    expected_state_version: state.state_version(),
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut ScriptedRoller::empty(),
            )
            .expect("advance through suppressed villain");
        assert_eq!(
            apply_game_event(&state, &result.event).expect("replay suppression duration"),
            result.state
        );
        state = result.state;
        assert_eq!(
            state
                .effect_world()
                .hero_resource(2, EffectResource::Health),
            Some(10)
        );
    }
    assert_eq!(
        state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(6)
    );
    assert!(
        state
            .effect_world()
            .entity("villain")
            .expect("villain")
            .1
            .turn_state()
            .expect("turn state")
            .suppressed_by
            .is_empty()
    );
}

#[test]
fn ron_counts_attack_across_villains_and_rewards_once_even_without_a_defeat() {
    let mut entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::hero(1)
                .with_catalog_id("hero:011")
                .with_effect_rule("rule:ron")
                .with_turn_state(Some(game_domain::EffectTurnState::default()))
                .with_resource(EffectResource::Health, 4),
            EffectZone::Heroes,
        ),
        starter_card("attack", "attack", 1, "rule:attack", EffectZone::HeroHand),
    ];
    for id in ["villain-one", "villain-two"] {
        entities.push(EffectEntityPlacement::new(
            EffectEntity::villain(id, id, "rule:villain", 8)
                .with_turn_state(Some(game_domain::EffectTurnState::default())),
            EffectZone::ActiveVillains,
        ));
    }
    let rules = vec![
        resource_rule(
            "rule:attack",
            None,
            EffectTargetOwner::Actor,
            EffectResource::Attack,
            5,
        ),
        hero_ability_rule(
            "rule:ron",
            game_domain::HeroAbilityStrategy::RonGameThreeV1,
            EffectTargetOwner::Any,
            EffectResource::Health,
            2,
        ),
    ];
    let mut state = advance_to_hero_action(&entities, &rules);
    for command in [
        GameCommand::PlayCard {
            card_id: "attack".into(),
            targets: vec![],
        },
        GameCommand::AssignAttack {
            villain_id: "villain-one".into(),
            amount: 2,
        },
        GameCommand::AssignAttack {
            villain_id: "villain-two".into(),
            amount: 1,
        },
    ] {
        let result = decide(&state, state.state_version(), command, &rules).expect("assign attack");
        assert_eq!(
            apply_game_event(&state, &result.event).expect("replay attack and ability"),
            result.state
        );
        state = result.state;
    }
    let choice = state.pending_choice().expect("third attack activates Ron");
    state = decide(
        &state,
        state.state_version(),
        GameCommand::ResolveChoice {
            choice_id: choice.id.clone(),
            selected_options: vec!["hero:1".into()],
        },
        &rules,
    )
    .expect("Ron heals himself")
    .state;
    assert_eq!(
        state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(6)
    );
    let result = decide(
        &state,
        state.state_version(),
        GameCommand::AssignAttack {
            villain_id: "villain-one".into(),
            amount: 2,
        },
        &rules,
    )
    .expect("further attack");
    assert!(result.state.pending_choice().is_none());
    assert_eq!(
        result
            .state
            .effect_world()
            .hero_resource(1, EffectResource::Health),
        Some(6)
    );
}

#[test]
fn tarantallegra_limits_total_assigned_attack_per_villain_and_new_copies_do_not_reset_it() {
    let mut entities = vec![
        starter_card("limit-one", "limit", 1, "rule:limit", EffectZone::HeroHand),
        starter_card("limit-two", "limit", 1, "rule:limit", EffectZone::HeroHand),
    ];
    for id in ["villain-one", "villain-two"] {
        entities.push(EffectEntityPlacement::new(
            EffectEntity::villain(id, id, "rule:villain", 5)
                .with_turn_state(Some(game_domain::EffectTurnState::default())),
            EffectZone::ActiveVillains,
        ));
    }
    let rules = vec![EffectRule {
        id: "rule:limit".into(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Sequence {
            effects: vec![
                EffectDefinition::LimitVillainAttack { maximum: 1 },
                EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Attack,
                        amount: 4,
                    },
                },
            ],
        },
    }];
    let mut state = advance_to_hero_action(&entities, &rules);
    for command in [
        GameCommand::PlayCard {
            card_id: "limit-one".into(),
            targets: vec![],
        },
        GameCommand::AssignAttack {
            villain_id: "villain-one".into(),
            amount: 1,
        },
        GameCommand::PlayCard {
            card_id: "limit-two".into(),
            targets: vec![],
        },
    ] {
        let result =
            decide(&state, state.state_version(), command, &rules).expect("legal limited action");
        assert_eq!(
            apply_game_event(&state, &result.event).expect("replay limit"),
            result.state
        );
        state = result.state;
    }
    assert!(
        decide(
            &state,
            state.state_version(),
            GameCommand::AssignAttack {
                villain_id: "villain-one".into(),
                amount: 1
            },
            &rules
        )
        .is_err()
    );
    assert!(
        decide(
            &state,
            state.state_version(),
            GameCommand::AssignAttack {
                villain_id: "villain-two".into(),
                amount: 2
            },
            &rules
        )
        .is_err()
    );
    assert!(
        decide(
            &state,
            state.state_version(),
            GameCommand::AssignAttack {
                villain_id: "villain-two".into(),
                amount: 1
            },
            &rules
        )
        .is_ok()
    );
}

#[test]
fn hermione_activates_once_after_four_actual_spells_and_neville_heals_each_hero_once() {
    for (strategy, catalog, resource, amount) in [
        (
            game_domain::HeroAbilityStrategy::HermioneGameThreeV1,
            "hero:005",
            EffectResource::Influence,
            1,
        ),
        (
            game_domain::HeroAbilityStrategy::NevilleGameThreeV1,
            "hero:008",
            EffectResource::Health,
            1,
        ),
    ] {
        let mut entities = vec![EffectEntityPlacement::new(
            EffectEntity::hero(1)
                .with_catalog_id(catalog)
                .with_effect_rule("rule:ability")
                .with_turn_state(Some(game_domain::EffectTurnState::default()))
                .with_resource(EffectResource::Health, 4),
            EffectZone::Heroes,
        )];
        for i in 0..5 {
            entities.push(starter_card(
                &format!("card:{i}"),
                "spell",
                1,
                "rule:spell",
                EffectZone::HeroHand,
            ));
        }
        let rules = vec![
            hero_ability_rule(
                "rule:ability",
                strategy,
                if resource == EffectResource::Health {
                    EffectTargetOwner::Actor
                } else {
                    EffectTargetOwner::Any
                },
                resource,
                amount,
            ),
            healing_spell_rule(),
        ];
        let mut state = advance_to_hero_action(&entities, &rules);
        for i in 0..5 {
            let next = decide(
                &state,
                state.state_version(),
                GameCommand::PlayCard {
                    card_id: format!("card:{i}"),
                    targets: vec![],
                },
                &rules,
            )
            .expect("play spell");
            assert_eq!(
                apply_game_event(&state, &next.event).expect("replay ability state"),
                next.state
            );
            state = next.state;
            if resource == EffectResource::Influence && i == 3 {
                let choice = state
                    .pending_choice()
                    .expect("fourth spell activates Hermione");
                state = decide(
                    &state,
                    state.state_version(),
                    GameCommand::ResolveChoice {
                        choice_id: choice.id.clone(),
                        selected_options: vec!["hero:2".into()],
                    },
                    &rules,
                )
                .expect("choose recipient")
                .state;
            } else {
                assert!(state.pending_choice().is_none());
            }
            if resource == EffectResource::Health {
                assert_eq!(
                    state
                        .effect_world()
                        .hero_resource(1, EffectResource::Health),
                    Some(6 + i)
                );
            }
        }
        if resource == EffectResource::Influence {
            assert_eq!(state.effect_world().hero_resource(2, resource), Some(1));
        }
    }
}

#[test]
fn harry_chooses_a_hero_only_for_the_first_actual_control_removal_and_replays() {
    let entities = vec![
        EffectEntityPlacement::new(
            EffectEntity::hero(1)
                .with_catalog_id("hero:002")
                .with_effect_rule("rule:harry")
                .with_turn_state(Some(game_domain::EffectTurnState::default())),
            EffectZone::Heroes,
        ),
        EffectEntityPlacement::new(
            EffectEntity::location("location", "location", "rule:location", 5, 1)
                .with_resource(EffectResource::Control, 2),
            EffectZone::ActiveLocation,
        ),
        starter_card(
            "finite-one",
            "finite",
            1,
            "rule:finite",
            EffectZone::HeroHand,
        ),
        starter_card(
            "finite-two",
            "finite",
            1,
            "rule:finite",
            EffectZone::HeroHand,
        ),
    ];
    let rules = vec![
        hero_ability_rule(
            "rule:harry",
            game_domain::HeroAbilityStrategy::HarryGameThreeV1,
            EffectTargetOwner::Any,
            EffectResource::Attack,
            1,
        ),
        control_removal_rule(),
    ];
    let state = advance_to_hero_action(&entities, &rules);
    let first = decide(
        &state,
        state.state_version(),
        GameCommand::PlayCard {
            card_id: "finite-one".into(),
            targets: vec![],
        },
        &rules,
    )
    .expect("first removal");
    assert_eq!(
        apply_game_event(&state, &first.event).expect("replay Harry's choice"),
        first.state
    );
    let choice = first
        .state
        .pending_choice()
        .expect("Harry chooses the beneficiary");
    assert_eq!(choice.responsible_position, 1);
    let chosen = decide(
        &first.state,
        first.state.state_version(),
        GameCommand::ResolveChoice {
            choice_id: choice.id.clone(),
            selected_options: vec!["hero:2".into()],
        },
        &rules,
    )
    .expect("grant attack to Hermione");
    assert_eq!(
        chosen
            .state
            .effect_world()
            .hero_resource(2, EffectResource::Attack),
        Some(1)
    );
    let second = decide(
        &chosen.state,
        chosen.state.state_version(),
        GameCommand::PlayCard {
            card_id: "finite-two".into(),
            targets: vec![],
        },
        &rules,
    )
    .expect("second removal");
    assert!(second.state.pending_choice().is_none());
    assert_eq!(
        second
            .state
            .effect_world()
            .hero_resource(2, EffectResource::Attack),
        Some(1)
    );
    assert_eq!(
        apply_game_event(&chosen.state, &second.event).expect("replay used ability"),
        second.state
    );
}

#[test]
fn revealing_the_top_card_checks_its_cost_without_drawing_it_and_replays_reshuffles() {
    for (zone, cost) in [
        (EffectZone::HeroDrawPile, 0),
        (EffectZone::HeroDrawPile, 2),
        (EffectZone::HeroDiscardPile, 2),
    ] {
        let entities = vec![
            starter_card("reveal", "reveal", 1, "rule:reveal", EffectZone::HeroHand),
            EffectEntityPlacement::new(
                EffectEntity::card(
                    "top",
                    "top",
                    EffectEntityKind::HogwartsCard,
                    Some(1),
                    "rule:card",
                    Some(cost),
                ),
                zone,
            ),
        ];
        let rules = vec![EffectRule {
            id: "rule:reveal".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::RevealTopCard {
                minimum_cost: 1,
                effect: Box::new(EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Health,
                        amount: -2,
                    },
                }),
            },
        }];
        let state = advance_to_hero_action(&entities, &rules);
        let result = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: "reveal".into(),
                targets: vec![],
            },
            &rules,
        )
        .expect("reveal top");
        assert_eq!(
            result.state.effect_world().entity_zone("top"),
            Some(if cost == 0 {
                EffectZone::HeroDrawPile
            } else {
                EffectZone::HeroDiscardPile
            })
        );
        assert_eq!(
            result
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Health),
            Some(if cost == 0 { 10 } else { 8 })
        );
        assert!(
            result
                .state
                .effect_world()
                .cards_in_zone(1, EffectZone::HeroHand)
                .is_empty()
        );
        assert_eq!(
            apply_game_event(&state, &result.event).expect("replay top reveal"),
            result.state
        );
    }
}

#[test]
fn sybill_awards_influence_only_for_the_spell_actually_discarded_and_replays_the_choice() {
    for (discarded, bonus) in [("spell", 2), ("item", 0)] {
        let entities = vec![
            starter_card("sybill", "sybill", 1, "rule:sybill", EffectZone::HeroHand),
            starter_card("spell", "spell", 1, "rule:spell", EffectZone::HeroHand),
            starter_card("item", "item", 1, "rule:item", EffectZone::HeroHand),
        ];
        let mut rules = vec![EffectRule {
            id: "rule:sybill".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::ForEachTarget {
                target: EffectSelector {
                    min: 0,
                    ..single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor)
                },
                effect: Box::new(EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::HeroHand,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::DiscardForSpellBonus { influence: 2 },
                }),
            },
        }];
        for (id, card_type) in [
            ("spell", game_domain::EffectCardType::Spell),
            ("item", game_domain::EffectCardType::Item),
        ] {
            rules.push(EffectRule {
                id: format!("rule:{id}"),
                trigger: EffectTrigger::Manual,
                order: 0,
                cost: vec![],
                effect: EffectDefinition::CardType { card_type },
            });
        }
        let state = advance_to_hero_action(&entities, &rules);
        let pending = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: "sybill".into(),
                targets: vec![],
            },
            &rules,
        )
        .expect("play Sybill");
        assert_eq!(
            apply_game_event(&state, &pending.event).expect("replay choice opening"),
            pending.state
        );
        let choice = pending.state.pending_choice().expect("choose a discard");
        let result = decide(
            &pending.state,
            pending.state.state_version(),
            GameCommand::ResolveChoice {
                choice_id: choice.id.clone(),
                selected_options: vec![discarded.into()],
            },
            &rules,
        )
        .expect("resolve discard");
        assert_eq!(
            result
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Influence),
            Some(bonus)
        );
        assert_eq!(
            result.state.effect_world().entity_zone(discarded),
            Some(EffectZone::HeroDiscardPile)
        );
        assert_eq!(
            apply_game_event(&pending.state, &result.event).expect("replay selected discard"),
            result.state
        );
    }
}

#[test]
fn butterbeer_gives_both_resources_to_the_same_two_distinct_selected_heroes() {
    let entities = vec![starter_card(
        "butterbeer",
        "butterbeer",
        1,
        "rule:butterbeer",
        EffectZone::HeroHand,
    )];
    let mut players = participants();
    players.push(LobbyParticipant {
        role: ParticipantRole::Guest,
        position: 3,
        hero: Some(HeroId::Ron),
        ready: true,
    });
    let mut target =
        single_target_selector(Some("heroes"), EffectZone::Heroes, EffectTargetOwner::Any);
    target.min = 2;
    target.max = 2;
    let rules = vec![EffectRule {
        id: "rule:butterbeer".into(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Apply {
            target,
            operation: EffectOperation::GainInfluenceAndHealth {
                influence: 1,
                health: 1,
            },
        },
    }];
    let validated = ValidatedGameRules::new(rules.clone()).expect("bounded rules");
    let state = GameEngine::new(&validated)
        .start(
            StartGameInput {
                actor_role: ParticipantRole::Host,
                participants: &players,
                content: content(&entities),
            },
            &mut ScriptedRoller::empty(),
        )
        .expect("fixture game");
    let play = |ids: &[&str]| GameCommand::PlayCard {
        card_id: "butterbeer".into(),
        targets: vec![EffectTargetBinding {
            selector_id: "heroes".into(),
            target_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        }],
    };
    assert!(
        decide(
            &state,
            state.state_version(),
            play(&["hero:1", "hero:1"]),
            &rules
        )
        .is_err()
    );
    let result = decide(
        &state,
        state.state_version(),
        play(&["hero:1", "hero:3"]),
        &rules,
    )
    .expect("choose two heroes");
    for (position, influence) in [(1, 1), (2, 0), (3, 1)] {
        assert_eq!(
            result
                .state
                .effect_world()
                .hero_resource(position, EffectResource::Influence),
            Some(influence)
        );
    }
    let game_domain::GameEvent::CardPlayed { effects, .. } = &result.event else {
        panic!("card event")
    };
    let health_targets = effects
        .iter()
        .filter_map(|effect| match effect {
            game_domain::EffectOutcome::ResourceChanged {
                resource: EffectResource::Health,
                target_position,
                ..
            } => *target_position,
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(health_targets, [1, 3]);
    assert_eq!(
        apply_game_event(&state, &result.event).expect("replay"),
        result.state
    );
}

fn petrificus_rules() -> Vec<EffectRule> {
    vec![
        EffectRule {
            id: "rule:petrificus".into(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: single_target_selector(
                    Some("villain"),
                    EffectZone::ActiveVillains,
                    EffectTargetOwner::Any,
                ),
                operation: EffectOperation::SuppressVillain,
            },
        },
        EffectRule {
            id: "rule:villain".into(),
            trigger: EffectTrigger::Villains,
            order: 0,
            cost: vec![],
            effect: EffectDefinition::Apply {
                target: single_target_selector(None, EffectZone::Heroes, EffectTargetOwner::Actor),
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Health,
                    amount: -2,
                },
            },
        },
    ]
}

fn hero_ability_rule(
    id: &str,
    strategy: game_domain::HeroAbilityStrategy,
    owner: EffectTargetOwner,
    resource: EffectResource,
    amount: i16,
) -> EffectRule {
    EffectRule {
        id: id.to_owned(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::HeroAbility {
            strategy,
            effect: Box::new(resource_rule(id, None, owner, resource, amount).effect),
        },
    }
}

fn control_removal_rule() -> EffectRule {
    EffectRule {
        id: "rule:finite".into(),
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
                amount: -1,
            },
        },
    }
}

fn healing_spell_rule() -> EffectRule {
    EffectRule {
        id: "rule:spell".into(),
        trigger: EffectTrigger::Manual,
        order: 0,
        cost: vec![],
        effect: EffectDefinition::Sequence {
            effects: vec![
                EffectDefinition::CardType {
                    card_type: game_domain::EffectCardType::Spell,
                },
                EffectDefinition::Apply {
                    target: single_target_selector(
                        None,
                        EffectZone::Heroes,
                        EffectTargetOwner::Actor,
                    ),
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Health,
                        amount: 1,
                    },
                },
            ],
        },
    }
}

fn advance_to_hero_action(
    entities: &[EffectEntityPlacement],
    rules: &[EffectRule],
) -> game_domain::InitialGameState {
    let count = entities
        .iter()
        .filter_map(|placement| placement.entity().owner_position())
        .max()
        .unwrap_or(2)
        .max(2);
    let mut players = (1..=count)
        .map(|position| LobbyParticipant {
            role: if position == 1 {
                ParticipantRole::Host
            } else {
                ParticipantRole::Guest
            },
            position,
            ready: true,
            hero: entities
                .iter()
                .find(|placement| {
                    placement.entity().kind() == EffectEntityKind::Hero
                        && placement.entity().owner_position() == Some(position)
                })
                .and_then(|placement| match placement.entity().catalog_id() {
                    Some("hero:002") => Some(HeroId::Harry),
                    Some("hero:005") => Some(HeroId::Hermione),
                    Some("hero:008") => Some(HeroId::Neville),
                    Some("hero:011") => Some(HeroId::Ron),
                    _ => None,
                }),
        })
        .collect::<Vec<_>>();
    for index in 0..players.len() {
        if players[index].hero.is_none() {
            players[index].hero = [
                HeroId::Harry,
                HeroId::Hermione,
                HeroId::Ron,
                HeroId::Neville,
            ]
            .into_iter()
            .find(|hero| !players.iter().any(|player| player.hero == Some(*hero)));
        }
    }
    let initial = initialize_game(StartGameInput {
        actor_role: ParticipantRole::Host,
        participants: &players,
        content: content(entities),
    })
    .expect("consistent fixture identities");
    decide(&initial, 1, GameCommand::CompleteDarkArts, rules)
        .expect("hero actions")
        .state
}

#[test]
fn neville_adds_one_effective_heal_per_recipient_and_does_not_recurse() {
    let mut entities = (1..=4)
        .map(|position| {
            let mut hero = EffectEntity::hero(position).with_resource(EffectResource::Health, 4);
            if position == 1 {
                hero = hero
                    .with_catalog_id("hero:008")
                    .with_effect_rule("rule:neville")
                    .with_turn_state(Some(game_domain::EffectTurnState::default()));
            }
            EffectEntityPlacement::new(hero, EffectZone::Heroes)
        })
        .collect::<Vec<_>>();
    for id in ["heal-one", "heal-two"] {
        entities.push(starter_card(
            id,
            "heal",
            1,
            "rule:heal",
            EffectZone::HeroHand,
        ));
    }
    let mut heal = resource_rule(
        "rule:heal",
        None,
        EffectTargetOwner::Any,
        EffectResource::Health,
        1,
    );
    if let EffectDefinition::Apply { target, .. } = &mut heal.effect {
        target.min = 0;
        target.max = 4;
    }
    let rules = [
        heal,
        hero_ability_rule(
            "rule:neville",
            game_domain::HeroAbilityStrategy::NevilleGameThreeV1,
            EffectTargetOwner::Actor,
            EffectResource::Health,
            1,
        ),
    ];
    let mut state = advance_to_hero_action(&entities, &rules);
    for (id, expected) in [("heal-one", 6), ("heal-two", 7)] {
        let next = decide(
            &state,
            state.state_version(),
            GameCommand::PlayCard {
                card_id: id.to_owned(),
                targets: vec![],
            },
            &rules,
        )
        .expect("heal every Hero");
        assert_eq!(
            apply_game_event(&state, &next.event).expect("replay recipient tracking"),
            next.state
        );
        state = next.state;
        for owner in 1..=4 {
            assert_eq!(
                state
                    .effect_world()
                    .hero_resource(owner, EffectResource::Health),
                Some(expected)
            );
        }
    }
    assert_eq!(
        state
            .effect_world()
            .entity("hero:1")
            .expect("Neville")
            .1
            .turn_state()
            .expect("tracking")
            .healed_positions,
        [1, 2, 3, 4]
    );
}

#[test]
fn petrificus_cast_by_two_heroes_expires_separately_for_each_caster() {
    let entities = vec![
        starter_card(
            "cast-one",
            "petrificus",
            1,
            "rule:petrificus",
            EffectZone::HeroHand,
        ),
        starter_card(
            "cast-two",
            "petrificus",
            2,
            "rule:petrificus",
            EffectZone::HeroHand,
        ),
        EffectEntityPlacement::new(
            EffectEntity::villain("villain", "villain", "rule:villain", 8)
                .with_turn_state(Some(game_domain::EffectTurnState::default())),
            EffectZone::ActiveVillains,
        ),
    ];
    let rules = ValidatedGameRules::new(petrificus_rules()).expect("rules");
    let mut state = advance_to_hero_action(&entities, rules.effect_rules());
    for actor in [1, 2, 1] {
        if actor == 2 || state.turn() == 1 {
            let next = decide_game_command(GameCommandInput {
                state: &state,
                actor_position: actor,
                expected_state_version: state.state_version(),
                command: GameCommand::PlayCard {
                    card_id: if actor == 1 { "cast-one" } else { "cast-two" }.to_owned(),
                    targets: vec![target_binding("villain", "villain")],
                },
                effect_rules: rules.effect_rules(),
                die_roller: &mut ScriptedRoller::empty(),
            })
            .expect("cast");
            assert_eq!(
                apply_game_event(&state, &next.event).expect("replay cast"),
                next.state
            );
            state = next.state;
        }
        let next = GameEngine::new(&rules)
            .decide(
                GameIntentInput {
                    state: &state,
                    actor_position: actor,
                    expected_state_version: state.state_version(),
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut ScriptedRoller::empty(),
            )
            .expect("next caster turn");
        assert_eq!(
            apply_game_event(&state, &next.event).expect("replay independent duration"),
            next.state
        );
        state = next.state;
        assert_eq!(
            state
                .effect_world()
                .hero_resource(state.active_position(), EffectResource::Health),
            Some(if state.turn() == 4 { 8 } else { 10 })
        );
    }
}
