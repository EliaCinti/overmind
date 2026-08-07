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

/// The profile text for one run.
fn profile(config: &Config, cage: &Cage<'_>) -> Option<String> {
    let mut writable: Vec<PathBuf> = vec![cage.run_dir.to_path_buf()];
    // Temp: compilers, package managers and the test stubs all expect it.
    // This session's TMPDIR specifically, resolved through the /var -> /private
    // symlink because sandbox rules match the real path — granting all of
    // `/private/var/folders` would hand over every per-user cache on the
    // machine to buy the same thing.
    writable.push(PathBuf::from("/private/tmp"));
    let tmp = std::env::temp_dir();
    writable.push(tmp.canonicalize().unwrap_or(tmp));
    writable.extend(adapter_paths());
    writable.extend(config.sandbox_allow.iter().cloned());

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

/// Build the command that runs `script`, caged when we can cage it.
///
/// Falls back to a bare `sh -c` when sandboxing is off, unavailable, or the
/// profile cannot be expressed — and those are the only three cases, each of
/// them a decision rather than an accident.
pub fn command(config: &Config, cage: &Cage<'_>, script: &str) -> Command {
    if !config.sandbox || !available() {
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
        None => {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(script);
            cmd
        }
    }
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
