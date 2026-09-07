use game_domain::{EffectEntity, EffectOutcome, InitialGameState};

use super::{TableSummary, card_summary};
use crate::content_catalog::ContentCatalog;

// The previous client rejects unknown projection keys. Keep the public shape
// stable and describe Game 3 state on the card that owns the effect.
pub(super) fn describe_table(
    table: &mut TableSummary,
    state: &InitialGameState,
    content: &ContentCatalog,
    digest: &str,
) {
    if state.snapshot_version() != 7 {
        return;
    }
    for villain in &mut table.active_villains {
        let Some((_, entity)) = state.effect_world().entity(&villain.instance_id) else {
            continue;
        };
        append(&mut villain.description, villain_status(entity, state));
        append(
            &mut villain.description,
            revelations(
                state,
                state.last_effects(),
                content,
                digest,
                entity.effect_rule_id(),
            ),
        );
    }
    if let Some(dark_arts) = &mut table.revealed_dark_arts {
        // A turn can resolve two Dark Arts before publishing its projection.
        // Retain each revelation's source name even when a later card is shown.
        append(
            &mut dark_arts.description,
            revelations(state, state.last_effects(), content, digest, None),
        );
    }
}

fn append(description: &mut Option<String>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(description) = description {
        description.push(' ');
        description.push_str(&text);
    } else {
        *description = Some(text);
    }
}

fn hero_name(state: &InitialGameState, position: u8) -> &'static str {
    state
        .players()
        .iter()
        .find(|player| player.position() == position)
        .map_or("Herói", |player| player.hero().name())
}

fn villain_status(entity: &EffectEntity, state: &InitialGameState) -> String {
    let Some(turn) = entity.turn_state() else {
        return String::new();
    };
    let mut descriptions = Vec::new();
    for position in &turn.suppressed_by {
        descriptions.push(format!(
            "Bloqueio ativo até o início do próximo turno de {}.",
            hero_name(state, *position)
        ));
    }
    if let Some(limit) = turn.attack_limit {
        descriptions.push(format!(
            "Pode receber mais {} de Ataque neste turno.",
            u16::from(limit).saturating_sub(turn.attack_assigned)
        ));
    }
    descriptions.join(" ")
}

fn revelations(
    state: &InitialGameState,
    outcomes: &[EffectOutcome],
    content: &ContentCatalog,
    digest: &str,
    villain_rule: Option<&str>,
) -> String {
    outcomes
        .iter()
        .filter_map(|outcome| {
            let EffectOutcome::TopCardRevealed {
                rule_id,
                card_id,
                owner_position,
            } = outcome
            else {
                return None;
            };
            let source_is_villain = state.effect_world().entities().any(|(_, entity)| {
                entity.kind() == game_domain::EffectEntityKind::Villain
                    && entity.effect_rule_id() == Some(rule_id)
            });
            if villain_rule.map_or(source_is_villain, |expected| expected != rule_id) {
                return None;
            }
            let (_, card) = state.effect_world().entity(card_id)?;
            let card = card_summary(card, content, digest)?;
            let source = content.rule_name(digest, rule_id)?;
            Some(format!(
                "{source}: {} revelou {}.",
                hero_name(state, *owner_position),
                card.name
            ))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_runtime::game_one_tests::prepared_adventure;
    use game_domain::{EffectTurnState, EffectZone};

    #[test]
    fn game_three_status_describes_each_caster_and_the_remaining_attack_quota() {
        let (state, _, _) = prepared_adventure(2, crate::game_three_manifest(), "adventure:003");
        let villain = state.effect_world().entities_in(EffectZone::ActiveVillains)[0]
            .clone()
            .with_turn_state(Some(EffectTurnState {
                suppressed_by: vec![1, 2],
                attack_limit: Some(1),
                attack_assigned: 1,
                ..EffectTurnState::default()
            }));
        assert_eq!(
            villain_status(&villain, &state),
            "Bloqueio ativo até o início do próximo turno de Harry. Bloqueio ativo até o início do próximo turno de Hermione. Pode receber mais 0 de Ataque neste turno."
        );
    }

    #[test]
    fn game_three_revelations_retain_the_source_owner_and_card_without_exposing_other_cards() {
        let manifest = crate::game_three_manifest();
        let digest = manifest.digest.clone();
        let catalog = ContentCatalog::new(vec![manifest]);
        let (state, _, _) = prepared_adventure(2, crate::game_three_manifest(), "adventure:003");
        let card = state
            .effect_world()
            .entities_in(EffectZone::HeroDrawPile)
            .iter()
            .find(|card| card.owner_position() == Some(2))
            .expect("Hermione's card");
        let dark_rule = catalog
            .effect_rules(&digest)
            .expect("rules")
            .iter()
            .find(|rule| catalog.rule_name(&digest, &rule.id).as_deref() == Some("Opugno"))
            .expect("Opugno")
            .id
            .clone();
        let peter = state
            .effect_world()
            .entities()
            .find(|(_, entity)| entity.catalog_id() == Some("villain:008"))
            .expect("Peter")
            .1;
        let peter_rule = peter.effect_rule_id().expect("Peter's rule");
        let make_reveal = |rule_id: &str| EffectOutcome::TopCardRevealed {
            rule_id: rule_id.to_owned(),
            card_id: card.id().to_owned(),
            owner_position: 2,
        };
        let outcomes = [make_reveal(&dark_rule), make_reveal(peter_rule)];
        let card_name = catalog
            .entity_name(&digest, card.catalog_id().expect("catalog"))
            .expect("name");
        assert_eq!(
            revelations(&state, &outcomes, &catalog, &digest, None),
            format!("Opugno: Hermione revelou {card_name}.")
        );
        assert_eq!(
            revelations(&state, &outcomes, &catalog, &digest, Some(peter_rule)),
            format!("Pedro Pettigrew: Hermione revelou {card_name}.")
        );
    }
}
