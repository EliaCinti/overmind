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

    /// A memory bound to a specific brain directory (used by the managed-brain
    /// path in M8). Sets `BRAIN_DIR` and gets its **own** connection pool (a
    /// different brain needs different server processes).
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

    /// Record a memory about completed work. Best-effort.
    pub async fn store_memory(
        &self,
        title: &str,
        content: &str,
        project: &str,
        tags: &[&str],
        category: &str,
    ) {
        if !self.is_enabled() {
            return;
        }
        let args = json!({
            "title": title,
            "content": content,
            "project": project,
            "tags": tags,
            "category": category,
        });
        if let Err(e) = self.call("store_memory", args).await {
            eprintln!("memory store_memory failed (ignored): {e}");
        }
    }

    /// Record a decision with its rationale. Best-effort.
    pub async fn store_decision(&self, decision: &str, rationale: &str, project: &str) {
        if !self.is_enabled() {
            return;
        }
        let args = json!({ "decision": decision, "rationale": rationale, "project": project });
        if let Err(e) = self.call("store_decision", args).await {
            eprintln!("memory store_decision failed (ignored): {e}");
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
    use super::extract_text;
    use serde_json::json;

    #[test]
    fn extracts_joined_text_content() {
        let r = json!({ "content": [ {"type":"text","text":"a"}, {"type":"text","text":"b"} ] });
        assert_eq!(extract_text(&r), "a\nb");
        assert_eq!(extract_text(&json!({})), "");
    }
}
