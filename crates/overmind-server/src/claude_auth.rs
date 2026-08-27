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
    /// The CLI rejected the code and is prompting again (measured 27 Aug
    /// 2026: an invalid code does not end the process — it re-prompts, the
    /// pty stays open, and without this state the interface spun on
    /// "checking the code" forever with no way out).
    CodeRejected(String),
    /// The CLI hit an OAuth error and offered "Press Enter to retry" — the
    /// retry mints a FRESH authorization URL (new PKCE challenge), so the old
    /// link is dead. Enter has been pressed for the person; waiting for the
    /// new URL to appear. (Measured 27 Aug 2026 on a friend's install: a 400
    /// on the code exchange lands here, and re-offering the old URL loops on
    /// 400 forever.)
    Restarting(String),
    /// The CLI exited successfully and the economy was re-detected.
    Done,
    /// The CLI exited without succeeding; the tail of its output rides along.
    Failed(String),
}

pub struct Flow {
    pub state: FlowState,
    /// The authorization URL once seen — kept so a rejected code can
    /// re-offer the same page and paste box instead of a dead spinner.
    url: Option<String>,
    /// Everything the CLI printed, ANSI stripped -- the failure tail comes
    /// from here, and the URL is scraped out of it.
    output: String,
    /// The same bytes, unstripped. The token is scraped from HERE: the ANSI
    /// stripper ate exactly one character of the first stored token (the `o`
    /// of `oat01`, lost to a TUI redraw sequence), and a credential is the
    /// one string that must never pass through a lossy cleaner.
    raw: String,
    /// Write end of the pty: where the pasted code goes.
    master: Option<std::fs::File>,
    child: Option<std::process::Child>,
    started: Instant,
    /// Byte offset into `output` from which the URL is scraped. Reset when
    /// the CLI restarts its flow: the URL to offer is the one printed AFTER
    /// the restart, never the first one in the transcript.
    scan_from: usize,
    /// Byte offset into `output` at the moment a code was submitted; the
    /// rejection/retry recognizers read only what the CLI said after that.
    exchange_from: usize,
    /// A refusal the person should still see once the fresh URL is up.
    rejected_note: Option<String>,
    /// Seconds this machine's clock differs from the world's, measured once
    /// per flow — the one fact that turns "every code answers 400" from a
    /// mystery into a sentence.
    skew: Option<i64>,
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

/// Where the long-lived token lives once a sign-in has produced one.
///
/// `setup-token` does not store a credential: it **prints** one, meant for
/// the `CLAUDE_CODE_OAUTH_TOKEN` environment variable -- measured live, the
/// day the first person completed the flow and the economy still answered
/// "not signed in". So Overmind keeps the token itself: one file under the
/// data dir, 0600, injected into every agent spawn the way an API key rides
/// the environment.
pub fn token_path(config: &crate::db::Config) -> std::path::PathBuf {
    config.data_dir.join("claude-oauth-token")
}

/// The stored token, if any. Read per spawn: spawns are rare, and a cached
/// copy would survive a revocation the file did not.
pub fn stored_token(config: &crate::db::Config) -> Option<String> {
    let t = std::fs::read_to_string(token_path(config)).ok()?;
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// Scrape the token out of the CLI's **raw** output.
///
/// The whole transcript is walked as escape-free runs of token characters —
/// an escape sequence anywhere (including INSIDE the `sk-ant-` prefix) is
/// skipped, never allowed to end the run or eat a character. Twice burned:
/// the ANSI stripper ate the `o` of `oat01` on the first live sign-in, and
/// on the first fresh-machine sign-in (27 Aug 2026) the anchor `sk-ant-oat`
/// itself failed to match — a redraw had landed inside it, the CLI said
/// "token created successfully", and the flow reported failure over a token
/// it was holding. So: no long anchor in the raw bytes, any `sk-ant-*`
/// subtype, longest qualifying run wins.
fn find_token(raw: &str) -> Option<String> {
    let mut best: Option<String> = None;
    let mut run = String::new();
    let mut chars = raw.chars().peekable();
    let consider = |run: &mut String, best: &mut Option<String>| {
        // The prefix may sit mid-run: a cursor-positioned label glued to the
        // token by a skipped escape ("token" ESC[5C "sk-ant-…") must not
        // hide it.
        if let Some(pos) = run.find("sk-ant-") {
            let tok = &run[pos..];
            if tok.len() > 40 && best.as_ref().map(|b| tok.len() > b.len()).unwrap_or(true) {
                *best = Some(tok.to_string());
            }
        }
        run.clear();
    };
    while let Some(c) = chars.next() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            run.push(c);
        } else if c == '\u{1b}' {
            // A redraw mid-run: skip the sequence, keep collecting.
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for d in chars.by_ref() {
                        if ('@'..='~').contains(&d) {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else {
            consider(&mut run, &mut best);
        }
    }
    consider(&mut run, &mut best);
    best
}

/// Blot credentials out of text that is about to be logged or shown.
///
/// Learned the hard way (27 Aug 2026): "its last words" on a failed token
/// scrape carried the token itself — the one the scraper had not recognized
/// — into `docker compose logs` and a pasted bug report. Anything shaped
/// like a long `sk-ant-…` run is replaced before the text leaves this
/// module; a lost character or an unknown subtype must not defeat it, so
/// the match is the shape, not an exact prefix list.
fn scrub_secrets(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let is_tok = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        // A run of token characters containing "sk-ant" and long enough to
        // be a credential is not for anyone's eyes.
        if is_tok(chars[i]) {
            let start = i;
            while i < chars.len() && is_tok(chars[i]) {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            if run.len() > 20 && run.contains("sk-ant") {
                out.push_str("sk-ant-…[redacted]");
            } else {
                out.push_str(&run);
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// How far this machine's clock is from the world's, in seconds — positive
/// when this clock runs ahead, negative when it lags. Judged against the
/// `Date` header of the API the CLI itself talks to (no new party learns
/// anything); `None` when the check cannot run — offline is not a skew.
///
/// Why it lives here: OAuth codes are minutes-lived, and Docker Desktop's VM
/// wakes from host sleep with its clock frozen at the moment of sleep. Every
/// code then answers 400, first paste included, and nothing in the flow says
/// why (measured 27 Aug 2026 on a friend's install — the friend's container
/// was minutes behind the world). `curl` rather than an HTTP client crate:
/// the image carries it already for the healthcheck, and the economy
/// detector set the house pattern of shelling out for a fact.
pub async fn clock_skew_secs() -> Option<i64> {
    let out = tokio::process::Command::new("curl")
        .args(["-sI", "-m", "5", "https://api.anthropic.com/"])
        .output()
        .await
        .ok()?;
    skew_from(
        &String::from_utf8_lossy(&out.stdout),
        std::time::SystemTime::now(),
    )
}

/// The skew, from raw response headers and a local "now". Pure for the test.
fn skew_from(headers: &str, now: std::time::SystemTime) -> Option<i64> {
    let value = headers.lines().find_map(|l| {
        let (name, v) = l.split_once(':')?;
        name.trim().eq_ignore_ascii_case("date").then(|| v.trim())
    })?;
    let server = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let now = chrono::DateTime::<chrono::Utc>::from(now);
    Some((now - server.with_timezone(&chrono::Utc)).num_seconds())
}

/// The first `https://` URL in the output that looks like the OAuth page.
/// The last lines of output, for the interface: what the CLI is saying
/// *right now*. Spinner leftovers and blank lines trimmed. Measured need:
/// the first person to use the flow sat on "Exchanging…" while the CLI was
/// saying something nobody could see.
fn tail(text: &str, max_chars: usize) -> String {
    let cleaned: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| l.trim().len() > 1)
        .collect();
    let joined = cleaned.join("\n");
    match joined.char_indices().rev().nth(max_chars.saturating_sub(1)) {
        Some((i, _)) => joined[i..].to_string(),
        None => joined,
    }
}

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

/// The argument vector that runs `program args…` tethered to `parent`'s
/// life: a shell whose background watchdog polls the parent and, the moment
/// it is gone, TERMs then KILLs the program -- which runs in the foreground
/// *as the shell itself* (`exec`), so the pid the watchdog holds is the
/// program's, the pty and session leadership pass through untouched, and a
/// program that simply finishes hands back its own exit status.
///
/// Why a shell and not `kill_on_drop`: the child calls `setsid()` and
/// therefore leaves the server's process group, and nothing in-process can
/// fire once the server is gone. Measured live (22 Aug): a dozen
/// `setup-token` processes outlived their servers, each reopening the owner's
/// browser. `prctl(PR_SET_PDEATHSIG)` would do it on Linux alone; this does
/// it wherever there is a `sh`.
pub(crate) fn tethered(parent: u32, program: &str, args: &[&str]) -> Vec<String> {
    // The watchdog lets go of the pty (its fds go to /dev/null) so the
    // server's reader sees EOF the moment the program exits, and it only
    // ever signals when the parent is actually gone -- never on the
    // program's own exit, where the pid could already belong to someone else.
    // `alive` cannot be `kill -0` alone: in the image the program runs as the
    // agent uid and the server is root, so `kill -0` answers EPERM for a
    // parent that is perfectly alive -- there, `/proc` is the witness.
    const SCRIPT: &str = r#"parent=$1; shift
alive() { kill -0 "$1" 2>/dev/null || [ -d "/proc/$1" ]; }
( while alive "$parent" && alive $$; do sleep 1; done
  alive "$parent" || { kill -TERM $$ 2>/dev/null; sleep 2; kill -KILL $$ 2>/dev/null; }
) </dev/null >/dev/null 2>&1 &
exec "$@""#;
    let mut argv = vec![
        "sh".to_string(),
        "-c".to_string(),
        SCRIPT.to_string(),
        "sh".to_string(),
        parent.to_string(),
        program.to_string(),
    ];
    argv.extend(args.iter().map(|a| a.to_string()));
    argv
}

/// Begin the sign-in: spawn `claude setup-token` as the agent on a pty.
///
/// Answers with an error string when a flow is already running or the spawn
/// fails; the caller turns that into an HTTP status.
pub fn start(state: &AppState) -> Result<(), String> {
    // A custom adapter is not the Claude CLI, and `setup-token` is not a
    // contract it signed -- the same honesty the economy detector applies.
    // Measured on the owner's desk: without this, the door suite (which runs
    // with a custom agent command) still spawned the REAL CLI on a machine
    // that has one, and every `cargo test` opened the browser on an OAuth
    // page -- a dozen times a day, for two days, before anyone saw why.
    if state.config.agent_cmd.is_some() {
        return Err(
            "a custom OVERMIND_AGENT_CMD is configured; the subscription sign-in is for the Claude CLI"
                .into(),
        );
    }
    let mut slot = FLOW.lock().map_err(|_| "flow state poisoned".to_string())?;
    if let Some(f) = slot.as_mut() {
        let stale = f.started.elapsed() > Duration::from_secs(600)
            || matches!(
                f.state,
                FlowState::Done | FlowState::Failed(_) | FlowState::CodeRejected(_)
            )
            // A flow stuck mid-exchange (or mid-restart) is a flow the person
            // cannot use: after two minutes a fresh click means "start over",
            // not "adopt the zombie" (measured: retry adopted it, nothing
            // moved).
            || (matches!(f.state, FlowState::Exchanging | FlowState::Restarting(_))
                && f.started.elapsed() > Duration::from_secs(120));
        if !stale {
            // Idempotent: the second click adopts the flow the first one
            // started, instead of being told off for it.
            return Ok(());
        }
        // A stale flow's CLI must die with it: measured live, a replaced
        // flow left its `setup-token` running for an hour, ignoring the
        // pty it had lost.
        if let Some(mut c) = f.child.take() {
            let _ = c.kill();
            let _ = c.wait();
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

    // Tethered to this server's life: see `tethered`.
    let argv = tethered(std::process::id(), "claude", &["setup-token"]);
    let mut cmd = crate::sandbox::as_agent_std(&state.config, &argv[0]);
    cmd.args(&argv[1..])
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
    let token_file = token_path(&state.config);

    eprintln!("claude sign-in: `claude setup-token` spawned on a pty");
    *slot = Some(Flow {
        state: FlowState::Starting,
        url: None,
        output: String::new(),
        raw: String::new(),
        master: Some(master_file),
        child: Some(child),
        started: Instant::now(),
        scan_from: 0,
        exchange_from: 0,
        rejected_note: None,
        skew: None,
    });
    drop(slot);

    // Measure the clock once per flow, off the request path. A skewed clock
    // dooms every code before the person pastes the first one — say so in
    // the log and hand the number to the interface.
    tokio::spawn(async move {
        if let Some(s) = clock_skew_secs().await {
            if s.abs() > 120 {
                eprintln!(
                    "claude sign-in: this machine's clock is {}s {} the world — OAuth codes will be refused until it is fixed (Docker Desktop: restart it; its VM wakes from sleep with a frozen clock)",
                    s.abs(),
                    if s > 0 { "ahead of" } else { "behind" }
                );
            }
            if let Ok(mut slot) = FLOW.lock()
                && let Some(f) = slot.as_mut()
            {
                f.skew = Some(s);
            }
        }
    });

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
                        f.raw.push_str(&chunk);
                        f.output.push_str(&strip_ansi(&chunk));
                        if matches!(f.state, FlowState::Starting | FlowState::Restarting(_))
                            && let Some(url) = find_url(&f.output[f.scan_from..])
                        {
                            eprintln!("claude sign-in: authorization URL ready");
                            f.url = Some(url.clone());
                            f.state = FlowState::UrlReady(url);
                        }
                        // An invalid code does not end the CLI — it re-prompts
                        // (same url), or hits an OAuth error and offers "Press
                        // Enter to retry" (fresh url after the Enter). Catch
                        // the words as they accumulate, or the interface spins
                        // on "exchanging" forever.
                        if matches!(f.state, FlowState::Exchanging) {
                            match exchange_verdict(&f.output[f.exchange_from..]) {
                                ExchangeVerdict::Wait => {}
                                ExchangeVerdict::Rejected => {
                                    eprintln!(
                                        "claude sign-in: the CLI refused the code and is re-prompting on the same URL"
                                    );
                                    f.state = FlowState::CodeRejected(scrub_secrets(&tail(
                                        &f.output, 200,
                                    )));
                                }
                                ExchangeVerdict::Restart => {
                                    // The CLI's own retry: give it the Enter it
                                    // asked for, and scrape only what it prints
                                    // from here on — the old link is dead.
                                    let why =
                                        scrub_secrets(&tail(&f.output[f.exchange_from..], 200));
                                    eprintln!(
                                        "claude sign-in: OAuth error from the CLI — pressing Enter for a fresh URL ({})",
                                        why.replace('\n', " · ")
                                    );
                                    if let Some(m) = f.master.as_mut() {
                                        let _ = m.write_all(b"\r").and_then(|_| m.flush());
                                    }
                                    f.scan_from = f.output.len();
                                    f.url = None;
                                    f.rejected_note = Some(why.clone());
                                    f.state = FlowState::Restarting(why);
                                }
                            }
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
                // The flow's whole point: keep the token the CLI printed.
                match find_token(&f.raw) {
                    Some(tok) => {
                        if let Err(e) = write_token(&token_file, &tok) {
                            eprintln!("claude sign-in: token could not be stored: {e}");
                            FlowState::Failed(format!(
                                "the sign-in succeeded but the token could not be stored: {e}"
                            ))
                        } else {
                            eprintln!(
                                "claude sign-in: done, token stored at {}",
                                token_file.display()
                            );
                            FlowState::Done
                        }
                    }
                    // A clean exit without a credential is still a failure —
                    // and the CLI's last words are the only clue anyone has.
                    // SCRUBBED: "none was found" once meant "the scraper did
                    // not recognize it", and the unrecognized token rode this
                    // very message into the logs (27 Aug 2026).
                    None => {
                        let why = format!(
                            "the CLI finished but no token appeared in its output; its last words:\n{}",
                            scrub_secrets(&tail(&f.output, 400))
                        );
                        eprintln!("claude sign-in: {}", why.replace('\n', " · "));
                        FlowState::Failed(why)
                    }
                }
            } else {
                let words: String = f
                    .output
                    .chars()
                    .rev()
                    .take(400)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let words = scrub_secrets(&words);
                eprintln!(
                    "claude sign-in: the CLI exited unsuccessfully: {}",
                    words.replace('\n', " · ")
                );
                FlowState::Failed(words)
            };
        }
    });

    Ok(())
}

/// Write the token where only the server can read it.
fn write_token(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(token.as_bytes())
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
    // The code and the Enter travel separately, with a beat between them.
    // The CLI's raw-mode input treats a rapid burst as one paste event, and a
    // carriage return glued to the end of the burst is swallowed *into* the
    // paste instead of registering as the Enter key -- measured live: the
    // prompt showed a full line of asterisks and then sat there.
    if matches!(f.state, FlowState::Done) {
        return Err("the sign-in already completed".into());
    }
    eprintln!(
        "claude sign-in: forwarding a pasted code ({} chars) to the CLI",
        code.trim().chars().count()
    );
    master
        .write_all(code.trim().as_bytes())
        .and_then(|_| master.flush())
        .map_err(|e| format!("could not hand the code to the CLI: {e}"))?;
    std::thread::sleep(Duration::from_millis(300));
    master
        .write_all(b"\r")
        .and_then(|_| master.flush())
        .map_err(|e| format!("could not press Enter for the CLI: {e}"))?;
    // The recognizers judge only what the CLI says from here on — an old
    // "Invalid code" further up the transcript must not condemn this one.
    f.exchange_from = f.output.len();
    f.state = FlowState::Exchanging;
    Ok(())
}

/// A snapshot for the interface, and the economy refresh on completion.
pub async fn status(state: &AppState) -> serde_json::Value {
    let snapshot = {
        let slot = FLOW.lock().ok();
        slot.and_then(|s| {
            s.as_ref().map(|f| {
                (
                    f.state.clone(),
                    f.started.elapsed().as_secs(),
                    // Scrubbed: the live tail streams the CLI's words to the
                    // interface, and on the success path those words include
                    // the token.
                    scrub_secrets(&tail(&f.output, 400)),
                    f.url.clone(),
                    f.rejected_note.clone(),
                    f.skew,
                )
            })
        })
    };
    match snapshot {
        None => serde_json::json!({ "state": "idle" }),
        Some((FlowState::Starting, secs, t, _, _, skew)) => {
            serde_json::json!({ "state": "starting", "seconds": secs, "tail": t, "clock_skew_secs": skew })
        }
        Some((FlowState::UrlReady(url), _, t, _, rejected, skew)) => {
            serde_json::json!({ "state": "url_ready", "url": url, "tail": t, "rejected": rejected, "clock_skew_secs": skew })
        }
        Some((FlowState::Exchanging, _, t, _, _, skew)) => {
            serde_json::json!({ "state": "exchanging", "tail": t, "clock_skew_secs": skew })
        }
        Some((FlowState::CodeRejected(why), _, _, url, _, skew)) => {
            serde_json::json!({ "state": "code_rejected", "tail": why, "url": url, "clock_skew_secs": skew })
        }
        Some((FlowState::Restarting(why), _, _, _, _, skew)) => {
            serde_json::json!({ "state": "restarting", "tail": why, "clock_skew_secs": skew })
        }
        Some((FlowState::Done, _, _, _, _, _)) => {
            // The proof is the economy, not the exit code: re-detect and let
            // the sign-in notice disappear because the CLI is now signed in.
            let economy = crate::economy::detect(&state.config).await;
            state.set_economy(economy.clone());
            serde_json::json!({
                "state": "done",
                "economy": crate::economy::as_json(&economy),
            })
        }
        Some((FlowState::Failed(t), _, _, _, _, skew)) => {
            serde_json::json!({ "state": "failed", "tail": t, "clock_skew_secs": skew })
        }
    }
}

/// Do these CLI words mean "that code was not accepted, try again"?
/// Matched against the words the setup-token flow actually prints; broad on
/// purpose — a false positive re-offers the paste box, a false negative is
/// an eternal spinner.
pub(crate) fn code_rejected_in(chunk: &str) -> bool {
    let lower = chunk.to_lowercase();
    [
        "invalid",
        "not valid",
        "expired",
        "try again",
        "denied",
        "error",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

/// Lowercased, letters and digits only. The TUI positions words with cursor
/// moves instead of spaces, so after the ANSI strip a real transcript reads
/// "PressEntertoretry." — matching must not depend on whitespace (measured
/// 27 Aug 2026, verbatim from a friend's install).
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// What the CLI's words since the code was submitted mean for the flow.
#[derive(Debug, PartialEq)]
pub(crate) enum ExchangeVerdict {
    /// Nothing conclusive yet; keep reading.
    Wait,
    /// "Invalid code…" — the CLI re-prompts on the SAME url; re-offer the
    /// paste box.
    Rejected,
    /// "OAuth error: … Press Enter to retry." — the retry regenerates the
    /// PKCE challenge, so the CLI must be given its Enter and the NEXT url
    /// scraped; the old one is dead.
    Restart,
}

/// Decide from everything the CLI printed since the code went in. Reading
/// the accumulated text (not the arriving chunk) matters twice over: the
/// error line and the retry prompt can land in different reads, and an
/// "OAuth error" seen alone must WAIT for its retry prompt rather than be
/// mistaken for the same-url re-prompt shape.
pub(crate) fn exchange_verdict(since: &str) -> ExchangeVerdict {
    let q = squash(since);
    if q.contains("oautherror") {
        if q.contains("entertoretry") {
            ExchangeVerdict::Restart
        } else {
            ExchangeVerdict::Wait
        }
    } else if code_rejected_in(since) {
        ExchangeVerdict::Rejected
    } else {
        ExchangeVerdict::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::{ExchangeVerdict, code_rejected_in, exchange_verdict, find_token, find_url};

    /// Measured 27 Aug 2026: a wrong code left the CLI re-prompting on an
    /// open pty; without recognizing its words the flow spun forever and
    /// Retry adopted the zombie.
    #[test]
    fn a_rejected_code_is_recognized_and_a_prompt_is_not() {
        assert!(code_rejected_in(
            "Invalid authorization code. Please try again:"
        ));
        assert!(code_rejected_in("OAuth error: code expired"));
        assert!(!code_rejected_in("Paste the code from the browser:"));
        assert!(!code_rejected_in("Browser didn't open? Use the url below"));
    }

    /// Verbatim from a friend's install (27 Aug 2026), spaces already eaten
    /// by the TUI's cursor-positioned redraw: an OAuth 400 offers "Press
    /// Enter to retry", and the retry mints a FRESH url — this must restart
    /// the scrape, never re-offer the old link (which loops on 400 forever).
    #[test]
    fn an_oauth_error_with_a_retry_prompt_restarts_the_flow() {
        assert_eq!(
            exchange_verdict("OAuth error: Requstfailed withstatus code 400\nPressEntertoretry."),
            ExchangeVerdict::Restart
        );
        // The error line can arrive a read before its retry prompt: wait for
        // the prompt instead of misreading the shape as a same-url re-prompt.
        assert_eq!(
            exchange_verdict("OAuth error: Request failed with status code 400"),
            ExchangeVerdict::Wait
        );
    }

    /// The same-url shape stays a rejection, and the CLI's own prompts stay
    /// nothing at all.
    #[test]
    fn an_invalid_code_reoffers_the_same_url_and_a_prompt_does_nothing() {
        assert_eq!(
            exchange_verdict("Invalid code. Please make sure the full code was copied"),
            ExchangeVerdict::Rejected
        );
        assert_eq!(
            exchange_verdict("Paste code here if prompted > "),
            ExchangeVerdict::Wait
        );
    }

    /// A machine minutes behind the world (a Docker Desktop VM woken from
    /// host sleep) must be measured as such from the `Date` header — and the
    /// timezone must not be able to lie: both sides are judged in UTC.
    #[test]
    fn a_frozen_clock_is_measured_and_a_true_clock_reads_zero() {
        use super::skew_from;
        // (27 Aug 2026 is a Thursday — chrono validates the weekday, and a
        // wrong one is a parse error, not a shrug.)
        let headers = "HTTP/2 200\r\ndate: Thu, 27 Aug 2026 21:50:00 GMT\r\nserver: x\r\n";
        // The machine believes it is 21:44:30 UTC — five and a half minutes
        // behind the world.
        let local =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_787_867_070);
        assert_eq!(skew_from(headers, local), Some(-330));
        // The same instant on both sides: no skew.
        let honest =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_787_867_400);
        assert_eq!(skew_from(headers, honest), Some(0));
        // No Date header, no verdict — offline is not a skew.
        assert_eq!(skew_from("HTTP/2 200\r\nserver: x\r\n", honest), None);
    }

    /// After a restart the transcript holds two urls; the scrape must offer
    /// the one printed after the restart point, never the dead first one.
    #[test]
    fn the_url_after_the_restart_wins() {
        let before = "Open: https://claude.ai/oauth/authorize?state=OLD\n";
        let after = "Open: https://claude.ai/oauth/authorize?state=NEW\n";
        let output = format!("{before}OAuth error…{after}");
        let scan_from = output.len() - after.len();
        assert_eq!(
            find_url(&output[scan_from..]).as_deref(),
            Some("https://claude.ai/oauth/authorize?state=NEW")
        );
    }

    /// The shape that corrupted the first stored token, reconstructed: a CSI
    /// sequence landing mid-token must be skipped, not allowed to end the
    /// run or eat a character.
    #[test]
    fn a_redraw_inside_the_token_does_not_corrupt_it() {
        let raw = "token: sk-ant-oat01-abc\u{1b}[2Kdef-ghi_jkl0123456789012345678901234567890 done";
        assert_eq!(
            find_token(raw).as_deref(),
            Some("sk-ant-oat01-abcdef-ghi_jkl0123456789012345678901234567890")
        );
    }

    /// Too-short matches (fragments of a wrapped echo) are refused rather
    /// than stored as a credential that will 401 later.
    #[test]
    fn a_fragment_is_not_a_token() {
        assert_eq!(find_token("sk-ant-oat01-tooshort"), None);
    }

    /// The first fresh-machine sign-in (27 Aug 2026): the CLI said "token
    /// created successfully", but a redraw had landed INSIDE the old anchor
    /// and the subtype differed — the flow declared failure over a token it
    /// was holding. Escapes inside the prefix, unknown subtypes, and a label
    /// glued on by a cursor move must all still yield the credential.
    #[test]
    fn a_token_survives_a_redraw_inside_its_own_prefix_and_any_subtype() {
        let tail = "3BrYJsbWQSRjWgqoWSud8cWu6nFBNIqo9F19xKrQsFBZ";
        // A redraw between "sk-ant-" and the subtype.
        let raw = format!("token:\nsk-ant-\u{1b}[2Kat01-{tail}\nStore this");
        assert_eq!(
            find_token(&raw).as_deref(),
            Some(&*format!("sk-ant-at01-{tail}"))
        );
        // The eaten-character shape: the escape lands mid-"oat01".
        let raw = format!("sk-ant-o\u{1b}[0mat01-{tail} done");
        assert_eq!(
            find_token(&raw).as_deref(),
            Some(&*format!("sk-ant-oat01-{tail}"))
        );
        // A cursor-positioned label glued straight onto the token.
        let raw = format!("token\u{1b}[5Csk-ant-at01-{tail}\n");
        assert_eq!(
            find_token(&raw).as_deref(),
            Some(&*format!("sk-ant-at01-{tail}"))
        );
    }

    /// What leaves this module carries no credential: the very failure
    /// message that once ferried an unrecognized token into the logs must
    /// blot it out, while ordinary words pass untouched.
    #[test]
    fn a_credential_is_scrubbed_from_outbound_text() {
        let text = "Your OAuth token:\nsk-ant-at01-3BrYJsbWQSRjWgqoWSud8cWu6nFBNIqo9F19xKrQsFBZ\nStore this token securely.";
        let scrubbed = super::scrub_secrets(text);
        assert!(!scrubbed.contains("3BrYJsbW"), "token survived: {scrubbed}");
        assert!(scrubbed.contains("sk-ant-…[redacted]"));
        assert!(scrubbed.contains("Store this token securely."));
        // A bare mention of the prefix is prose, not a credential.
        assert_eq!(
            super::scrub_secrets("set sk-ant-… as the key"),
            "set sk-ant-… as the key"
        );
    }

    fn spawn_tethered(parent: u32, program: &str, args: &[&str]) -> std::process::Child {
        let argv = super::tethered(parent, program, args);
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn tethered command")
    }

    fn exits_within(
        child: &mut std::process::Child,
        secs: u64,
    ) -> Option<std::process::ExitStatus> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                return Some(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        None
    }

    /// Measured live (22 Aug): a dozen `setup-token` processes outlived the
    /// servers that spawned them, each reopening the browser. The CLI must
    /// die with the server -- whatever killed the server, timeout or not.
    #[test]
    fn the_cli_dies_with_the_server_that_started_it() {
        let mut fake_server = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("a stand-in for the server");
        let mut cli = spawn_tethered(fake_server.id(), "sleep", &["60"]);
        // Still alive while the server is.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(
            cli.try_wait().expect("poll").is_none(),
            "the cli died too early"
        );

        fake_server.kill().expect("kill the stand-in");
        let _ = fake_server.wait();
        assert!(
            exits_within(&mut cli, 8).is_some(),
            "the cli outlived the server"
        );
    }

    /// The tether must be invisible to a CLI that simply finishes: its exit
    /// status is the flow's verdict, and the wrapper must not launder it.
    #[test]
    fn a_tethered_cli_keeps_its_own_exit_status() {
        let mut cli = spawn_tethered(std::process::id(), "sh", &["-c", "exit 7"]);
        let status = exits_within(&mut cli, 8).expect("a finished cli exits");
        assert_eq!(status.code(), Some(7));
    }
}
