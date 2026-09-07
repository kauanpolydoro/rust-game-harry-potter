use crate::{
    EffectDie, EffectEntityKind, EffectResource, EffectRoller, EffectZone, InitialGameState,
    StartGameError,
};

pub(crate) fn prepare_game_one(
    state: &mut InitialGameState,
    random: &mut dyn EffectRoller,
) -> Result<(), StartGameError> {
    prepare(state, random, false)
}

pub(crate) fn prepare_game_two(
    state: &mut InitialGameState,
    random: &mut dyn EffectRoller,
) -> Result<(), StartGameError> {
    prepare(state, random, true)
}

fn prepare(
    state: &mut InitialGameState,
    random: &mut dyn EffectRoller,
    game_two: bool,
) -> Result<(), StartGameError> {
    validate_initial_layout(state, game_two)?;
    let world = &mut state.effect_world;
    let samples = &mut state.preparation_samples;
    for zone in [
        EffectZone::HogwartsDeck,
        EffectZone::DarkArtsDeck,
        EffectZone::VillainDeck,
    ] {
        state.prng_counter += world
            .shuffle_pile(
                zone,
                None,
                &mut PreparationRecorder {
                    random,
                    samples,
                    zone,
                    owner_position: None,
                },
            )
            .map_err(|_| StartGameError::EffectExecutionFailed)?;
    }
    for player in &state.players {
        let position = player.position;
        state.prng_counter += world
            .shuffle_pile(
                EffectZone::HeroDrawPile,
                Some(position),
                &mut PreparationRecorder {
                    random,
                    samples,
                    zone: EffectZone::HeroDrawPile,
                    owner_position: Some(position),
                },
            )
            .map_err(|_| StartGameError::EffectExecutionFailed)?;
        for _ in 0..5 {
            let id = world
                .top_card_id(position, EffectZone::HeroDrawPile)
                .ok_or(StartGameError::InvalidInitialEntities)?;
            world
                .move_card(&id, EffectZone::HeroDrawPile, EffectZone::HeroHand)
                .map_err(|_| StartGameError::InvalidInitialEntities)?;
        }
    }
    for (from, to, count) in [
        (EffectZone::HogwartsDeck, EffectZone::Market, 6),
        (EffectZone::VillainDeck, EffectZone::ActiveVillains, 1),
    ] {
        for _ in 0..count {
            let id = world
                .entities_in(from)
                .first()
                .ok_or(StartGameError::InvalidInitialEntities)?
                .id()
                .to_owned();
            world
                .move_to_back(&id, from, to, None)
                .map_err(|_| StartGameError::InvalidInitialEntities)?;
        }
    }
    state.active_villain_limit = 1;
    Ok(())
}

fn validate_initial_layout(state: &InitialGameState, game_two: bool) -> Result<(), StartGameError> {
    let world = &state.effect_world;
    let valid_counts = [
        (EffectZone::HogwartsDeck, if game_two { 44 } else { 30 }),
        (EffectZone::DarkArtsDeck, if game_two { 15 } else { 10 }),
        (EffectZone::VillainDeck, if game_two { 6 } else { 3 }),
        (EffectZone::ActiveLocation, 1),
        (EffectZone::LocationDeck, if game_two { 2 } else { 1 }),
        (EffectZone::Heroes, state.players.len()),
    ]
    .into_iter()
    .all(|(zone, count)| world.entities_in(zone).len() == count);
    let valid_hero_decks = state.players.iter().all(|player| {
        world
            .cards_in_zone(player.position, EffectZone::HeroDrawPile)
            .len()
            == 10
    });
    let valid_entities = world
        .entities()
        .all(|(zone, entity)| match (zone, entity.kind()) {
            (EffectZone::Heroes, EffectEntityKind::Hero) => {
                entity.resource(EffectResource::Health) == 10
                    && entity.resource(EffectResource::Attack) == 0
                    && entity.resource(EffectResource::Influence) == 0
                    && !entity.drawing_blocked()
            }
            (EffectZone::HeroDrawPile, EffectEntityKind::StarterCard)
            | (EffectZone::HogwartsDeck, EffectEntityKind::HogwartsCard)
            | (EffectZone::DarkArtsDeck, EffectEntityKind::DarkArts)
            | (EffectZone::VillainDeck, EffectEntityKind::Villain) => true,
            (EffectZone::ActiveLocation | EffectZone::LocationDeck, EffectEntityKind::Location) => {
                entity.resource(EffectResource::Control) == 0
                    && entity.resource_limit(EffectResource::Control)
                        == Some(if game_two && entity.catalog_id() == Some("location:005") {
                            5
                        } else {
                            4
                        })
                    && entity.dark_arts_count()
                        == Some(if game_two && entity.catalog_id() == Some("location:005") {
                            2
                        } else {
                            1
                        })
            }
            _ => false,
        });
    if valid_counts && valid_hero_decks && valid_entities {
        Ok(())
    } else {
        Err(StartGameError::InvalidInitialEntities)
    }
}

pub(crate) fn valid_samples(
    samples: &[PreparationSample],
    positions: &[u8],
    counter: u64,
    adventure_id: &str,
) -> bool {
    if samples.is_empty() {
        return true;
    }
    let game_two = adventure_id == "adventure:002";
    let expected = [
        (
            EffectZone::HogwartsDeck,
            None,
            if game_two { 44 } else { 30 },
        ),
        (
            EffectZone::DarkArtsDeck,
            None,
            if game_two { 15 } else { 10 },
        ),
        (EffectZone::VillainDeck, None, if game_two { 6 } else { 3 }),
    ]
    .into_iter()
    .chain(
        positions
            .iter()
            .copied()
            .map(|position| (EffectZone::HeroDrawPile, Some(position), 10)),
    )
    .flat_map(|(zone, owner, size)| (2..=size).rev().map(move |upper| (zone, owner, upper)))
    .collect::<Vec<_>>();
    samples.len() == expected.len()
        && u64::try_from(samples.len()).is_ok_and(|count| count <= counter)
        && samples.iter().zip(expected).all(|(sample, expected)| {
            (sample.zone, sample.owner_position, sample.upper_exclusive) == expected
                && sample.result < sample.upper_exclusive
        })
}

/// One unbiased sample used to shuffle a preparation pile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparationSample {
    pub zone: EffectZone,
    pub owner_position: Option<u8>,
    pub upper_exclusive: u32,
    pub result: u32,
}

struct PreparationRecorder<'a> {
    random: &'a mut dyn EffectRoller,
    samples: &'a mut Vec<PreparationSample>,
    zone: EffectZone,
    owner_position: Option<u8>,
}

impl EffectRoller for PreparationRecorder<'_> {
    fn roll(&mut self, _die: EffectDie) -> Option<u8> {
        None
    }

    fn sample_below(&mut self, upper_exclusive: u32) -> Option<u32> {
        let result = self
            .random
            .sample_below(upper_exclusive)
            .filter(|result| *result < upper_exclusive)?;
        self.samples.push(PreparationSample {
            zone: self.zone,
            owner_position: self.owner_position,
            upper_exclusive,
            result,
        });
        Some(result)
    }
}
