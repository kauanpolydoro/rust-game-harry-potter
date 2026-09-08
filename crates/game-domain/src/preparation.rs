use crate::{
    EffectDie, EffectEntityKind, EffectResource, EffectRoller, EffectZone, InitialGameState,
    StartGameError,
};

pub(crate) fn prepare_game_one(
    state: &mut InitialGameState,
    random: &mut dyn EffectRoller,
) -> Result<(), StartGameError> {
    prepare(state, random, &GAME_ONE)
}

pub(crate) fn prepare_game_two(
    state: &mut InitialGameState,
    random: &mut dyn EffectRoller,
) -> Result<(), StartGameError> {
    prepare(state, random, &GAME_TWO)
}

struct PreparationInventory {
    hogwarts: usize,
    dark_arts: usize,
    villains: usize,
    active_villains: u8,
    locations: &'static [(&'static str, u16, u8)],
}

const GAME_ONE: PreparationInventory = PreparationInventory {
    hogwarts: 30,
    dark_arts: 10,
    villains: 3,
    active_villains: 1,
    locations: &[("location:001", 4, 1), ("location:002", 4, 1)],
};
const GAME_TWO: PreparationInventory = PreparationInventory {
    hogwarts: 44,
    dark_arts: 15,
    villains: 6,
    active_villains: 1,
    locations: &[
        ("location:003", 4, 1),
        ("location:004", 4, 1),
        ("location:005", 5, 2),
    ],
};
const GAME_THREE: PreparationInventory = PreparationInventory {
    hogwarts: 60,
    dark_arts: 19,
    villains: 8,
    active_villains: 2,
    locations: &[
        ("location:006", 5, 1),
        ("location:007", 6, 2),
        ("location:008", 6, 2),
    ],
};

pub(crate) fn prepare_game_three(
    state: &mut InitialGameState,
    random: &mut dyn EffectRoller,
) -> Result<(), StartGameError> {
    if !valid_game_three_entities(&state.effect_world, &state.players)
        || state.adventure_id != "adventure:003"
        || state.manifest_version != 6
        || state.players.iter().any(|player| {
            !state
                .effect_world
                .entities_in(EffectZone::Heroes)
                .iter()
                .any(|hero| {
                    hero.owner_position() == Some(player.position)
                        && hero.catalog_id() == Some(player.hero.game_three_catalog_id())
                        && hero.turn_state() == Some(&crate::EffectTurnState::default())
                })
        })
    {
        return Err(StartGameError::InvalidInitialEntities);
    }
    prepare(state, random, &GAME_THREE)
}

fn prepare(
    state: &mut InitialGameState,
    random: &mut dyn EffectRoller,
    inventory: &PreparationInventory,
) -> Result<(), StartGameError> {
    validate_initial_layout(state, inventory)?;
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
        (
            EffectZone::VillainDeck,
            EffectZone::ActiveVillains,
            inventory.active_villains,
        ),
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
    state.active_villain_limit = inventory.active_villains;
    Ok(())
}

fn validate_initial_layout(
    state: &InitialGameState,
    inventory: &PreparationInventory,
) -> Result<(), StartGameError> {
    let world = &state.effect_world;
    let valid_counts = [
        (EffectZone::HogwartsDeck, inventory.hogwarts),
        (EffectZone::DarkArtsDeck, inventory.dark_arts),
        (EffectZone::VillainDeck, inventory.villains),
        (EffectZone::ActiveLocation, 1),
        (EffectZone::LocationDeck, inventory.locations.len() - 1),
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
                    && inventory
                        .locations
                        .iter()
                        .find(|(id, _, _)| entity.catalog_id() == Some(*id))
                        .or_else(|| {
                            (inventory.active_villains == 1).then(|| &inventory.locations[0])
                        })
                        .is_some_and(|(_, control, dark_arts)| {
                            entity.resource_limit(EffectResource::Control) == Some(*control)
                                && entity.dark_arts_count() == Some(*dark_arts)
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
    let inventory = match adventure_id {
        "adventure:003" => &GAME_THREE,
        "adventure:002" => &GAME_TWO,
        _ => &GAME_ONE,
    };
    let expected = [
        (EffectZone::HogwartsDeck, None, inventory.hogwarts),
        (EffectZone::DarkArtsDeck, None, inventory.dark_arts),
        (EffectZone::VillainDeck, None, inventory.villains),
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
            (
                sample.zone,
                sample.owner_position,
                usize::try_from(sample.upper_exclusive).unwrap_or(usize::MAX),
            ) == expected
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

pub(crate) fn valid_game_three_entities(
    world: &crate::EffectWorld,
    players: &[crate::InitialPlayer],
) -> bool {
    world.entities().all(|(_, entity)| match entity.kind() {
        EffectEntityKind::Hero => {
            entity.turn_state().is_some()
                && players.iter().any(|player| {
                    entity.owner_position() == Some(player.position())
                        && entity.catalog_id() == Some(player.hero().game_three_catalog_id())
                        && entity.effect_rule_id()
                            == Some(
                                format!(
                                    "rule:g3-{}-ability",
                                    player.hero().game_three_catalog_id().replace(':', "-")
                                )
                                .as_str(),
                            )
                })
        }
        EffectEntityKind::Villain => entity.turn_state().is_some(),
        _ => entity.turn_state().is_none(),
    })
}
