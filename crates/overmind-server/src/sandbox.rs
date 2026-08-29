//! OS-level sandboxing of agent runs (ADR-0023).
//!
//! Until M10 an agent ran as the user, in a `sh -c`, with the whole machine
//! reachable: `~/.ssh`, the browser profile, and — pointedly — Overmind's own
//! source and its `overmind.sqlite`, audit chain included. Since M14 the
//! declared capabilities (`repo:write`, `web:read`, …) have been honest about
//! not being policed, and M17 made the surface much larger by giving agents
//! arbitrary file I/O.
//!
//! The threat model is not an external attacker — Overmind runs on one person's
//! machine, and anyone holding it can run the CLI directly. It is an agent that
//! misreads its task, and a **prompt injection** arriving inside material the
//! user handed it. Both are accidents of capability, and capability is what
//! this removes.
//!
//! **Deny by default.** The alternative — allow everything, forbid a list of
//! known-sensitive places — protects exactly the places someone thought of, and
//! ADR-0005 already rejected that shape of reasoning under the name "security
//! by prayer". Deny-by-default fails in the useful direction: when the profile
//! is wrong the agent does not start, loudly, rather than quietly having more
//! reach than intended.
//!
//! **What this does not do:** it cannot close the network — the agent's whole
//! job is to reach the API — so anything reachable with the user's ambient
//! credentials stays reachable. Git credential isolation is a separate slice,
//! and until it lands "push to main" is not stopped here.
//!
//! # More than one mechanism (ADR-0029)
//!
//! ADR-0023 named `bubblewrap` as the natural counterpart when Linux arrived.
//! Measured, it is not: Docker's default seccomp profile denies it a user
//! namespace, and buying one back costs the user a `--cap-add SYS_ADMIN` —
//! weakening their container to gain a security feature. Landlock, the other
//! candidate, is absent from Docker Desktop's kernel, which is how everyone on
//! macOS and Windows has Overmind at all.
//!
//! So the cage is a **set**, chosen by what the platform offers rather than by
//! what it is called: `sandbox-exec` on macOS, and in our own image an agent
//! that runs as its own unprivileged uid, below the server, which needs no
//! kernel feature and therefore works wherever the image does. [`caged`] asks
//! the whole set, because what earns the permission flag is *a* real boundary,
//! not any particular one.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::db::Config;

/// Where a caged run may write, beyond the system paths every process needs.
pub struct Cage<'a> {
    /// The run's own directory: the git worktree for a `code` task, the scratch
    /// dir for a `knowledge` task or a conversational turn.
    pub run_dir: &'a Path,
}

/// The unprivileged user agent work runs as, when Overmind is privileged enough
/// to drop to one (ADR-0029).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentUser {
    pub uid: u32,
    pub gid: u32,
}

/// What is actually holding this run.
///
/// Not *whether* something is: mechanisms are chosen by what the platform
/// offers, and in our image on Linux two of them apply at once. Rendering the
/// macOS profile is part of choosing it — a profile that cannot be expressed is
/// a mechanism that is not available — so it is carried here rather than built
/// twice.
#[derive(Debug, Clone, Default)]
pub struct Confinement {
    /// macOS: the deny-by-default `sandbox-exec` profile (ADR-0023).
    pub profile: Option<String>,
    /// The image: agent work runs below the server, as its own uid (ADR-0029).
    pub agent_user: Option<AgentUser>,
    /// Linux with a kernel that has it: confinement to the run's own directory,
    /// which the uid alone does not give (ADR-0029).
    ///
    /// Built here rather than at the spawn for the same reason the macOS
    /// profile is: building it *is* how we learn whether it can be had, and a
    /// mechanism promised by one function and found impossible by the next is
    /// how an agent ends up holding the permission flag without a cage. Shared
    /// rather than cloned — a ruleset is a file descriptor, not a value.
    #[cfg(target_os = "linux")]
    pub landlock: Option<std::sync::Arc<crate::landlock::Ruleset>>,
}

impl Confinement {
    /// Is a real boundary in place?
    ///
    /// Asked of the whole set on purpose. What the permission flag rides on is
    /// that the operating system is holding the process somehow, and a caller
    /// that asked about one named mechanism would answer "no cage" on a
    /// platform that has a different one.
    pub fn is_real(&self) -> bool {
        if self.profile.is_some() || self.agent_user.is_some() {
            return true;
        }
        #[cfg(target_os = "linux")]
        {
            return self.landlock.is_some();
        }
        #[allow(unreachable_code)]
        false
    }
}

/// What a run may write, beyond the system paths every process needs.
///
/// Shared by both mechanisms that need the answer, because two lists would
/// eventually disagree and the disagreement would be a cage that grants
/// different things depending on which kernel you are on.
fn writable_paths(config: &Config, cage: &Cage<'_>) -> Vec<PathBuf> {
    let mut writable: Vec<PathBuf> = Vec::new();
    writable.extend(real_path(cage.run_dir));
    writable.extend(real_path(&std::env::temp_dir()));
    // Where the adapter keeps credentials and state. In the image this is the
    // agent's own home; on a machine it is the user's, and the run would fail
    // without it long before it failed usefully.
    writable.extend(config.agent_home.iter().filter_map(|p| real_path(p)));
    // Mounted repositories. Writable, not readable: a `code` run commits, and a
    // worktree keeps its git metadata in `<repo>/.git/worktrees/<name>` — inside
    // the repository, not inside the run directory. Granting the run directory
    // alone would give an agent a checkout it could edit and never commit.
    writable.extend(config.repos_dir.iter().filter_map(|p| real_path(p)));
    writable.extend(adapter_paths().iter().filter_map(|p| real_path(p)));
    writable.extend(config.sandbox_allow.iter().filter_map(|p| real_path(p)));
    writable
}

/// What agent runs will get, for the line the server prints at startup.
///
/// ADR-0023 promised the server would *say* when it cannot protect rather than
/// pretend to; with more than one mechanism the useful sentence is which
/// boundary you got, not merely that you got one. Asked without a run
/// directory, so the macOS profile is reported as available rather than
/// rendered — whether a particular profile can be expressed is a question with
/// a run in it.
pub fn announce(config: &Config) -> String {
    if !config.sandbox {
        return "off by configuration (OVERMIND_SANDBOX=off) — agents are read-only".to_string();
    }
    let mut held = Vec::new();
    if profile_available() {
        held.push("sandbox-exec profile".to_string());
    }
    if let Some(u) = agent_user(config) {
        held.push(format!("unprivileged uid {}", u.uid));
    }
    #[cfg(target_os = "linux")]
    if let Some(abi) = crate::landlock::abi() {
        held.push(format!("Landlock (ABI {abi})"));
    }
    if held.is_empty() {
        // Naming the likeliest cause: the image sets the uid, so seeing this
        // inside a container means the server is not privileged enough to drop
        // to it — a `user:` override in compose is how that happens.
        return match config.agent_uid {
            Some(_) => "none — an agent uid is configured but this process cannot drop to it; agents are read-only".to_string(),
            None => "none on this platform — agents are read-only".to_string(),
        };
    }
    held.join(" + ")
}

/// Is the macOS mechanism available? Used by its own tests, which have nothing
/// to say on a platform without it.
pub fn profile_available() -> bool {
    cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").exists()
}

/// Can this process change uid at all?
///
/// The honest form of the question. ADR-0029 chose a root PID 1 that drops
/// privilege per spawn, so this is `euid == 0`; a build that instead carried
/// `CAP_SETUID` would answer here too, and nothing above would change.
fn can_drop_privilege() -> bool {
    #[cfg(unix)]
    {
        // Reading our own effective uid: no state is touched and no pointer is
        // dereferenced, which is the whole of the unsafety.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// The uid to hand agent work to, if this is a place where that means anything.
///
/// `None` outside our image is the expected answer, not a failure: on a user's
/// own machine Overmind runs as that user, and inventing a second uid on
/// somebody's laptop is not ours to do.
fn agent_user(config: &Config) -> Option<AgentUser> {
    let uid = config.agent_uid?;
    // Root is not an unprivileged uid. It would also be a cage that reads as
    // one and is not one: the adapter refuses to skip permissions as root, so
    // the flag this predicate grants would be rejected by the process it was
    // granted for.
    if uid == 0 || !can_drop_privilege() {
        return None;
    }
    Some(AgentUser {
        uid,
        // What `useradd` produces when it makes a user's own group.
        gid: config.agent_gid.unwrap_or(uid),
    })
}

/// Which mechanisms hold this run.
pub fn confinement(config: &Config, cage: &Cage<'_>) -> Confinement {
    if !config.sandbox {
        return Confinement::default();
    }
    Confinement {
        profile: if profile_available() {
            profile(config, cage)
        } else {
            None
        },
        agent_user: agent_user(config),
        #[cfg(target_os = "linux")]
        landlock: build_landlock(config, cage),
    }
}

/// The Landlock ruleset for this run, if this kernel has Landlock at all.
///
/// A failure is this layer being unavailable, never this layer being smaller
/// than advertised — so it is reported and dropped rather than narrowed. The
/// run is not left uncaged by that: in the image the uid is still holding it,
/// and where nothing is, [`Confinement::is_real`] answers false and the agent
/// is read-only.
#[cfg(target_os = "linux")]
fn build_landlock(
    config: &Config,
    cage: &Cage<'_>,
) -> Option<std::sync::Arc<crate::landlock::Ruleset>> {
    crate::landlock::abi()?;
    // Enough of the system to exist: the loader, the shells, the toolchain —
    // the same shape as the macOS profile's read grants, spelled for Linux.
    // `/dev` is in the writable set because `/dev/null` is, and a run that
    // cannot open it fails in ways nobody enjoys reading.
    let readable: Vec<&Path> = [
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/opt", "/proc", "/run",
    ]
    .iter()
    .map(Path::new)
    .collect();
    let mut writable = writable_paths(config, cage);
    writable.push(PathBuf::from("/dev"));

    match crate::landlock::build(&writable, &readable) {
        Ok(rules) => Some(std::sync::Arc::new(rules)),
        Err(e) => {
            eprintln!("landlock unavailable for this run (ignored): {e}");
            None
        }
    }
}

/// Quote a path for a sandbox profile literal.
///
/// Profile paths are double-quoted strings; a backslash or a quote in a path
/// would otherwise end the literal early and change what the rule means. Paths
/// containing a newline cannot be expressed at all — the caller refuses to cage
/// those rather than emit a rule that silently means something else.
fn quote(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    if s.contains('\n') {
        return None;
    }
    Some(s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Paths the adapter itself needs, which are an installation fact rather than
/// something we get to decide. Defaults suit a standard Claude Code install;
/// `OVERMIND_SANDBOX_ALLOW` covers anything else.
fn adapter_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        // Where the binary lives (a symlink into `.local/share` on a current
        // install, so the whole tree is granted rather than the link alone).
        home.join(".local"),
        // Where it keeps credentials, sessions and settings.
        home.join(".claude"),
        home.join(".claude.json"),
        // Shared tool configuration a coding task legitimately reads.
        home.join(".config"),
    ]
}

/// Paths the adapter needs to *read* for its own sign-in, never to write (M23).
///
/// A subscription's OAuth token lives in the login Keychain on macOS --
/// `~/Library/Keychains` -- not under `~/.claude`, which is why the cage
/// passed every key-authenticated run (the key rides the environment) and
/// silently killed every subscription one: exit 1, stderr empty, measured
/// live the day the owner asked whether his plan works. Keychain items stay
/// encrypted and per-item ACLs are enforced by securityd either way; this
/// grants the file, not the secrets.
fn adapter_read_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        home.join("Library/Keychains"),
        home.join("Library/Preferences"),
    ]
}

/// A path as the sandbox will match it: absolute, with symlinks and `..`
/// resolved.
///
/// A profile matches the **real** path of a file, and matches it literally.
/// That makes a relative path the worst kind of mistake here: it is a perfectly
/// good string, so the rule is accepted and grants nothing at all. The failure
/// then lands a long way from its cause — the agent is denied its own working
/// directory, the shell's `getcwd` cannot walk up out of it, and the CLI dies
/// with `EPERM` before it has read a word of the prompt.
///
/// This is not hypothetical: `data_dir` defaults to `./overmind-data`, so every
/// caged run under the default configuration was denied its own run directory,
/// and only ever worked because every test and every earlier live run happened
/// to pass an absolute `OVERMIND_DATA_DIR`. Symlinked ancestors do the same
/// thing more quietly — on macOS `$TMPDIR` lives under `/var`, which is a
/// symlink to `/private/var`, and that one line of canonicalisation is why the
/// tests passed over the defect for a month.
fn real_path(path: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(path).ok()?;
    // Canonicalising needs the path to exist. Anything granted ahead of time —
    // a `sandbox_allow` entry, `~/.claude.json` on a fresh install — keeps its
    // absolute form, which is wrong only if a symlink is in its way.
    Some(abs.canonicalize().unwrap_or(abs))
}

/// The profile text for one run.
fn profile(config: &Config, cage: &Cage<'_>) -> Option<String> {
    // No real path for the run directory means no profile, and therefore no
    // cage and no `--dangerously-skip-permissions` — the same safe direction
    // an unquotable path takes below.
    real_path(cage.run_dir)?;
    // One list, shared with Landlock: two would eventually disagree, and the
    // disagreement would be a cage that grants different things depending on
    // which kernel you happen to be on.
    let mut writable = writable_paths(config, cage);
    // Temp: compilers, package managers and the test stubs all expect it. This
    // session's TMPDIR is already in the shared list, resolved through the
    // /var -> /private symlink because sandbox rules match the real path —
    // granting all of `/private/var/folders` would hand over every per-user
    // cache on the machine to buy the same thing. `/private/tmp` is the macOS
    // spelling of the other one, and belongs only here.
    writable.push(PathBuf::from("/private/tmp"));

    let mut allow_read = String::new();
    for p in adapter_read_paths() {
        if let Some(rp) = real_path(&p)
            && let Some(q) = quote(&rp)
        {
            allow_read.push_str(&format!("  (subpath \"{q}\")\n"));
        }
    }

    let mut allow_write = String::new();
    for p in &writable {
        // A path we cannot express is a path we do not grant. The run may fail
        // for lack of it, which is the loud failure this design prefers.
        if let Some(q) = quote(p) {
            allow_write.push_str(&format!("  (subpath \"{q}\")\n"));
        }
    }

    Some(format!(
        r#"(version 1)
(import "/System/Library/Sandbox/Profiles/bsd.sb")
(deny default)
(allow process-exec process-fork signal)
(allow file-read-metadata)

; Enough of the system to exist: the loader, the shells, the toolchain.
(allow file-read*
  (subpath "/usr") (subpath "/bin") (subpath "/sbin") (subpath "/System")
  (subpath "/Library") (subpath "/opt") (subpath "/private/etc")
  (subpath "/private/var") (subpath "/dev") (subpath "/Applications"))

; The adapter's own sign-in, readable and never writable: a subscription's
; token lives in the login Keychain, and a cage that starves the adapter of
; its credential kills the run before the first word (M23).
(allow file-read*
{allow_read})

; The run's own directory, temp, and whatever the adapter needs to exist.
(allow file*
{allow_write})

; The network stays open: reaching the API is the job. This is the boundary
; this profile explicitly does not draw (ADR-0023).
(allow network* system-socket)
(allow sysctl-read mach-lookup ipc-posix-shm iokit-open)
"#
    ))
}

/// Whether this run will *actually* be caged.
///
/// The same question [`command`] answers, exposed because how much rope the
/// adapter gets depends on the answer: inside the cage the agent may work
/// freely in its run directory, outside it may not. Two predicates that could
/// drift apart would eventually give an uncaged agent the caged agent's
/// freedom, which is the one combination that must never happen — so there is
/// one predicate, and both callers ask it.
pub fn caged(config: &Config, cage: &Cage<'_>) -> bool {
    confinement(config, cage).is_real()
}

/// Build the command that runs `script`, caged when we can cage it.
///
/// Falls back to a bare `sh -c` when sandboxing is off, unavailable, or the
/// profile cannot be expressed — and those are the only three cases, each of
/// them a decision rather than an accident.
/// A subscription's long-lived token rides the environment, like a key.
///
/// Injected wherever the adapter runs (caged work, probes): the CLI reads
/// `CLAUDE_CODE_OAUTH_TOKEN`, and the file it comes from is the server's,
/// 0600 under the data dir (M23). An explicit variable already in the
/// environment wins -- the operator outranks the stored token.
fn inject_oauth_token<C: CommandEnv>(config: &Config, cmd: &mut C) {
    if std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_none()
        && let Some(tok) = crate::claude_auth::stored_token(config)
    {
        cmd.set_env("CLAUDE_CODE_OAUTH_TOKEN", &tok);
    }
}

/// Who pays (ADR-0037): while the person has chosen the plan, the key stays
/// out of the agent's environment. The server never calls the API itself, so
/// it loses nothing; the CLI, with nothing overriding its login, bills the
/// plan. Applied to every command that runs *as the agent* — the probe, the
/// caged run, the blocking sign-in — because a single path that still
/// carried the key would be a single path that still billed it.
fn keep_key_away<C: CommandEnv>(config: &Config, cmd: &mut C) {
    if crate::economy::plan_is_preferred(config) {
        cmd.remove_env("ANTHROPIC_API_KEY");
    }
}

/// Everything a command needs to run with the agent's credentials and not one
/// credential more.
fn credentials<C: CommandEnv>(config: &Config, cmd: &mut C) {
    inject_oauth_token(config, cmd);
    keep_key_away(config, cmd);
}

/// The two Command types, one env call. A trait beats duplicating the
/// injection rule until the copies disagree.
trait CommandEnv {
    fn set_env(&mut self, k: &str, v: &str);
    fn remove_env(&mut self, k: &str);
}
impl CommandEnv for Command {
    fn set_env(&mut self, k: &str, v: &str) {
        self.env(k, v);
    }
    fn remove_env(&mut self, k: &str) {
        self.env_remove(k);
    }
}
impl CommandEnv for std::process::Command {
    fn set_env(&mut self, k: &str, v: &str) {
        self.env(k, v);
    }
    fn remove_env(&mut self, k: &str) {
        self.env_remove(k);
    }
}

pub fn command(config: &Config, cage: &Cage<'_>, script: &str) -> Command {
    let held = confinement(config, cage);
    let mut cmd = match &held.profile {
        Some(text) => {
            let mut cmd = Command::new("/usr/bin/sandbox-exec");
            cmd.arg("-p").arg(text).arg("/bin/sh").arg("-c").arg(script);
            cmd
        }
        None => {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(script);
            cmd
        }
    };
    if let Some(user) = held.agent_user {
        drop_to(&mut cmd, config, user);
    }
    #[cfg(target_os = "linux")]
    if let Some(rules) = held.landlock {
        // The child restricts *itself*, which is what Landlock is: no helper
        // process, no wrapper binary, nothing to install. `pre_exec` runs after
        // the uid change and before `exec`, which is exactly right — the
        // restriction needs no privilege, and applying it before `exec` is what
        // makes it cover the adapter and everything the adapter starts.
        //
        // The closure owns a share of the ruleset, so the descriptor outlives
        // the builder and closes when the command is done with it.
        unsafe {
            cmd.pre_exec(move || crate::landlock::restrict(rules.as_raw_fd()));
        }
    }
    credentials(config, &mut cmd);
    cmd
}

/// Run the child as the agent's uid instead of the server's (ADR-0029).
///
/// `uid`/`gid` are enough, and the reason is worth knowing rather than
/// rediscovering: when a uid is set and no explicit group list was given, the
/// standard library calls `setgroups(0, NULL)` before `setuid`, in that order.
/// So the child does not keep the server's supplementary groups — which would
/// be a boundary with a hole in it, and the hole would be invisible.
///
/// Ordering also rules out `pre_exec` for this: those closures run *after* the
/// uid change, when the process is no longer privileged enough to make it.
#[cfg(unix)]
fn drop_to(cmd: &mut Command, config: &Config, user: AgentUser) {
    cmd.uid(user.uid).gid(user.gid);
    // The adapter CLI keeps credentials and session state in `$HOME`, and the
    // server's home is not the agent's — it may not even be readable now.
    if let Some(home) = &config.agent_home {
        cmd.env("HOME", home);
    }
}

#[cfg(not(unix))]
fn drop_to(_cmd: &mut Command, _config: &Config, _user: AgentUser) {
    // `agent_user` never answers `Some` here — `can_drop_privilege` is false —
    // so this exists to keep the module compiling, not to be called.
}

/// A command that runs *as the agent does* — same uid, same `HOME` — and is not
/// caged.
///
/// For asking questions about the agent's own environment (ADR-0030). In the
/// image the server is root and the agent's credentials are not root's, so a
/// probe run as the server answers confidently about the wrong home directory:
/// it would report "not signed in" for a perfectly well signed-in agent, and
/// Overmind would then believe it was in an economy nobody is in.
///
/// No cage, on the same reasoning [`command`] applies to `git` and the memory
/// server: this is our own program with our own arguments, not agent-controlled
/// work. The uid is not a cage here either — it is how we ask the question from
/// the right place.
pub fn as_agent(config: &Config, program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(user) = agent_user(config) {
        drop_to(&mut cmd, config, user);
    }
    credentials(config, &mut cmd);
    cmd
}

/// [`as_agent`], but for `std::process::Command` (M23).
///
/// The sign-in flow needs a *blocking* child wired to a pty, which is
/// `std::process` territory; everything `as_agent` promises -- same uid, same
/// `HOME` -- holds here for the same reasons.
pub fn as_agent_std(config: &Config, program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(unix)]
    if let Some(user) = agent_user(config) {
        use std::os::unix::process::CommandExt;
        cmd.uid(user.uid).gid(user.gid);
        if let Some(home) = &config.agent_home {
            cmd.env("HOME", home);
        }
    }
    credentials(config, &mut cmd);
    cmd
}

/// Hand a path to the agent's uid, so the agent can work in it.
///
/// A no-op when there is no agent uid, which is every platform but our image.
/// Where there is one, this is what makes the boundary usable rather than
/// merely present: the server creates the run directory, places the inputs and
/// writes the run's MCP token, and none of it is the agent's until this runs.
///
/// **Failure here must be loud.** A run whose directory was never handed over
/// is a run that cannot write a single file, and M19 exists because that
/// failure used to arrive as a success with an empty deliverable.
pub async fn hand_over(config: &Config, path: &Path) -> std::io::Result<()> {
    let Some(user) = agent_user(config) else {
        return Ok(());
    };
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || chown_tree(&path, user))
        .await
        .map_err(std::io::Error::other)?
}

#[cfg(unix)]
fn chown_tree(path: &Path, user: AgentUser) -> std::io::Result<()> {
    // Links are changed as links and never followed. A run directory is the
    // agent's, and on a retry it already holds whatever the previous attempt
    // put there — following a symlink out of it would point this chown at
    // anything the server can reach.
    std::os::unix::fs::lchown(path, Some(user.uid), Some(user.gid))?;
    if path.symlink_metadata()?.is_dir() {
        for entry in std::fs::read_dir(path)? {
            chown_tree(&entry?.path(), user)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn chown_tree(_path: &Path, _user: AgentUser) -> std::io::Result<()> {
    Ok(())
}

/// The data directory's own layout, which is what makes the uid a boundary
/// rather than a label (ADR-0029).
///
/// Two kinds of directory, and the difference between them *is* the boundary:
///
/// - What belongs to Overmind — every company's brain, the collected artifacts,
///   the files people attached — is `0700` and stays the server's. The agent
///   gets its inputs copied into its run directory; it never reads the shelf
///   they came from.
/// - What merely *holds* run directories is `0711`: an agent enters its own run
///   by the exact path it was handed and cannot list its siblings. Not a
///   boundary between runs — they share a uid, and that is Landlock's half of
///   ADR-0029 — but enumeration is not the same as access, and a directory
///   listing is how you find something to try.
///
/// The data directory itself is `0755`: it holds these names and nothing else,
/// and the agent has to walk through it to reach its own run.
///
/// Applied only where an agent uid is configured. On a user's own machine
/// Overmind runs as that user and there is no second party to keep out;
/// quietly re-permissioning somebody's directory is not this function's call.
pub async fn lay_out_data_dir(config: &Config) -> std::io::Result<()> {
    let Some(user) = agent_user(config) else {
        return Ok(());
    };
    let root = config.data_dir.clone();
    let home = config.agent_home.clone();
    tokio::task::spawn_blocking(move || {
        mkdir_mode(&root, 0o755)?;
        // Overmind's own shelves.
        for name in ["companies", "artifacts", "attachments", "backups"] {
            mkdir_mode(&root.join(name), 0o700)?;
        }
        // Traversable, not listable: these hold the runs.
        for name in ["sessions", "worktrees", "chat", "meetings"] {
            mkdir_mode(&root.join(name), 0o711)?;
        }
        // The agent's home is the agent's. A named volume mounted there — which
        // is how credentials survive a rebuild — arrives owned by root and empty,
        // so without this the adapter CLI cannot write the session it just
        // authenticated, and the failure surfaces as a login that never sticks.
        //
        // Only the home itself and the CLI's own directory: whatever the agent
        // has already put in there is its own, and re-owning a tree on every
        // boot would be a slow way to say nothing.
        if let Some(home) = home {
            mkdir_mode(&home, 0o700)?;
            chown_one(&home, user)?;
            let cli_state = home.join(".claude");
            mkdir_mode(&cli_state, 0o700)?;
            chown_one(&cli_state, user)?;
        }
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

#[cfg(unix)]
fn chown_one(path: &Path, user: AgentUser) -> std::io::Result<()> {
    std::os::unix::fs::lchown(path, Some(user.uid), Some(user.gid))
}

#[cfg(not(unix))]
fn chown_one(_path: &Path, _user: AgentUser) -> std::io::Result<()> {
    Ok(())
}

/// Create a directory if absent and set its mode either way.
///
/// Either way matters: these directories outlive an upgrade, and a data
/// directory built before this function existed would otherwise keep whatever
/// the umask gave it — a boundary that protects only fresh installations is one
/// nobody can reason about.
fn mkdir_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    set_mode(path, mode)
}

/// Keep a file to the server alone (`0600`).
///
/// For the database: it holds the audit chain and every company's data, and
/// SQLite creates it `0644`. Without this an agent on another uid could read
/// all of it — not write, which the chain would catch, but read, which nothing
/// would.
pub fn keep_to_server(config: &Config, path: &Path) -> std::io::Result<()> {
    if agent_user(config).is_none() || !path.exists() {
        return Ok(());
    }
    set_mode(path, 0o600)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// Environment that isolates an agent's git from the user's credentials
/// (ADR-0023, slice 2).
///
/// The cage alone does not settle this. It denies `~/.ssh` and the keychain,
/// which happens to stop a push — but it stops git *entirely*, because git
/// reads `~/.gitconfig` before it does anything at all and a denial there is
/// fatal. An agent on a `code` task could not run `git status`. Breaking the
/// tool is not the same as securing it.
///
/// So git is given its own configuration instead of the user's:
///
/// - `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM` point at `/dev/null`, so
///   `~/.gitconfig` is neither read nor needed — git works again.
/// - `credential.helper` is reset to empty through `GIT_CONFIG_COUNT`, which
///   git applies at command-line precedence. That matters: a *repository* can
///   configure its own helper in `.git/config`, and the run directory is
///   writable, so an agent could otherwise configure one for itself. An empty
///   value resets the list rather than adding to it.
/// - No prompt, no askpass, and no ssh transport at all.
///
/// Measured, not assumed: outside any sandbox, the same push against a
/// nonexistent repository answers `Repository not found` without this (git
/// authenticated fine) and `could not read Username` with it. The two layers
/// are genuinely independent.
///
/// What stays possible on purpose: everything local — status, diff, log,
/// commit in the worktree — and anonymous fetches over HTTPS. Removing
/// credentials is not the same as removing the network, and read-only access
/// to public code is a legitimate part of the job.
pub fn git_isolation() -> Vec<(&'static str, String)> {
    vec![
        // The user's git identity and settings are not the agent's.
        ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
        ("GIT_CONFIG_SYSTEM", "/dev/null".into()),
        // Never block a run waiting for a password nobody will type.
        ("GIT_TERMINAL_PROMPT", "0".into()),
        ("GIT_ASKPASS", "/usr/bin/false".into()),
        ("SSH_ASKPASS", "/usr/bin/false".into()),
        // No ssh transport: the keys are denied by the cage anyway, and this
        // turns a confusing host-key error into a plain refusal.
        ("GIT_SSH_COMMAND", "/usr/bin/false".into()),
        // Command-line precedence, so a repository cannot out-configure it.
        ("GIT_CONFIG_COUNT", "3".into()),
        ("GIT_CONFIG_KEY_0", "user.name".into()),
        ("GIT_CONFIG_VALUE_0", "Overmind agent".into()),
        ("GIT_CONFIG_KEY_1", "user.email".into()),
        ("GIT_CONFIG_VALUE_1", "agent@overmind.local".into()),
        ("GIT_CONFIG_KEY_2", "credential.helper".into()),
        ("GIT_CONFIG_VALUE_2", String::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(allow: Vec<PathBuf>) -> Config {
        Config {
            sandbox: true,
            sandbox_allow: allow,
            ..Config::default()
        }
    }

    #[test]
    fn the_run_directory_is_writable_and_nothing_else_is_named() {
        let cfg = config(Vec::new());
        let dir = PathBuf::from("/tmp/run-me");
        let text = profile(&cfg, &Cage { run_dir: &dir }).expect("profile");
        assert!(text.contains("(deny default)"), "{text}");
        assert!(text.contains("(subpath \"/tmp/run-me\")"), "{text}");
        // The home itself is never granted wholesale — only the adapter's own
        // corners of it, which is the difference between a cage and a gesture.
        assert!(!text.contains("(subpath \"/Users\")"), "{text}");
        // The adapter's sign-in is readable and never writable (M23): the
        // keychain grant must sit in the read-only stanza, and must never
        // migrate into the `file*` one -- an agent that can rewrite the login
        // keychain is a different threat model, not a wider grant.
        let read_stanza = text
            .split("(allow file-read*\n")
            .nth(2)
            .expect("the credential read stanza exists");
        let write_stanza = text.split("(allow file*\n").nth(1).expect("write stanza");
        if let Some(home) = std::env::var_os("HOME") {
            let kc = format!("{}/Library/Keychains", home.to_string_lossy());
            if std::path::Path::new(&kc).exists() {
                assert!(read_stanza.contains("Library/Keychains"), "{text}");
                assert!(!write_stanza.contains("Library/Keychains"), "{text}");
            }
        }
    }

    #[test]
    fn extra_paths_are_granted_and_quoted() {
        let cfg = config(vec![PathBuf::from("/data/with \"quotes\"")]);
        let text = profile(
            &cfg,
            &Cage {
                run_dir: Path::new("/tmp/x"),
            },
        )
        .expect("profile");
        assert!(
            text.contains(r#"(subpath "/data/with \"quotes\"")"#),
            "{text}"
        );
    }

    #[test]
    fn a_path_we_cannot_express_is_a_path_we_do_not_grant() {
        // A newline would end the profile line and turn the rest into
        // something else entirely; the path is dropped instead.
        let cfg = config(vec![PathBuf::from("/data/two\nlines")]);
        let text = profile(
            &cfg,
            &Cage {
                run_dir: Path::new("/tmp/x"),
            },
        )
        .expect("profile");
        assert!(!text.contains("two\nlines"), "{text}");
    }

    #[test]
    fn git_gets_its_own_identity_and_no_credentials() {
        let env: std::collections::HashMap<_, _> = git_isolation().into_iter().collect();
        assert_eq!(
            env.get("GIT_CONFIG_GLOBAL").map(String::as_str),
            Some("/dev/null")
        );
        assert_eq!(
            env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
            Some("0")
        );
        // The empty value is the whole point: it resets the helper list rather
        // than appending to it, and it must survive being put in a map.
        assert_eq!(
            env.get("GIT_CONFIG_KEY_2").map(String::as_str),
            Some("credential.helper")
        );
        assert_eq!(env.get("GIT_CONFIG_VALUE_2").map(String::as_str), Some(""));
    }

    /// Root is not an unprivileged uid, whoever we are when we ask.
    ///
    /// It would also be a cage that grants a flag the adapter then refuses:
    /// the CLI will not skip permissions as root, so `caged` would be true and
    /// every run would die at the spawn.
    #[test]
    fn root_is_never_the_agent_uid() {
        let cfg = Config {
            agent_uid: Some(0),
            ..config(Vec::new())
        };
        assert_eq!(agent_user(&cfg), None);
    }

    /// A configured uid is a boundary only if we can actually drop to it.
    ///
    /// Both directions are asserted because the answer depends on who is
    /// running the suite: a developer's shell cannot drop privilege, a test run
    /// inside a container as root can, and the contract has to hold either way.
    #[test]
    fn an_agent_uid_only_counts_when_we_can_drop_to_it() {
        let cfg = Config {
            agent_uid: Some(10_001),
            ..config(Vec::new())
        };
        if can_drop_privilege() {
            assert_eq!(
                agent_user(&cfg),
                Some(AgentUser {
                    uid: 10_001,
                    // No gid configured: a user's own group, as `useradd` makes it.
                    gid: 10_001
                })
            );
        } else {
            assert_eq!(
                agent_user(&cfg),
                None,
                "an unprivileged server cannot hand work to another uid, and \
                 must not report that it did"
            );
        }
    }

    /// The escape hatch turns off *every* mechanism, not just the one someone
    /// had in mind when they wrote it.
    #[test]
    fn turning_the_cage_off_leaves_no_mechanism_at_all() {
        let cfg = Config {
            sandbox: false,
            agent_uid: Some(10_001),
            ..Config::default()
        };
        let held = confinement(
            &cfg,
            &Cage {
                run_dir: Path::new("/tmp/x"),
            },
        );
        assert!(held.profile.is_none());
        assert!(held.agent_user.is_none());
        #[cfg(target_os = "linux")]
        assert!(
            held.landlock.is_none(),
            "the escape hatch must reach the kernel layer too, not only the \
             two a reader happens to remember"
        );
        assert!(!held.is_real());
    }

    /// Nothing to hand over is not a failure — it is every platform but the
    /// image, and the run must proceed exactly as it did before.
    #[tokio::test]
    async fn handing_over_without_an_agent_uid_is_a_no_op() {
        let dir = std::env::temp_dir().join(format!("overmind-ho-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cfg = Config {
            agent_uid: None,
            ..Config::default()
        };
        assert!(hand_over(&cfg, &dir).await.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The startup line has to name the mechanism, because "the cage is on" was
    /// true on the day no run had ever changed a file.
    #[test]
    fn the_announcement_names_what_is_holding_the_agent() {
        let off = Config {
            sandbox: false,
            ..Config::default()
        };
        assert!(announce(&off).contains("read-only"), "{}", announce(&off));

        // Either the line names a mechanism or it says plainly that nothing is
        // holding the agent — never both, never neither. Asserted as that
        // invariant rather than by re-deriving which mechanism this machine
        // happens to have, which would only restate the function back to
        // itself. The first version of this test did re-derive it, assumed
        // "not macOS" meant "nothing", and failed the moment it met a Linux
        // kernel with Landlock — which is the answer it should have welcomed.
        let said = announce(&Config::default());
        let names_a_mechanism = said.contains("sandbox-exec") || said.contains("Landlock");
        assert_ne!(
            names_a_mechanism,
            said.contains("read-only"),
            "the startup line must name the boundary or say there is none: {said}"
        );
        if profile_available() {
            assert!(said.contains("sandbox-exec"), "{said}");
        }
    }

    #[test]
    fn turning_it_off_gives_a_plain_shell() {
        let cfg = Config {
            sandbox: false,
            ..Config::default()
        };
        let cmd = command(
            &cfg,
            &Cage {
                run_dir: Path::new("/tmp/x"),
            },
            "echo hi",
        );
        assert_eq!(cmd.as_std().get_program(), "sh");
    }

    fn fresh_data_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "overmind-sandbox-{}",
            uuid::Uuid::now_v7().simple()
        ));
        std::fs::create_dir_all(&dir).expect("data dir");
        dir
    }

    fn key_is_removed(cmd: &std::process::Command) -> bool {
        cmd.get_envs()
            .any(|(k, v)| k == "ANTHROPIC_API_KEY" && v.is_none())
    }

    /// ADR-0037: once the person chose the plan, every command that runs as
    /// the agent — the probe, the caged run, the blocking sign-in — starts
    /// without the key. The choice is a file in the data dir, read per spawn
    /// like the stored token, so it survives a restart and needs no migration.
    #[test]
    fn when_the_plan_is_chosen_the_key_stays_out_of_the_agents_hands() {
        let cfg = Config {
            sandbox: false,
            data_dir: fresh_data_dir(),
            ..Config::default()
        };
        crate::economy::prefer_plan(&cfg, true).expect("choose the plan");
        assert!(key_is_removed(as_agent(&cfg, "claude").as_std()));
        assert!(key_is_removed(&as_agent_std(&cfg, "claude")));
        let run = cfg.data_dir.join("run");
        assert!(key_is_removed(
            command(&cfg, &Cage { run_dir: &run }, "true").as_std()
        ));
    }

    #[test]
    fn without_the_choice_the_environment_is_left_alone() {
        let cfg = Config {
            sandbox: false,
            data_dir: fresh_data_dir(),
            ..Config::default()
        };
        assert!(!key_is_removed(as_agent(&cfg, "claude").as_std()));
        crate::economy::prefer_plan(&cfg, true).expect("choose");
        crate::economy::prefer_plan(&cfg, false).expect("and change your mind");
        assert!(!key_is_removed(as_agent(&cfg, "claude").as_std()));
    }
}
