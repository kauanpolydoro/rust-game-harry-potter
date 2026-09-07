use super::{codec, game_one_tests::prepared_adventure};
use game_domain::EffectZone;

#[test]
fn game_two_prepares_cumulative_piles_and_round_trips_without_changing_game_one() {
    for count in 2..=4 {
        let (state, participants, _) =
            prepared_adventure(count, crate::game_two_manifest(), "adventure:002");
        assert_eq!(state.snapshot_version(), 6);
        assert_eq!(state.preparation_samples().len(), 62 + 9 * count);
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
            Some("location:003")
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
fn game_two_snapshots_cannot_be_mislabelled_as_the_previous_adventure_codec() {
    let (state, participants, _) =
        prepared_adventure(2, crate::game_two_manifest(), "adventure:002");
    let mut snapshot =
        serde_json::to_value(codec::persisted_snapshot(&state, &participants)).expect("JSON");
    snapshot["snapshot_version"] = serde_json::json!(5);
    assert!(codec::decode_persisted_snapshot(&snapshot.to_string()).is_err());
}

#[test]
fn gilderoys_description_limits_drawing_and_discarding_to_its_owner() {
    let manifest = crate::game_two_manifest();
    let digest = manifest.digest.clone();
    let catalog = crate::content_catalog::ContentCatalog::new(vec![manifest]);
    let description = catalog
        .entity_description(
            &digest,
            "hogwarts-card:019",
            game_content::FunctionalField::Effect,
        )
        .expect("Gilderoy description");
    assert!(!description.contains("cada Herói"), "{description}");
    assert!(description.contains("Você descarta"), "{description}");
}
