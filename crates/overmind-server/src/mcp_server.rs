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
//!
//! ## The second caller (M9, ADR-0028)
//!
//! The same endpoint answers a Claude Code session *outside* Overmind — the
//! owner, in their editor. It looks like the first caller and is not: it is
//! trusted where an agent is not, and durable where a run token dies with its
//! run. So the credential decides the grant, and `tools/list` answers with what
//! *that* caller may call. An outside session never sees the memory tools; an
//! agent never sees `create_task`, because agents already open tasks through the
//! CEO's plan layer, where a human sees the shape of the work first. A direct
//! `create_task` would not break that gate — it would never meet it.

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

/// What an outside caller may call (ADR-0028): file work, read the board, read
/// the log. Never start a task — that spends money and runs an agent, and since
/// M6 that decision has been a human's.
const WORK_TOOLS: &[(&str, &str)] = &[
    (
        "create_task",
        "File a task in this company's backlog. It is filed, not started: a \
         human decides who picks it up and when, which is what keeps the budget \
         and approval gates meaningful.",
    ),
    (
        "list_tasks",
        "The board — every task in this company, with its status and priority. \
         Optional `status` narrows it (backlog, todo, in_progress, in_review, \
         done, blocked).",
    ),
    (
        "get_task",
        "One task in full: its description, where it is, who has it, and what \
         the last run left behind.",
    ),
    (
        "verify_audit",
        "Check the append-only audit chain. Answers how many events were \
         verified and, if it is broken, where.",
    ),
    (
        "list_events",
        "The most recent audit events — what this company has actually done, in \
         order. Optional `limit` (default 20, max 200).",
    ),
];

pub fn router() -> Router<AppState> {
    Router::new().route("/mcp", post(handle))
}

/// Who is calling, and therefore what they may call (ADR-0028).
///
/// The grant is a property of the credential, not of the request. There is
/// deliberately no way to ask for the other one.
enum Caller {
    /// An agent Overmind is running. Reads memory; writes nothing.
    Run { company_id: String },
    /// A session outside Overmind, holding a credential the owner issued.
    /// Files work and reads the board; starts nothing.
    Integration { company_id: String },
}

impl Caller {
    fn company_id(&self) -> &str {
        match self {
            Caller::Run { company_id } | Caller::Integration { company_id } => company_id,
        }
    }

    fn tools(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            Caller::Run { .. } => TOOLS,
            Caller::Integration { .. } => WORK_TOOLS,
        }
    }
}

/// Resolve a bearer token to the caller it identifies.
///
/// Returns `None` for an absent, malformed, unknown, retired or revoked token —
/// all of them the same answer, deliberately: distinguishing them would tell
/// someone probing the endpoint which of their guesses was closer.
///
/// Run tokens are checked first because they are the hot path: every caged run
/// makes several calls, and there is one integration token per person.
async fn caller_for(state: &AppState, headers: &HeaderMap) -> Option<Caller> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim();
    if token.is_empty() {
        return None;
    }
    let run = sqlx::query_scalar::<_, String>(
        "SELECT t.company_id
           FROM agent_task_sessions s
           JOIN tasks t ON t.id = s.task_id
          WHERE s.mcp_token = ?",
    )
    .bind(token)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    if let Some(company_id) = run {
        return Some(Caller::Run { company_id });
    }

    let integration = sqlx::query_as::<_, (String, String)>(
        "SELECT id, company_id FROM company_tokens WHERE token = ? AND revoked_at IS NULL",
    )
    .bind(token)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let (token_id, company_id) = integration?;
    // Best-effort: "is anything still using this?" is the question that makes
    // revoking a credential a decision instead of a guess, and a lost update
    // here costs nothing.
    let _ = sqlx::query("UPDATE company_tokens SET last_used_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&token_id)
        .execute(&state.pool)
        .await;
    Some(Caller::Integration { company_id })
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

    let Some(caller) = caller_for(&state, &headers).await else {
        // 401 rather than a JSON-RPC error: the caller has not been identified,
        // so there is nobody to answer on behalf of.
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unknown or retired token" })),
        )
            .into_response();
    };
    let company_id = caller.company_id().to_string();
    let tools = caller.tools();

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
                "tools": tools.iter().map(|(name, description)| json!({
                    "name": name,
                    "description": description,
                    // Deliberately open: for the memory tools the shapes belong
                    // to the provider (ADR-0003 promises tool names and
                    // free-form results, not schemas), and a schema invented
                    // here would go stale the first time a provider changed
                    // one. The work tools describe their arguments in prose for
                    // the same reason they are lenient about them — a model
                    // that omits `execution_kind` should get a task, not an
                    // error.
                    "inputSchema": { "type": "object", "additionalProperties": true }
                })).collect::<Vec<_>>()
            }),
        )
        .into_response(),

        "tools/call" => {
            let name = body
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !tools.iter().any(|(t, _)| *t == name) {
                return tool_text(id, refusal(&caller, &name), true).into_response();
            }

            let args = body
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            match caller {
                // The company is the token's, never the request's.
                Caller::Run { .. } => {
                    let memory = state.memory_for(&company_id).await;
                    match memory.call_tool(&name, args).await {
                        Ok(text) => tool_text(id, text, false).into_response(),
                        // Reported, not swallowed. The orchestrator's own memory
                        // calls are best-effort because nobody is waiting on
                        // them; here an agent asked a question, and silence
                        // would be read as "there is nothing".
                        Err(e) => tool_text(id, e, true).into_response(),
                    }
                }
                Caller::Integration { .. } => {
                    match work_tool(&state, &company_id, &name, &args).await {
                        Ok(text) => tool_text(id, text, false).into_response(),
                        Err(e) => tool_text(id, e, true).into_response(),
                    }
                }
            }
        }

        "" => rpc_error(id, -32600, "invalid request").into_response(),
        other => rpc_error(id, -32601, &format!("method not found: {other}")).into_response(),
    }
}

/// Why a tool is not on offer *to this caller*.
///
/// Named rather than generic, and different per caller: an agent that asked for
/// `store_memory` should learn that memories are written by the orchestrator,
/// and an integration that asked to start a task should learn that starting is
/// a human's decision. "Unknown tool" would teach neither, and a model that is
/// told *why* stops asking.
fn refusal(caller: &Caller, name: &str) -> String {
    let offered = caller
        .tools()
        .iter()
        .map(|(t, _)| *t)
        .collect::<Vec<_>>()
        .join(", ");
    match caller {
        Caller::Run { .. } => format!(
            "`{name}` is not available to agents. Overmind exposes {offered} — memories are \
             written by the orchestrator when a task completes, with the task recorded as \
             their provenance."
        ),
        Caller::Integration { .. } => format!(
            "`{name}` is not available to integrations. Overmind exposes {offered} — work is \
             filed here and started by a person, so the budget and approval gates stay the \
             only way an agent begins spending."
        ),
    }
}

/// (title, description, status, priority, execution_kind, assignee, last_error)
type TaskDetail = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

/// One of [`WORK_TOOLS`], for a caller outside Overmind (ADR-0028).
///
/// The `Err` side is a sentence for the model, not a type: MCP carries tool
/// failures inside a successful result, and the reader is something that will
/// try again if told what was wrong with the first attempt.
async fn work_tool(
    state: &AppState,
    company_id: &str,
    name: &str,
    args: &Value,
) -> Result<String, String> {
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    match name {
        "create_task" => {
            let title = text(&args["title"]);
            if title.is_empty() {
                return Err("a task needs a `title`.".into());
            }
            // Through the HTTP handler's own function, so what counts as a valid
            // task is decided in one place (ADR-0028).
            let req = crate::api::CreateTask {
                title,
                description: text(&args["description"]),
                goal_id: args["goal_id"].as_str().map(str::to_string),
                priority: args["priority"].as_str().map(str::to_string),
                execution_kind: args["execution_kind"].as_str().map(str::to_string),
            };
            let task = crate::api::open_task(state, company_id, &req)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "Filed in the backlog: {} ({}).\nid: {}\nNobody is on it yet — a person picks \
                 it up from the board.",
                task["title"].as_str().unwrap_or(""),
                task["execution_kind"].as_str().unwrap_or(""),
                task["id"].as_str().unwrap_or(""),
            ))
        }

        "list_tasks" => {
            let wanted = text(&args["status"]);
            let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
                "SELECT t.id, t.title, t.status, t.priority, a.name
                   FROM tasks t
                   LEFT JOIN agents a ON a.id = t.assignee_agent_id
                  WHERE t.company_id = ? AND (? = '' OR t.status = ?)
                  ORDER BY t.updated_at DESC",
            )
            .bind(company_id)
            .bind(&wanted)
            .bind(&wanted)
            .fetch_all(&state.pool)
            .await
            .map_err(|e| e.to_string())?;
            if rows.is_empty() {
                return Ok(if wanted.is_empty() {
                    "The board is empty.".into()
                } else {
                    format!("Nothing is in `{wanted}`.")
                });
            }
            Ok(rows
                .into_iter()
                .map(|(id, title, status, priority, assignee)| {
                    let who = assignee.unwrap_or_else(|| "unassigned".into());
                    format!("{status:<12} {priority:<7} {title}  [{who}]  {id}")
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }

        "get_task" => {
            let task_id = text(&args["task_id"]);
            if task_id.is_empty() {
                return Err("which task? pass `task_id`.".into());
            }
            // Scoped by company as well as id: the token's company is the only
            // one it can see, and a guessed id from another must miss.
            let row: Option<TaskDetail> = sqlx::query_as(
                "SELECT t.title, t.description, t.status, t.priority, t.execution_kind,
                        a.name,
                        (SELECT s.last_error FROM agent_task_sessions s
                          WHERE s.task_id = t.id
                          ORDER BY s.created_at DESC LIMIT 1)
                   FROM tasks t
                   LEFT JOIN agents a ON a.id = t.assignee_agent_id
                  WHERE t.id = ? AND t.company_id = ?",
            )
            .bind(&task_id)
            .bind(company_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| e.to_string())?;
            let Some((title, description, status, priority, kind, assignee, last_error)) = row
            else {
                return Err(format!("no task `{task_id}` in this company."));
            };
            let mut out = format!(
                "{title}\nstatus: {status}   priority: {priority}   kind: {kind}\nassignee: {}\n\n{description}",
                assignee.unwrap_or_else(|| "unassigned".into()),
            );
            if let Some(e) = last_error.filter(|e| !e.trim().is_empty()) {
                out.push_str(&format!("\n\nlast error: {e}"));
            }
            Ok(out)
        }

        "verify_audit" => {
            let report = crate::audit::verify(&state.pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(match report.first_invalid_seq {
                None => format!(
                    "The chain verifies: {} events, none altered.",
                    report.events_checked
                ),
                Some(seq) => format!(
                    "BROKEN at sequence {seq}, after {} verified events.",
                    report.events_checked
                ),
            })
        }

        "list_events" => {
            let limit = args["limit"].as_i64().unwrap_or(20).clamp(1, 200);
            let rows: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
                "SELECT seq, kind, created_at, payload
                   FROM audit_events WHERE company_id = ? ORDER BY seq DESC LIMIT ?",
            )
            .bind(company_id)
            .bind(limit)
            .fetch_all(&state.pool)
            .await
            .map_err(|e| e.to_string())?;
            if rows.is_empty() {
                return Ok("Nothing has happened in this company yet.".into());
            }
            Ok(rows
                .into_iter()
                .map(|(seq, kind, at, payload)| {
                    let what = payload
                        .and_then(|p| serde_json::from_str::<Value>(&p).ok())
                        .and_then(|p| {
                            p.get("title")
                                .or_else(|| p.get("name"))
                                .or_else(|| p.get("label"))
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default();
                    format!("{seq:>5}  {at}  {kind}  {what}")
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }

        // Unreachable while the dispatch checks membership first, and honest if
        // a tool is ever added to the list and not to this match.
        other => Err(format!("`{other}` is listed but not implemented.")),
    }
}
