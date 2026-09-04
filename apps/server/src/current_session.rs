use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};

use crate::{
    AppState, http_support::ApiError, http_support::no_store_json, identity_access, match_runtime,
    session::authenticated_participant,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/api/session", get(restore_session))
}

async fn restore_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let participant_id = authenticated_participant(&state, &headers).await?;
    if let Some(projection) =
        match_runtime::projection_for_participant(&state.database, participant_id).await?
    {
        return Ok(no_store_json(StatusCode::OK, projection));
    }
    let lobby = identity_access::lobby_for_participant(&state, participant_id).await?;
    Ok(no_store_json(StatusCode::OK, lobby))
}
