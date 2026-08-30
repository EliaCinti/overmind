use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tokio::sync::broadcast;

use crate::domain::{AgentTraits, Autonomy, DomainPatch, ReviewStrictness};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    /// Session ids with a live runner in *this* process. Sessions marked
    /// running in the DB but absent here are orphans (e.g. after a server
    /// restart) and get picked up by the heartbeat scheduler.
    pub running: Arc<Mutex<HashSet<String>>>,
    /// Live change notifications for connected UI clients. Each message is a
    /// JSON string; the board refetches the affected scope (see ADR-0010).
    pub events: broadcast::Sender<String>,
    /// Asked to stop so a supervisor can bring the server back on restored
    /// data (ADR-0044). The swap is a boot's work, never a live pool's, and
    /// this is how the request that staged one says so. Nobody listens in
    /// tests; `main` does, and returns from `serve`.
    pub restart: broadcast::Sender<()>,
    /// Organizational memory over MCP (Wadachi reference). A no-op when no
    /// memory server is configured — Overmind is fully functional without it.
    ///
    /// This is the *unbound* handle: it speaks to whatever brain the memory
    /// command defaults to. Work done on a company's behalf must go through
    /// [`AppState::memory_for`] instead, which binds it to that company's own
    /// brain (ADR-0024).
    pub memory: crate::mcp::Memory,
    /// One `Memory` per company, each bound to that company's brain directory.
    /// Cached because [`crate::mcp::Memory::with_brain_dir`] builds a fresh
    /// connection pool every time it is called — binding per task would respawn
    /// memory servers forever instead of reusing warm ones.
    brains: Arc<Mutex<HashMap<String, crate::mcp::Memory>>>,
    /// Which economy this Overmind pays in (ADR-0030).
    ///
    /// Behind a lock rather than fixed at construction, and detected by the
    /// *binary* rather than by `init_with`: asking costs a subprocess, and a
    /// library that shells out when you build it makes every test pay for a
    /// fact none of them are about. It also leaves room for re-asking when
    /// credentials change under a running server.
    economy: Arc<std::sync::RwLock<crate::economy::Economy>>,
    /// Where a subscription stands in its current window (ADR-0030).
    ///
    /// Learned from the adapter's own `rate_limit_event` on each run rather
    /// than asked for, so it costs nothing and is as fresh as the last piece of
    /// work. `None` under an API key, where plan windows do not apply at all,
    /// and before the first run has reported one.
    plan_windows: Arc<std::sync::RwLock<crate::economy::PlanWindows>>,
    /// Conversations with an agent turn in flight, and whether another turn
    /// is owed once it ends (ADR-0038 addendum). In memory on purpose: a turn
    /// does not survive a restart, so neither should the claim that one is
    /// running — the interface asks, it does not remember.
    answering: Arc<Mutex<HashMap<String, bool>>>,
    /// What each in-flight run is doing right now (ADR-0039), keyed by the
    /// conversation id (chat turns) or session id (task runs). In memory for
    /// the same reason `answering` is: narration of a process that does not
    /// survive a restart.
    activity: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    /// The newest task completion each conversation has been told about (or
    /// deliberately skipped) — ADR-0041.
    digested: Arc<Mutex<HashMap<String, String>>>,
}

impl AppState {
    /// Is an agent answering in this conversation (or about to, again)?
    pub fn is_answering(&self, conversation_id: &str) -> bool {
        self.answering
            .lock()
            .map(|m| m.contains_key(conversation_id))
            .unwrap_or(false)
    }

    /// Claim the turn for a conversation. `true` when this caller should run
    /// it; `false` when one is already in flight — the next turn is then owed,
    /// and the caller that finishes the current one runs it.
    pub fn begin_answering(&self, conversation_id: &str) -> bool {
        let Ok(mut m) = self.answering.lock() else {
            return true;
        };
        match m.get_mut(conversation_id) {
            Some(owed) => {
                *owed = true;
                false
            }
            None => {
                m.insert(conversation_id.to_string(), false);
                true
            }
        }
    }

    /// Release the turn. `true` when another turn is owed (a message came in
    /// meanwhile): the conversation stays claimed and the caller runs again.
    pub fn end_answering(&self, conversation_id: &str) -> bool {
        let Ok(mut m) = self.answering.lock() else {
            return false;
        };
        match m.get_mut(conversation_id) {
            Some(owed) if *owed => {
                *owed = false;
                true
            }
            _ => {
                m.remove(conversation_id);
                false
            }
        }
    }

    /// A transaction that intends to write, and says so at the door (M23).
    ///
    /// `BEGIN IMMEDIATE` takes SQLite's write lock up front, where
    /// `busy_timeout` can do its waiting. The default deferred `BEGIN` reads
    /// from a snapshot first and upgrades to a writer later -- and an upgrade
    /// over a snapshot another writer has since moved past is refused on the
    /// spot (`SQLITE_BUSY_SNAPSHOT`), timeout or not. The burn-in measured
    /// it: twelve simultaneous task checkouts, eight answered 500.
    pub async fn write_tx(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
        self.pool.begin_with("BEGIN IMMEDIATE").await
    }

    /// The brain directory a company's memories live in (ADR-0024). Derived
    /// from the id rather than stored, so a company and its brain cannot point
    /// at different places.
    pub fn brain_dir(&self, company_id: &str) -> PathBuf {
        self.config
            .data_dir
            .join("companies")
            .join(company_id)
            .join("brain")
    }

    /// How a prompt names the company: "«Rossi Vini», an AI company" — or
    /// just "an AI company" when the row cannot be read, because a persona
    /// line must never be silently blank (M21).
    ///
    /// Every agent prompt used to say "an AI company" and stop there. That
    /// read as harmless until M19's acceptance run, where an agent asked to
    /// write about the company wrote about somebody else's product of the
    /// same name: with an empty brain and no name in the prompt, world
    /// knowledge was all it had. Naming the company is the cheap half of the
    /// fix; the founding memory is the other.
    pub async fn company_descriptor(&self, company_id: &str) -> String {
        sqlx::query_as::<_, (String,)>("SELECT name FROM companies WHERE id = ?")
            .bind(company_id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .map(|(name,)| format!("«{name}», an AI company"))
            .unwrap_or_else(|| "an AI company".to_string())
    }

    /// The memory handle to use for work done on `company_id`'s behalf.
    ///
    /// Provisioning is the directory: a memory server pointed at an empty one
    /// builds whatever layout it needs on first connection, so "give this
    /// company a brain" is a `create_dir_all` and an env var — no init step,
    /// and nothing Wadachi-specific in the provider-generic path (ADR-0024).
    ///
    /// Returns a disabled (no-op) handle when the company has its brain
    /// switched off, which is the same path as having no provider configured.
    /// Falls back to the shared handle when managed brains are off, or when the
    /// directory cannot be created — losing isolation is bad, losing the
    /// agent's memory over a filesystem hiccup is worse, and memory is
    /// best-effort by contract.
    pub async fn memory_for(&self, company_id: &str) -> crate::mcp::Memory {
        if !self.config.managed_brain || !self.memory.is_enabled() {
            return self.memory.clone();
        }
        if !self.brain_is_enabled(company_id).await {
            return crate::mcp::Memory::disabled();
        }
        if let Ok(cache) = self.brains.lock()
            && let Some(m) = cache.get(company_id)
        {
            return m.clone();
        }
        let dir = self.brain_dir(company_id);
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            eprintln!("brain dir {} unusable (ignored): {e}", dir.display());
            return self.memory.clone();
        }
        let bound = self.memory.with_brain_dir(&dir.to_string_lossy());
        // Another task may have raced us here; whoever inserted first wins, so
        // the company keeps one pool rather than two. A poisoned lock costs a
        // fresh pool, not a failed call — the same trade the runner makes.
        match self.brains.lock() {
            Ok(mut cache) => cache.entry(company_id.to_string()).or_insert(bound).clone(),
            Err(_) => bound,
        }
    }

    /// Drop the cached memory handle for a company that is going away
    /// (ADR-0034). The cache entry owns a pool of live MCP server processes
    /// bound to the company's brain directory; dropping it kills them
    /// (`kill_on_drop`), which must happen *before* the directory is removed
    /// or the servers keep open handles into a deleted tree. A poisoned lock
    /// means nobody can be holding a handle to hand out anyway.
    pub fn forget_brain(&self, company_id: &str) {
        if let Ok(mut cache) = self.brains.lock() {
            cache.remove(company_id);
        }
    }

    /// Record which task or meeting produced a memory the provider just stored
    /// (ADR-0025). `memory_ref` is whatever identifier it gave back, so `None`
    /// — a provider that answers without one, or a call that failed — records
    /// nothing and is not an error: the memory exists either way, it just has
    /// no subject in the browser.
    ///
    /// The whole thing is best-effort, memory's standing bargain since
    /// ADR-0003. A link we fail to write must never fail the work that earned
    /// it.
    pub async fn link_memory(
        &self,
        company_id: &str,
        kind: &str,
        memory_ref: Option<&str>,
        subject_type: &str,
        subject_id: &str,
        subject_title: &str,
    ) {
        let Some(memory_ref) = memory_ref else { return };
        let result = sqlx::query(
            "INSERT OR IGNORE INTO memory_links
               (id, company_id, kind, memory_ref, subject_type, subject_id, subject_title, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(company_id)
        .bind(kind)
        .bind(memory_ref)
        .bind(subject_type)
        .bind(subject_id)
        .bind(subject_title)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await;
        if let Err(e) = result {
            eprintln!("memory link not recorded (ignored): {e}");
        }
    }

    /// Whether this company's brain is switched on. A missing company answers
    /// "on": the caller is already about to fail on the real query, and memory
    /// must never be the thing that decides an operation's fate.
    async fn brain_is_enabled(&self, company_id: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT brain_enabled FROM companies WHERE id = ?1")
            .bind(company_id)
            .fetch_optional(&self.pool)
            .await
            .unwrap_or(None)
            .map(|v| v != 0)
            .unwrap_or(true)
    }

    /// Tell connected clients that `company_id`'s board changed. Coarse by
    /// design: the client refetches rather than applying a delta, which keeps
    /// the contract trivial and impossible to desync. A send error just means
    /// no clients are listening — never fatal.
    pub fn notify(&self, company_id: &str) {
        let _ = self
            .events
            .send(json!({ "type": "changed", "company_id": company_id }).to_string());
    }

    /// Push a typed event carrying content (a notification to surface, not just
    /// "refetch"). Same channel, same "no listeners is fine" contract.
    pub fn push(&self, event: Value) {
        let _ = self.events.send(event.to_string());
    }

    /// Which economy this Overmind pays in (ADR-0030).
    ///
    /// A poisoned lock answers `Unknown` rather than panicking: not knowing how
    /// you pay is a survivable state that the interface already has words for,
    /// and taking the server down over it would be a worse answer than saying
    /// so.
    pub fn economy(&self) -> crate::economy::Economy {
        match self.economy.read() {
            Ok(e) => e.clone(),
            Err(_) => crate::economy::Economy::Unknown {
                kind: crate::economy::UnknownKind::Unreadable,
                reason: "the economy could not be read".into(),
            },
        }
    }

    /// Record what the probe found. Called by the binary at startup, after
    /// [`crate::economy::detect`].
    pub fn set_economy(&self, economy: crate::economy::Economy) {
        if let Ok(mut slot) = self.economy.write() {
            *slot = economy;
        }
    }

    /// Is any turn in flight anywhere? (tests and the digest scheduler use
    /// this to avoid stacking a turn on a turn.)
    pub fn is_answering_anywhere(&self) -> bool {
        self.answering
            .lock()
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    }

    /// Advance the digest watermark for a conversation (ADR-0041). `true`
    /// when `up_to` is newer than what was already announced or skipped —
    /// i.e. the caller should run a digest turn. In memory: after a restart
    /// the worst case is one redundant ask, and SKIP handles it.
    pub fn digest_advance(&self, conversation_id: &str, up_to: &str) -> bool {
        let Ok(mut m) = self.digested.lock() else {
            return false;
        };
        match m.get(conversation_id) {
            Some(seen) if seen.as_str() >= up_to => false,
            _ => {
                m.insert(conversation_id.to_string(), up_to.to_string());
                true
            }
        }
    }

    /// What this run is doing right now, if it said (ADR-0039).
    pub fn activity(&self, key: &str) -> Option<serde_json::Value> {
        self.activity.lock().ok()?.get(key).cloned()
    }

    /// Record the run's current activity — one narration line, latest wins.
    pub fn set_activity(&self, key: &str, activity: serde_json::Value) {
        if let Ok(mut m) = self.activity.lock() {
            m.insert(key.to_string(), activity);
        }
    }

    /// The run ended, however it ended: the narration goes with it.
    pub fn clear_activity(&self, key: &str) {
        if let Ok(mut m) = self.activity.lock() {
            m.remove(key);
        }
    }

    /// What each of the plan's windows was last reported to be doing.
    pub fn plan_windows(&self) -> crate::economy::PlanWindows {
        self.plan_windows
            .read()
            .map(|w| w.clone())
            .unwrap_or_default()
    }

    /// Remember what an adapter run reported about the plan.
    ///
    /// Best-effort throughout: a lock we cannot take costs a stale reading, and
    /// a stale reading of a *courtesy* number must never be allowed to affect
    /// the work that produced it.
    pub fn set_plan_window(&self, window: crate::economy::PlanWindow) {
        if let Ok(mut slot) = self.plan_windows.write() {
            // Keyed by which window it is, so a report about the five-hour
            // limit never overwrites what we know about the seven-day one.
            slot.insert(window.window.clone(), window);
        }
    }
}

/// Server configuration (env-driven; tests inject their own via `init_with`).
#[derive(Clone, Debug)]
pub struct Config {
    /// Where this server can be reached by the agents it runs (ADR-0027).
    /// Derived from the same `OVERMIND_ADDR` the listener uses, so the two
    /// cannot drift: a URL configured separately would be right until someone
    /// moved the port.
    pub self_url: String,

    /// Override for the agent adapter command (`OVERMIND_AGENT_CMD`).
    /// `None` uses the default Claude Code CLI invocation.
    pub agent_cmd: Option<String>,
    /// The tools an agent *may* be granted (ADR-0036): MCP servers declared
    /// by whoever runs the box, in the CLI's own `{"mcpServers": {...}}`
    /// shape, read once from the file `OVERMIND_AGENT_TOOLS` names. A tool is
    /// a command the server will spawn, which is what makes it the
    /// operator's to declare -- like `OVERMIND_AGENT_CMD` and
    /// `OVERMIND_MEMORY_CMD`. Granting is per agent, as a trait.
    pub agent_tools: AgentTools,
    /// Where worktrees and other runtime data live (`OVERMIND_DATA_DIR`).
    pub data_dir: PathBuf,
    /// Where archives land (`OVERMIND_BACKUP_DIR`, ADR-0044); `None` means
    /// `<data>/backups/`.
    pub backup_dir: Option<PathBuf>,
    /// Scheduler tick interval (`OVERMIND_HEARTBEAT_SECS`).
    pub heartbeat_ms: u64,
    /// Kill sessions running longer than this (`OVERMIND_SESSION_TIMEOUT_SECS`).
    pub session_timeout_secs: u64,
    /// Cents reserved against an agent's budget at task start, before the real
    /// cost is known (`OVERMIND_START_ESTIMATE_CENTS`).
    pub start_estimate_cents: i64,
    /// Command that launches the MCP memory server (`OVERMIND_MEMORY_CMD`);
    /// `None` disables organizational memory entirely (graceful degradation).
    pub memory_cmd: Option<String>,
    /// Compact a conversation before a turn once its transcript exceeds this
    /// many characters (ADR-0040). `OVERMIND_CHAT_COMPACT_CHARS`; 0 = never.
    pub chat_compact_chars: usize,
    /// The CEO writes back on its own when tasks born in a thread finish
    /// after the person's last word there (ADR-0041). `OVERMIND_CEO_DIGEST=off`
    /// disables it.
    pub ceo_digest: bool,
    /// How long a thread must have been quiet before an unprompted update
    /// (`OVERMIND_DIGEST_DEBOUNCE_SECS`, default 180).
    pub digest_debounce_secs: u64,
    /// Built frontend directory (`OVERMIND_WEB_DIR`). Served at the root when
    /// it exists; absent in dev (Vite serves the UI and proxies to us).
    pub web_dir: PathBuf,
    /// Cage agent runs in an OS sandbox (ADR-0023). `OVERMIND_SANDBOX=off`
    /// disables it — deliberately and visibly, which is better than someone
    /// widening the profile until it permits everything.
    pub sandbox: bool,
    /// Extra paths the cage may write (`OVERMIND_SANDBOX_ALLOW`, colon-separated).
    /// The profile cannot know every toolchain; this is how a setup it did not
    /// anticipate gets fixed without turning the cage off.
    pub sandbox_allow: Vec<PathBuf>,
    /// The unprivileged user agent work runs as (`OVERMIND_AGENT_UID`,
    /// `OVERMIND_AGENT_GID`, `OVERMIND_AGENT_HOME` — ADR-0029).
    ///
    /// Set by our image and unset everywhere else, deliberately: on a user's own
    /// machine Overmind runs as that user, and inventing a second uid on
    /// somebody's laptop is not ours to do. `None` here is not a failure, it is
    /// the answer "this mechanism does not apply to how I am running".
    pub agent_uid: Option<u32>,
    /// The group that goes with `agent_uid`; defaults to the uid's own value,
    /// which is what `useradd` produces.
    pub agent_gid: Option<u32>,
    /// The agent's own `HOME`. It needs one it can write: the adapter CLI keeps
    /// credentials and session state there, and the server's home is not the
    /// agent's to use even when it could reach it.
    pub agent_home: Option<PathBuf>,
    /// Declare the economy instead of detecting it (`OVERMIND_ECONOMY=key` or
    /// `subscription`, ADR-0030).
    ///
    /// For the case where we read it wrong, and for an adapter that is not
    /// Claude Code and so cannot be asked. Deliberately not the primary path: a
    /// setting that can disagree with reality is a setting that will, and here
    /// the disagreement is somebody's bill. Anything we do not recognise is
    /// ignored rather than guessed at.
    pub economy_override: Option<crate::economy::Economy>,
    /// Where host repositories are mounted (`OVERMIND_REPOS_DIR`, `/repos` in
    /// the image).
    ///
    /// Set so the server can *say* it. A workspace's `cwd` has to be the
    /// in-container path, and the way you learned that was a comment in
    /// `docker-compose.yml` — so the failure mode was typing your own path,
    /// being told it "is not a directory", and having no idea why. Knowing the
    /// mount point turns that into a sentence that names it.
    pub repos_dir: Option<PathBuf>,
    /// Give each company its own brain under the data dir (ADR-0024).
    /// `OVERMIND_MANAGED_BRAIN=off` restores M7's behaviour — one shared brain,
    /// wherever `memory_cmd` points — for the user who deliberately wants their
    /// agents writing into a brain they chose.
    pub managed_brain: bool,
    /// Mark the session cookie `Secure` (`OVERMIND_COOKIE_SECURE=on`).
    /// Required behind TLS, wrong on plain-http localhost -- the browser
    /// would drop the cookie and every login would silently not stick.
    pub cookie_secure: bool,
}

/// The operator's tool registry (ADR-0036): name -> the MCP server definition
/// exactly as the CLI wants it, plus a one-line description of our own for
/// the interface and the prompt. Empty is the ordinary case.
#[derive(Clone, Debug, Default)]
pub struct AgentTools {
    pub servers: std::collections::BTreeMap<String, serde_json::Value>,
    pub descriptions: std::collections::BTreeMap<String, String>,
    /// Tools that fit one hand at a time (`"exclusive": ["blender"]` in the
    /// registry): granting one to a second active agent is refused with the
    /// holder's name. Blender has one socket; two agents on it would be two
    /// hands on one mouse.
    pub exclusive: std::collections::BTreeSet<String>,
}

impl AgentTools {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.servers.contains_key(name)
    }
    pub fn is_exclusive(&self, name: &str) -> bool {
        self.exclusive.contains(name)
    }
    pub fn description(&self, name: &str) -> Option<&str> {
        self.descriptions.get(name).map(String::as_str)
    }
}

impl Config {
    /// Read the registry from a file in the CLI's `{"mcpServers": {...}}`
    /// shape, with our optional top-level `"descriptions"`. A file that
    /// cannot be read or parsed yields an empty registry **and says so on
    /// stderr**: a misnamed path must not turn into "no tools" in silence.
    pub fn load_agent_tools(path: &std::path::Path) -> AgentTools {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "OVERMIND_AGENT_TOOLS: cannot read {} ({e}); no tools declared",
                    path.display()
                );
                return AgentTools::default();
            }
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "OVERMIND_AGENT_TOOLS: {} is not valid JSON ({e}); no tools declared",
                    path.display()
                );
                return AgentTools::default();
            }
        };
        let servers = v
            .get("mcpServers")
            .and_then(|s| s.as_object())
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let descriptions = v
            .get("descriptions")
            .and_then(|s| s.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|d| (k.clone(), d.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let exclusive = v
            .get("exclusive")
            .and_then(|s| s.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        AgentTools {
            servers,
            descriptions,
            exclusive,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            self_url: "http://127.0.0.1:7070".to_string(),
            agent_cmd: None,
            agent_tools: AgentTools::default(),
            data_dir: PathBuf::from("./overmind-data"),
            backup_dir: None,
            heartbeat_ms: 30_000,
            session_timeout_secs: 3_600,
            start_estimate_cents: 50,
            memory_cmd: None,
            chat_compact_chars: 60_000,
            ceo_digest: true,
            digest_debounce_secs: 180,
            web_dir: PathBuf::from("./web/dist"),
            sandbox: true,
            sandbox_allow: Vec::new(),
            agent_uid: None,
            agent_gid: None,
            agent_home: None,
            economy_override: None,
            repos_dir: None,
            managed_brain: true,
            cookie_secure: false,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let defaults = Config::default();
        Config {
            self_url: format!(
                "http://{}",
                std::env::var("OVERMIND_ADDR").unwrap_or_else(|_| "127.0.0.1:7070".to_string())
            ),
            agent_cmd: std::env::var("OVERMIND_AGENT_CMD").ok(),
            agent_tools: std::env::var("OVERMIND_AGENT_TOOLS")
                .ok()
                .map(|p| Config::load_agent_tools(std::path::Path::new(&p)))
                .unwrap_or_default(),
            data_dir: std::env::var("OVERMIND_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(defaults.data_dir),
            heartbeat_ms: std::env::var("OVERMIND_HEARTBEAT_SECS")
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|secs| (secs * 1000.0) as u64)
                .unwrap_or(defaults.heartbeat_ms),
            session_timeout_secs: std::env::var("OVERMIND_SESSION_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(defaults.session_timeout_secs),
            start_estimate_cents: std::env::var("OVERMIND_START_ESTIMATE_CENTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(defaults.start_estimate_cents),
            memory_cmd: std::env::var("OVERMIND_MEMORY_CMD")
                .ok()
                .filter(|s| !s.is_empty()),
            chat_compact_chars: std::env::var("OVERMIND_CHAT_COMPACT_CHARS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(defaults.chat_compact_chars),
            ceo_digest: std::env::var("OVERMIND_CEO_DIGEST").as_deref() != Ok("off"),
            digest_debounce_secs: std::env::var("OVERMIND_DIGEST_DEBOUNCE_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(defaults.digest_debounce_secs),
            web_dir: std::env::var("OVERMIND_WEB_DIR")
                .map(PathBuf::from)
                .unwrap_or(defaults.web_dir),
            // Anything but an explicit "off" leaves the cage on: a typo must
            // not silently disable a security control.
            sandbox: std::env::var("OVERMIND_SANDBOX")
                .map(|v| !v.eq_ignore_ascii_case("off"))
                .unwrap_or(defaults.sandbox),
            sandbox_allow: std::env::var("OVERMIND_SANDBOX_ALLOW")
                .map(|v| {
                    v.split(':')
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or(defaults.sandbox_allow),
            // A uid we cannot parse is a uid we do not use: the cage is then
            // absent and says so, which is the direction ADR-0023 chose. The
            // alternative — falling back to some default uid — would mean
            // guessing at a security boundary.
            agent_uid: std::env::var("OVERMIND_AGENT_UID")
                .ok()
                .and_then(|s| s.parse().ok()),
            agent_gid: std::env::var("OVERMIND_AGENT_GID")
                .ok()
                .and_then(|s| s.parse().ok()),
            backup_dir: std::env::var("OVERMIND_BACKUP_DIR")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            agent_home: std::env::var("OVERMIND_AGENT_HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            // Only the two names mean anything; a typo leaves detection in
            // charge rather than silently picking one.
            economy_override: match std::env::var("OVERMIND_ECONOMY")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "key" => Some(crate::economy::Economy::Key {
                    // A declaration says which economy, not what it is
                    // shadowing; only the probe can see that.
                    overrides_login: false,
                }),
                "subscription" => Some(crate::economy::Economy::Subscription { plan: None }),
                _ => None,
            },
            repos_dir: std::env::var("OVERMIND_REPOS_DIR")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            // Same shape as `sandbox` above, same reason: only an explicit
            // "off" opts out, so a typo does not silently put two companies
            // back in one brain.
            cookie_secure: std::env::var("OVERMIND_COOKIE_SECURE")
                .map(|v| v == "on")
                .unwrap_or(defaults.cookie_secure),
            managed_brain: std::env::var("OVERMIND_MANAGED_BRAIN")
                .map(|v| !v.eq_ignore_ascii_case("off"))
                .unwrap_or(defaults.managed_brain),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("seed serialization error: {0}")]
    Seed(#[from] serde_json::Error),
    #[error("invalid database url: {0}")]
    Url(String),
}

/// The database file and the two SQLite keeps beside it in WAL mode.
///
/// Parsed rather than configured, because the URL is where the path already
/// lives and a second setting naming the same file is a second thing to get
/// wrong. Anything that is not a plain `sqlite://path` yields nothing, and the
/// caller's restriction is then simply not applied — the honest answer for a
/// URL shape we do not understand.
pub(crate) fn sqlite_files(database_url: &str) -> Vec<PathBuf> {
    let Some(rest) = database_url.strip_prefix("sqlite://") else {
        return Vec::new();
    };
    let path = rest.split('?').next().unwrap_or(rest);
    if path.is_empty() {
        return Vec::new();
    }
    vec![
        PathBuf::from(path),
        PathBuf::from(format!("{path}-wal")),
        PathBuf::from(format!("{path}-shm")),
    ]
}

/// Open (creating if missing), migrate and seed the database, with
/// configuration from the environment.
pub async fn init(database_url: &str) -> Result<AppState, InitError> {
    init_with(database_url, Config::from_env()).await
}

/// Like [`init`] but with explicit configuration (used by tests).
pub async fn init_with(database_url: &str, config: Config) -> Result<AppState, InitError> {
    let options = SqliteConnectOptions::from_str(database_url)
        .map_err(|e| InitError::Url(e.to_string()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // Writers wait their turn instead of failing on the spot (M23).
        // WAL lets readers run beside one writer, but two writers still
        // collide -- and without a busy timeout SQLite answers the second
        // one with "database is locked" immediately. Measured in the first
        // minute of real use: a CEO turn was writing its messages, creating
        // a project at the same moment got a 500. Five seconds is far above
        // any transaction this server holds, so hitting it means a real
        // deadlock, which deserves the error.
        .busy_timeout(std::time::Duration::from_secs(5))
        .foreign_keys(true);

    // An in-memory database exists per-connection: the pool must never open
    // a second connection or drop its only one, or the data vanishes.
    let is_memory = database_url.contains(":memory:");
    let mut pool_options = SqlitePoolOptions::new();
    if is_memory {
        pool_options = pool_options
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None);
    }
    let pool = pool_options.connect_with(options).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    seed_archetypes(&pool).await?;
    seed_domains(&pool).await?;

    // Where agent work runs as another uid (ADR-0029), the boundary is only as
    // real as the filesystem under it. Both calls are no-ops without an agent
    // uid, which is every install but the image.
    if let Err(e) = crate::sandbox::lay_out_data_dir(&config).await {
        eprintln!("data dir layout failed (agents may be able to read it): {e}");
    }
    // SQLite makes these 0644, and they hold the audit chain and every
    // company's data. An agent has no business reading any of it. The WAL and
    // shared-memory files carry the same rows before a checkpoint, so leaving
    // them open would make the main file's mode decorative.
    if !is_memory {
        for path in sqlite_files(database_url) {
            if let Err(e) = crate::sandbox::keep_to_server(&config, &path) {
                eprintln!("cannot restrict {} (ignored): {e}", path.display());
            }
        }
    }

    // Whatever was declared; the binary probes and refines it at startup.
    let economy =
        config
            .economy_override
            .clone()
            .unwrap_or_else(|| crate::economy::Economy::Unknown {
                kind: crate::economy::UnknownKind::Unreadable,
                reason: "not detected yet".into(),
            });
    let (events, _) = broadcast::channel(256);
    let (restart, _) = broadcast::channel(1);
    let memory = crate::mcp::Memory::from_config(config.memory_cmd.clone());
    Ok(AppState {
        pool,
        config: Arc::new(config),
        running: Arc::new(Mutex::new(HashSet::new())),
        events,
        restart,
        memory,
        brains: Arc::new(Mutex::new(HashMap::new())),
        economy: Arc::new(std::sync::RwLock::new(economy)),
        plan_windows: Arc::new(std::sync::RwLock::new(Default::default())),
        answering: Arc::new(Mutex::new(HashMap::new())),
        activity: Arc::new(Mutex::new(HashMap::new())),
        digested: Arc::new(Mutex::new(HashMap::new())),
    })
}

/// The archetype every company is founded with (M15).
pub const CEO_ARCHETYPE: &str = "chief-executive";

/// The domain an agent has when none was chosen (ADR-0021).
pub const GENERAL_DOMAIN: &str = "general";

/// The CEO thinks for the whole company, so it gets the strongest model. A
/// lookup rather than a constant (ADR-0021): "the strongest model" stays true
/// as the catalog moves, instead of being true on the day it was typed.
pub fn ceo_model() -> &'static str {
    crate::model::strongest().id
}

/// Names the system picks from when it founds a company's CEO. Short, easy to
/// say, and deliberately not tied to a gender or a language — you will be
/// talking to this one every day.
const CEO_NAMES: &[&str] = &[
    "Aria", "Nova", "Kai", "Iris", "Ada", "Ren", "Sol", "Vera", "Nico", "Yuki", "Rune", "Mira",
    "Theo", "Zara", "Enzo", "Lumi",
];

/// A name for a newly founded company's CEO. Not cryptographically random and
/// it does not need to be: it only has to feel chosen rather than defaulted.
///
/// It used to index the list by `subsec_nanos()`, which is not random at all
/// where the clock is coarse. macOS reports whole microseconds, so the value is
/// always a multiple of 1000 and `1000 % 16 == 8` — only indices 0 and 8 could
/// ever come up, and every company founded on a Mac got "Aria" or "Nico".
///
/// That was a cosmetic bug with an expensive shadow: the same skew meant a CI
/// flake that depends on which name is drawn was unreproducible on the machine
/// the code is written on. A random source that differs by platform makes
/// "works locally" mean less than it should.
pub fn random_ceo_name() -> &'static str {
    // v7's tail is 62 random bits, and uuid is already here.
    let n = uuid::Uuid::now_v7().as_bytes()[15] as usize;
    CEO_NAMES[n % CEO_NAMES.len()]
}

/// The built-in **function** catalog (UX.md: "the catalog is a product
/// surface"). Since ADR-0021 an archetype answers exactly one question — *what
/// kind of work does this agent do* — and the field it does it in is a separate
/// axis (see [`builtin_domains`]). "Media & A/V quality" is `reviewer ×
/// media-av`: two clicks and no free text, rather than a row of its own.
///
/// Idempotent: only slugs that don't exist yet are inserted, so user-added
/// archetypes, future catalog versions, and the pre-ADR-0021 rows already
/// sitting in an existing database all coexist.
fn builtin_archetypes() -> Vec<(&'static str, &'static str, &'static str, AgentTraits)> {
    let base = |focus: &[&str], perms: &[&str], autonomy, strictness| AgentTraits {
        focus_areas: focus.iter().map(|s| s.to_string()).collect(),
        permissions: perms.iter().map(|s| s.to_string()).collect(),
        autonomy,
        review_strictness: strictness,
        monthly_budget_cents: 5_000,
        model: crate::model::default_model().id.to_string(),
        // A function is not visual by itself; the domain decides that.
        multimodal: false,
        // Tools are granted to a person's agent, never to a function (ADR-0036).
        tools: Vec::new(),
    };
    vec![
        (
            "chief-executive",
            "Chief Executive",
            "Runs the company. Turns what you want into an organization and a plan: decides who to hire, who reports to whom, and what gets worked on first. Delegates rather than executing, and escalates to you the calls that are yours.",
            AgentTraits {
                focus_areas: vec![
                    "direction".into(),
                    "delegation".into(),
                    "prioritisation".into(),
                    "judgement".into(),
                ],
                // The CEO is the one agent that may take on anything; the
                // budget, not the capability list, is what bounds it (M15).
                permissions: [
                    "task:code",
                    "task:knowledge",
                    "repo:read",
                    "web:read",
                    "docs:write",
                    "pr:create",
                    "pr:comment",
                    "pr:approve",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                autonomy: Autonomy::ActWithinBudget,
                review_strictness: ReviewStrictness::Standard,
                monthly_budget_cents: 2_000,
                model: ceo_model().to_string(),
                // The leader reads whatever you bring to the conversation.
                multimodal: true,
                tools: Vec::new(),
            },
        ),
        (
            "builder",
            "Builder",
            "Builds the thing itself: implements, assembles, configures, and makes it work. Hands changes over for review rather than putting them live.",
            base(
                &["implementation", "assembly", "configuration", "tests"],
                &[
                    "task:code",
                    "task:knowledge",
                    "repo:read",
                    "repo:write-branch",
                    "pr:create",
                ],
                Autonomy::ActWithApproval,
                ReviewStrictness::Standard,
            ),
        ),
        (
            "reviewer",
            "Reviewer",
            "Judges work against a standard — correctness, quality, safety, compliance — and says what is wrong and why. Reads everything, changes nothing.",
            base(
                &["correctness", "quality", "risks", "standards"],
                &[
                    "task:code",
                    "task:knowledge",
                    "repo:read",
                    "pr:comment",
                    "pr:approve",
                ],
                Autonomy::ProposeOnly,
                ReviewStrictness::Strict,
            ),
        ),
        (
            "researcher",
            "Researcher",
            "Investigates open questions, compares the options honestly, and writes up what it found with the sources it found it in. Needs no access to your code.",
            base(
                &["investigation", "comparison", "sources"],
                &["task:knowledge", "web:read", "docs:write"],
                Autonomy::ActWithinBudget,
                ReviewStrictness::Lenient,
            ),
        ),
        (
            "writer",
            "Writer",
            "Turns what the company knows into something a person can read: guides, references, briefs, changelogs.",
            base(
                &["clarity", "structure", "accuracy"],
                &["task:knowledge", "repo:read", "docs:write", "pr:create"],
                Autonomy::ActWithApproval,
                ReviewStrictness::Standard,
            ),
        ),
        (
            "analyst",
            "Analyst",
            "Works the numbers: costs, projections, comparisons, unit economics. Shows the model it used, not only the answer it reached.",
            base(
                &["modelling", "estimates", "trade-offs"],
                &["task:knowledge", "web:read", "docs:write"],
                Autonomy::ActWithinBudget,
                ReviewStrictness::Standard,
            ),
        ),
    ]
}

/// The built-in **domain** catalog (ADR-0021): the field an agent works in,
/// orthogonal to the function it performs there.
///
/// A domain is additive by construction — it contributes focus areas, declared
/// capabilities, whether the field is visual by nature, and one line telling
/// the agent where it is standing. It never grants `task:code` /
/// `task:knowledge`: which kind of work an agent may be checked out onto is a
/// property of the function, not of the subject matter.
fn builtin_domains() -> Vec<(&'static str, &'static str, &'static str, DomainPatch)> {
    let d = |focus: &[&str], perms: &[&str], multimodal, context: &str| DomainPatch {
        focus_areas: focus.iter().map(|s| s.to_string()).collect(),
        permissions: perms.iter().map(|s| s.to_string()).collect(),
        multimodal,
        context: context.to_string(),
    };
    vec![
        (
            GENERAL_DOMAIN,
            "General",
            "No particular field. Pick this when the work is not about one subject in particular — the function alone describes it.",
            DomainPatch::default(),
        ),
        (
            "software",
            "Software",
            "Software as a whole: source, architecture, and the tests that hold it up.",
            d(
                &["architecture", "maintainability", "tests"],
                &["repo:read"],
                false,
                "You work on software: its source, its architecture, and the tests that hold it up.",
            ),
        ),
        (
            "backend",
            "Backend",
            "The server side: APIs, data models, business logic, and the durability of all three.",
            d(
                &["api", "data-model", "business-logic"],
                &["repo:read"],
                false,
                "You work on the server side: APIs, data models, business logic, and the durability of all three.",
            ),
        ),
        (
            "frontend",
            "Frontend",
            "The interface people actually touch: components, styling, accessibility, client state.",
            d(
                &["ui-components", "styling", "accessibility", "client-state"],
                &["repo:read"],
                true,
                "You work on the interface people actually touch: components, styling, accessibility and client state. Screenshots and mockups are evidence, not decoration — look at them.",
            ),
        ),
        (
            "security",
            "Security",
            "Vulnerabilities, secrets handling, dependency risk, and who is allowed to do what.",
            d(
                &[
                    "vulnerabilities",
                    "secrets-handling",
                    "dependencies",
                    "authz",
                ],
                &["repo:read"],
                false,
                "You work on security: OWASP-class vulnerabilities, secrets handling, dependency and supply-chain risk, and who is allowed to do what.",
            ),
        ),
        (
            "media-av",
            "Media & A/V",
            "Picture and sound: display and projection, audio reproduction, calibration, room acoustics, equipment.",
            d(
                &[
                    "picture-quality",
                    "sound-quality",
                    "calibration",
                    "room-acoustics",
                    "equipment",
                ],
                &["web:read"],
                true,
                "You work on picture and sound: display and projection quality, audio reproduction, calibration, room acoustics, and the equipment that produces them. You are expected to look at the material and judge what it actually is, not only read its specifications.",
            ),
        ),
        (
            "home-systems",
            "Home & Building Systems",
            "Physical spaces and what gets installed in them: layout, wiring, mounting, standards, cost.",
            d(
                &["layout", "wiring", "installation", "standards", "budget"],
                &["web:read"],
                true,
                "You work on physical spaces and the systems installed in them: layout, wiring routes, mounting, the standards that apply, and what it costs to do properly. Photographs and plans of the actual space are evidence — look at them.",
            ),
        ),
        (
            "finance",
            "Finance",
            "Money: costs, projections, unit economics, and the risk hiding in both.",
            d(
                &["costs", "projections", "unit-economics", "risk"],
                &["web:read"],
                false,
                "You work on money: costs, projections, unit economics, and the risk hiding in all of them. State your assumptions where you had to make them.",
            ),
        ),
        (
            "legal",
            "Legal & Compliance",
            "Contracts, licensing, compliance and obligations — and knowing when a qualified human must sign off.",
            d(
                &["contracts", "compliance", "licensing", "obligations"],
                &["web:read"],
                false,
                "You work on contracts, licensing, compliance and obligations. Say plainly when something needs a qualified human to sign off, rather than answering as if you were one.",
            ),
        ),
    ]
}

async fn seed_archetypes(pool: &SqlitePool) -> Result<(), InitError> {
    for (slug, name, description, traits) in builtin_archetypes() {
        let traits_json = serde_json::to_string(&traits)?;
        sqlx::query(
            "INSERT INTO archetypes (id, slug, name, description, default_traits, created_at)
             SELECT ?, ?, ?, ?, ?, ?
             WHERE NOT EXISTS (SELECT 1 FROM archetypes WHERE slug = ?)",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(slug)
        .bind(name)
        .bind(description)
        .bind(traits_json)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(slug)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Seed the domain catalog (ADR-0021). Idempotent on the same terms as
/// [`seed_archetypes`]: a slug that already exists is left exactly as it is,
/// so a user's own domain is never overwritten by a catalog update.
async fn seed_domains(pool: &SqlitePool) -> Result<(), InitError> {
    for (slug, name, description, patch) in builtin_domains() {
        let patch_json = serde_json::to_string(&patch)?;
        sqlx::query(
            "INSERT INTO domains (id, slug, name, description, traits_patch, created_at)
             SELECT ?, ?, ?, ?, ?, ?
             WHERE NOT EXISTS (SELECT 1 FROM domains WHERE slug = ?)",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(slug)
        .bind(name)
        .bind(description)
        .bind(patch_json)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(slug)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Load a domain's patch by slug. `None` for an unknown slug, so callers can
/// tell "no such domain" from "the general domain, which adds nothing".
pub async fn domain_patch(
    pool: &SqlitePool,
    slug: &str,
) -> Result<Option<DomainPatch>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT traits_patch FROM domains WHERE slug = ?")
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    // A patch that will not parse is a seeding bug, not a reason to refuse the
    // hire: fall back to adding nothing rather than failing the request.
    Ok(row.map(|(json,)| serde_json::from_str(&json).unwrap_or_default()))
}

#[cfg(test)]
mod ceo_name_tests {
    use super::{CEO_NAMES, random_ceo_name};
    use std::collections::BTreeSet;

    /// The whole list has to be reachable. Indexing by `subsec_nanos()` looked
    /// random and, on a platform with a microsecond clock, could only ever
    /// produce two of the sixteen — which is also why a CI flake that turned on
    /// the drawn name never once reproduced on a Mac.
    #[test]
    fn every_name_can_come_up() {
        let seen: BTreeSet<&str> = (0..4_000).map(|_| random_ceo_name()).collect();
        assert_eq!(
            seen.len(),
            CEO_NAMES.len(),
            "unreachable names: {:?}",
            CEO_NAMES
                .iter()
                .filter(|n| !seen.contains(*n))
                .collect::<Vec<_>>()
        );
    }
}
