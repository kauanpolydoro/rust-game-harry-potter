use game_domain::{GameCommand, legal_game_commands};
use serde::Serialize;
use uuid::Uuid;

use super::{
    StoredRoomParticipant, codec::command_domain_state, codec::decode_persisted_snapshot,
    codec::verify_persisted_snapshot, hero_name, postgres,
};
use crate::http_support::ApiError;

#[derive(Serialize)]
pub(crate) struct GameProjectionResponse {
    pub(super) game: GameSummary,
    pub(super) snapshot: SnapshotSummary,
    turn: TurnSummary,
    participant: GameParticipant,
    participants: Vec<GameParticipant>,
    legal_actions: Vec<String>,
    choice: ChoiceSummary,
}

#[derive(Serialize)]
pub(super) struct GameSummary {
    pub(super) id: String,
    status: String,
    adventure: AdventureSummary,
    expires_at: String,
}

#[derive(Serialize)]
struct AdventureSummary {
    id: String,
    name: String,
}

#[derive(Serialize)]
pub(super) struct SnapshotSummary {
    pub(super) snapshot_version: i16,
    state_version: i64,
    sequence: i64,
    pub(super) cursor: i64,
    digest: String,
    versions: GameVersions,
}

#[derive(Serialize)]
struct ChoiceSummary {
    status: &'static str,
}

#[derive(Serialize)]
struct GameVersions {
    content: String,
    ruleset: String,
    manifest: i16,
    manifest_digest: String,
    prng: String,
    shuffle: String,
    sampling: String,
}

#[derive(Serialize)]
struct TurnSummary {
    number: u32,
    phase: String,
    active_position: u8,
}

#[derive(Serialize)]
struct GameParticipant {
    display_name: String,
    role: String,
    position: i16,
    hero: GameHero,
}

#[derive(Serialize)]
struct GameHero {
    id: String,
    name: &'static str,
}

pub(crate) async fn projection_for_participant(
    database: &sqlx::PgPool,
    participant_id: Uuid,
) -> Result<Option<GameProjectionResponse>, ApiError> {
    let Some(game) = postgres::game_for_participant(database, participant_id).await? else {
        return Ok(None);
    };
    let persisted = decode_persisted_snapshot(&game.snapshot_json)?;
    verify_persisted_snapshot(&game, &persisted)?;
    let participants = postgres::game_participants(database, game.id).await?;
    let current = participants
        .iter()
        .find(|participant| participant.id == participant_id)
        .ok_or_else(ApiError::internal)?;
    let domain_state = command_domain_state(&persisted)?;
    let actor_position = u8::try_from(current.position)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let legal_actions = if game.expired {
        Vec::new()
    } else {
        legal_game_commands(&domain_state, actor_position)
            .into_iter()
            .map(|command| match command {
                GameCommand::CompleteDarkArts => "complete_dark_arts".to_owned(),
            })
            .collect()
    };

    Ok(Some(GameProjectionResponse {
        game: GameSummary {
            id: game.id.to_string(),
            status: game.status,
            adventure: AdventureSummary {
                id: game.adventure_id,
                name: game.adventure_name,
            },
            expires_at: game.expires_at,
        },
        snapshot: SnapshotSummary {
            snapshot_version: game.snapshot_version,
            state_version: game.state_version,
            sequence: game.sequence,
            cursor: game.sequence,
            digest: game.state_digest,
            versions: GameVersions {
                content: game.content_version,
                ruleset: game.ruleset_version,
                manifest: game.manifest_version,
                manifest_digest: game.manifest_digest,
                prng: game.prng_algorithm,
                shuffle: game.shuffle_algorithm,
                sampling: game.sampling_algorithm,
            },
        },
        turn: TurnSummary {
            number: persisted.turn.number,
            phase: persisted.turn.phase,
            active_position: persisted.turn.active_position,
        },
        participant: game_participant(current)?,
        participants: participants
            .iter()
            .map(game_participant)
            .collect::<Result<Vec<_>, _>>()?,
        legal_actions,
        choice: ChoiceSummary { status: "none" },
    }))
}

fn game_participant(stored: &StoredRoomParticipant) -> Result<GameParticipant, ApiError> {
    let hero_id = stored.hero_id.as_deref().ok_or_else(ApiError::internal)?;
    Ok(GameParticipant {
        display_name: stored.display_name.clone(),
        role: stored.role.clone(),
        position: stored.position,
        hero: GameHero {
            id: hero_id.to_owned(),
            name: hero_name(hero_id)?,
        },
    })
}
