use game_domain::{
    EffectCardType, EffectDefinition, EffectOperation, EffectReactionTrigger, EffectResource,
    EffectSelector, EffectTargetOwner, EffectZone,
};

pub(super) fn describe(effect: &EffectDefinition, actor: &str) -> Option<String> {
    Some(match effect {
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
        EffectDefinition::Condition { .. }
        | EffectDefinition::Roll { .. }
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
            | (EffectDefinition::Repeat { effect, .. }, Path::RepeatEffect) => effect,
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
        EffectZone::Heroes | EffectZone::HeroHand => actor,
        _ => return None,
    };
    Some(match operation {
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
        EffectOperation::Discard => format!(
            "{subject} descarta {} {} da mão.",
            target.min,
            if target.min == 1 { "carta" } else { "cartas" }
        ),
        EffectOperation::GainAttackPerAllyPlayed { amount } => {
            format!("{subject} recebe {amount} de Ataque por Aliado já jogado neste turno.")
        }
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
