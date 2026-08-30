//! The archive is the instance (ADR-0044, M31).
//!
//! Slice A: the export. One `tar.gz`, assembled from a staging directory so
//! every entry's hash is known before the archive is written, the manifest
//! first. The database and every managed brain are taken with `VACUUM INTO` —
//! a consistent snapshot, never a file copy of a database in WAL mode. Two
//! columns are scrubbed in the snapshot before it is hashed: the per-run MCP
//! bearer and the editor's integration tokens; a restored instance does not
//! honour credentials that lived in a file. The subscription token travels
//! only sealed, under a passphrase the server never keeps.
//!
//! What stays out, by name: `sessions/`, `chat/`, `worktrees/` and `meetings/`
//! (scratch by the runner's, the CEO's and `lay_out_data_dir`'s own word — a
//! meeting room is handed to the agent uid, and its transcript is durable in
//! `meeting_turns`, not in the room), the staging directory, and `backups/`
//! itself — an archive does not contain the archives.
//!
//! Everything the export writes is the server's alone while it is being
//! written: the staging tree is `0700` inside the `0700` backup folder, the
//! snapshots `0600`, and no copy ever follows a symlink.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::db::{AppState, Config};

/// The archive format this server writes. A restore refuses a format it does
/// not know rather than guessing at it.
pub const FORMAT: u32 = 1;

const SCOPE_INSTANCE: &str = "instance";
const DB_ENTRY: &str = "overmind.sqlite";
const MANIFEST_ENTRY: &str = "MANIFEST.json";
const TOKEN_ENTRY: &str = "secrets/claude-oauth-token.enc";
const BRAIN_DB: &str = "brain.db";

/// argon2id parameters for the sealed token, written down because the door's
/// `Argon2::default()` is not a spec: 64 MiB, three passes, one lane.
const KDF_M_KIB: u32 = 64 * 1024;
const KDF_T: u32 = 3;
const KDF_P: u32 = 1;

/// The shortest passphrase that may seal a token. Longer than the door's
/// password floor because the archive is the thing that travels.
const MIN_PASSPHRASE_CHARS: usize = 12;

/// The chain report of the *snapshot* — computed by opening the `VACUUM INTO`
/// output, not the live pool, which keeps writing while the export runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainSummary {
    pub valid: bool,
    pub events_checked: i64,
    pub first_invalid_seq: Option<i64>,
    pub last_seq: Option<i64>,
    pub last_hash: Option<String>,
}

/// How the subscription token was sealed. Salt and nonce ride here in the
/// clear; the key exists only for the duration of the request that made it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSeal {
    pub kdf: String,
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
    pub cipher: String,
    pub salt: String,
    pub nonce: String,
}

/// The archive's word for itself. Restore checks every entry against it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub overmind_version: String,
    pub created_at: String,
    pub scope: String,
    /// `snapshot` when every brain was taken with `VACUUM INTO`, `copied` when
    /// at least one was a plain directory copy (another memory provider),
    /// `none` when memory is off or no company has a brain yet.
    pub brain: String,
    pub chain: ChainSummary,
    /// Entry path → SHA-256 (hex), for every entry after the manifest.
    pub entries: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<TokenSeal>,
}

/// What an export answers with.
#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub name: String,
    pub bytes: u64,
    pub scope: String,
    pub chain: ChainSummary,
    pub sealed_token: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// A token is on the box and no passphrase came with the request. Said
    /// before anything is written.
    #[error(
        "a subscription token is stored on this instance: give a passphrase to seal it, \
         or remove the sign-in first"
    )]
    PassphraseRequired,
    #[error(
        "a passphrase that seals an archive should be at least {MIN_PASSPHRASE_CHARS} characters: \
         the archive leaves this machine, and nothing rate-limits an attempt on it"
    )]
    PassphraseTooShort,
    #[error("{0}")]
    Config(String),
    #[error("the archive could not be written: {0}")]
    Io(#[from] std::io::Error),
    #[error("the snapshot could not be taken: {0}")]
    Db(String),
    #[error("the token could not be sealed: {0}")]
    Seal(String),
    #[error("the manifest could not be written: {0}")]
    Manifest(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum UnsealError {
    #[error("the manifest names no sealed token")]
    NoToken,
    #[error("the manifest's seal parameters are not readable: {0}")]
    Manifest(String),
    #[error("the passphrase does not open this token")]
    Refused,
}

/// Where archives land: `OVERMIND_BACKUP_DIR`, or `<data>/backups/`.
pub fn backup_dir(config: &Config) -> PathBuf {
    config
        .backup_dir
        .clone()
        .unwrap_or_else(|| config.data_dir.join("backups"))
}

/// Export the whole instance. Owner-only and claimed-only are the caller's
/// checks (the API's); this function is about the archive.
pub async fn export(
    state: &AppState,
    passphrase: Option<&str>,
) -> Result<ExportReport, ExportError> {
    let config = state.config.clone();
    let token = crate::claude_auth::stored_token(&config);
    let passphrase = passphrase.map(str::trim).filter(|p| !p.is_empty());
    if token.is_some() && passphrase.is_none() {
        return Err(ExportError::PassphraseRequired);
    }
    if let Some(p) = passphrase {
        // The door refuses a password under eight characters, and that
        // credential never leaves the box. This one does: an archive on
        // somebody's disk can be attacked for as long as they like, with no
        // rate limiter in the way. Count characters, not bytes.
        if p.chars().count() < MIN_PASSPHRASE_CHARS {
            return Err(ExportError::PassphraseTooShort);
        }
    }

    let folder = backup_dir(&config);
    if config.data_dir.starts_with(&folder) {
        // `OVERMIND_BACKUP_DIR=/data` is a reasonable thing to type and a
        // disaster to obey: the folder is forced 0700, and the data dir is
        // 0755 precisely so a caged agent can walk through it to its own run.
        return Err(ExportError::Config(format!(
            "OVERMIND_BACKUP_DIR ({}) holds the data directory: choose a folder of its own",
            folder.display()
        )));
    }
    private_dir(&folder)?;

    // Staged INSIDE the backup folder, and private itself. Under the data dir
    // it would have sat in a directory the cage deliberately leaves
    // traversable (`lay_out_data_dir`), at whatever mode the umask felt like —
    // and for the length of an export the agent uid could have read a
    // world-readable copy of every company's database and brain. `is_archive_name`
    // keeps the dot-prefixed staging out of the listing.
    let export_id = uuid::Uuid::now_v7().simple().to_string();
    let staging = folder.join(format!(".staging-{export_id}"));
    private_dir(&staging)?;

    let outcome = build(
        state,
        &config,
        &staging,
        &folder,
        token.as_deref(),
        passphrase,
    )
    .await;
    // The staging directory does not outlive the export, success or not.
    let _ = std::fs::remove_dir_all(&staging);
    let report = outcome?;

    // "Every export is on the chain": an archive whose event could not be
    // written is an archive that never existed.
    let audited = audit_the_export(state, &report).await;
    if let Err(e) = audited {
        let _ = std::fs::remove_file(folder.join(&report.name));
        return Err(e);
    }
    Ok(report)
}

async fn audit_the_export(state: &AppState, report: &ExportReport) -> Result<(), ExportError> {
    let mut conn = state
        .pool
        .acquire()
        .await
        .map_err(|e| ExportError::Db(format!("audit: {e}")))?;
    crate::audit::append(
        &mut conn,
        None,
        None,
        "backup.exported",
        &json!({
            "name": report.name,
            "bytes": report.bytes,
            "scope": report.scope,
            "chain": report.chain,
            "sealed_token": report.sealed_token,
        }),
    )
    .await
    .map_err(|e| ExportError::Db(format!("audit: {e}")))?;
    Ok(())
}

async fn build(
    state: &AppState,
    config: &Config,
    staging: &Path,
    folder: &Path,
    token: Option<&str>,
    passphrase: Option<&str>,
) -> Result<ExportReport, ExportError> {
    let created_at = chrono::Utc::now();

    // 1. The database: a consistent snapshot of the live pool.
    let snapshot = staging.join(DB_ENTRY);
    vacuum_into(&state.pool, &snapshot)
        .await
        .map_err(|e| ExportError::Db(format!("VACUUM INTO {}: {e}", snapshot.display())))?;
    if !snapshot.is_file() {
        // Said before anything touches the file: sqlx's in-memory database
        // answers `VACUUM INTO` with success and no file, and "No such file"
        // from a chmod is not an explanation.
        return Err(ExportError::Db(format!(
            "VACUUM INTO {} produced no file",
            snapshot.display()
        )));
    }
    keep_private(&snapshot)?;
    let chain = scrub_and_report(&snapshot)
        .await
        .map_err(|e| ExportError::Db(format!("reading the snapshot back: {e}")))?;

    // 2. Every company's brain the same way. The snapshot is SQLite's work;
    //    the rest of the directory is copied with everything else, below.
    let companies: Vec<(String,)> = sqlx::query_as("SELECT id FROM companies")
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ExportError::Db(format!("listing companies: {e}")))?;
    let mut brain_mode = "none";
    let mut brains = Vec::new();
    for (company_id,) in &companies {
        let brain = state.brain_dir(company_id);
        if !brain.is_dir() {
            continue;
        }
        let dest = staging.join("companies").join(company_id).join("brain");
        private_dir(&dest)?;
        let db = brain.join(BRAIN_DB);
        // `symlink_metadata`, not `is_file`: a `brain.db` that is a link
        // pointing at another company's brain — or at the live database —
        // would be read through by SQLite as the server. The copy below
        // refuses links; so does this.
        let is_a_database = db
            .symlink_metadata()
            .map(|m| m.file_type().is_file())
            .unwrap_or(false);
        if is_a_database {
            snapshot_brain(&db, &dest.join(BRAIN_DB))
                .await
                .map_err(|e| ExportError::Db(format!("brain of {company_id}: {e}")))?;
            keep_private(&dest.join(BRAIN_DB))?;
            if brain_mode == "none" {
                brain_mode = "snapshot";
            }
        } else {
            brain_mode = "copied";
        }
        brains.push((brain, dest));
    }

    // 3-5. The rest is files, hashes and compression: blocking work, and a
    //      1 GB instance would hold a runtime worker for minutes — with the
    //      socket, the board and the scheduler waiting behind it.
    let job = Assembly {
        staging: staging.to_path_buf(),
        folder: folder.to_path_buf(),
        data_dir: config.data_dir.clone(),
        marker: crate::economy::plan_marker(config),
        brains,
        brain_mode,
        token: token.map(str::to_string),
        passphrase: passphrase.map(str::to_string),
        chain,
        created_at,
    };
    tokio::task::spawn_blocking(move || assemble(job))
        .await
        .map_err(|e| ExportError::Io(std::io::Error::other(e)))?
}

/// Everything the export does with files, in one place so it can be done off
/// the runtime.
struct Assembly {
    staging: PathBuf,
    folder: PathBuf,
    data_dir: PathBuf,
    marker: PathBuf,
    /// `(source brain directory, its place in the staging tree)`.
    brains: Vec<(PathBuf, PathBuf)>,
    brain_mode: &'static str,
    token: Option<String>,
    passphrase: Option<String>,
    chain: ChainSummary,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn assemble(job: Assembly) -> Result<ExportReport, ExportError> {
    let Assembly {
        staging,
        folder,
        data_dir,
        marker,
        brains,
        brain_mode,
        token,
        passphrase,
        chain,
        created_at,
    } = job;

    // The brains' own files, beside the snapshot already taken.
    for (brain, dest) in &brains {
        copy_tree(brain, dest, &|name| {
            !(name == BRAIN_DB
                || name == "brain.db-wal"
                || name == "brain.db-shm"
                || name.ends_with(".lock"))
        })?;
    }

    // Files, as files. `meetings/` is not among them: the room is handed to
    // the agent uid for every turn (`hand_over`) and outlives the meeting, so
    // copying it as the server would be a privileged read of an
    // agent-writable tree — and what a meeting decided is in `meeting_turns`,
    // which rides in the database.
    for name in ["attachments", "artifacts"] {
        let src = data_dir.join(name);
        if src.is_dir() {
            copy_tree(&src, &staging.join(name), &|_| true)?;
        }
    }
    // The marker gets the same treatment as everything else: a link in its
    // place would otherwise be read through, as `pay-with-plan`, by the
    // server.
    if marker
        .symlink_metadata()
        .map(|m| m.file_type().is_file())
        .unwrap_or(false)
    {
        copy_file_nofollow(&marker, &staging.join("pay-with-plan"))?;
    }

    // The token, sealed — never in the clear.
    let scope = SCOPE_INSTANCE.to_string();
    let created_at_text = created_at.to_rfc3339();
    let mut seal = None;
    if let (Some(token), Some(passphrase)) = (&token, &passphrase) {
        let aad = associated_data(FORMAT, &scope, &created_at_text);
        let (params, sealed) = seal_token(token, passphrase, aad.as_bytes())?;
        let path = staging.join(TOKEN_ENTRY);
        if let Some(parent) = path.parent() {
            private_dir(parent)?;
        }
        std::fs::write(&path, sealed)?;
        keep_private(&path)?;
        seal = Some(params);
    }

    // Hash everything, write the manifest, then the archive.
    let mut entries = BTreeMap::new();
    for rel in list_files(&staging)? {
        let hash = hash_file(&staging.join(&rel))?;
        entries.insert(rel, hash);
    }
    let manifest = Manifest {
        format: FORMAT,
        overmind_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: created_at_text,
        scope: scope.clone(),
        brain: brain_mode.to_string(),
        chain: chain.clone(),
        entries: entries.clone(),
        token: seal.clone(),
    };
    let manifest_path = staging.join(MANIFEST_ENTRY);
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    keep_private(&manifest_path)?;

    let (name, file) = reserve_archive(&folder, &scope, &created_at)?;
    let bytes = write_archive(&staging, file, entries.keys())?;

    Ok(ExportReport {
        name,
        bytes,
        scope,
        chain,
        sealed_token: seal.is_some(),
    })
}

/// A file's SHA-256, read through rather than into memory: the staged
/// snapshot is the size of the whole database, and an attachment is whatever
/// somebody uploaded.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// `VACUUM INTO` the live database. The path is quoted for SQL; SQLite has
/// no parameter binding for this statement.
async fn vacuum_into(pool: &sqlx::SqlitePool, dest: &Path) -> Result<(), sqlx::Error> {
    let target = dest.to_string_lossy().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{target}'"))
        .execute(pool)
        .await?;
    Ok(())
}

/// Open the snapshot on its own, scrub the credentials that must not travel,
/// verify its chain and read the chain's head. Closed in rollback-journal
/// mode so no `-wal`/`-shm` sidecar is left beside it.
async fn scrub_and_report(snapshot: &Path) -> Result<ChainSummary, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(snapshot)
        .journal_mode(SqliteJournalMode::Delete)
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    // An UPDATE rewrites the row and leaves the old bytes in the page's free
    // space; a scrub that leaves the credential readable with `strings` is
    // no scrub. Overwrite on delete, then rebuild every page.
    sqlx::query("PRAGMA secure_delete = ON")
        .execute(&pool)
        .await?;
    // A run in flight at export time must not survive as a credential.
    sqlx::query("UPDATE agent_task_sessions SET mcp_token = NULL WHERE mcp_token IS NOT NULL")
        .execute(&pool)
        .await?;
    // A restored instance re-mints its editor tokens; the rows stay so the
    // audit log's names keep pointing at something.
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE company_tokens
            SET token = 'revoked-by-export:' || id,
                revoked_at = COALESCE(revoked_at, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await?;
    sqlx::query("VACUUM").execute(&pool).await?;
    let report = crate::audit::verify(&pool).await?;
    let head: Option<(i64, String)> =
        sqlx::query_as("SELECT seq, hash FROM audit_events ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&pool)
            .await?;
    pool.close().await;
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = snapshot.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(sidecar);
    }
    let (last_seq, last_hash) = match head {
        Some((s, h)) => (Some(s), Some(h)),
        None => (None, None),
    };
    Ok(ChainSummary {
        valid: report.valid,
        events_checked: report.events_checked,
        first_invalid_seq: report.first_invalid_seq,
        last_seq,
        last_hash,
    })
}

/// A brain's database through a read-only connection: Wadachi's own process
/// may hold it open, and WAL makes a concurrent `VACUUM INTO` safe.
async fn snapshot_brain(src: &Path, dest: &Path) -> Result<(), sqlx::Error> {
    let options = SqliteConnectOptions::new().filename(src).read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let result = vacuum_into(&pool, dest).await;
    pool.close().await;
    result
}

fn copy_tree(src: &Path, dest: &Path, keep: &dyn Fn(&str) -> bool) -> std::io::Result<()> {
    private_dir(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if !keep(&name_text) {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to, keep)?;
        } else if kind.is_file() {
            copy_file_nofollow(&from, &to)?;
        }
        // Symlinks and specials are not data of ours.
    }
    Ok(())
}

/// Copy one file without ever following a link.
///
/// `read_dir` says what a name *was*; `std::fs::copy` resolves it again, and
/// between the two anything that can write the directory can put a symlink in
/// the way — the same shape `chown_tree` avoids by using `lchown`. So the
/// source is opened `O_NOFOLLOW`, the *open file* is asked what it is, and the
/// destination is created new and private. A name that turned into a link
/// since the listing is skipped and said out loud: it is either a bug or
/// somebody trying something.
#[cfg(unix)]
fn copy_file_nofollow(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut source = match std::fs::OpenOptions::new()
        .read(true)
        // `O_NOFOLLOW` for the link a writable directory can swap in;
        // `O_NONBLOCK` for the fifo it can swap in instead, which would
        // otherwise block the open forever waiting for a writer. Neither
        // changes anything about reading an ordinary file.
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(from)
    {
        Ok(file) => file,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            eprintln!(
                "backup: {} is a symbolic link, not a file -- not copied",
                from.display()
            );
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if !source.metadata()?.is_file() {
        // A fifo, a socket, a device: not data of ours, and not something to
        // read from during an export.
        return Ok(());
    }
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)?;
    std::io::copy(&mut source, &mut target)?;
    Ok(())
}

#[cfg(not(unix))]
fn copy_file_nofollow(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::copy(from, to).map(|_| ())
}

/// Every regular file under `root`, as archive paths (forward slashes),
/// sorted so the archive is written in a stable order.
fn list_files(root: &Path) -> std::io::Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(std::io::Error::other)?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

/// Pick a free name and *take* it in the same breath: `create_new` is the
/// reservation. Two exports started in the same second would otherwise both
/// find the name free, and the second would lose its whole archive to
/// `AlreadyExists` at the last step.
fn reserve_archive(
    folder: &Path,
    scope: &str,
    at: &chrono::DateTime<chrono::Utc>,
) -> std::io::Result<(String, std::fs::File)> {
    let stamp = at.format("%Y%m%dT%H%M%SZ");
    let base = format!("overmind-{scope}-{stamp}");
    for n in 1..1_000 {
        let name = if n == 1 {
            format!("{base}.tar.gz")
        } else {
            format!("{base}-{n}.tar.gz")
        };
        match open_private(&folder.join(&name)) {
            Ok(file) => return Ok((name, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(
        "a thousand archives share this second's name",
    ))
}

/// Write the archive into the file `reserve_archive` took: the manifest
/// first, then every entry in manifest order.
fn write_archive<'a>(
    staging: &Path,
    file: std::fs::File,
    entries: impl Iterator<Item = &'a String>,
) -> Result<u64, ExportError> {
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.append_path_with_name(staging.join(MANIFEST_ENTRY), MANIFEST_ENTRY)?;
    for rel in entries {
        tar.append_path_with_name(staging.join(rel), rel)?;
    }
    let gz = tar.into_inner()?;
    let mut file = gz.finish()?;
    file.flush()?;
    Ok(file.metadata()?.len())
}

/// A directory the server alone can enter, whatever the umask says.
///
/// `create_dir_all` takes the umask's word for it — 0755 in the image, whose
/// server is root — and the export writes a copy of every company's database
/// and brain into this tree.
fn private_dir(path: &Path) -> std::io::Result<()> {
    crate::sandbox::mkdir_mode(path, 0o700)
}

/// A file the server alone can read. SQLite writes `VACUUM INTO` output 0644,
/// which is why the live database has needed `keep_to_server` since ADR-0029.
fn keep_private(path: &Path) -> std::io::Result<()> {
    crate::sandbox::set_mode(path, 0o600)
}

#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

// ---------------------------------------------------------------------------
// The seal
// ---------------------------------------------------------------------------

/// What the cipher authenticates beside the token: the archive's identity.
/// Not the manifest's bytes — the manifest names this entry's hash, which
/// would make the two circular.
fn associated_data(format: u32, scope: &str, created_at: &str) -> String {
    format!("overmind-backup/{format}/{scope}/{created_at}")
}

fn derive_key(
    passphrase: &str,
    salt: &[u8],
    m_kib: u32,
    t: u32,
    p: u32,
) -> Result<[u8; 32], String> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(m_kib, t, p, Some(32)).map_err(|e| e.to_string())?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

fn seal_token(
    token: &str,
    passphrase: &str,
    aad: &[u8],
) -> Result<(TokenSeal, Vec<u8>), ExportError> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut salt).map_err(|e| ExportError::Seal(e.to_string()))?;
    getrandom::fill(&mut nonce).map_err(|e| ExportError::Seal(e.to_string()))?;
    let key = derive_key(passphrase, &salt, KDF_M_KIB, KDF_T, KDF_P).map_err(ExportError::Seal)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: token.as_bytes(),
                aad,
            },
        )
        .map_err(|_| ExportError::Seal("encryption failed".into()))?;
    Ok((
        TokenSeal {
            kdf: "argon2id".into(),
            m_kib: KDF_M_KIB,
            t: KDF_T,
            p: KDF_P,
            cipher: "xchacha20poly1305".into(),
            salt: hex::encode(salt),
            nonce: hex::encode(nonce),
        },
        sealed,
    ))
}

/// Open a sealed token with the manifest it travelled in. A wrong passphrase
/// and a flipped bit refuse the same way: the cipher authenticates, it does
/// not guess.
pub fn unseal(manifest: &Value, sealed: &[u8], passphrase: &str) -> Result<String, UnsealError> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
    let seal = manifest
        .get("token")
        .filter(|t| !t.is_null())
        .ok_or(UnsealError::NoToken)?;
    let seal: TokenSeal =
        serde_json::from_value(seal.clone()).map_err(|e| UnsealError::Manifest(e.to_string()))?;
    let text = |key: &str| {
        manifest
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| UnsealError::Manifest(format!("missing {key}")))
    };
    let format = manifest
        .get("format")
        .and_then(Value::as_u64)
        .ok_or_else(|| UnsealError::Manifest("missing format".into()))? as u32;
    let aad = associated_data(format, &text("scope")?, &text("created_at")?);
    let salt = hex::decode(&seal.salt).map_err(|e| UnsealError::Manifest(e.to_string()))?;
    let nonce = hex::decode(&seal.nonce).map_err(|e| UnsealError::Manifest(e.to_string()))?;
    if nonce.len() != 24 || seal.cipher != "xchacha20poly1305" || seal.kdf != "argon2id" {
        return Err(UnsealError::Manifest("unknown seal".into()));
    }
    let key =
        derive_key(passphrase, &salt, seal.m_kib, seal.t, seal.p).map_err(UnsealError::Manifest)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
    let opened = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: sealed,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| UnsealError::Refused)?;
    String::from_utf8(opened).map_err(|_| UnsealError::Refused)
}

/// Is this a name the folder could hold? One path component, our own
/// suffix, nothing hidden. The caller still checks it is *in* the folder.
pub fn is_archive_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.ends_with(".tar.gz")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The archives in the folder, newest first: name, size, and when.
pub fn list(config: &Config) -> std::io::Result<Vec<Value>> {
    let folder = backup_dir(config);
    let mut out = Vec::new();
    if !folder.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&folder)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_archive_name(&name) || !entry.file_type()?.is_file() {
            continue;
        }
        let meta = entry.metadata()?;
        let created_at = meta
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
            .map(|t| t.to_rfc3339());
        out.push(json!({ "name": name, "bytes": meta.len(), "created_at": created_at }));
    }
    out.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
    Ok(out)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "overmind-backup-unit-{label}-{}",
            uuid::Uuid::now_v7().simple()
        ));
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn a_staged_directory_is_the_servers_alone_whatever_the_umask_says() {
        let root = scratch("private-dir");
        let staged = root.join("deep").join("staging");
        private_dir(&staged).expect("private dir");
        assert_eq!(mode_of(&staged), 0o700);
        let file = staged.join("snapshot.sqlite");
        std::fs::write(&file, b"x").expect("write");
        keep_private(&file).expect("private file");
        assert_eq!(mode_of(&file), 0o600);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_name_that_became_a_symlink_is_skipped_not_followed() {
        let root = scratch("nofollow");
        let secret = root.join("secret.txt");
        std::fs::write(&secret, b"sk-ant-oat01-the-live-token").expect("secret");
        let src = root.join("src");
        let dest = root.join("dest");
        std::fs::create_dir_all(&src).expect("src");
        std::os::unix::fs::symlink(&secret, src.join("notes.md")).expect("symlink");
        std::fs::write(src.join("real.md"), b"an ordinary file").expect("real file");

        // What `read_dir` reports as a link never reaches the copy; and even
        // handed the path directly, the copy refuses to follow it.
        copy_tree(&src, &dest, &|_| true).expect("copy tree");
        assert!(!dest.join("notes.md").exists(), "a link was copied");
        assert_eq!(
            std::fs::read(dest.join("real.md")).expect("real file"),
            b"an ordinary file"
        );
        copy_file_nofollow(&src.join("notes.md"), &dest.join("smuggled.md"))
            .expect("skipped, not an error");
        assert!(!dest.join("smuggled.md").exists(), "the link was followed");
        let _ = std::fs::remove_dir_all(&root);
    }
}
