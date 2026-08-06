//! Live update channel for the UI (M4, ADR-0010).
//!
//! One WebSocket endpoint. On connect the client is told to do a full
//! refresh; thereafter it receives a coarse `{ "type": "changed",
//! "company_id": ... }` whenever a board changes and refetches that scope.
//! Coarse-by-design: the wire contract can't desync from server state.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};

use crate::db::AppState;

/// Origins allowed to open the live socket during development, when the UI is
/// served by Vite instead of by us.
const DEV_ORIGINS: &[&str] = &["http://localhost:5173", "http://127.0.0.1:5173"];

/// Refuse the upgrade to any page that is not ours, **before** the upgrade is
/// attempted.
///
/// WebSockets are not subject to CORS: the browser opens the connection and
/// hands the server an `Origin` to judge for itself. Without this, any page you
/// have open could subscribe to the company's live feed — notifications,
/// decisions, everything — from 127.0.0.1. Same reasoning as the CORS policy in
/// `api.rs`: local does not mean private.
///
/// A missing `Origin` is a non-browser client (curl, a test, an MCP client) and
/// is allowed; only browsers send one, and then it must be ours.
pub async fn guard_origin(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let headers = request.headers();
    if let Some(origin) = headers.get(axum::http::header::ORIGIN) {
        let origin = origin.to_str().unwrap_or_default();
        let ours = headers
            .get(axum::http::header::HOST)
            .and_then(|h| h.to_str().ok())
            .map(|host| origin == format!("http://{host}") || origin == format!("https://{host}"))
            .unwrap_or(false);
        if !ours && !DEV_ORIGINS.contains(&origin) {
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }
    }
    next.run(request).await
}

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
