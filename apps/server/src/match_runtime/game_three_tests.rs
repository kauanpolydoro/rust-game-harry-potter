use super::{codec, game_one_tests::prepared_adventure};
use game_domain::EffectZone;

#[test]
fn game_three_prepares_cumulative_piles_and_round_trips_without_changing_game_one() {
    for count in 2..=4 {
        let (state, participants, _) =
            prepared_adventure(count, crate::game_three_manifest(), "adventure:003");
        assert_eq!(state.snapshot_version(), 7);
        assert_eq!(state.preparation_samples().len(), 84 + 9 * count);
        assert_eq!(
            state.effect_world().entities_in(EffectZone::Market).len(),
            6
        );
        assert_eq!(
            state
                .effect_world()
                .entities_in(EffectZone::LocationDeck)
                .len(),
            2
        );
        assert_eq!(
            state.effect_world().entities_in(EffectZone::ActiveLocation)[0].catalog_id(),
            Some("location:006")
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
        let (game_one, _, _) =
            prepared_adventure(count, crate::game_one_manifest(), "adventure:001");
        assert_eq!(game_one.snapshot_version(), 5);
        assert_eq!(game_one.preparation_samples().len(), 40 + 9 * count);
    }
}

#[test]
fn game_three_snapshots_cannot_be_mislabelled_as_the_previous_adventure_codec() {
    let (state, participants, _) =
        prepared_adventure(2, crate::game_three_manifest(), "adventure:003");
    let mut snapshot =
        serde_json::to_value(codec::persisted_snapshot(&state, &participants)).expect("JSON");
    snapshot["snapshot_version"] = serde_json::json!(5);
    assert!(codec::decode_persisted_snapshot(&snapshot.to_string()).is_err());
}

#[test]
fn game_three_restore_rejects_replaced_or_missing_hero_abilities_and_villain_state() {
    let (state, participants, _) =
        prepared_adventure(2, crate::game_three_manifest(), "adventure:003");
    let snapshot = codec::persisted_snapshot(&state, &participants);
    for mutation in ["identity", "rule", "legacy_hero", "villain_state"] {
        let mut forged = snapshot.clone();
        let entity = forged
            .effects
            .entities
            .iter_mut()
            .find(|entity| {
                if mutation == "villain_state" {
                    entity.kind.as_deref() == Some("villain")
                } else {
                    entity.kind.as_deref() == Some("hero") && entity.owner_position == Some(2)
                }
            })
            .expect("entity");
        match mutation {
            "identity" => {
                entity.catalog_id = Some("hero:002".to_owned());
                entity.effect_rule_id = Some("rule:g3-hero-002-ability".to_owned());
            }
            "rule" => entity.effect_rule_id = Some("rule:g3-hero-002-ability".to_owned()),
            "legacy_hero" => {
                entity.catalog_id = None;
                entity.effect_rule_id = None;
                entity.turn_state = None;
            }
            "villain_state" => entity.turn_state = None,
            _ => unreachable!(),
        }
        assert!(codec::command_domain_state(&forged).is_err(), "{mutation}");
    }
}
