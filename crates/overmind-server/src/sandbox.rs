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
    /// The run's own directory: the clone for a `code` task (ADR-0052), the
    /// scratch dir for a `knowledge` task or a conversational turn.
    pub run_dir: &'a Path,
    /// Read, never written. Empty for every run: the one use is the diff of a
    /// session from before ADR-0052, whose git dir lives in the operator's
    /// repository -- reading it does not widen what an agent can plant there.
    pub readable: &'a [PathBuf],
}

impl<'a> Cage<'a> {
    /// The cage every run gets: its own directory, nothing to read beyond it.
    pub fn new(run_dir: &'a Path) -> Self {
        Cage {
            run_dir,
            readable: &[],
        }
    }
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
    /// The uid was measured not to hold on this filesystem (ADR-0054). The
    /// agent still runs as it, and it no longer counts as a boundary.
    pub uid_refuted: bool,
}

impl Confinement {
    /// Is a real boundary in place?
    ///
    /// Asked of the whole set on purpose. What the permission flag rides on is
    /// that the operating system is holding the process somehow, and a caller
    /// that asked about one named mechanism would answer "no cage" on a
    /// platform that has a different one.
    pub fn is_real(&self) -> bool {
        if self.profile.is_some() || (self.agent_user.is_some() && !self.uid_refuted) {
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
fn writable_paths(config: &Config, cage: &Cage<'_>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut writable: Vec<PathBuf> = Vec::new();
    writable.extend(path_the_agent_owns(cage.run_dir));
    writable.extend(path_the_agent_owns(&std::env::temp_dir()));
    // Where the adapter keeps credentials and state. In the image this is the
    // agent's own home; on a machine it is the user's, and the run would fail
    // without it long before it failed usefully.
    writable.extend(
        config
            .agent_home
            .iter()
            .filter_map(|p| path_the_agent_owns(p)),
    );
    // Not the mounted repositories (ADR-0052). A `code` run works in a clone
    // whose whole `.git` is in its run directory; granting the operator's
    // repository let an agent plant hooks and config there, which then ran as
    // the server at the next checkout and as the owner on the host.
    writable.extend(
        adapter_paths(home)
            .iter()
            .filter_map(|p| path_the_agent_owns(p)),
    );
    writable.extend(
        config
            .sandbox_allow
            .iter()
            .filter_map(|p| path_the_agent_owns(p)),
    );
    writable
}

/// What a run may read and never write, beyond the system paths. One list for
/// both mechanisms, for the reason [`writable_paths`] is one.
fn readable_paths(config: &Config, cage: &Cage<'_>, home: Option<&Path>) -> Vec<PathBuf> {
    adapter_read_paths(home)
        .iter()
        .chain(cage.readable)
        .chain(&config.sandbox_read)
        .filter_map(|p| real_path(p))
        .collect()
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
    if let Some(u) = agent_user(config).filter(|_| config.uid_refuted.is_none()) {
        held.push(format!("unprivileged uid {}", u.uid));
    }
    let refuted = config.uid_refuted.as_ref().map(|why| {
        format!(
            "the agent uid holds nothing on this filesystem: {why} (ADR-0054). Keep what the agent \
             must not reach on a filesystem that keeps ownership; in the image, mount the data at \
             /data and repositories at /repos, as docker-compose.yml does"
        )
    });
    #[cfg(target_os = "linux")]
    if let Some(abi) = crate::landlock::abi() {
        held.push(format!("Landlock (ABI {abi})"));
    }
    if held.is_empty() {
        if let Some(refuted) = refuted {
            return format!("none — {refuted}; agents are read-only");
        }
        // Naming the likeliest cause: the image sets the uid, so seeing this
        // inside a container means the server is not privileged enough to drop
        // to it — a `user:` override in compose is how that happens.
        return match config.agent_uid {
            Some(_) => "none — an agent uid is configured but this process cannot drop to it; agents are read-only".to_string(),
            None => "none on this platform — agents are read-only".to_string(),
        };
    }
    let held = held.join(" + ");
    if let Some(refuted) = refuted {
        return format!("{held} — {refuted}");
    }
    // The image's agent has a home of its own and the uid is the boundary. On a
    // host the home is the owner's, and the line says what that leaves open.
    if agent_user(config).is_some() {
        return held;
    }
    format!(
        "{held} — not a boundary against a hostile agent: ~/.claude stays writable, and a hook \
         planted there runs as you (ADR-0053). A toolchain the cage refuses is fitted with \
         OVERMIND_SANDBOX_READ or OVERMIND_SANDBOX_ALLOW"
    )
}

/// Does agent work run as its own uid here? Only then can a path stand where the
/// agent cannot reach it while the server can.
pub(crate) fn drops_to_agent_uid(config: &Config) -> bool {
    agent_user(config).is_some()
}

/// Does the agent uid hold on this filesystem? `Some(why)` when it does not.
///
/// The modes the server sets only mean something where the filesystem keeps
/// ownership. A VirtioFS bind mount shows every file as owned by whoever looks,
/// exFAT and network shares have their own ways, and none of them say so
/// (ADR-0054). So the question goes to the kernel, as the agent uid, against a
/// `0600` file of the server's in each place it keeps something — the data
/// dir, the backups, the database's own directory — and the repositories dir.
///
/// A measurement that cannot be made counts as a failed one: the claim it
/// would have supported is what grants the permission flag.
pub async fn measure_uid(config: &Config, db_files: &[PathBuf]) -> Option<String> {
    let user = agent_user(config)?;
    let mut shelves = vec![config.data_dir.clone(), crate::backup::backup_dir(config)];
    shelves.extend(
        db_files
            .iter()
            .filter_map(|f| f.parent().map(Path::to_path_buf)),
    );
    let mut canaries: Vec<PathBuf> = Vec::new();
    for shelf in shelves.into_iter().filter(|d| d.is_dir()) {
        let canary = shelf.join(".uid-canary");
        if !canaries.contains(&canary) {
            canaries.push(canary);
        }
    }
    let repos = config.repos_dir.clone();
    tokio::task::spawn_blocking(move || {
        // A probe that cannot even start would read as "refused" below, which
        // is the answer that grants the permission flag.
        if !uid_runs(user) {
            return Some(format!(
                "it could not be measured: uid {} could not run a shell",
                user.uid
            ));
        }
        for canary in &canaries {
            if let Err(e) = plant_canary(canary) {
                return Some(format!(
                    "it could not be measured at {}: {e}",
                    canary.display()
                ));
            }
        }
        let reached = uid_reaches(user, &canaries, repos.as_deref());
        for canary in &canaries {
            let _ = std::fs::remove_file(canary);
        }
        (!reached.is_empty()).then(|| reached.join("; "))
    })
    .await
    .unwrap_or_else(|e| Some(format!("it could not be measured: {e}")))
}

/// A file only the server should be able to read.
///
/// The directory may have been the agent's to write until this very check says
/// otherwise, so whatever is at that name goes first and nothing is followed: a
/// link planted there would turn this into a write as root.
#[cfg(unix)]
fn plant_canary(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map(drop)
}

#[cfg(not(unix))]
fn plant_canary(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Can `user` run anything at all? The control for [`uid_reaches`].
#[cfg(unix)]
fn uid_runs(user: AgentUser) -> bool {
    use std::os::unix::process::CommandExt;
    std::process::Command::new("sh")
        .args(["-c", "true"])
        .uid(user.uid)
        .gid(user.gid)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
fn uid_runs(_user: AgentUser) -> bool {
    false
}

/// What `user` reached of what it must not, each in a sentence.
#[cfg(unix)]
fn uid_reaches(user: AgentUser, canaries: &[PathBuf], repos: Option<&Path>) -> Vec<String> {
    use std::os::unix::process::CommandExt;
    let can = |script: &str, path: &Path| {
        std::process::Command::new("sh")
            .args(["-c", script, "sh"])
            .arg(path)
            .uid(user.uid)
            .gid(user.gid)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let mut reached = Vec::new();
    for canary in canaries {
        if can("cat -- \"$1\"", canary) {
            reached.push(format!("uid {} read {}", user.uid, canary.display()));
        }
    }
    if let Some(repos) = repos
        && can("ls -- \"$1\"", repos)
    {
        reached.push(format!("uid {} listed {}", user.uid, repos.display()));
    }
    reached
}

#[cfg(not(unix))]
fn uid_reaches(_user: AgentUser, _canaries: &[PathBuf], _repos: Option<&Path>) -> Vec<String> {
    Vec::new()
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
    let home = cage_home(config, std::env::var_os("HOME").map(PathBuf::from));
    confinement_in(config, cage, home.as_deref())
}

/// Whose home the adapter's paths are granted in: the agent's, where one is
/// configured, and only otherwise the server's.
///
/// In the image the server is root and the agent is `/home/agent`; reading
/// the server's `HOME` granted `/root/.claude` to a process that never uses it
/// while the agent's real one came in only through `agent_home`. On the native
/// instance there is no agent home yet, and the host's is the one the CLI
/// actually uses until it has its own (ADR-0053, M34 step 4).
fn cage_home(config: &Config, server_home: Option<PathBuf>) -> Option<PathBuf> {
    config.agent_home.clone().or(server_home)
}

/// [`confinement`] with the host's home as an argument, so what it grants there
/// can be tested without changing `HOME` for a whole test process.
fn confinement_in(config: &Config, cage: &Cage<'_>, home: Option<&Path>) -> Confinement {
    if !config.sandbox {
        return Confinement::default();
    }
    Confinement {
        profile: if profile_available() {
            profile(config, cage, home)
        } else {
            None
        },
        agent_user: agent_user(config),
        #[cfg(target_os = "linux")]
        landlock: build_landlock(config, cage, home),
        uid_refuted: config.uid_refuted.is_some(),
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
    home: Option<&Path>,
) -> Option<std::sync::Arc<crate::landlock::Ruleset>> {
    crate::landlock::abi()?;
    // Enough of the system to exist: the loader, the shells, the toolchain —
    // the same shape as the macOS profile's read grants, spelled for Linux.
    // `/dev` is in the writable set because `/dev/null` is, and a run that
    // cannot open it fails in ways nobody enjoys reading.
    let mut readable: Vec<&Path> = [
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/opt", "/proc", "/run",
    ]
    .iter()
    .map(Path::new)
    .collect();
    let granted = readable_paths(config, cage, home);
    readable.extend(granted.iter().map(PathBuf::as_path));
    let mut writable = writable_paths(config, cage, home);
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

/// Paths the adapter has to *write*, which are an installation fact rather than
/// something we get to decide. Defaults suit a standard Claude Code install;
/// `OVERMIND_SANDBOX_ALLOW` and `OVERMIND_SANDBOX_READ` cover anything else.
///
/// The rule is ADR-0053's: a path enters only if the run needs it, with the
/// least access the run needs, and a path the host executes or reads as
/// configuration does not enter writable.
///
/// These two break that rule and the break is stated, not fixed here: the CLI
/// writes them on every run while it runs on the host's login, and a hook
/// planted in them runs as the owner at their next Claude Code session. It
/// closes with the agent's own home on the native instance (M34 step 4).
fn adapter_paths(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    // Where it keeps credentials, sessions and settings.
    vec![home.join(".claude"), home.join(".claude.json")]
}

/// Paths the adapter needs to *read*, never to write: its own sign-in (M23) and
/// its own install (ADR-0053).
///
/// A subscription's OAuth token lives in the login Keychain on macOS --
/// `~/Library/Keychains` -- not under `~/.claude`, which is why the cage
/// passed every key-authenticated run (the key rides the environment) and
/// silently killed every subscription one: exit 1, stderr empty, measured
/// live the day the owner asked whether his plan works. Keychain items stay
/// encrypted and per-item ACLs are enforced by securityd either way; this
/// grants the file, not the secrets.
fn adapter_read_paths(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    vec![
        home.join("Library/Keychains"),
        home.join("Library/Preferences"),
        // Where the binary lives: a symlink in `bin` into `share/claude`. The
        // agent runs it and never replaces it, since the host runs it too. So
        // the CLI cannot update itself from inside a run, which is the host's
        // job (ADR-0053).
        home.join(".local/bin"),
        home.join(".local/share/claude"),
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

/// [`real_path`] for a path the agent may write, which is never followed if it
/// is a link. That is every writable grant.
///
/// An agent may write these, so it may replace them, and a sandbox rule on a
/// path lets it do that. Resolving the link at the next build would then grant
/// whatever the agent pointed it at. Measured: `~/.claude.json` replaced from
/// inside the cage by a link to `~/.config`, and the next run read the keys
/// there and planted beside them. Only the last component: an agent's grant on
/// a path does not reach the directories above it.
///
/// Refusing costs an operator whose grant is a link of their own one
/// `OVERMIND_SANDBOX_ALLOW` naming the real place.
fn path_the_agent_owns(path: &Path) -> Option<PathBuf> {
    // Without a trailing slash, which would make `lstat` follow the link.
    let path: PathBuf = path.components().collect();
    let path = path.as_path();
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        eprintln!(
            "the cage does not follow a link at {} (not granted)",
            path.display()
        );
        return None;
    }
    real_path(path)
}

/// One `allow` rule over these paths, or nothing when there are none.
///
/// Never a rule with an empty list: a rule with no filter matches everything,
/// so `(allow file-read*)` with no path in it grants the whole disk. Measured:
/// `HOME` unset, nothing else to read, and the agent opened `~/.zshrc`.
fn allow_stanza(operation: &str, paths: &[PathBuf]) -> String {
    // A path we cannot express is a path we do not grant. The run may fail for
    // lack of it, which is the loud failure this design prefers.
    let rules: String = paths
        .iter()
        .filter_map(|p| quote(p))
        .map(|q| format!("  (subpath \"{q}\")\n"))
        .collect();
    if rules.is_empty() {
        return String::new();
    }
    format!("(allow {operation}\n{rules})\n")
}

/// The profile text for one run.
fn profile(config: &Config, cage: &Cage<'_>, home: Option<&Path>) -> Option<String> {
    // No real path for the run directory means no profile, and therefore no
    // cage and no `--dangerously-skip-permissions`. Not so for a run directory
    // an earlier run replaced with a link: that one keeps its cage, which grants
    // it nothing, because no profile would mean an uncaged shell inside the
    // link's target.
    real_path(cage.run_dir)?;
    // One list, shared with Landlock: two would eventually disagree, and the
    // disagreement would be a cage that grants different things depending on
    // which kernel you happen to be on.
    let mut writable = writable_paths(config, cage, home);
    // Temp: compilers, package managers and the test stubs all expect it. This
    // session's TMPDIR is already in the shared list, resolved through the
    // /var -> /private symlink because sandbox rules match the real path —
    // granting all of `/private/var/folders` would hand over every per-user
    // cache on the machine to buy the same thing. `/private/tmp` is the macOS
    // spelling of the other one, and belongs only here.
    writable.push(PathBuf::from("/private/tmp"));

    let allow_read = allow_stanza("file-read*", &readable_paths(config, cage, home));
    let allow_write = allow_stanza("file*", &writable);

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
; its credential kills the run before the first word (M23). Also its install,
; which the host runs (ADR-0053), a git dir from before ADR-0052, for its diff,
; and what the operator named in OVERMIND_SANDBOX_READ.
{allow_read}
; The run's own directory, temp, and whatever the adapter has to write.
{allow_write}
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
///
/// **Prefer holding one [`Confinement`] and reading `is_real()` twice.** The
/// predicate was never the risk; *asking it twice* was. This and [`command`]
/// each used to build their own ruleset against a filesystem that can change
/// between the two calls, and `build_landlock` degrades to `None` with a log
/// line when it fails — so the first call could answer yes, which is what
/// grants `--dangerously-skip-permissions`, while the second quietly produced
/// no Landlock at all. An uncaged run holding the caged agent's freedom: the
/// one combination named above as never allowed, reachable on a Linux host
/// outside the image where Landlock is all `is_real` has to go on.
pub fn caged(config: &Config, cage: &Cage<'_>) -> bool {
    confinement(config, cage).is_real()
}

/// Build the command that runs `script`, caged when we can cage it, carrying
/// the credential of the account this run was spawned on and no other.
///
/// Falls back to a bare `sh -c` when sandboxing is off, unavailable, or the
/// profile cannot be expressed — and those are the only three cases, each of
/// them a decision rather than an accident.
///
/// Who pays is `creds`' to say (ADR-0051): it strips every variable the
/// provider reads a credential from and sets the active account's. Resolved by
/// the caller, once, at spawn, so a switch of account mid-run cannot reach a
/// run that already started -- and so the cost event can name the account the
/// run was actually given.
pub fn command(
    config: &Config,
    held: &Confinement,
    creds: &crate::accounts::Spawn,
    script: &str,
) -> Command {
    let mut cmd = caged_shell(config, held, script);
    creds.apply(&mut cmd);
    cmd
}

/// [`command`] without the adapter's credentials, for our own tools run over
/// what an agent left behind (ADR-0052): the server's git over a run's clone.
///
/// The cage is the agent's; the token is not needed. A command an agent
/// planted for that git to run -- a clean filter, say -- would otherwise read
/// the subscription token from its environment every time someone opened a
/// diff. Every credential variable is removed rather than merely not added:
/// the server's own environment may carry any of them.
pub fn command_without_credentials(config: &Config, held: &Confinement, script: &str) -> Command {
    let mut cmd = caged_shell(config, held, script);
    crate::accounts::Spawn::nobody().apply(&mut cmd);
    cmd
}

fn caged_shell(config: &Config, held: &Confinement, script: &str) -> Command {
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
    if let Some(rules) = held.landlock.clone() {
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
pub fn as_agent(config: &Config, creds: &crate::accounts::Spawn, program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(user) = agent_user(config) {
        drop_to(&mut cmd, config, user);
    }
    creds.apply(&mut cmd);
    cmd
}

/// [`as_agent`], but for `std::process::Command` (M23).
///
/// The sign-in flow needs a *blocking* child wired to a pty, which is
/// `std::process` territory; everything `as_agent` promises -- same uid, same
/// `HOME` -- holds here for the same reasons.
pub fn as_agent_std(
    config: &Config,
    creds: &crate::accounts::Spawn,
    program: &str,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(unix)]
    if let Some(user) = agent_user(config) {
        use std::os::unix::process::CommandExt;
        cmd.uid(user.uid).gid(user.gid);
        if let Some(home) = &config.agent_home {
            cmd.env("HOME", home);
        }
    }
    creds.apply(&mut cmd);
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
    // Iterative, with a depth limit, because the tree is the agent's.
    //
    // Recursion here descended once per directory level inside
    // `spawn_blocking`'s 2 MiB stack, over a directory the PREVIOUS run wrote.
    // An agent that made a deep enough nest would overflow the stack on the
    // next hand-over and abort the process -- a way to kill the server from
    // inside the cage, which is exactly what the cage exists to prevent.
    //
    // The cap is generous by any honest measure: a worktree is somebody's
    // repository, and a repository a hundred levels deep is not a repository.
    // A tree that reaches it is refused rather than half-done, because a
    // partial chown leaves files the agent cannot read in a directory it owns.
    const MAX_DEPTH: usize = 100;
    let mut work = vec![(path.to_path_buf(), 0usize)];
    while let Some((p, depth)) = work.pop() {
        std::os::unix::fs::lchown(&p, Some(user.uid), Some(user.gid))?;
        if p.symlink_metadata()?.is_dir() {
            if depth >= MAX_DEPTH {
                return Err(std::io::Error::other(format!(
                    "refusing to hand over a tree more than {MAX_DEPTH} directories deep at {}",
                    p.display()
                )));
            }
            for entry in std::fs::read_dir(&p)? {
                work.push((entry?.path(), depth + 1));
            }
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
/// The data directory itself is `0755` when it holds the runs too, which the
/// agent has to walk through to reach its own, and `0700` when they live in
/// `OVERMIND_RUNS_DIR` (ADR-0054).
///
/// Applied only where an agent uid is configured. On a user's own machine
/// Overmind runs as that user and there is no second party to keep out;
/// quietly re-permissioning somebody's directory is not this function's call.
pub async fn lay_out_data_dir(config: &Config) -> std::io::Result<()> {
    let Some(user) = agent_user(config) else {
        return Ok(());
    };
    let root = config.data_dir.clone();
    let runs = config.runs_dir().to_path_buf();
    let home = config.agent_home.clone();
    tokio::task::spawn_blocking(move || lay_out(&root, &runs, home.as_deref(), user))
        .await
        .map_err(std::io::Error::other)?
}

fn lay_out(root: &Path, runs: &Path, home: Option<&Path>, user: AgentUser) -> std::io::Result<()> {
    // These run as root at every start, over a tree an agent could write on an
    // install that was exposed (ADR-0054): a link where a shelf should be is
    // refused, never followed, or the mode lands on whatever it names.
    let shelf = |path: &Path, mode: u32| {
        if path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(std::io::Error::other(format!(
                "{} is a link where Overmind keeps a directory of its own; remove it",
                path.display()
            )));
        }
        mkdir_mode(path, mode)
    };
    // With the runs elsewhere, the agent has no reason to walk through here.
    // Runs inside it -- the default, or OVERMIND_RUNS_DIR under the data dir --
    // are reached through it.
    mkdir_mode(root, if runs.starts_with(root) { 0o755 } else { 0o700 })?;
    // Overmind's own shelves.
    for name in ["companies", "artifacts", "attachments", "backups"] {
        shelf(&root.join(name), 0o700)?;
    }
    mkdir_mode(runs, 0o755)?;
    // Traversable, not listable: these hold the runs.
    for name in ["sessions", "worktrees", "chat", "meetings"] {
        shelf(&runs.join(name), 0o711)?;
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
        mkdir_mode(home, 0o700)?;
        chown_one(home, user)?;
        let cli_state = home.join(".claude");
        shelf(&cli_state, 0o700)?;
        chown_one(&cli_state, user)?;
    }
    Ok(())
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
pub(crate) fn mkdir_mode(path: &Path, mode: u32) -> std::io::Result<()> {
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
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
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
/// - `core.excludesFile` and `core.attributesFile` go to `/dev/null`: git reads
///   them from `~/.config/git` whatever `GIT_CONFIG_GLOBAL` says, the cage
///   denies them (ADR-0053), and every git would warn about the denial.
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
        ("GIT_CONFIG_COUNT", "5".into()),
        ("GIT_CONFIG_KEY_0", "user.name".into()),
        ("GIT_CONFIG_VALUE_0", "Overmind agent".into()),
        ("GIT_CONFIG_KEY_1", "user.email".into()),
        ("GIT_CONFIG_VALUE_1", "agent@overmind.local".into()),
        ("GIT_CONFIG_KEY_2", "credential.helper".into()),
        ("GIT_CONFIG_VALUE_2", String::new()),
        ("GIT_CONFIG_KEY_3", "core.excludesFile".into()),
        ("GIT_CONFIG_VALUE_3", "/dev/null".into()),
        ("GIT_CONFIG_KEY_4", "core.attributesFile".into()),
        ("GIT_CONFIG_VALUE_4", "/dev/null".into()),
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

    /// The profile on this machine's own home, which is what most tests ask.
    fn profile(config: &Config, cage: &Cage<'_>) -> Option<String> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        super::profile(config, cage, home.as_deref())
    }

    /// A home nobody has, so its name appears in a profile only if granted.
    const NO_ONES_HOME: &str = "/no-ones-home";

    /// The profile's read-only stanza and its writable one.
    fn stanzas(text: &str) -> (&str, &str) {
        let (reads, write) = text.split_once("(allow file*\n").expect("write stanza");
        let read = reads
            .split("(allow file-read*\n")
            .nth(2)
            .expect("read stanza");
        (read, write)
    }

    /// The owner's keys live under `~/.config`, and what an agent can read it
    /// can send (ADR-0053). Neither to read nor to write.
    #[test]
    fn the_cage_grants_nothing_under_config() {
        let home = Path::new(NO_ONES_HOME);
        let cfg = config(Vec::new());
        let cage = Cage::new(Path::new("/tmp/x"));
        let text = super::profile(&cfg, &cage, Some(home)).expect("profile");
        assert!(!text.contains("/no-ones-home/.config"), "{text}");
        let config_dir = home.join(".config");
        assert!(!writable_paths(&cfg, &cage, Some(home)).contains(&config_dir));
        assert!(!readable_paths(&cfg, &cage, Some(home)).contains(&config_dir));
    }

    /// The host runs `~/.local/bin`, so the agent may run it too and never
    /// replace it. And only the adapter's corner of `~/.local/share`, where
    /// every other tool's data is. `~/.claude` stays writable, which is the
    /// hole ADR-0053 states rather than closes.
    #[test]
    fn the_adapters_install_is_readable_and_never_writable() {
        let home = Path::new(NO_ONES_HOME);
        let cfg = config(Vec::new());
        let cage = Cage::new(Path::new("/tmp/x"));
        let text = super::profile(&cfg, &cage, Some(home)).expect("profile");
        let (read, write) = stanzas(&text);
        assert!(
            read.contains("(subpath \"/no-ones-home/.local/bin\")"),
            "{text}"
        );
        assert!(
            read.contains("(subpath \"/no-ones-home/.local/share/claude\")"),
            "{text}"
        );
        assert!(
            !read.contains("(subpath \"/no-ones-home/.local\")"),
            "{text}"
        );
        assert!(!write.contains("/no-ones-home/.local"), "{text}");
        assert!(
            write.contains("(subpath \"/no-ones-home/.claude\")"),
            "{text}"
        );

        // The same answer from the lists Landlock is built from.
        let writable = writable_paths(&cfg, &cage, Some(home));
        let readable = readable_paths(&cfg, &cage, Some(home));
        assert!(!writable.iter().any(|p| p.starts_with(home.join(".local"))));
        assert!(readable.contains(&home.join(".local/bin")));
        assert!(readable.contains(&home.join(".local/share/claude")));
    }

    /// `OVERMIND_SANDBOX_READ`: a toolchain the agent has to read no longer has
    /// to be granted writable (ADR-0053).
    #[test]
    fn a_read_only_allow_is_never_writable() {
        let tool = PathBuf::from("/srv/a-readable-tool");
        let cfg = Config {
            sandbox_read: vec![tool.clone()],
            ..config(Vec::new())
        };
        let cage = Cage::new(Path::new("/tmp/x"));
        let text = super::profile(&cfg, &cage, None).expect("profile");
        let (read, write) = stanzas(&text);
        assert!(
            read.contains("(subpath \"/srv/a-readable-tool\")"),
            "{text}"
        );
        assert!(!write.contains("a-readable-tool"), "{text}");
        assert!(readable_paths(&cfg, &cage, None).contains(&tool));
        assert!(!writable_paths(&cfg, &cage, None).contains(&tool));
    }

    /// A rule with no filter matches everything, so an empty list rendered as
    /// a stanza grants the whole disk. `HOME` unset with nothing else to read
    /// is how a list gets there. Paired against the same read uncaged.
    #[tokio::test]
    async fn a_cage_with_nothing_to_read_still_refuses_reads() {
        let cfg = config(Vec::new());
        let cage = Cage::new(Path::new("/tmp/x"));
        let text = super::profile(&cfg, &cage, None).expect("profile");
        assert!(!text.contains("(allow file-read*\n)"), "{text}");

        // Outside temp, which the cage grants outright.
        let root = std::path::absolute(format!(
            "target/overmind-noread-{}",
            uuid::Uuid::now_v7().simple()
        ))
        .expect("absolute root");
        let run_dir = root.join("run");
        std::fs::create_dir_all(&run_dir).expect("run dir");
        let secret = root.join("secret");
        std::fs::write(&secret, "x\n").expect("secret");

        let held = confinement_in(&cfg, &Cage::new(&run_dir), None);
        if !held.is_real() {
            eprintln!("no cage on this platform — skipping");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let read = async |held: &Confinement| {
            let out = caged_shell(
                &cfg,
                held,
                r#"cat "$F" >/dev/null 2>&1 && echo OPENED || echo REFUSED"#,
            )
            .env("F", &secret)
            .current_dir(&run_dir)
            .output()
            .await
            .expect("run the read");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let caged = read(&held).await;
        let open = read(&Confinement::default()).await;
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(caged, "REFUSED");
        assert_eq!(open, "OPENED", "uncaged, the same read has to work");
    }

    /// An agent can replace what it may write: `~/.claude.json`, `~/.claude`,
    /// its own run directory. Measured: a link planted there from inside the
    /// cage made the next build grant `~/.config`, to read and to write.
    #[tokio::test]
    async fn a_link_planted_in_place_of_a_grant_does_not_carry_it() {
        use std::os::unix::fs::symlink;
        let root = std::path::absolute(format!(
            "target/overmind-link-{}",
            uuid::Uuid::now_v7().simple()
        ))
        .expect("absolute root");
        let home = root.join("home");
        let run_dir = root.join("run");
        let planted_run = root.join("planted-run");
        let config_dir = home.join(".config");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        std::fs::create_dir_all(&run_dir).expect("run dir");
        let keys = config_dir.join("keys.env");
        std::fs::write(&keys, "KEY=x\n").expect("keys");
        symlink(&config_dir, home.join(".claude.json")).expect("plant over the file");
        symlink(&config_dir, home.join(".claude")).expect("plant over the directory");
        symlink(&config_dir, &planted_run).expect("plant over the run dir");
        let real_config = config_dir.canonicalize().expect("real config dir");

        let cfg = config(Vec::new());
        let writable = writable_paths(&cfg, &Cage::new(&run_dir), Some(&home));
        assert!(
            !writable.iter().any(|p| p.starts_with(&real_config)),
            "a planted link carried the grant: {writable:?}"
        );
        let replaced = writable_paths(&cfg, &Cage::new(&planted_run), Some(&home));
        assert!(
            !replaced.iter().any(|p| p.starts_with(&real_config)),
            "a planted run dir carried the grant: {replaced:?}"
        );
        // Still a cage, one that grants nothing there. "No cage" would be the
        // uncaged shell, with its working directory in the link's target.
        if profile_available() {
            let held = confinement_in(&cfg, &Cage::new(&planted_run), Some(&home));
            assert!(held.is_real(), "a planted run dir must not remove the cage");
        }

        // What an operator allowed is the agent's to replace too, and a
        // trailing slash must not make the link look like its target.
        let allowed = root.join("allowed");
        symlink(&config_dir, &allowed).expect("plant over an allowed path");
        for spelled in [allowed.clone(), allowed.join("")] {
            let cfg = config(vec![spelled]);
            let writable = writable_paths(&cfg, &Cage::new(&run_dir), Some(&home));
            assert!(
                !writable.iter().any(|p| p.starts_with(&real_config)),
                "a link at an allowed path carried the grant: {writable:?}"
            );
        }

        let held = confinement_in(&cfg, &Cage::new(&run_dir), Some(&home));
        if held.is_real() {
            let read = async |held: &Confinement| {
                let script = r#"cat "$F" >/dev/null 2>&1 && echo OPENED || echo REFUSED"#;
                let out = caged_shell(&cfg, held, script)
                    .env("F", &keys)
                    .current_dir(&run_dir)
                    .output()
                    .await
                    .expect("run the read");
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            };
            assert_eq!(read(&held).await, "REFUSED");
            assert_eq!(read(&Confinement::default()).await, "OPENED");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What a script reaches in a home laid out like a real one.
    ///
    /// Each answer is one line, `name=REACHABLE` or `name=DENIED`.
    const HOME_PROBE: &str = r#"
probe() { name=$1; shift; if "$@" >/dev/null 2>&1; then echo "$name=REACHABLE"; else echo "$name=DENIED"; fi; }
write() { echo planted > "$1"; }
probe read_keys cat "$H/.config/keys.env"
probe plant_git_config write "$H/.config/git/config"
probe run_cli "$H/.local/bin/cli"
probe plant_cli write "$H/.local/bin/planted"
probe read_cli_install cat "$H/.local/share/claude/version"
probe plant_cli_install write "$H/.local/share/claude/planted"
probe read_other_tool cat "$H/.local/share/other/secret"
probe write_cli_state write "$H/.claude/settings.json"
"#;

    /// Run [`HOME_PROBE`] under `held`, from `run_dir`, and read its answers.
    async fn probe_home(
        cfg: &Config,
        held: &Confinement,
        home: &Path,
        run_dir: &Path,
    ) -> std::collections::HashMap<String, String> {
        let out = caged_shell(cfg, held, HOME_PROBE)
            .env("H", home)
            .current_dir(run_dir)
            .output()
            .await
            .expect("run the probe");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// ADR-0053 through a real cage, paired against the same script uncaged.
    ///
    /// The home and the run are **outside temp**, which the cage grants
    /// outright: a home under it would answer REACHABLE through that grant, and
    /// nothing here would be about the home.
    #[tokio::test]
    async fn an_agent_cannot_read_or_plant_in_the_hosts_config() {
        let root = std::path::absolute(format!(
            "target/overmind-home-{}",
            uuid::Uuid::now_v7().simple()
        ))
        .expect("absolute root");
        let home = root.join("home");
        let run_dir = root.join("run");
        for dir in [
            ".config/git",
            ".local/bin",
            ".local/share/claude",
            ".local/share/other",
            ".claude",
        ] {
            std::fs::create_dir_all(home.join(dir)).expect("home dir");
        }
        std::fs::create_dir_all(&run_dir).expect("run dir");
        std::fs::write(home.join(".config/keys.env"), "KEY=x\n").expect("keys");
        std::fs::write(home.join(".local/share/claude/version"), "1\n").expect("install");
        std::fs::write(home.join(".local/share/other/secret"), "x\n").expect("other");
        let cli = home.join(".local/bin/cli");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").expect("cli");
        set_mode(&cli, 0o755).expect("cli mode");

        let cfg = config(Vec::new());
        let held = confinement_in(&cfg, &Cage::new(&run_dir), Some(&home));
        if !held.is_real() {
            eprintln!("no cage on this platform — skipping");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let caged = probe_home(&cfg, &held, &home, &run_dir).await;
        let open = probe_home(&cfg, &Confinement::default(), &home, &run_dir).await;
        let _ = std::fs::remove_dir_all(&root);

        for (name, expected) in [
            ("read_keys", "DENIED"),
            ("plant_git_config", "DENIED"),
            ("plant_cli", "DENIED"),
            ("plant_cli_install", "DENIED"),
            ("read_other_tool", "DENIED"),
            // The adapter still works, and the hole ADR-0053 states is there.
            ("run_cli", "REACHABLE"),
            ("read_cli_install", "REACHABLE"),
            ("write_cli_state", "REACHABLE"),
        ] {
            assert_eq!(
                caged.get(name).map(String::as_str),
                Some(expected),
                "{name}: {caged:?}"
            );
            // Without which a denial is a broken probe, not a boundary.
            assert_eq!(
                open.get(name).map(String::as_str),
                Some("REACHABLE"),
                "{name}, uncaged: {open:?}"
            );
        }
    }

    #[test]
    fn the_run_directory_is_writable_and_nothing_else_is_named() {
        let cfg = config(Vec::new());
        let dir = PathBuf::from("/tmp/run-me");
        let text = profile(&cfg, &Cage::new(&dir)).expect("profile");
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
        let text = profile(&cfg, &Cage::new(Path::new("/tmp/x"))).expect("profile");
        assert!(
            text.contains(r#"(subpath "/data/with \"quotes\"")"#),
            "{text}"
        );
    }

    /// ADR-0052: a run works in a clone, so the operator's repositories are
    /// granted neither to write nor to read.
    #[test]
    fn the_operators_repositories_are_not_in_the_cage() {
        let cfg = Config {
            repos_dir: Some(PathBuf::from("/srv/the-operators-repos")),
            ..config(Vec::new())
        };
        let text = profile(&cfg, &Cage::new(Path::new("/tmp/x"))).expect("profile");
        assert!(!text.contains("the-operators-repos"), "{text}");
    }

    /// The one read-only grant -- a git dir from before ADR-0052, for its
    /// diff -- is read and never written.
    #[test]
    fn a_readable_path_is_never_writable() {
        let cfg = config(Vec::new());
        let readable = [PathBuf::from("/srv/repo/.git")];
        let cage = Cage {
            run_dir: Path::new("/tmp/x"),
            readable: &readable,
        };
        let text = profile(&cfg, &cage).expect("profile");
        let write_stanza = text.split("(allow file*\n").nth(1).expect("write stanza");
        assert!(text.contains("(subpath \"/srv/repo/.git\")"), "{text}");
        assert!(!write_stanza.contains("/srv/repo"), "{text}");
    }

    #[test]
    fn a_path_we_cannot_express_is_a_path_we_do_not_grant() {
        // A newline would end the profile line and turn the rest into
        // something else entirely; the path is dropped instead.
        let cfg = config(vec![PathBuf::from("/data/two\nlines")]);
        let text = profile(&cfg, &Cage::new(Path::new("/tmp/x"))).expect("profile");
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
        // Read from `~/.config/git` whatever `GIT_CONFIG_GLOBAL` says, and the
        // cage denies them: without these keys every git warns about the denial.
        assert_eq!(
            env.get("GIT_CONFIG_KEY_3").map(String::as_str),
            Some("core.excludesFile")
        );
        assert_eq!(
            env.get("GIT_CONFIG_KEY_4").map(String::as_str),
            Some("core.attributesFile")
        );
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

    #[cfg(unix)]
    fn this_user() -> AgentUser {
        // Reading our own ids: nothing is touched and no pointer is followed.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        AgentUser { uid, gid }
    }

    fn fresh_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("om-{name}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// What VirtioFS does to the boundary, reproduced without VirtioFS and
    /// without root: a `0600` file the agent uid can read after all.
    #[cfg(unix)]
    #[test]
    fn a_shelf_the_agent_uid_can_read_means_the_uid_is_no_boundary() {
        let dir = fresh_dir("canary");
        let canary = dir.join("canary");
        std::fs::write(&canary, "x").expect("canary");
        set_mode(&canary, 0o600).expect("mode");
        let reached = uid_reaches(this_user(), &[dir.join("refused"), canary.clone()], None);
        assert_eq!(reached.len(), 1, "reached: {reached:?}");
        assert!(
            reached[0].contains(&canary.display().to_string()),
            "{reached:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_repositories_dir_the_agent_uid_can_list_means_the_same() {
        let dir = fresh_dir("repos");
        let reached = uid_reaches(this_user(), &[dir.join("no-canary")], Some(&dir));
        assert_eq!(reached.len(), 1, "reached: {reached:?}");
        assert!(
            reached[0].contains(&dir.display().to_string()),
            "{reached:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_probe_has_a_control_that_runs() {
        assert!(uid_runs(this_user()));
    }

    #[cfg(unix)]
    #[test]
    fn what_the_agent_uid_is_refused_leaves_the_uid_standing() {
        let dir = fresh_dir("refused");
        let reached = uid_reaches(
            this_user(),
            &[dir.join("no-canary")],
            Some(&dir.join("no-repos")),
        );
        assert!(reached.is_empty(), "reached: {reached:?}");
    }

    #[test]
    fn a_refuted_uid_holds_nothing_and_the_agent_still_runs_as_it() {
        let user = AgentUser {
            uid: 10_001,
            gid: 10_001,
        };
        let standing = Confinement {
            agent_user: Some(user),
            ..Confinement::default()
        };
        assert!(standing.is_real());
        let refuted = Confinement {
            uid_refuted: true,
            ..standing
        };
        assert!(!refuted.is_real());
        assert_eq!(refuted.agent_user, Some(user));
    }

    #[test]
    fn the_startup_line_says_the_uid_was_refuted_and_what_to_do() {
        let cfg = Config {
            uid_refuted: Some("uid 10001 read /data/.uid-canary".to_string()),
            ..config(Vec::new())
        };
        let line = announce(&cfg);
        assert!(line.contains("uid 10001 read /data/.uid-canary"), "{line}");
        assert!(line.contains("ADR-0054"), "{line}");
    }

    #[cfg(unix)]
    #[test]
    fn runs_stand_apart_from_the_shelves() {
        let dir = fresh_dir("layout");
        let (data, runs) = (dir.join("data"), dir.join("runs"));
        lay_out(&data, &runs, None, this_user()).expect("layout");
        let mode = |p: PathBuf| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p).expect("exists").permissions().mode() & 0o777
        };
        for shelf in ["companies", "artifacts", "attachments", "backups"] {
            assert_eq!(mode(data.join(shelf)), 0o700, "{shelf}");
            assert!(!runs.join(shelf).exists(), "{shelf}");
        }
        for parent in ["sessions", "worktrees", "chat", "meetings"] {
            assert_eq!(mode(runs.join(parent)), 0o711, "{parent}");
            assert!(!data.join(parent).exists(), "{parent}");
        }
        assert_eq!(mode(data), 0o700);
        assert_eq!(mode(runs), 0o755);
    }

    /// An install that was exposed (ADR-0054) may carry links an agent planted
    /// in the data dir. The layout runs as root at every start, and a mode set
    /// through one lands on whatever it names.
    #[cfg(unix)]
    #[test]
    fn a_link_where_a_shelf_should_be_is_refused_not_followed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fresh_dir("planted");
        let (data, runs, target) = (dir.join("data"), dir.join("runs"), dir.join("target"));
        std::fs::create_dir_all(&data).expect("data");
        std::fs::create_dir_all(&target).expect("target");
        set_mode(&target, 0o755).expect("mode");
        std::os::unix::fs::symlink(&target, data.join("backups")).expect("link");

        let refused = lay_out(&data, &runs, None, this_user());
        assert!(refused.is_err(), "the layout went through the link");
        let mode = std::fs::metadata(&target)
            .expect("target")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755, "the link's target was re-moded");
    }

    #[cfg(unix)]
    #[test]
    fn runs_inside_the_data_dir_keep_it_walkable() {
        let dir = fresh_dir("nested");
        let data = dir.join("data");
        lay_out(&data, &data.join("runs"), None, this_user()).expect("layout");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&data).expect("data").permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn runs_stay_in_the_data_dir_unless_told() {
        let data = PathBuf::from("/some/data");
        let cfg = Config {
            data_dir: data.clone(),
            ..Config::default()
        };
        assert_eq!(cfg.runs_dir(), data);
        let cfg = Config {
            runs_dir: Some(PathBuf::from("/runs")),
            ..cfg
        };
        assert_eq!(cfg.runs_dir(), Path::new("/runs"));
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
        let held = confinement(&cfg, &Cage::new(Path::new("/tmp/x")));
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

    /// ADR-0053 decision 6: on a host, the line says what the cage leaves open
    /// and how a toolchain it refuses is fitted.
    #[test]
    fn the_startup_line_states_what_the_cage_leaves_open_on_a_host() {
        let said = announce(&Config::default());
        if said.contains("read-only") {
            eprintln!("no cage on this platform — skipping");
            return;
        }
        assert!(said.contains("~/.claude"), "{said}");
        assert!(said.contains("OVERMIND_SANDBOX_READ"), "{said}");
        assert!(said.contains("OVERMIND_SANDBOX_ALLOW"), "{said}");
    }

    #[test]
    fn turning_it_off_gives_a_plain_shell() {
        let cfg = Config {
            sandbox: false,
            ..Config::default()
        };
        let cage = Cage::new(Path::new("/tmp/x"));
        let none = crate::accounts::Spawn::none(crate::provider::current());
        let cmd = command(&cfg, &confinement(&cfg, &cage), &none, "echo hi");
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

    /// In the image the server is root and the agent is not: the adapter's
    /// writable home is the agent's, `/home/agent`, and `/root/.claude` is
    /// nobody's business. Reading the server's `HOME` granted exactly that
    /// (NEXT, "the sign-in and the cage", one of the six that travel with
    /// `sign_in`).
    #[test]
    fn the_cage_grants_the_agents_home_and_not_the_servers() {
        let agent_home = fresh_data_dir().join("agent-home");
        let cfg = Config {
            data_dir: fresh_data_dir(),
            agent_home: Some(agent_home.clone()),
            ..Config::default()
        };
        let run = cfg.data_dir.join("run");
        let home = cage_home(&cfg, Some(PathBuf::from("/root")));
        let writable = writable_paths(&cfg, &Cage::new(&run), home.as_deref());
        assert!(
            writable.iter().any(|p| p.ends_with("agent-home/.claude")),
            "the agent's own adapter state is writable: {writable:?}"
        );
        assert!(
            !writable.iter().any(|p| p.starts_with("/root")),
            "the server's home was granted: {writable:?}"
        );
        // And where no agent home is configured -- the native instance, until
        // it gets one (M34 step 4) -- the host's home is still the one used.
        let native = Config {
            agent_home: None,
            ..cfg.clone()
        };
        assert_eq!(
            cage_home(&native, Some(PathBuf::from("/Users/someone"))),
            Some(PathBuf::from("/Users/someone"))
        );
    }

    /// ADR-0051: every command that runs as the agent — the probe, the caged
    /// run, the blocking sign-in — carries the spawn it was given, and a key
    /// the server's environment holds never rides along on its own. A single
    /// path that still carried it would be a single path that still billed it.
    #[test]
    fn every_way_of_running_as_the_agent_carries_only_the_account_it_was_given() {
        let cfg = Config {
            sandbox: false,
            data_dir: fresh_data_dir(),
            ..Config::default()
        };
        let none = crate::accounts::Spawn::none(crate::provider::current());
        assert!(key_is_removed(as_agent(&cfg, &none, "claude").as_std()));
        assert!(key_is_removed(&as_agent_std(&cfg, &none, "claude")));
        let run = cfg.data_dir.join("run");
        assert!(key_is_removed(
            command(&cfg, &confinement(&cfg, &Cage::new(&run)), &none, "true").as_std()
        ));
        assert!(key_is_removed(
            command_without_credentials(&cfg, &confinement(&cfg, &Cage::new(&run)), "true")
                .as_std()
        ));
    }
    /// A run directory belongs to the agent, and on a retry it already holds
    /// whatever the previous attempt put there. The hand-over used to recurse
    /// once per level inside `spawn_blocking`'s 2 MiB stack, so a deep enough
    /// nest overflowed and aborted the whole process — an agent ending the
    /// server from inside the cage.
    ///
    /// What this proves is the **refusal**, not the crash: reproducing the
    /// overflow would abort the test process along with everything else, so
    /// the depth here is far below what a 2 MiB stack actually gives out. The
    /// guarantee is that a tree past the cap stops with a sentence, and that an
    /// ordinary one still goes through — the second half matters as much,
    /// since a cap that refused real repositories would be its own outage.
    ///
    /// Chowned to the current user, which every uid may do to its own files,
    /// so this exercises the walk rather than the permission.
    #[test]
    fn a_tree_too_deep_to_hand_over_is_refused_rather_than_overflowing() {
        let root = std::env::temp_dir().join(format!("om-deep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut p = root.clone();
        for i in 0..150 {
            p = p.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&p).expect("build a deep tree");

        let me = AgentUser {
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        };
        let err = chown_tree(&root, me).expect_err("a tree this deep is refused");
        assert!(
            err.to_string().contains("deep"),
            "the refusal says what was wrong: {err}"
        );

        // And an ordinary tree still goes through.
        let shallow = std::env::temp_dir().join(format!("om-shallow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&shallow);
        std::fs::create_dir_all(shallow.join("a/b/c")).expect("build a shallow tree");
        std::fs::write(shallow.join("a/b/c/f"), b"x").expect("a file in it");
        chown_tree(&shallow, me).expect("an ordinary run directory is handed over");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&shallow);
    }
}
