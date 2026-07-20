use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use serde_json::json;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tokio::sync::broadcast;

use crate::domain::{AgentTraits, Autonomy, ReviewStrictness};

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
    /// Organizational memory over MCP (Wadachi reference). A no-op when no
    /// memory server is configured — Overmind is fully functional without it.
    pub memory: crate::mcp::Memory,
}

impl AppState {
    /// Tell connected clients that `company_id`'s board changed. Coarse by
    /// design: the client refetches rather than applying a delta, which keeps
    /// the contract trivial and impossible to desync. A send error just means
    /// no clients are listening — never fatal.
    pub fn notify(&self, company_id: &str) {
        let _ = self
            .events
            .send(json!({ "type": "changed", "company_id": company_id }).to_string());
    }
}

/// Server configuration (env-driven; tests inject their own via `init_with`).
#[derive(Clone, Debug)]
pub struct Config {
    /// Override for the agent adapter command (`OVERMIND_AGENT_CMD`).
    /// `None` uses the default Claude Code CLI invocation.
    pub agent_cmd: Option<String>,
    /// Where worktrees and other runtime data live (`OVERMIND_DATA_DIR`).
    pub data_dir: PathBuf,
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
    /// Built frontend directory (`OVERMIND_WEB_DIR`). Served at the root when
    /// it exists; absent in dev (Vite serves the UI and proxies to us).
    pub web_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            agent_cmd: None,
            data_dir: PathBuf::from("./overmind-data"),
            heartbeat_ms: 30_000,
            session_timeout_secs: 3_600,
            start_estimate_cents: 50,
            memory_cmd: None,
            web_dir: PathBuf::from("./web/dist"),
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let defaults = Config::default();
        Config {
            agent_cmd: std::env::var("OVERMIND_AGENT_CMD").ok(),
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
            web_dir: std::env::var("OVERMIND_WEB_DIR")
                .map(PathBuf::from)
                .unwrap_or(defaults.web_dir),
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
    let (events, _) = broadcast::channel(256);
    let memory = crate::mcp::Memory::from_config(config.memory_cmd.clone());
    Ok(AppState {
        pool,
        config: Arc::new(config),
        running: Arc::new(Mutex::new(HashSet::new())),
        events,
        memory,
    })
}

/// The built-in archetype catalog (UX.md: "the catalog is a product surface").
/// Idempotent: inserts only slugs that don't exist yet, so user-added
/// archetypes and future catalog versions coexist.
fn builtin_archetypes() -> Vec<(&'static str, &'static str, &'static str, AgentTraits)> {
    let base = |focus: &[&str], perms: &[&str], autonomy, strictness| AgentTraits {
        focus_areas: focus.iter().map(|s| s.to_string()).collect(),
        permissions: perms.iter().map(|s| s.to_string()).collect(),
        autonomy,
        review_strictness: strictness,
        monthly_budget_cents: 5_000,
        model: "claude-sonnet".to_string(),
    };
    vec![
        (
            "security-engineer",
            "Security Engineer",
            "Reviews code and configuration for vulnerabilities (OWASP-class issues, secrets handling, dependency risks). Reads everything, changes nothing without review.",
            base(
                &[
                    "vulnerabilities",
                    "secrets-handling",
                    "dependencies",
                    "authz",
                ],
                &["repo:read", "pr:comment", "pr:create"],
                Autonomy::ProposeOnly,
                ReviewStrictness::Strict,
            ),
        ),
        (
            "backend-developer",
            "Backend Developer",
            "Implements server-side features and fixes: APIs, data models, business logic, tests.",
            base(
                &["api", "data-model", "business-logic", "tests"],
                &["repo:read", "repo:write-branch", "pr:create"],
                Autonomy::ActWithApproval,
                ReviewStrictness::Standard,
            ),
        ),
        (
            "frontend-developer",
            "Frontend Developer",
            "Implements UI components, styling and client-side logic, with attention to accessibility.",
            base(
                &["ui-components", "styling", "accessibility", "client-state"],
                &["repo:read", "repo:write-branch", "pr:create"],
                Autonomy::ActWithApproval,
                ReviewStrictness::Standard,
            ),
        ),
        (
            "code-reviewer",
            "Code Reviewer",
            "Reviews pull requests for correctness, clarity and maintainability. Never pushes code.",
            base(
                &["correctness", "maintainability", "style"],
                &["repo:read", "pr:comment", "pr:approve"],
                Autonomy::ProposeOnly,
                ReviewStrictness::Strict,
            ),
        ),
        (
            "researcher",
            "Researcher",
            "Investigates questions, compares options, produces sourced write-ups. No code access needed.",
            base(
                &["investigation", "comparison", "sources"],
                &["web:read", "docs:write"],
                Autonomy::ActWithinBudget,
                ReviewStrictness::Lenient,
            ),
        ),
        (
            "technical-writer",
            "Technical Writer",
            "Writes and maintains documentation: guides, references, changelogs.",
            base(
                &["guides", "reference", "changelog"],
                &["repo:read", "docs:write", "pr:create"],
                Autonomy::ActWithApproval,
                ReviewStrictness::Standard,
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
