use crate::{Effect, EffectTrigger, EntryKind, GameSetupOwner, ImportFailure, Zone};

use super::CandidateBundle;

pub(super) fn validate_setup(
    bundle: &CandidateBundle,
    inventory: super::super::Inventory,
) -> Result<(), ImportFailure> {
    let game_four = matches!(inventory, super::super::Inventory::GameFour);
    let game_three = matches!(
        inventory,
        super::super::Inventory::GameThree | super::super::Inventory::GameFour
    );
    let game_two = matches!(inventory, super::super::Inventory::GameTwo);
    let Some(setup) = bundle.game_setups.first() else {
        return Ok(());
    };
    if bundle.game_setups.len() != 1
        || setup.adventure_id.as_str()
            != if game_four {
                "adventure:004"
            } else if game_three {
                "adventure:003"
            } else if game_two {
                "adventure:002"
            } else {
                "adventure:001"
            }
    {
        return Err(ImportFailure {
            message: "adventure requires exactly its own preparation".to_owned(),
        });
    }
    let dark_arts = bundle
        .rules
        .iter()
        .filter(|rule| rule.trigger == EffectTrigger::DarkArts)
        .collect::<Vec<_>>();
    if dark_arts.len() != 1 || !matches!(dark_arts[0].effect, Effect::RevealDarkArts) {
        return Err(ImportFailure {
            message: "adventure requires exactly one Dark Arts revelation root".to_owned(),
        });
    }
    let locations = setup
        .entities
        .iter()
        .filter(|entity| entity.zone == Zone::LocationDeck)
        .map(|entity| entity.catalog_id.as_str())
        .collect::<Vec<_>>();
    if locations
        != if game_four {
            vec!["location:010", "location:011"]
        } else if game_three {
            vec!["location:007", "location:008"]
        } else if game_two {
            vec!["location:004", "location:005"]
        } else {
            vec!["location:002"]
        }
    {
        return Err(ImportFailure {
            message: "location deck must preserve the adventure progression".to_owned(),
        });
    }
    let mut expected_entities = 0;
    for entry in &bundle.entries {
        let Some((zone, owner, copies)) = placement(
            entry.kind,
            entry.id.as_str(),
            entry.copies,
            game_two,
            game_three,
            game_four,
        ) else {
            continue;
        };
        expected_entities += 1;
        let valid = setup.entities.iter().any(|entity| {
            entity.catalog_id == entry.id
                && (entity.zone, entity.owner, entity.copies) == (zone, owner, copies)
        });
        if !valid {
            return Err(ImportFailure {
                message: format!(
                    "adventure preparation must place {copies} copies of {} in {zone:?} for {owner:?}",
                    entry.id
                ),
            });
        }
    }
    if setup.entities.len() != expected_entities {
        return Err(ImportFailure {
            message: "adventure preparation contains extra entities".to_owned(),
        });
    }
    Ok(())
}

fn placement(
    kind: EntryKind,
    id: &str,
    copies: u16,
    game_two: bool,
    game_three: bool,
    game_four: bool,
) -> Option<(Zone, GameSetupOwner, u16)> {
    match kind {
        EntryKind::HogwartsCard => Some((Zone::HogwartsDeck, GameSetupOwner::None, copies)),
        EntryKind::DarkArts => Some((Zone::DarkArtsDeck, GameSetupOwner::None, copies)),
        EntryKind::Villain => Some((Zone::VillainDeck, GameSetupOwner::None, copies)),
        EntryKind::Location => Some((
            if id
                == if game_four {
                    "location:009"
                } else if game_three {
                    "location:006"
                } else if game_two {
                    "location:003"
                } else {
                    "location:001"
                }
            {
                Zone::ActiveLocation
            } else {
                Zone::LocationDeck
            },
            GameSetupOwner::None,
            1,
        )),
        EntryKind::Hero if game_three => Some((
            Zone::Heroes,
            match id {
                "hero:002" => GameSetupOwner::Harry,
                "hero:005" => GameSetupOwner::Hermione,
                "hero:008" => GameSetupOwner::Neville,
                "hero:011" => GameSetupOwner::Ron,
                _ => return None,
            },
            1,
        )),
        EntryKind::StarterCard => Some((
            Zone::HeroDrawPile,
            match id {
                "starter:001" => GameSetupOwner::EachParticipant,
                "starter:002" | "starter:003" | "starter:004" => GameSetupOwner::Harry,
                "starter:005" | "starter:006" | "starter:007" => GameSetupOwner::Ron,
                "starter:008" | "starter:009" | "starter:010" => GameSetupOwner::Hermione,
                "starter:011" | "starter:012" | "starter:013" => GameSetupOwner::Neville,
                _ => return None,
            },
            if id == "starter:001" { 7 } else { 1 },
        )),
        _ => None,
    }
}
