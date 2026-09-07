mod replay;

use game_domain::{EffectEntityKind, EffectZone, ParticipantRole, ValidatedGameRules};

use super::{ChaChaEffectRoller, StoredRoomParticipant, codec, domain_participant};
use crate::content_catalog::ContentCatalog;

fn participants(count: usize) -> Vec<StoredRoomParticipant> {
    ["harry", "hermione", "ron", "neville"]
        .into_iter()
        .take(count)
        .enumerate()
        .map(|(index, hero)| StoredRoomParticipant {
            id: uuid::Uuid::from_u128(index as u128 + 1),
            display_name: hero.to_owned(),
            role: if index == 0 { "host" } else { "guest" }.to_owned(),
            position: i16::try_from(index + 1).expect("position"),
            hero_id: Some(hero.to_owned()),
            ready: true,
        })
        .collect()
}

fn prepared_game(
    count: usize,
) -> (
    game_domain::InitialGameState,
    Vec<StoredRoomParticipant>,
    ValidatedGameRules,
) {
    let manifest = crate::game_one_manifest();
    let digest = manifest.digest.clone();
    let version = manifest.ruleset_version.clone();
    let catalog = ContentCatalog::new(vec![manifest]);
    let content = catalog
        .selection("adventure:001", &digest, &version)
        .expect("selection");
    let rules = ValidatedGameRules::new(catalog.effect_rules(&digest).expect("compiled rules"))
        .expect("validated rules");
    let participants = participants(count);
    let domain_players = participants
        .iter()
        .map(domain_participant)
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .expect("participants");
    let state = content
        .start_game(
            ParticipantRole::Host,
            &domain_players,
            &rules,
            &mut ChaChaEffectRoller::new(&[7; 32], 0).ok().expect("seed"),
        )
        .expect("real preparation");
    (state, participants, rules)
}

#[test]
fn real_game_one_preparation_and_opening_phases_survive_snapshot_round_trip() {
    for count in 2..=4 {
        let (state, participants, _) = prepared_game(count);
        assert_eq!(
            prepared_game(count).0,
            state,
            "same seed and content produce the same opening"
        );
        assert_eq!(state.preparation_samples().len(), 40 + 9 * count);
        assert_eq!(
            state.effect_world().entities_in(EffectZone::Market).len(),
            6
        );
        assert_eq!(
            state
                .effect_world()
                .entities_in(EffectZone::DarkArtsDiscard)
                .len(),
            1
        );
        for player in state.players() {
            let cards = state
                .effect_world()
                .entities()
                .filter(|(_, entity)| {
                    entity.owner_position() == Some(player.position())
                        && entity.kind() == EffectEntityKind::StarterCard
                })
                .map(|(_, entity)| entity.catalog_id().expect("catalog ID"))
                .collect::<Vec<_>>();
            assert_eq!(cards.len(), 10);
            assert_eq!(cards.iter().filter(|id| **id == "starter:001").count(), 7);
        }
        let snapshot = codec::persisted_snapshot(&state, &participants);
        let encoded = serde_json::to_string(&snapshot).expect("snapshot encoding");
        let decoded = codec::decode_persisted_snapshot(&encoded)
            .ok()
            .expect("snapshot decoding");
        assert_eq!(
            codec::command_domain_state(&decoded)
                .ok()
                .expect("restored state"),
            state
        );
        let repersisted = codec::persisted_after_decision(&decoded, &state);
        assert_eq!(
            serde_json::to_string(&repersisted).expect("canonical snapshot"),
            encoded
        );

        let mut invalid = decoded.clone();
        invalid.preparation_samples[0].result = 30;
        assert!(
            codec::command_domain_state(&invalid).is_err(),
            "out-of-range sample"
        );
        invalid = decoded.clone();
        invalid.preparation_samples[40].owner_position = Some(2);
        assert!(
            codec::command_domain_state(&invalid).is_err(),
            "wrong pile owner"
        );
        invalid = decoded.clone();
        invalid.preparation_samples.pop();
        assert!(
            codec::command_domain_state(&invalid).is_err(),
            "truncated preparation"
        );
        invalid.preparation_samples.clear();
        assert!(
            codec::command_domain_state(&invalid).is_err(),
            "missing preparation"
        );
    }
}
