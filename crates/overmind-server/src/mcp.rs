//! Organizational memory over MCP (M7, ADR-0003/0007-memory).
//!
//! A `Memory` speaks the Model Context Protocol (JSON-RPC 2.0 over a spawned
//! process's stdio) to a memory server — Wadachi is the reference, but any
//! MCP server exposing `get_context` / `store_memory` / `store_decision`
//! works. Everything here is **best-effort**: with no server configured every
//! call is a no-op, and any failure (spawn, timeout, protocol) is logged and
//! swallowed — memory never breaks a task (the graceful-degradation rule).

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, Semaphore};

/// Concurrent memory calls allowed at once — each uses its own connection
/// (its own server process). Safe now that Wadachi ≥ 0.14 handles concurrent
/// access (WAL + busy_timeout). `OVERMIND_MEMORY_POOL` overrides.
const DEFAULT_POOL_SIZE: usize = 4;

/// A live MCP connection: the spawned server plus its stdio, reused across
/// calls so we handshake once per server lifetime, not once per call.
struct Conn {
    _child: Child, // kept alive; kill_on_drop tears it down when the Conn drops
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    next_id: i64,
}

/// A small pool of persistent connections. `permits` caps how many calls run
/// at once; `idle` holds warm connections for reuse. A connection is returned
/// to `idle` only after a successful call; on any error it's dropped (killed)
/// and the next call opens a fresh one.
struct Pool {
    idle: Mutex<Vec<Conn>>,
    permits: Semaphore,
}

#[derive(Clone)]
pub struct Memory {
    /// Shell command that launches the MCP memory server, or `None` (disabled).
    cmd: Option<String>,
    /// Extra env for the spawned server (e.g. `BRAIN_DIR` for a managed
    /// per-company brain in M8). Empty for a plain externally-configured server.
    env: Arc<Vec<(String, String)>>,
    timeout: Duration,
    pool: Arc<Pool>,
}

fn pool_size() -> usize {
    std::env::var("OVERMIND_MEMORY_POOL")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or(DEFAULT_POOL_SIZE)
}

impl Memory {
    pub fn from_config(cmd: Option<String>) -> Self {
        let size = pool_size();
        Memory {
            cmd,
            env: Arc::new(Vec::new()),
            timeout: Duration::from_secs(30),
            pool: Arc::new(Pool {
                idle: Mutex::new(Vec::new()),
                permits: Semaphore::new(size),
            }),
        }
    }

    /// A memory that does nothing, for a company whose brain is switched off.
    /// Identical to having no provider configured — the graceful-degradation
    /// path M7 already tests, reused rather than re-invented.
    pub fn disabled() -> Self {
        Memory::from_config(None)
    }

    /// A memory bound to a specific brain directory — the managed per-company
    /// brain of ADR-0024. Sets `BRAIN_DIR` and gets its **own** connection pool,
    /// because a pooled connection is a live server process already pointed at
    /// a brain: reusing one across brains would answer from the wrong company.
    ///
    /// Callers should go through [`crate::db::AppState::memory_for`], which
    /// caches the result — each call here builds a fresh pool.
    pub fn with_brain_dir(&self, brain_dir: &str) -> Self {
        let mut env = (*self.env).clone();
        env.push(("BRAIN_DIR".to_string(), brain_dir.to_string()));
        Memory {
            cmd: self.cmd.clone(),
            env: Arc::new(env),
            timeout: self.timeout,
            pool: Arc::new(Pool {
                idle: Mutex::new(Vec::new()),
                permits: Semaphore::new(pool_size()),
            }),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.cmd.is_some()
    }

    /// Relevant memories for an agent about to start work, or `None` if memory
    /// is off or unavailable. The text is injected into the agent's prompt.
    ///
    /// `get_context` takes only `cwd` + `task_description` (Wadachi derives the
    /// project from the cwd); per-company isolation comes from the brain
    /// directory, not an argument (see [`Memory::with_brain_dir`], M8).
    pub async fn get_context(&self, cwd: &str, task: &str) -> Option<String> {
        let result = self
            .call(
                "get_context",
                json!({ "cwd": cwd, "task_description": task }),
            )
            .await;
        match result {
            Ok(v) => {
                let text = extract_text(&v);
                if text.trim().is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
            Err(e) => {
                eprintln!("memory get_context failed (ignored): {e}");
                None
            }
        }
    }

    /// Call a tool by name and hand back its text answer verbatim.
    ///
    /// The proxy path for ADR-0027: an agent asks Overmind, Overmind asks the
    /// provider, and the answer travels back untouched. Deliberately does not
    /// parse — the agent is the one reading it, and inventing a shape here
    /// would be Overmind deciding what a provider is allowed to say.
    ///
    /// `Err` carries something the agent can read. Unlike the orchestrator's
    /// own best-effort calls, a failure here must be *reported*: an agent that
    /// asked a question and got silence would take the silence for an answer.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<String, String> {
        if !self.is_enabled() {
            return Err("no memory provider is configured".to_string());
        }
        match self.call(name, args).await {
            Ok(v) => Ok(extract_text(&v)),
            Err(e) => Err(format!("memory provider failed: {e}")),
        }
    }

    /// Where the brain is right now, verbatim, or `None` when there is nothing
    /// to ask (memory off, tool absent, call failed).
    ///
    /// Taken at checkout and handed back at completion (ADR-0026). The value is
    /// opaque here on purpose: Overmind moves it between two tools of the same
    /// provider and never reads inside it, because ADR-0003 promises tool names
    /// and free-form results — not a shape.
    pub async fn watermark(&self, project: &str) -> Option<String> {
        if !self.is_enabled() {
            return None;
        }
        match self
            .call("brain_watermark", json!({ "project": project }))
            .await
        {
            Ok(v) => {
                let text = extract_text(&v);
                // A provider that does not implement the tool may still answer
                // something; only take it if it parses as an object.
                serde_json::from_str::<Value>(text.trim())
                    .ok()
                    .filter(|v| v.is_object())
                    .map(|v| v.to_string())
            }
            Err(_) => None, // Not an error: the tool is optional (ADR-0003).
        }
    }

    /// Record a memory about completed work. Best-effort.
    ///
    /// Returns the provider's own identifier for what it stored, when it gives
    /// one — that is what lets Overmind record which task produced it
    /// (ADR-0025). `None` means the call failed, memory is off, or the provider
    /// answered without an id; all three are survivable and none is an error.
    pub async fn store_memory(
        &self,
        title: &str,
        content: &str,
        project: &str,
        tags: &[&str],
        category: &str,
        since_watermark: Option<&str>,
    ) -> Stored {
        if !self.is_enabled() {
            return Stored::default();
        }
        let mut args = json!({
            "title": title,
            "content": content,
            "project": project,
            "tags": tags,
            "category": category,
        });
        // Only sent when we have one. A provider that does not know the
        // argument would otherwise receive a null it never asked for.
        if let Some(Ok(v)) = since_watermark.map(serde_json::from_str::<Value>) {
            args["since_watermark"] = v;
        }
        match self.call("store_memory", args).await {
            Ok(v) => Stored {
                memory_ref: stored_ref(&v),
                collisions: collisions(&v),
            },
            Err(e) => {
                eprintln!("memory store_memory failed (ignored): {e}");
                Stored::default()
            }
        }
    }

    /// Record a decision with its rationale. Best-effort; returns the
    /// provider's identifier like [`Memory::store_memory`].
    pub async fn store_decision(
        &self,
        decision: &str,
        rationale: &str,
        project: &str,
        tags: &[&str],
    ) -> Option<String> {
        if !self.is_enabled() {
            return None;
        }
        let args = json!({
            "decision": decision,
            "rationale": rationale,
            "project": project,
            "tags": tags,
        });
        match self.call("store_decision", args).await {
            Ok(v) => stored_ref(&v),
            Err(e) => {
                eprintln!("memory store_decision failed (ignored): {e}");
                None
            }
        }
    }

    /// Enumerate what this brain holds, newest-first if the provider says so.
    /// `None` when memory is off or the answer is not a list we recognize —
    /// which the UI reports as "this provider cannot be browsed" rather than as
    /// an empty brain (ADR-0025).
    pub async fn list_memories(&self, project: Option<&str>) -> Option<Vec<Value>> {
        let args = match project {
            Some(p) => json!({ "project": p }),
            None => json!({}),
        };
        self.listing("list_memories", args, "memories").await
    }

    /// Recent decisions. Separate from memories because the provider keeps them
    /// apart and flattening the two would lose which is which.
    pub async fn list_decisions(&self, project: Option<&str>, limit: u32) -> Option<Vec<Value>> {
        let mut args = json!({ "limit": limit });
        if let Some(p) = project {
            args["project"] = json!(p);
        }
        self.listing("list_decisions", args, "decisions").await
    }

    /// Semantic search. Deliberately *not* modelled as a filtered `list`: one
    /// ranks by meaning and the other enumerates, and calling them the same
    /// thing would misrepresent both (ADR-0025).
    pub async fn recall(
        &self,
        query: &str,
        project: Option<&str>,
        limit: u32,
    ) -> Option<Vec<Value>> {
        let mut args = json!({ "query": query, "limit": limit });
        if let Some(p) = project {
            args["project"] = json!(p);
        }
        self.listing("recall", args, "results").await
    }

    /// Shared shape of the three read tools: call, take the text out of the MCP
    /// envelope, and look for an array in it. Defensive on purpose — the
    /// contract promises tool names, not response shapes.
    async fn listing(&self, tool: &str, args: Value, key: &str) -> Option<Vec<Value>> {
        if !self.is_enabled() {
            return None;
        }
        match self.call(tool, args).await {
            Ok(v) => {
                let items = extract_array(&extract_text(&v), key);
                if items.is_none() {
                    eprintln!("memory {tool} answered nothing browsable (ignored)");
                }
                items
            }
            Err(e) => {
                eprintln!("memory {tool} failed (ignored): {e}");
                None
            }
        }
    }

    /// Call a tool on the memory server, using one connection from the pool so
    /// up to `pool` calls run concurrently. Bounded by `self.timeout`. On
    /// success the connection returns to the pool; on any error (or timeout)
    /// it's dropped and the next call opens a fresh one.
    async fn call(&self, tool: &str, args: Value) -> Result<Value, McpError> {
        let Some(cmd) = &self.cmd else {
            return Err(McpError::Disabled);
        };
        let _permit = self
            .pool
            .permits
            .acquire()
            .await
            .map_err(|_| McpError::Closed)?;
        let taken = self.pool.idle.lock().await.pop();

        let attempt =
            tokio::time::timeout(self.timeout, self.run_call(cmd, taken, tool, args)).await;
        match attempt {
            Ok(Ok((conn, value))) => {
                self.pool.idle.lock().await.push(conn); // healthy → reuse
                Ok(value)
            }
            Ok(Err(e)) => Err(e), // connection already dropped inside run_call
            Err(_) => Err(McpError::Timeout), // taken connection dropped with the future
        }
    }

    /// Run one tool call on a pooled or fresh connection. On success returns
    /// the still-healthy connection to hand back to the pool; on error the
    /// connection is dropped here (kill_on_drop).
    async fn run_call(
        &self,
        cmd: &str,
        taken: Option<Conn>,
        tool: &str,
        args: Value,
    ) -> Result<(Conn, Value), McpError> {
        let mut conn = match taken {
            Some(c) => c,
            None => self.connect(cmd).await?,
        };
        let id = conn.next_id;
        conn.next_id += 1;
        write_msg(
            &mut conn.stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            }),
        )
        .await?;
        let value = read_result(&mut conn.reader, id).await?;
        Ok((conn, value))
    }

    /// Spawn the server and complete the MCP handshake once.
    async fn connect(&self, cmd: &str) -> Result<Conn, McpError> {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(cmd)
            .envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| McpError::Spawn(e.to_string()))?;

        let mut stdin = child.stdin.take().ok_or(McpError::Pipe)?;
        let stdout = child.stdout.take().ok_or(McpError::Pipe)?;
        let mut reader = BufReader::new(stdout).lines();

        write_msg(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "overmind", "version": env!("CARGO_PKG_VERSION") }
                }
            }),
        )
        .await?;
        read_result(&mut reader, 1).await?;
        write_msg(
            &mut stdin,
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await?;

        Ok(Conn {
            _child: child,
            stdin,
            reader,
            next_id: 2, // 1 was the initialize
        })
    }
}

async fn write_msg(stdin: &mut tokio::process::ChildStdin, msg: &Value) -> Result<(), McpError> {
    let mut line = msg.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| McpError::Io(e.to_string()))?;
    stdin
        .flush()
        .await
        .map_err(|e| McpError::Io(e.to_string()))?;
    Ok(())
}

/// Read newline-delimited JSON-RPC messages until the response with `id`
/// arrives; skip logs/notifications. Returns its `result` or an error.
async fn read_result<R>(reader: &mut tokio::io::Lines<R>, id: i64) -> Result<Value, McpError>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|e| McpError::Io(e.to_string()))?
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            continue; // non-JSON stdout noise
        };
        if msg.get("id").and_then(Value::as_i64) != Some(id) {
            continue; // a notification or a different response
        }
        if let Some(err) = msg.get("error") {
            return Err(McpError::Rpc(err.to_string()));
        }
        return msg
            .get("result")
            .cloned()
            .ok_or_else(|| McpError::Rpc("response had no result".into()));
    }
    Err(McpError::Closed)
}

/// The provider's identifier for something it just stored, if it gave one.
///
/// Tool results are text by protocol, so a server that wants to say "I stored
/// this as #7" has to say it inside the text. Wadachi answers with a JSON
/// object carrying `id`; we accept a number or a string there and nothing else,
/// because guessing harder than that would mean inventing links that are not
/// real (ADR-0025).
/// What a write to the brain gave back: the provider's identifier for the thing
/// it stored, and anything close that appeared while this run was working.
///
/// Both are optional and both being empty is the ordinary case. `collisions` is
/// advisory — the write already succeeded, and a provider that does not
/// implement the check simply never mentions it.
#[derive(Debug, Default, Clone)]
pub struct Stored {
    pub memory_ref: Option<String>,
    pub collisions: Vec<Collision>,
}

/// One item the provider judged close to what we just wrote. `similarity` is
/// the provider's own number on its own scale: Overmind reports it, ranks by
/// it, and does not interpret it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Collision {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub similarity: f64,
}

fn collisions(result: &Value) -> Vec<Collision> {
    let text = extract_text(result);
    let Ok(parsed) = serde_json::from_str::<Value>(text.trim()) else {
        return Vec::new();
    };
    let Some(items) = parsed.get("collisions").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|c| {
            let id = match c.get("id")? {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                _ => return None,
            };
            Some(Collision {
                kind: c
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("memory")
                    .to_string(),
                id,
                title: c
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                similarity: c.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0),
            })
        })
        .collect()
}

fn stored_ref(result: &Value) -> Option<String> {
    let text = extract_text(result);
    let parsed: Value = serde_json::from_str(text.trim()).ok()?;
    match parsed.get("id")? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Pull a list out of a tool's text answer. Accepts either an object with the
/// expected key (`{"memories": [...]}`, Wadachi's shape) or a bare array, and
/// gives up otherwise rather than inventing a reading of it.
fn extract_array(text: &str, key: &str) -> Option<Vec<Value>> {
    let parsed: Value = serde_json::from_str(text.trim()).ok()?;
    match parsed {
        Value::Array(items) => Some(items),
        Value::Object(ref map) => map.get(key)?.as_array().cloned(),
        _ => None,
    }
}

/// Flatten an MCP tool result's `content` array into a single string.
fn extract_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[derive(Debug, thiserror::Error)]
enum McpError {
    #[error("memory disabled")]
    Disabled,
    #[error("timed out")]
    Timeout,
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("no stdio pipe")]
    Pipe,
    #[error("io: {0}")]
    Io(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("server closed before responding")]
    Closed,
}

#[cfg(test)]
mod tests {
    use super::{extract_array, extract_text, stored_ref};
    use serde_json::json;

    fn text_result(text: &str) -> serde_json::Value {
        json!({ "content": [ { "type": "text", "text": text } ] })
    }

    #[test]
    fn extracts_joined_text_content() {
        let r = json!({ "content": [ {"type":"text","text":"a"}, {"type":"text","text":"b"} ] });
        assert_eq!(extract_text(&r), "a\nb");
        assert_eq!(extract_text(&json!({})), "");
    }

    /// The real shapes: Wadachi's `store_memory` and `store_decision` both
    /// answer with a JSON object carrying `id`.
    #[test]
    fn reads_the_providers_id_out_of_a_stored_result() {
        let r = text_result(r#"{"id": 7, "title": "T", "filepath": "projects/x/t.md"}"#);
        assert_eq!(stored_ref(&r).as_deref(), Some("7"));
        assert_eq!(
            stored_ref(&text_result(r#"{"id": "abc"}"#)).as_deref(),
            Some("abc")
        );
    }

    /// No id means no link, never a guessed one — a wrong provenance is worse
    /// than none (ADR-0025).
    #[test]
    fn invents_no_id_when_the_provider_gives_none() {
        assert_eq!(stored_ref(&text_result("stored")), None);
        assert_eq!(stored_ref(&text_result(r#"{"ok": true}"#)), None);
        assert_eq!(stored_ref(&text_result(r#"{"id": null}"#)), None);
        assert_eq!(stored_ref(&text_result(r#"{"id": ""}"#)), None);
        assert_eq!(stored_ref(&json!({})), None);
    }

    #[test]
    fn reads_a_listing_from_either_shape() {
        let keyed = r#"{"memories": [{"id": 1}], "count": 1}"#;
        assert_eq!(extract_array(keyed, "memories").map(|v| v.len()), Some(1));
        let bare = r#"[{"id": 1}, {"id": 2}]"#;
        assert_eq!(extract_array(bare, "memories").map(|v| v.len()), Some(2));
    }

    /// An answer we cannot read is "not browsable", which the UI distinguishes
    /// from "empty" — so it must not come back as an empty list.
    #[test]
    fn an_unreadable_listing_is_none_not_empty() {
        assert_eq!(extract_array("I remember three things.", "memories"), None);
        assert_eq!(extract_array(r#"{"count": 0}"#, "memories"), None);
        assert_eq!(
            extract_array(r#"{"memories": []}"#, "memories").map(|v| v.len()),
            Some(0)
        );
    }
}
