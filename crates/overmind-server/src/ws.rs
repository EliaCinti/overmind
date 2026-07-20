//! Live update channel for the UI (M4, ADR-0010).
//!
//! One WebSocket endpoint. On connect the client is told to do a full
//! refresh; thereafter it receives a coarse `{ "type": "changed",
//! "company_id": ... }` whenever a board changes and refetches that scope.
//! Coarse-by-design: the wire contract can't desync from server state.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;

use crate::db::AppState;

pub async fn handler(
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| pump(socket, state))
}

async fn pump(mut socket: WebSocket, state: AppState) {
    let mut rx = state.events.subscribe();

    // Nudge the freshly-connected client to load current state.
    if socket
        .send(Message::Text(r#"{"type":"hello"}"#.into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // Lagged: the client fell behind. Tell it to resync wholesale.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    if socket
                        .send(Message::Text(r#"{"type":"hello"}"#.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            // Drain client frames so pings/pongs and close are handled.
            client = socket.recv() => match client {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
}
