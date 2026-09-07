use crate::{Effect, EffectTrigger, EntryKind, GameSetupOwner, ImportFailure, Zone};

use super::CandidateBundle;

pub(super) fn validate_setup(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    let Some(setup) = bundle.game_setups.first() else {
        return Ok(());
    };
    if bundle.game_setups.len() != 1 || setup.adventure_id.as_str() != "adventure:001" {
        return Err(ImportFailure {
            message: "Game 1 requires exactly its own preparation".to_owned(),
        });
    }
    let dark_arts = bundle
        .rules
        .iter()
        .filter(|rule| rule.trigger == EffectTrigger::DarkArts)
        .collect::<Vec<_>>();
    if dark_arts.len() != 1 || !matches!(dark_arts[0].effect, Effect::RevealDarkArts) {
        return Err(ImportFailure {
            message: "Game 1 requires exactly one Dark Arts revelation root".to_owned(),
        });
    }
    let mut expected_entities = 0;
    for entry in &bundle.entries {
        let Some((zone, owner, copies)) = placement(entry.kind, entry.id.as_str(), entry.copies)
        else {
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
                    "Game 1 preparation must place {copies} copies of {} in {zone:?} for {owner:?}",
                    entry.id
                ),
            });
        }
    }
    if setup.entities.len() != expected_entities {
        return Err(ImportFailure {
            message: "Game 1 preparation contains extra entities".to_owned(),
        });
    }
    Ok(())
}

fn placement(kind: EntryKind, id: &str, copies: u16) -> Option<(Zone, GameSetupOwner, u16)> {
    match kind {
        EntryKind::HogwartsCard => Some((Zone::HogwartsDeck, GameSetupOwner::None, copies)),
        EntryKind::DarkArts => Some((Zone::DarkArtsDeck, GameSetupOwner::None, copies)),
        EntryKind::Villain => Some((Zone::VillainDeck, GameSetupOwner::None, copies)),
        EntryKind::Location => Some((
            if id == "location:001" {
                Zone::ActiveLocation
            } else {
                Zone::LocationDeck
            },
            GameSetupOwner::None,
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
