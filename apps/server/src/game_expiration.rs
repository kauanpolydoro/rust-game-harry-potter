use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::http_support::ApiError;

pub(crate) async fn participant_game_expired(
    database: &PgPool,
    participant_id: Uuid,
) -> Result<bool, ApiError> {
    // Most reads do not take a lock. A suspected expiration is rechecked under
    // the same game root as commands before its revocation becomes durable.
    sqlx::query_scalar::<_, bool>(
        r"
        SELECT CASE
            WHEN games.access_expired_at IS NOT NULL THEN TRUE
            WHEN clock_timestamp() >= games.expires_at THEN expire_game_access(games.id)
            ELSE FALSE
        END
        FROM games
        JOIN participants ON participants.room_id = games.room_id
        WHERE participants.id = $1
        ",
    )
    .bind(participant_id)
    .fetch_optional(database)
    .await
    .map(|expired| expired.unwrap_or(false))
    .map_err(|error| ApiError::internal_with("enforce game expiration", error))
}

pub(crate) async fn expire_locked_game(
    transaction: &mut Transaction<'_, Postgres>,
    game_id: Uuid,
) -> Result<bool, ApiError> {
    sqlx::query_scalar("SELECT expire_game_access($1)")
        .bind(game_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| ApiError::internal_with("enforce locked game expiration", error))
}

pub(crate) async fn expire_due_games(database: &PgPool) -> Result<(), sqlx::Error> {
    let mut transaction = database.begin().await?;
    let games = sqlx::query_scalar::<_, Uuid>(
        r"
        SELECT id FROM games
        WHERE access_expired_at IS NULL AND expires_at <= statement_timestamp()
        ORDER BY expires_at
        LIMIT 100
        FOR UPDATE SKIP LOCKED
        ",
    )
    .fetch_all(&mut *transaction)
    .await?;
    for game_id in games {
        sqlx::query("SELECT expire_game_access($1)")
            .bind(game_id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await
}
