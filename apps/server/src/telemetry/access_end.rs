use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};

use crate::{AppState, session::AuthenticatedSession};

/// Close acknowledgement measures the remote end, including return transit.
/// Missing commit-time evidence never delays or prevents access termination.
pub(crate) async fn close_access(
    socket: &mut WebSocket,
    state: &AppState,
    session: AuthenticatedSession,
) {
    let code = crate::session::inactive_session_close_code(state, session).await;
    let operation = if code == 4001 {
        "expiration"
    } else {
        "revocation"
    };
    super::observe("access_end_socket_closures", operation, "started", 1.0);
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        socket
            .send(Message::Close(Some(CloseFrame {
                code,
                reason: "access ended".into(),
            })))
            .await?;
        loop {
            match socket.recv().await {
                None | Some(Ok(Message::Close(_))) => return Ok::<_, axum::Error>(()),
                Some(Err(error)) => return Err(error),
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    let outcome = match result {
        Ok(Ok(())) => "success",
        Ok(Err(_)) => "error",
        Err(_) => "timeout",
    };
    super::observe("access_end_socket_closures", operation, outcome, 1.0);
    if outcome != "success" {
        return;
    }
    // Telemetry queries run only after the remote end has closed. They must not
    // compete with authorization queries or delay sending the Close frame.
    let measured = Instant::now();
    let age = tokio::time::timeout(
        Duration::from_millis(50),
        sqlx::query_scalar::<_, Option<f64>>(
            r"SELECT EXTRACT(EPOCH FROM (clock_timestamp() - CASE
               WHEN $2 THEN games.expires_at
               WHEN current_setting('track_commit_timestamp') = 'on' AND devices.status <> 'active'
               THEN pg_xact_commit_timestamp(devices.xmin)
               ELSE NULL END))::float8
             FROM device_sessions AS devices
             JOIN participants ON participants.id = devices.participant_id
             LEFT JOIN games ON games.room_id = participants.room_id
            WHERE devices.guest_session_id = $1",
        )
        .bind(session.session_id)
        .bind(code == 4001)
        .fetch_optional(&state.database),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
    .flatten()
    .filter(|value| value.is_finite() && *value >= 0.0);
    super::observe(
        "access_end_time_observations",
        operation,
        if age.is_some() {
            "success"
        } else {
            "unavailable"
        },
        1.0,
    );
    if let Some(age) = age {
        super::observe(
            "access_end_to_close_upper_bound_seconds",
            operation,
            "success",
            age + measured.elapsed().as_secs_f64(),
        );
    }
}
