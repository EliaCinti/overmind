//! The accounts Overmind holds, and which one pays (ADR-0051).
//!
//! Until now there was one credential in one place: `<data>/claude-oauth-token`,
//! for one provider, plus whatever key the server's environment carried, with
//! ADR-0037's `pay-with-plan` marker choosing between the two. A second Claude
//! account had nowhere to go, and a second vendor's would have overwritten the
//! first.
//!
//! An account is now a row -- `(provider, kind, label, active)` -- and, when
//! Overmind holds its secret, a file under `credentials/<provider>/<id>/`. Any
//! number per provider; exactly one active, and that one is said by the
//! **database**: a partial unique index refuses a second active row, so no code
//! path can forget the rule.
//!
//! `provider` here is the vendor whose bill it is -- `anthropic`, `openai`,
//! `google` -- which is the unit the panel lists. The CLI that spends it is a
//! [`crate::provider::Provider`], and says which vendor it bills to.
//!
//! **The secret never leaves this module except into an agent's environment.**
//! The row carries a label and, for a key, its last four characters; the API
//! returns the row; the value is read per spawn from a 0600 file and handed to
//! [`Spawn`], which has no `Debug`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::db::{AppState, Config};

/// How an account pays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A plan the person already pays for: Claude Pro / Max, ChatGPT Plus.
    Subscription,
    /// A metered API key.
    Key,
}

impl Kind {
    pub fn slug(&self) -> &'static str {
        match self {
            Kind::Subscription => "subscription",
            Kind::Key => "key",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "subscription" => Some(Kind::Subscription),
            "key" => Some(Kind::Key),
            _ => None,
        }
    }
}

/// Where the secret lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A file Overmind wrote, under `credentials/`.
    Stored,
    /// A variable in the server's environment (ADR-0051 decision 3):
    /// selectable, never removable from here.
    Environment,
}

/// One account, as anybody but an agent may see it.
#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: String,
    pub provider: String,
    pub kind: Kind,
    pub source: Source,
    /// What the provider's own probe says the account is -- an e-mail, an org
    /// -- or `unknown`. Never what somebody typed (ADR-0051 decision 1).
    pub label: String,
    pub active: bool,
    /// The last four characters of a key, so two keys can be told apart.
    /// `None` for a subscription: a token's tail identifies nothing a person
    /// recognises.
    pub last_four: Option<String>,
    pub created_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("no such account")]
    NotFound,
    #[error("an account from the environment is removed by removing the variable, not from here")]
    FromEnvironment,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The credential variables the server's environment carries, read once.
///
/// No derived `Debug`: [`Config`] is `Debug`, and a `{:?}` of it in a log line
/// would otherwise print every key the server was started with.
#[derive(Clone, Default)]
pub struct EnvCredentials(BTreeMap<String, String>);

impl std::fmt::Debug for EnvCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

impl EnvCredentials {
    /// Every variable some provider reads a credential from, if set.
    pub fn from_env() -> Self {
        let mut found = BTreeMap::new();
        for var in credential_vars() {
            if let Ok(value) = std::env::var(var)
                && !value.trim().is_empty()
            {
                found.insert(var.to_string(), value.trim().to_string());
            }
        }
        EnvCredentials(found)
    }

    /// For tests and for a deploy that wants to say it explicitly.
    pub fn with(mut self, var: &str, value: &str) -> Self {
        self.0.insert(var.to_string(), value.to_string());
        self
    }

    pub fn get(&self, var: &str) -> Option<&str> {
        self.0.get(var).map(String::as_str)
    }
}

/// What one spawn receives: the active account's credential, and the removal
/// of every other variable the provider would read one from.
///
/// No `Debug`, no `Clone`: it holds a secret, and the only place it goes is a
/// child's environment.
pub struct Spawn {
    /// The account this run is billed to, recorded on its cost events.
    /// `None` while no account is active -- the host's own login, until the
    /// accounts panel exists to repair that (M34 step 4).
    pub account_id: Option<String>,
    set: Vec<(&'static str, String)>,
    strip: Vec<&'static str>,
}

impl Spawn {
    /// Every credential the provider reads, removed, and nothing added: for a
    /// sign-in, which must not run as some other account, and for our own
    /// tools run over what an agent left behind.
    pub fn none(provider: &dyn crate::provider::Provider) -> Self {
        Spawn {
            account_id: None,
            set: Vec::new(),
            strip: KINDS.iter().map(|k| provider.credential_var(*k)).collect(),
        }
    }

    /// One credential and only that one: for asking a provider's probe who an
    /// account is before it is an account at all.
    pub(crate) fn with(provider: &dyn crate::provider::Provider, kind: Kind, secret: &str) -> Self {
        let mut spawn = Spawn::none(provider);
        spawn
            .set
            .push((provider.credential_var(kind), secret.to_string()));
        spawn
    }

    /// Every credential variable of every provider removed: for a command that
    /// is no provider's, and must carry nobody's account.
    pub fn nobody() -> Self {
        Spawn {
            account_id: None,
            set: Vec::new(),
            strip: credential_vars(),
        }
    }

    /// Apply to a command: strip first, then set, so the active account wins
    /// over anything the server's own environment would have passed on.
    pub fn apply(&self, cmd: &mut impl EnvTarget) {
        for var in &self.strip {
            cmd.remove_env(var);
        }
        for (var, value) in &self.set {
            cmd.set_env(var, value);
        }
    }
}

/// The two command types, one environment call each.
pub trait EnvTarget {
    fn set_env(&mut self, k: &str, v: &str);
    fn remove_env(&mut self, k: &str);
}

impl EnvTarget for tokio::process::Command {
    fn set_env(&mut self, k: &str, v: &str) {
        self.env(k, v);
    }
    fn remove_env(&mut self, k: &str) {
        self.env_remove(k);
    }
}

impl EnvTarget for std::process::Command {
    fn set_env(&mut self, k: &str, v: &str) {
        self.env(k, v);
    }
    fn remove_env(&mut self, k: &str) {
        self.env_remove(k);
    }
}

/// Both kinds, in the order the environment is read: a key first, because a
/// key is what the CLI itself lets win.
const KINDS: [Kind; 2] = [Kind::Key, Kind::Subscription];

/// The name of the one file a stored account's secret is kept in. The kind is
/// the row's to say; a file per kind would be two places to disagree.
const SECRET_FILE: &str = "secret";

/// Every variable any known provider reads a credential from.
fn credential_vars() -> Vec<&'static str> {
    crate::provider::all()
        .iter()
        .flat_map(|p| KINDS.iter().map(|k| p.credential_var(*k)))
        .collect()
}

/// Is this a vendor some provider of this build bills to?
fn known_provider(provider: &str) -> bool {
    crate::provider::all()
        .iter()
        .any(|p| p.vendor() == provider)
}

/// Does this data directory hold any credential Overmind wrote?
///
/// What the archive asks before it agrees to leave without a passphrase
/// (ADR-0044, extended to every account by ADR-0051 decision 8). Read off the
/// files rather than the rows: the files are what an archive would carry.
pub fn holds_a_secret(config: &Config) -> bool {
    let root = config.data_dir.join("credentials");
    let Ok(providers) = std::fs::read_dir(&root) else {
        return false;
    };
    providers.flatten().any(|p| {
        std::fs::read_dir(p.path())
            .map(|ids| ids.flatten().any(|id| id.path().join(SECRET_FILE).exists()))
            .unwrap_or(false)
    })
}

/// Where an account's secret is kept.
pub fn credential_dir(config: &Config, provider: &str, id: &str) -> PathBuf {
    config.data_dir.join("credentials").join(provider).join(id)
}

/// Create `dir` and every level from `credentials/` down, each 0700.
///
/// Set explicitly rather than trusted to `DirBuilder::mode`, which applies only
/// to the levels it creates and is narrowed by the umask: a `credentials/`
/// left by an older build, or made under a loose umask, must still end up
/// readable by the server alone.
fn make_private_dirs(config: &Config, dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    let root = config.data_dir.join("credentials");
    let mut level = Some(dir);
    while let Some(d) = level {
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        if d == root {
            break;
        }
        level = d.parent();
    }
    Ok(())
}

/// Write a secret where only the server can read it.
fn write_secret(config: &Config, dir: &std::path::Path, secret: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    make_private_dirs(config, dir)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join(SECRET_FILE))?;
    f.write_all(secret.as_bytes())
}

/// A stored account's secret.
///
/// An error is an error: an unreadable file is not "never signed in", which
/// is what the old `stored_token` made of it by swallowing every failure.
fn read_secret(config: &Config, a: &Account) -> Result<String, AccountError> {
    let path = credential_dir(config, &a.provider, &a.id).join(SECRET_FILE);
    let secret = std::fs::read_to_string(&path)?.trim().to_string();
    if secret.is_empty() {
        return Err(AccountError::Invalid(format!(
            "the secret of account {} is empty",
            a.id
        )));
    }
    Ok(secret)
}

type Row = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    i64,
    String,
);

const COLUMNS: &str = "id, provider, kind, source, env_var, label, last_four, active, created_at";

fn from_row(r: Row) -> Account {
    let (id, provider, kind, source, _env_var, label, last_four, active, created_at) = r;
    Account {
        id,
        provider,
        // The table's CHECK constraints admit only these two words each.
        kind: Kind::from_slug(&kind).unwrap_or(Kind::Key),
        source: if source == "environment" {
            Source::Environment
        } else {
            Source::Stored
        },
        label,
        active: active == 1,
        last_four,
        created_at,
    }
}

async fn by_id(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
) -> Result<Option<(Account, Option<String>)>, sqlx::Error> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM accounts WHERE id = ?"))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(|r| {
        let env_var = r.4.clone();
        (from_row(r), env_var)
    }))
}

/// What an audit row says about an account: enough to know which one, and
/// never the secret.
fn audited(a: &Account) -> serde_json::Value {
    serde_json::json!({
        "account_id": a.id,
        "provider": a.provider,
        "kind": a.kind.slug(),
        "source": match a.source { Source::Stored => "stored", Source::Environment => "environment" },
        "label": a.label,
    })
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Link an account Overmind will hold. Active only when it is the provider's
/// first: a second account never changes who pays by arriving.
pub async fn link(
    state: &AppState,
    provider: &str,
    kind: Kind,
    secret: &str,
    label: Option<&str>,
) -> Result<Account, AccountError> {
    link_stored(
        &state.pool,
        &state.config,
        provider,
        kind,
        secret,
        label,
        None,
    )
    .await
}

async fn link_stored(
    pool: &sqlx::SqlitePool,
    config: &Config,
    provider: &str,
    kind: Kind,
    secret: &str,
    label: Option<&str>,
    migrated_from: Option<&str>,
) -> Result<Account, AccountError> {
    if !known_provider(provider) {
        return Err(AccountError::Invalid(format!(
            "no provider of this build bills to \"{provider}\""
        )));
    }
    let secret = secret.trim();
    if secret.is_empty() {
        return Err(AccountError::Invalid(
            "the credential must not be empty".into(),
        ));
    }
    if secret.chars().any(char::is_control) {
        // It is going into an environment variable; a newline in one is a
        // second variable somebody did not mean to set.
        return Err(AccountError::Invalid(
            "the credential must be one line of printable characters".into(),
        ));
    }
    // Registered before anything else can print it.
    crate::secrets::register(secret);

    let id = uuid::Uuid::now_v7().to_string();
    let dir = credential_dir(config, provider, &id);
    write_secret(config, &dir, secret)?;

    let account = Account {
        id: id.clone(),
        provider: provider.to_string(),
        kind,
        source: Source::Stored,
        label: label
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .unwrap_or("unknown")
            .to_string(),
        active: false,
        last_four: (kind == Kind::Key).then(|| last_four(secret)),
        created_at: now(),
    };
    let inserted = insert(pool, account, None, migrated_from).await;
    if inserted.is_err() {
        // A secret with no row is a secret nobody can see or remove.
        let _ = std::fs::remove_dir_all(&dir);
    }
    inserted
}

fn last_four(secret: &str) -> String {
    let n = secret.chars().count();
    secret.chars().skip(n.saturating_sub(4)).collect()
}

/// Insert the row, active when the provider has no account at all yet, and
/// audit it in the same transaction.
async fn insert(
    pool: &sqlx::SqlitePool,
    mut account: Account,
    env_var: Option<&str>,
    migrated_from: Option<&str>,
) -> Result<Account, AccountError> {
    let mut tx = pool.begin().await?;
    let (others,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE provider = ?")
        .bind(&account.provider)
        .fetch_one(&mut *tx)
        .await?;
    account.active = others == 0;
    sqlx::query(
        "INSERT INTO accounts (id, provider, kind, source, env_var, label, last_four, active, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&account.id)
    .bind(&account.provider)
    .bind(account.kind.slug())
    .bind(match account.source {
        Source::Stored => "stored",
        Source::Environment => "environment",
    })
    .bind(env_var)
    .bind(&account.label)
    .bind(&account.last_four)
    .bind(account.active)
    .bind(&account.created_at)
    .execute(&mut *tx)
    .await?;
    let mut payload = audited(&account);
    payload["active"] = account.active.into();
    if let Some(from) = migrated_from {
        payload["migrated_from"] = from.into();
    }
    crate::audit::append(
        &mut tx,
        None,
        None,
        crate::domain::event_kind::ACCOUNT_CONNECTED,
        &payload,
    )
    .await?;
    tx.commit().await?;
    Ok(account)
}

/// Make this the provider's payer; the one before it stops being.
pub async fn activate(state: &AppState, id: &str) -> Result<Account, AccountError> {
    let mut tx = state.pool.begin().await?;
    let Some((mut account, _)) = by_id(&mut tx, id).await? else {
        return Err(AccountError::NotFound);
    };
    // Off first, then on: the index would refuse the other order.
    sqlx::query("UPDATE accounts SET active = 0 WHERE provider = ? AND active = 1")
        .bind(&account.provider)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE accounts SET active = 1 WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    account.active = true;
    crate::audit::append(
        &mut tx,
        None,
        None,
        crate::domain::event_kind::ACCOUNT_ACTIVATED,
        &audited(&account),
    )
    .await?;
    tx.commit().await?;
    Ok(account)
}

/// Leave a provider with no payer at all, so its CLI falls back on the host's
/// own login -- ADR-0037's "let the plan pay" where the plan is the host's.
///
/// Exists only until the accounts panel does (M34 step 4), which removes the
/// fallback and with it the last reason to want nobody active.
pub(crate) async fn deactivate_provider(
    state: &AppState,
    provider: &str,
) -> Result<(), AccountError> {
    sqlx::query("UPDATE accounts SET active = 0 WHERE provider = ? AND active = 1")
        .bind(provider)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// Forget an account and delete its secret.
///
/// What this cannot do is said by the caller, not hidden: a subscription's
/// token is revoked only at the provider's own site (ADR-0051 decision 7).
pub async fn disconnect(state: &AppState, id: &str) -> Result<(), AccountError> {
    let mut tx = state.pool.begin().await?;
    let Some((account, _)) = by_id(&mut tx, id).await? else {
        return Err(AccountError::NotFound);
    };
    if account.source == Source::Environment {
        return Err(AccountError::FromEnvironment);
    }
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::audit::append(
        &mut tx,
        None,
        None,
        crate::domain::event_kind::ACCOUNT_DISCONNECTED,
        &audited(&account),
    )
    .await?;
    tx.commit().await?;
    // After the commit: a row that outlived its secret would be an account
    // that pays with nothing, and the reverse is only a file to sweep.
    match std::fs::remove_dir_all(credential_dir(&state.config, &account.provider, id)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Every stored account's secret, as `(provider, id, secret)`: what the
/// archive seals (ADR-0051 decision 8). An unreadable one is an error, never
/// skipped -- an archive that silently left one behind would restore an
/// account that pays with nothing.
pub async fn stored_secrets(
    pool: &sqlx::SqlitePool,
    config: &Config,
) -> Result<Vec<(String, String, String)>, AccountError> {
    let mut out = Vec::new();
    for a in list(pool).await? {
        if a.source == Source::Stored {
            let secret = read_secret(config, &a)?;
            out.push((a.provider, a.id, secret));
        }
    }
    Ok(out)
}

pub async fn list(pool: &sqlx::SqlitePool) -> Result<Vec<Account>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM accounts ORDER BY provider, created_at"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(from_row).collect())
}

pub async fn active(
    pool: &sqlx::SqlitePool,
    provider: &str,
) -> Result<Option<Account>, sqlx::Error> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM accounts WHERE provider = ? AND active = 1"
    ))
    .bind(provider)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(from_row))
}

/// What a spawn of this provider's CLI receives, decided now and carried for
/// the whole run.
pub async fn for_spawn(
    state: &AppState,
    provider: &dyn crate::provider::Provider,
) -> Result<Spawn, AccountError> {
    let mut spawn = Spawn::none(provider);
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM accounts WHERE provider = ? AND active = 1"
    ))
    .bind(provider.vendor())
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Ok(spawn);
    };
    let env_var = row.4.clone();
    let account = from_row(row);
    let secret = match (&account.source, env_var) {
        (Source::Environment, Some(var)) => state
            .config
            .env_credentials
            .get(&var)
            .map(str::to_string)
            .ok_or_else(|| {
                AccountError::Invalid(format!("the environment no longer carries {var}"))
            })?,
        _ => read_secret(&state.config, &account)?,
    };
    spawn
        .set
        .push((provider.credential_var(account.kind), secret));
    spawn.account_id = Some(account.id);
    Ok(spawn)
}

/// At boot: the environment's credentials become accounts, and an instance
/// from before accounts has its token and its choice carried over.
///
/// The environment first, so the migration can see the key it is choosing
/// between. Every secret is registered with the scrubber on the way.
pub async fn boot(pool: &sqlx::SqlitePool, config: &Config) -> Result<(), AccountError> {
    sync_environment(pool, config).await?;
    migrate_legacy(pool, config).await?;
    for a in list(pool).await? {
        if a.source == Source::Stored {
            match read_secret(config, &a) {
                Ok(secret) => crate::secrets::register(&secret),
                // Said, not swallowed; the run that needs it will fail loudly.
                Err(e) => eprintln!("accounts: the secret of {} cannot be read: {e}", a.id),
            }
        }
    }
    Ok(())
}

/// One row per credential variable the environment carries, and none for a
/// variable it no longer does.
///
/// A new row is active only when its provider had no account at all: a key
/// that appears in the environment of an instance where somebody already chose
/// a payer must not take over at the next restart.
async fn sync_environment(pool: &sqlx::SqlitePool, config: &Config) -> Result<(), AccountError> {
    for provider in crate::provider::all() {
        for kind in KINDS {
            let var = provider.credential_var(kind);
            let existing: Option<(String,)> =
                sqlx::query_as("SELECT id FROM accounts WHERE env_var = ?")
                    .bind(var)
                    .fetch_optional(pool)
                    .await?;
            match (config.env_credentials.get(var), existing) {
                (Some(value), None) => {
                    crate::secrets::register(value);
                    let account = Account {
                        id: uuid::Uuid::now_v7().to_string(),
                        provider: provider.vendor().to_string(),
                        kind,
                        source: Source::Environment,
                        label: "unknown".into(),
                        active: false,
                        last_four: (kind == Kind::Key).then(|| last_four(value)),
                        created_at: now(),
                    };
                    insert(pool, account, Some(var), None).await?;
                }
                (Some(value), Some((id,))) => {
                    crate::secrets::register(value);
                    // The variable may hold a different key than last boot.
                    sqlx::query("UPDATE accounts SET last_four = ? WHERE id = ?")
                        .bind((kind == Kind::Key).then(|| last_four(value)))
                        .bind(id)
                        .execute(pool)
                        .await?;
                }
                (None, Some((id,))) => {
                    sqlx::query("DELETE FROM accounts WHERE id = ?")
                        .bind(id)
                        .execute(pool)
                        .await?;
                }
                (None, None) => {}
            }
        }
    }
    Ok(())
}

/// The token file and ADR-0037's marker, carried into accounts once.
///
/// The marker meant "the plan pays over the key"; its absence, "the key wins".
/// So: with the marker the subscription is made active, and with no token
/// behind it nobody is -- the host's login, which is what the marker paid with
/// before. Without it, whoever [`insert`]'s first-account rule picked stays.
async fn migrate_legacy(pool: &sqlx::SqlitePool, config: &Config) -> Result<(), AccountError> {
    let token_file = config.data_dir.join(LEGACY_TOKEN_FILE);
    let marker = config.data_dir.join(LEGACY_MARKER_FILE);
    let plan_chosen = marker.exists();
    let token = match std::fs::read_to_string(&token_file) {
        Ok(t) => Some(t.trim().to_string()).filter(|t| !t.is_empty()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if token.is_none() && !plan_chosen {
        return Ok(());
    }
    let mut chosen: Option<String> = None;
    if let Some(token) = token {
        let a = link_stored(
            pool,
            config,
            "anthropic",
            Kind::Subscription,
            &token,
            None,
            Some(LEGACY_TOKEN_FILE),
        )
        .await?;
        chosen = Some(a.id);
    }
    if plan_chosen {
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE accounts SET active = 0 WHERE provider = 'anthropic' AND active = 1")
            .execute(&mut *tx)
            .await?;
        if let Some(id) = &chosen {
            sqlx::query("UPDATE accounts SET active = 1 WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
    }
    // Removed only once the account holds the token: a crash before this line
    // migrates again at the next boot, which links a second copy -- harmless,
    // and better than losing the only one.
    for f in [&token_file, &marker] {
        match std::fs::remove_file(f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    eprintln!("accounts: carried the old token and payer choice over into accounts");
    Ok(())
}

/// The file M23 kept the one subscription token in, before accounts.
pub const LEGACY_TOKEN_FILE: &str = "claude-oauth-token";
/// ADR-0037's "let the plan pay" marker, before accounts.
pub const LEGACY_MARKER_FILE: &str = "pay-with-plan";
