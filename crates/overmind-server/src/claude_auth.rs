//! Signing the agent CLI into a Claude subscription, from the product (M23).
//!
//! The owner's words: "il collegamento all'account deve avvenire da Overmind,
//! non da terminale". Before this, connecting a subscription meant knowing
//! that a CLI lives inside the container, that it has a `setup-token`
//! command, and that `docker compose exec` wants a TTY -- three facts a
//! person evaluating the product has no reason to hold.
//!
//! The flow is the CLI's own OAuth dance, orchestrated: the server spawns
//! `claude setup-token` **as the agent** on a pseudo-terminal (the CLI
//! refuses to draw its prompt on a pipe), reads its output until the
//! authorization URL appears, hands that URL to the browser, and forwards the
//! code the person pastes back. Success is not inferred from the exit code
//! alone: the economy is re-detected, and the interface's sign-in notice
//! disappears because the economy is now known, not because a flag was set.
//!
//! One flow at a time, ten-minute lifetime, loopback callers only (like every
//! endpoint until real authentication lands): this is a setup surface, not a
//! work surface.

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::db::AppState;

/// Where the flow stands. Serialized into `GET /api/claude-auth`.
#[derive(Debug, Clone, PartialEq)]
pub enum FlowState {
    /// Spawned, still waiting for the CLI to print the authorization URL.
    Starting,
    /// The URL is known; the person has not sent a code yet.
    UrlReady(String),
    /// A code was forwarded; waiting for the CLI to finish.
    Exchanging,
    /// The CLI exited successfully and the economy was re-detected.
    Done,
    /// The CLI exited without succeeding; the tail of its output rides along.
    Failed(String),
}

pub struct Flow {
    pub state: FlowState,
    /// Everything the CLI printed, ANSI stripped -- the failure tail comes
    /// from here, and the URL is scraped out of it.
    output: String,
    /// Write end of the pty: where the pasted code goes.
    master: Option<std::fs::File>,
    child: Option<std::process::Child>,
    started: Instant,
}

/// The one flow, if any. On `AppState` it would drag `Arc<Mutex<..>>` through
/// every constructor; a module-level slot keeps the blast radius here.
static FLOW: Mutex<Option<Flow>> = Mutex::new(None);

/// Strip ANSI escape sequences well enough to scrape a URL and show a tail.
fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                // CSI: ESC [ ... final byte in @-~
                Some('[') => {
                    chars.next();
                    for d in chars.by_ref() {
                        if ('@'..='~').contains(&d) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL or ESC \
                Some(']') => {
                    chars.next();
                    while let Some(d) = chars.next() {
                        if d == '\u{7}' {
                            break;
                        }
                        if d == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else if c == '\r' {
            // The TUI redraws with carriage returns; keep lines readable.
            out.push('\n');
        } else if !c.is_control() || c == '\n' {
            out.push(c);
        }
    }
    out
}

/// The first `https://` URL in the output that looks like the OAuth page.
fn find_url(text: &str) -> Option<String> {
    for start in text.match_indices("https://").map(|(i, _)| i) {
        let url: String = text[start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
            .collect();
        if url.contains("claude.ai") || url.contains("claude.com") || url.contains("anthropic.com")
        {
            return Some(url);
        }
    }
    None
}

/// Begin the sign-in: spawn `claude setup-token` as the agent on a pty.
///
/// Answers with an error string when a flow is already running or the spawn
/// fails; the caller turns that into an HTTP status.
pub fn start(state: &AppState) -> Result<(), String> {
    let mut slot = FLOW.lock().map_err(|_| "flow state poisoned".to_string())?;
    if let Some(f) = slot.as_ref() {
        let stale = f.started.elapsed() > Duration::from_secs(600)
            || matches!(f.state, FlowState::Done | FlowState::Failed(_));
        if !stale {
            // Idempotent: the second click adopts the flow the first one
            // started, instead of being told off for it.
            return Ok(());
        }
    }

    // A pty pair: the CLI draws its prompt only on a terminal.
    let mut master_fd: libc::c_int = 0;
    let mut slave_fd: libc::c_int = 0;
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err("could not allocate a pty".into());
    }
    // Owned wrappers so an early return cannot leak the descriptors.
    let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave_fd) };

    let mut cmd = crate::sandbox::as_agent_std(&state.config, "claude");
    cmd.arg("setup-token")
        .stdin(std::process::Stdio::from(
            slave.try_clone().map_err(|e| e.to_string())?,
        ))
        .stdout(std::process::Stdio::from(
            slave.try_clone().map_err(|e| e.to_string())?,
        ))
        .stderr(std::process::Stdio::from(slave));
    // The child becomes a session leader and adopts the pty as its
    // controlling terminal -- without this the CLI still sees "not a tty".
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            // A wide terminal, or the CLI wraps the OAuth URL across lines at
            // column 80 and the scraper reads back a truncated address --
            // measured, not imagined.
            let size = libc::winsize {
                ws_row: 40,
                ws_col: 400,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            libc::ioctl(0, libc::TIOCSWINSZ as _, &size);
            Ok(())
        });
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not run the agent CLI: {e}"))?;

    let master_file = std::fs::File::from(master);
    let reader = master_file.try_clone().map_err(|e| e.to_string())?;

    *slot = Some(Flow {
        state: FlowState::Starting,
        output: String::new(),
        master: Some(master_file),
        child: Some(child),
        started: Instant::now(),
    });
    drop(slot);

    // One thread reads the pty until it closes, folding output into the slot
    // and promoting the state as landmarks appear. A thread, not a task: the
    // read is a blocking fd read, and this happens at most a handful of times
    // in a server's life.
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Ok(mut slot) = FLOW.lock()
                        && let Some(f) = slot.as_mut()
                    {
                        f.output.push_str(&strip_ansi(&chunk));
                        if matches!(f.state, FlowState::Starting)
                            && let Some(url) = find_url(&f.output)
                        {
                            f.state = FlowState::UrlReady(url);
                        }
                    }
                }
            }
        }
        // The pty closed: the CLI is done, one way or the other.
        if let Ok(mut slot) = FLOW.lock()
            && let Some(f) = slot.as_mut()
        {
            let ok = f
                .child
                .take()
                .and_then(|mut c| c.wait().ok())
                .map(|s| s.success())
                .unwrap_or(false);
            f.master = None;
            f.state = if ok {
                FlowState::Done
            } else {
                let tail: String = f
                    .output
                    .chars()
                    .rev()
                    .take(400)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                FlowState::Failed(tail)
            };
        }
    });

    Ok(())
}

/// Forward the pasted authorization code to the CLI.
pub fn submit_code(code: &str) -> Result<(), String> {
    let mut slot = FLOW.lock().map_err(|_| "flow state poisoned".to_string())?;
    let Some(f) = slot.as_mut() else {
        return Err("no sign-in flow is running".into());
    };
    let Some(master) = f.master.as_mut() else {
        return Err("the flow is no longer accepting input".into());
    };
    master
        .write_all(format!("{}\r", code.trim()).as_bytes())
        .and_then(|_| master.flush())
        .map_err(|e| format!("could not hand the code to the CLI: {e}"))?;
    f.state = FlowState::Exchanging;
    Ok(())
}

/// A snapshot for the interface, and the economy refresh on completion.
pub async fn status(state: &AppState) -> serde_json::Value {
    let snapshot = {
        let slot = FLOW.lock().ok();
        slot.and_then(|s| {
            s.as_ref()
                .map(|f| (f.state.clone(), f.started.elapsed().as_secs()))
        })
    };
    match snapshot {
        None => serde_json::json!({ "state": "idle" }),
        Some((FlowState::Starting, secs)) => {
            serde_json::json!({ "state": "starting", "seconds": secs })
        }
        Some((FlowState::UrlReady(url), _)) => {
            serde_json::json!({ "state": "url_ready", "url": url })
        }
        Some((FlowState::Exchanging, _)) => serde_json::json!({ "state": "exchanging" }),
        Some((FlowState::Done, _)) => {
            // The proof is the economy, not the exit code: re-detect and let
            // the sign-in notice disappear because the CLI is now signed in.
            let economy = crate::economy::detect(&state.config).await;
            state.set_economy(economy.clone());
            serde_json::json!({
                "state": "done",
                "economy": crate::economy::as_json(&economy),
            })
        }
        Some((FlowState::Failed(tail), _)) => {
            serde_json::json!({ "state": "failed", "tail": tail })
        }
    }
}
