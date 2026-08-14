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

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::db::Config;

/// Where a caged run may write, beyond the system paths every process needs.
pub struct Cage<'a> {
    /// The run's own directory: the git worktree for a `code` task, the scratch
    /// dir for a `knowledge` task or a conversational turn.
    pub run_dir: &'a Path,
}

/// Is sandboxing available on this platform at all?
///
/// macOS only for now. Elsewhere this reports false and the wrapper is a no-op
/// — saying so rather than pretending to protect. Linux's counterpart is
/// `bubblewrap` when Linux support arrives (roadmap icebox).
pub fn available() -> bool {
    cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").exists()
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
    let mut writable: Vec<PathBuf> = vec![real_path(cage.run_dir)?];
    // Temp: compilers, package managers and the test stubs all expect it.
    // This session's TMPDIR specifically, resolved through the /var -> /private
    // symlink because sandbox rules match the real path — granting all of
    // `/private/var/folders` would hand over every per-user cache on the
    // machine to buy the same thing.
    writable.push(PathBuf::from("/private/tmp"));
    writable.extend(real_path(&std::env::temp_dir()));
    writable.extend(adapter_paths().iter().filter_map(|p| real_path(p)));
    writable.extend(config.sandbox_allow.iter().filter_map(|p| real_path(p)));

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
    config.sandbox && available() && profile(config, cage).is_some()
}

/// Build the command that runs `script`, caged when we can cage it.
///
/// Falls back to a bare `sh -c` when sandboxing is off, unavailable, or the
/// profile cannot be expressed — and those are the only three cases, each of
/// them a decision rather than an accident.
pub fn command(config: &Config, cage: &Cage<'_>, script: &str) -> Command {
    if !caged(config, cage) {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        return cmd;
    }
    match profile(config, cage) {
        Some(text) => {
            let mut cmd = Command::new("/usr/bin/sandbox-exec");
            cmd.arg("-p").arg(text).arg("/bin/sh").arg("-c").arg(script);
            cmd
        }
        // Unreachable while `caged` agrees with us, and harmless if it ever
        // stops: the fallback is the *safe* direction.
        None => {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(script);
            cmd
        }
    }
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
}
