//! Overmind speaking MCP **to agents** (ADR-0027, and M9's foundation).
//!
//! The other direction lives in [`crate::mcp`]: that is the *client*, Overmind
//! asking a memory provider. This is the *server*, an agent asking Overmind.
//! The two must not be confused — they share a protocol and nothing else.
//!
//! ## Why this exists at all
//!
//! ADR-0015 promised agents that call `recall` and `why` themselves. The direct
//! way — hand each agent a Wadachi over stdio pointed at its company's brain —
//! does not work: ADR-0023's `(deny default)` cage reaches neither the brain
//! directory nor anything outside the run dir, so a Wadachi spawned by the
//! agent would be spawned inside the cage and could not read what it was
//! pointed at. Rather than widen a three-week-old security boundary, the agent
//! talks to Overmind, which is already an HTTP server and already reachable
//! (the cage permits `network*`).
//!
//! ## Two rules that are not negotiable here
//!
//! **Agents read; Overmind writes.** `store_memory` and `store_decision` are
//! not exposed. ADR-0015 decided the completion-time write stays
//! orchestrator-authoritative *with the task as provenance*, and ADR-0025 made
//! that a real `memory_links` row. An agent writing directly would produce
//! memories with nothing behind them, and it would break quietly.
//!
//! **A request names no company.** It carries a bearer token; the company and
//! its brain are resolved from that token's session row. There is no argument
//! an agent could set to reach another company's memory, because there is no
//! such argument.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};

use crate::db::AppState;

/// The tools an agent may call. Read-only against memory, by decision.
const TOOLS: &[(&str, &str)] = &[
    (
        "recall",
        "Search what this company already knows — decisions, patterns, prior fixes. \
         Call this before investigating something: it may already be answered.",
    ),
    (
        "why",
        "Ask why a past choice was made. Returns the rationale, the context, and \
         what was rejected, so a settled question is not silently reopened.",
    ),
    (
        "brain_watermark",
        "The organization's memory position right now. Take one before a long \
         piece of work, then use `changed_since` to see what moved.",
    ),
    (
        "changed_since",
        "What was written after a position taken earlier — what you missed while \
         you were working. No query needed.",
    ),
];

pub fn router() -> Router<AppState> {
    Router::new().route("/mcp", post(handle))
}

/// Resolve a bearer token to the company whose work it belongs to.
///
/// Returns `None` for an absent, malformed, unknown or retired token — all four
/// are the same answer to the caller, deliberately: distinguishing them would
/// tell someone probing the endpoint which of their guesses was closer.
async fn company_for(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim();
    if token.is_empty() {
        return None;
    }
    sqlx::query_scalar::<_, String>(
        "SELECT t.company_id
           FROM agent_task_sessions s
           JOIN tasks t ON t.id = s.task_id
          WHERE s.mcp_token = ?",
    )
    .bind(token)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
}

fn rpc_result(id: Value, result: Value) -> Json<Value> {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Json<Value> {
    Json(json!({
        "jsonrpc": "2.0", "id": id,
        "error": { "code": code, "message": message }
    }))
}

/// A tool answer. MCP carries tool failures *inside* a successful result with
/// `isError`, not as a protocol error: the model is meant to read them and
/// decide what to do, and a transport-level error would just look like the tool
/// does not exist.
fn tool_text(id: Value, text: String, is_error: bool) -> Json<Value> {
    rpc_result(
        id,
        json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error
        }),
    )
}

async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Notifications carry no id and expect no reply.
    if id.is_null() && method.starts_with("notifications/") {
        return (StatusCode::ACCEPTED, Json(json!({}))).into_response();
    }

    let Some(company_id) = company_for(&state, &headers).await else {
        // 401 rather than a JSON-RPC error: the caller has not been identified,
        // so there is no session to answer on behalf of.
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unknown or retired token" })),
        )
            .into_response();
    };

    match method {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "overmind", "version": env!("CARGO_PKG_VERSION") }
            }),
        )
        .into_response(),

        "tools/list" => rpc_result(
            id,
            json!({
                "tools": TOOLS.iter().map(|(name, description)| json!({
                    "name": name,
                    "description": description,
                    // Deliberately open: the shapes belong to the provider
                    // (ADR-0003 promises tool names and free-form results, not
                    // schemas), and a schema invented here would go stale the
                    // first time a provider changed one.
                    "inputSchema": { "type": "object", "additionalProperties": true }
                })).collect::<Vec<_>>()
            }),
        )
        .into_response(),

        "tools/call" => {
            let name = body
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !TOOLS.iter().any(|(t, _)| *t == name) {
                // Named rather than generic: an agent that asked for
                // `store_memory` should learn it is not on offer here, not that
                // something went wrong.
                return tool_text(
                    id,
                    format!(
                        "`{name}` is not available to agents. Overmind exposes {} — \
                         memories are written by the orchestrator when a task completes, \
                         with the task recorded as their provenance.",
                        TOOLS.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ")
                    ),
                    true,
                )
                .into_response();
            }

            let args = body
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            // The company is the token's, never the request's.
            let memory = state.memory_for(&company_id).await;
            match memory.call_tool(name, args).await {
                Ok(text) => tool_text(id, text, false).into_response(),
                // Reported, not swallowed. The orchestrator's own memory calls
                // are best-effort because nobody is waiting on them; here an
                // agent asked a question, and silence would be read as "there
                // is nothing".
                Err(e) => tool_text(id, e, true).into_response(),
            }
        }

        "" => rpc_error(id, -32600, "invalid request").into_response(),
        other => rpc_error(id, -32601, &format!("method not found: {other}")).into_response(),
    }
}
