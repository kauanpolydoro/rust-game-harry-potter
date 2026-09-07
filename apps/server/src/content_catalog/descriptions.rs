use game_domain::{
    EffectCardType, EffectDefinition, EffectOperation, EffectReactionTrigger, EffectResource,
    EffectSelector, EffectTargetOwner, EffectZone,
};

pub(super) fn describe(effect: &EffectDefinition, actor: &str) -> Option<String> {
    Some(match effect {
        EffectDefinition::PreventExtraDrawing => "Enquanto este Vilão estiver ativo, os Heróis não podem comprar cartas extras. A reposição da mão no fim do turno continua permitida.".to_owned(),
        EffectDefinition::ForEachTarget { target, effect } => if target.zone == EffectZone::Heroes && target.owner == EffectTargetOwner::Actor {
            describe(effect, actor)?
        } else if target.zone == EffectZone::Heroes {
            format!("Para cada Herói: {}", describe(effect, "Esse Herói")?)
        } else { format!("Para cada {} na mão ao iniciar este efeito: {}", category_name(target), describe(effect, actor)?) },
        EffectDefinition::CardType { .. } | EffectDefinition::NoOp => String::new(),
        EffectDefinition::Apply { target, operation } => {
            describe_operation(target, operation, actor)?
        }
        EffectDefinition::Sequence { effects } => effects
            .iter()
            .map(|effect| describe(effect, actor))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        EffectDefinition::Choice { options, .. } => {
            let options = options
                .iter()
                .map(|option| describe(option, actor))
                .collect::<Option<Vec<_>>>()?;
            format!("Escolha uma opção: {}", options.join(" Ou: "))
        }
        EffectDefinition::TopDeckAcquisition { card_type } => format!(
            "Neste turno, você pode colocar {} adquiridos no topo do seu baralho.",
            match card_type {
                EffectCardType::Ally => "Aliados",
                EffectCardType::Item => "Itens",
                EffectCardType::Spell => "Feitiços",
            }
        ),
        EffectDefinition::HandDamageLimit { maximum } => format!(
            "Enquanto esta carta estiver na sua mão, cada efeito de Artes das Trevas ou Vilão tira no máximo {maximum} de Vida."
        ),
        EffectDefinition::Reaction { trigger, effect } => {
            let (condition, subject) = match trigger {
                EffectReactionTrigger::ControlAdded => {
                    ("A cada Controle adicionado ao Local", "O Herói ativo")
                }
                EffectReactionTrigger::HeroForcedDiscard => (
                    "Quando um Herói descartar uma carta por um efeito",
                    "Esse Herói",
                ),
                EffectReactionTrigger::SelfForcedDiscard => {
                    ("Ao descartar esta carta por um efeito", "Você")
                }
                EffectReactionTrigger::OwnerDefeatsVillain => {
                    ("Ao derrotar um Vilão depois de jogar esta carta", "Você")
                }
                EffectReactionTrigger::OwnerPlaysAlly => {
                    ("Ao jogar um Aliado depois desta carta", "Você")
                }
            };
            format!("{condition}: {}", describe(effect, subject)?)
        }
        EffectDefinition::RevealDarkArts => {
            "Revele uma carta de Artes das Trevas e resolva seu efeito.".to_owned()
        }
        EffectDefinition::Repeat { times, effect } => {
            format!("Repita {times} vezes: {}", describe(effect, actor)?)
        }
        EffectDefinition::Condition { condition, then, otherwise } => {
            let fallback = match otherwise.as_deref() { Some(effect) => format!(" Caso contrário: {}", describe(effect, actor)?), None => String::new() };
            let requirement = match condition {
                game_domain::EffectCondition::DrawingAllowed => "Se compras extras forem permitidas".to_owned(),
                game_domain::EffectCondition::HasEligibleTarget { target } => format!("Se houver um {} no próprio {}", category_name(target), if target.zone == EffectZone::HeroDiscardPile { "descarte" } else { "conjunto de cartas da mão" }),
                game_domain::EffectCondition::ResourceAtLeast { resource, amount, .. } => format!("Se {actor} tiver ao menos {amount} de {}", resource_name(*resource)),
            };
            format!("{requirement}: {}{fallback}", describe(then, actor)?)
        }
        EffectDefinition::Roll { .. }
        | EffectDefinition::Terminal { .. } => return None,
    })
}

pub(super) fn at_path<'a>(
    mut effect: &'a EffectDefinition,
    path: &[game_domain::EffectPathSegment],
) -> Option<&'a EffectDefinition> {
    use game_domain::EffectPathSegment as Path;
    for segment in path {
        effect = match (effect, segment) {
            (EffectDefinition::Choice { options, .. }, Path::ChoiceOption(index)) => {
                options.get(usize::from(*index))?
            }
            (EffectDefinition::Sequence { effects }, Path::SequenceEffect(index)) => {
                effects.get(usize::from(*index))?
            }
            (EffectDefinition::Roll { outcomes, .. }, Path::RollOutcome(index)) => {
                outcomes.get(usize::from(*index))?
            }
            (EffectDefinition::Condition { then, .. }, Path::ConditionThen) => then,
            (
                EffectDefinition::Condition {
                    otherwise: Some(effect),
                    ..
                },
                Path::ConditionOtherwise,
            )
            | (EffectDefinition::Reaction { effect, .. }, Path::ReactionEffect)
            | (
                EffectDefinition::ForEachTarget { effect, .. }
                | EffectDefinition::Repeat { effect, .. },
                Path::RepeatEffect,
            ) => effect,
            _ => return None,
        };
    }
    Some(effect)
}

fn describe_operation(
    target: &EffectSelector,
    operation: &EffectOperation,
    actor: &str,
) -> Option<String> {
    let subject = match target.zone {
        EffectZone::ActiveLocation => "O Local",
        EffectZone::Heroes if target.owner == EffectTargetOwner::Any && target.max == 1 => {
            "Um Herói à sua escolha"
        }
        EffectZone::Heroes if target.owner == EffectTargetOwner::Any => "Cada Herói",
        EffectZone::ActiveVillains => "Cada Vilão ativo",
        EffectZone::Heroes
        | EffectZone::HeroHand
        | EffectZone::HeroPlayArea
        | EffectZone::HeroDiscardPile => actor,
        _ => return None,
    };
    Some(match operation {
        EffectOperation::CopyPlayedAlly => {
            "Escolha um Aliado que você jogou neste turno e copie seus efeitos.".to_owned()
        }
        EffectOperation::ModifyResource { resource, amount } => format!(
            "{subject} {} {} de {}.",
            if *amount < 0 { "perde" } else { "recebe" },
            amount.unsigned_abs(),
            resource_name(*resource)
        ),
        EffectOperation::Draw { amount } => format!(
            "{subject} compra {amount} {}.",
            if *amount == 1 { "carta" } else { "cartas" }
        ),
        EffectOperation::PreventDrawing => {
            format!("{subject} não pode comprar cartas extras até o fim deste turno.")
        }
        EffectOperation::Discard | EffectOperation::DiscardVoluntarily => format!(
            "{subject} descarta {} {} da mão.",
            target.min,
            if target.eligibility.is_empty() {
                if target.min == 1 { "carta" } else { "cartas" }
            } else {
                category_name(target)
            }
        ),
        EffectOperation::GainAttackPerAllyPlayed { amount } => {
            format!("{subject} recebe {amount} de Ataque por Aliado já jogado neste turno.")
        }
        EffectOperation::Move {
            to: EffectZone::HeroHand,
        } if target.zone == EffectZone::HeroDiscardPile => format!(
            "{subject} escolhe um {} do próprio descarte e o coloca na mão.",
            category_name(target)
        ),
        EffectOperation::Move { .. } => return None,
    })
}

const fn resource_name(resource: EffectResource) -> &'static str {
    match resource {
        EffectResource::Attack => "Ataque",
        EffectResource::Control => "Controle",
        EffectResource::Health => "Vida",
        EffectResource::Influence => "Influência",
    }
}

fn category_name(target: &EffectSelector) -> &'static str {
    target
        .eligibility
        .iter()
        .find_map(|eligibility| match eligibility {
            game_domain::EffectEligibility::CardType { card_type } => Some(match card_type {
                EffectCardType::Ally => "Aliado",
                EffectCardType::Item => "Item",
                EffectCardType::Spell => "Feitiço",
            }),
            game_domain::EffectEligibility::ResourceAtLeast { .. } => None,
        })
        .unwrap_or("carta")
}
