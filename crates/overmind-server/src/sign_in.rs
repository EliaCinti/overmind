//! Connecting an account, from the product (M23, ADR-0051 decision 5).
//!
//! The owner's words: "il collegamento all'account deve avvenire da Overmind,
//! non da terminale". Before M23, connecting a subscription meant knowing that
//! a CLI lives inside the container, that it has a `setup-token` command, and
//! that `docker compose exec` wants a TTY -- three facts a person evaluating
//! the product has no reason to hold.
//!
//! **How an account is connected is the provider's to say.** Each provider
//! answers [`Provider::sign_in`] with one of three shapes: *paste-back* (a CLI
//! prints a URL, the person pastes a code back, the CLI prints the
//! credential), *key* (the person pastes a key, and nothing here runs), or
//! *unavailable*, with the reason in words. This module is the paste-back
//! machinery, which was Claude Code's alone until ADR-0051 moved what is
//! Claude's -- the token's prefix, the OAuth page's host, the words
//! `setup-token` prints, the API whose clock is trusted -- into its provider.
//!
//! The flow is the CLI's own OAuth dance, orchestrated: the server spawns the
//! provider's command **as the agent** on a pseudo-terminal (the CLI refuses
//! to draw its prompt on a pipe), reads its output until the authorization URL
//! appears, hands that URL to the browser, and forwards the code the person
//! pastes back. What the CLI prints at the end becomes an account
//! ([`crate::accounts::link`]), and the economy is re-detected **once**, when
//! the flow ends -- never on a poll.
//!
//! One flow at a time, ten-minute lifetime, owner only: this is a setup
//! surface, not a work surface.

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::accounts::Kind;
use crate::db::AppState;
use crate::provider::Provider;

/// How a provider connects one kind of account.
pub enum SignIn {
    /// A CLI is driven on a pty: URL out, code in, credential out.
    PasteBack(PasteBack),
    /// A key is pasted into a field. Nothing runs.
    Key,
    /// This kind cannot be connected for this provider, and why -- said, so
    /// the panel can list it rather than hide it.
    Unavailable(&'static str),
}

impl SignIn {
    /// The shape as the API names it.
    pub fn shape(&self) -> &'static str {
        match self {
            SignIn::PasteBack(_) => "paste_back",
            SignIn::Key => "key",
            SignIn::Unavailable(_) => "unavailable",
        }
    }
}

/// Everything about one CLI's paste-back flow that is that CLI's knowledge.
///
/// Plain functions rather than a trait: each is a pure reading of the CLI's
/// own words, tested against transcripts that were observed, and none of them
/// needs anything but the text.
pub struct PasteBack {
    /// The command, and its arguments.
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// Is this `https://…` the page the person should open?
    pub is_auth_url: fn(&str) -> bool,
    /// The credential the CLI printed, scraped from its **raw** bytes.
    pub find_credential: fn(&str) -> Option<String>,
    /// What the CLI's words since a code was pasted mean.
    pub exchange_verdict: fn(&str) -> ExchangeVerdict,
    /// An address on the API the CLI talks to, whose `Date` header is the
    /// clock OAuth codes are judged by.
    pub clock_reference: &'static str,
}

/// What a CLI's words since the code was submitted mean for the flow.
#[derive(Debug, PartialEq)]
pub enum ExchangeVerdict {
    /// Nothing conclusive yet; keep reading.
    Wait,
    /// The code was refused and the CLI re-prompts on the SAME url; re-offer
    /// the paste box.
    Rejected,
    /// The CLI hit an error and offered a retry that mints a NEW url: it must
    /// be given its Enter and the NEXT url scraped; the old one is dead.
    Restart,
}

/// What starting a sign-in led to.
pub enum Begun {
    /// A paste-back flow is running; poll [`status`].
    PasteBack,
    /// Nothing to run: the interface shows a field for the key.
    Key,
}

/// Why a sign-in could not start.
#[derive(Debug, thiserror::Error)]
pub enum BeginError {
    /// The provider says this kind cannot be connected, and why.
    #[error("{0}")]
    Unavailable(&'static str),
    /// It could, but not here, or not now.
    #[error("{0}")]
    Refused(String),
}

/// Where the flow stands. Serialized into `GET /api/accounts/sign-in`.
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
    /// The CLI exited with a credential, which is an account now. What was
    /// learned at that moment rides along, so a poll reads it instead of
    /// asking the CLI again.
    Done {
        account: serde_json::Value,
        economy: serde_json::Value,
    },
    /// The CLI exited without succeeding; the tail of its output rides along.
    Failed(String),
}

pub struct Flow {
    /// Which flow this is. A worker started for generation N must not touch
    /// generation N+1 — see [`GENERATION`].
    generation: u64,
    /// Whose account this is becoming.
    provider: &'static str,
    pub state: FlowState,
    /// The authorization URL once seen — kept so a rejected code can
    /// re-offer the same page and paste box instead of a dead spinner.
    url: Option<String>,
    /// Everything the CLI printed, ANSI stripped -- the failure tail comes
    /// from here, and the URL is scraped out of it.
    output: String,
    /// The same bytes, unstripped. The credential is scraped from HERE: the
    /// ANSI stripper ate exactly one character of the first stored token (the
    /// `o` of `oat01`, lost to a TUI redraw sequence), and a credential is the
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

/// Which flow is in the slot, counted rather than compared by address.
///
/// Every background worker here — the reader thread, the clock probe — takes
/// the global lock and then reaches for `slot.as_mut()`, which hands it
/// **whatever flow is there now**, not the one it was started for. A retry
/// replaces the flow while the old reader is blocked on that very lock, so the
/// old thread would wake holding somebody else's flow: it took the new child
/// and called `wait()` on a live process, still holding the mutex, and every
/// later `status`, `submit_code` and `start` blocked behind it — forever, on
/// tokio workers, because the handlers call these synchronously.
///
/// A generation is the cheapest thing that makes "is this still mine?"
/// answerable. Checked at every point where a worker reaches into the slot.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How long a flow may live. Enforced on every poll as well as on the next
/// start: a flow somebody walked away from used to keep its child, its pty and
/// its thread until the server exited.
const LIFETIME: Duration = Duration::from_secs(600);

/// True while the flow in the slot is still the one `mine` was minted for.
fn still_mine(f: &Flow, mine: u64) -> bool {
    f.generation == mine
}

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

/// Blot credentials out of text that is about to be logged or shown: every
/// credential Overmind knows by value, and anything shaped like a token the
/// CLI has printed and nobody has scraped yet (ADR-0051 decision 6).
fn scrub_secrets(text: &str) -> String {
    crate::secrets::scrub(text)
}

/// How far this machine's clock is from the world's, in seconds — positive
/// when this clock runs ahead, negative when it lags. Judged against the
/// `Date` header of the API the CLI itself talks to — the provider names it,
/// so no new party learns anything — and `None` when the check cannot run:
/// offline is not a skew.
///
/// Why it lives here: OAuth codes are minutes-lived, and Docker Desktop's VM
/// wakes from host sleep with its clock frozen at the moment of sleep. Every
/// code then answers 400, first paste included, and nothing in the flow says
/// why (measured 27 Aug 2026 on a friend's install — the friend's container
/// was minutes behind the world). `curl` rather than an HTTP client crate:
/// the image carries it already for the healthcheck, and the economy
/// detector set the house pattern of shelling out for a fact.
pub async fn clock_skew_secs(reference: &str) -> Option<i64> {
    let out = tokio::process::Command::new("curl")
        .args(["-sI", "-m", "5", reference])
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

/// The first `https://` URL in the output that the provider says is its
/// authorization page.
fn find_url(text: &str, is_auth_url: fn(&str) -> bool) -> Option<String> {
    for start in text.match_indices("https://").map(|(i, _)| i) {
        let url: String = text[start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
            .collect();
        if is_auth_url(&url) {
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

/// Begin connecting an account of `kind` for `provider`, in whatever shape
/// the provider says.
pub fn begin(
    state: &AppState,
    provider: &'static dyn Provider,
    kind: Kind,
) -> Result<Begun, BeginError> {
    match provider.sign_in(kind) {
        SignIn::Key => Ok(Begun::Key),
        SignIn::Unavailable(why) => Err(BeginError::Unavailable(why)),
        SignIn::PasteBack(spec) => {
            // A custom adapter replaces this provider's CLI, and the CLI's
            // sign-in command is not a contract it signed -- the same honesty
            // the economy detector applies. Measured on the owner's desk:
            // without this, the door suite (which runs with a custom agent
            // command) still spawned the REAL CLI on a machine that has one,
            // and every `cargo test` opened the browser on an OAuth page.
            //
            // Asked here, of the paste-back shape alone, and only for the
            // provider the custom command stands in for: a key spawns
            // nothing, and another provider's CLI is not the one replaced.
            let replaced = state.config.agent_cmd.is_some()
                && provider.id() == crate::provider::current().id();
            if replaced && state.config.sign_in_stub.is_none() {
                return Err(BeginError::Refused(
                    "a custom OVERMIND_AGENT_CMD is configured; the subscription sign-in is for the provider's own CLI"
                        .into(),
                ));
            }
            start(state, provider, spec)
                .map(|()| Begun::PasteBack)
                .map_err(BeginError::Refused)
        }
    }
}

/// Spawn the provider's paste-back command as the agent on a pty.
fn start(state: &AppState, provider: &'static dyn Provider, spec: PasteBack) -> Result<(), String> {
    let mut slot = FLOW.lock().map_err(|_| "flow state poisoned".to_string())?;
    if let Some(f) = slot.as_mut() {
        let stale = f.started.elapsed() > LIFETIME
            || f.provider != provider.vendor()
            || matches!(
                f.state,
                FlowState::Done { .. } | FlowState::Failed(_) | FlowState::CodeRejected(_)
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
    // Emptied now, not when the new flow is installed: a spawn that fails
    // below would otherwise leave the killed flow in the slot, advertising a
    // URL nobody is listening behind.
    *slot = None;

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
    let stub = state
        .config
        .sign_in_stub
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    let (program, args): (&str, &[&str]) = match &stub {
        Some(p) => (p.as_str(), &[]),
        None => (spec.program, spec.args),
    };
    let argv = tethered(std::process::id(), program, args);
    // With no account's credential: a sign-in that ran as the active account
    // would be signing that account in again, or arguing with it.
    let none = crate::accounts::Spawn::none(provider);
    let mut cmd = crate::sandbox::as_agent_std(&state.config, &none, &argv[0]);
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
        .map_err(|e| format!("could not run the provider's CLI: {e}"))?;

    let master_file = std::fs::File::from(master);
    let reader = master_file.try_clone().map_err(|e| e.to_string())?;

    eprintln!("sign-in: {} spawned on a pty", provider.id());
    // Minted before the flow is installed and captured by every worker this
    // start spawns, so each can tell its own flow from a later one.
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    *slot = Some(Flow {
        generation,
        provider: provider.vendor(),
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
    let reference = spec.clock_reference;
    tokio::spawn(async move {
        if let Some(s) = clock_skew_secs(reference).await {
            if s.abs() > 120 {
                eprintln!(
                    "sign-in: this machine's clock is {}s {} the world — OAuth codes will be refused until it is fixed (Docker Desktop: restart it; its VM wakes from sleep with a frozen clock)",
                    s.abs(),
                    if s > 0 { "ahead of" } else { "behind" }
                );
            }
            if let Ok(mut slot) = FLOW.lock()
                && let Some(f) = slot.as_mut()
                && still_mine(f, generation)
            {
                f.skew = Some(s);
            }
        }
    });

    // The reader thread ends by turning a credential into an account, which
    // is async work; it runs it on the server's runtime from outside it.
    let runtime = tokio::runtime::Handle::current();
    let state = state.clone();

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
                        && still_mine(f, generation)
                    {
                        read_chunk(f, &spec, &chunk);
                    }
                }
            }
        }
        // The pty closed: the CLI is done, one way or the other. Only if the
        // slot still holds OUR flow -- and the child is taken out and reaped
        // with the lock released, so nothing else waits on a process exiting.
        let taken = match FLOW.lock() {
            Ok(mut slot) => match slot.as_mut() {
                Some(f) if still_mine(f, generation) => {
                    f.master = None;
                    Some((f.child.take(), f.raw.clone(), f.output.clone()))
                }
                _ => None,
            },
            Err(_) => None,
        };
        let Some((child, raw, output)) = taken else {
            return;
        };
        let ok = child
            .and_then(|mut c| c.wait().ok())
            .map(|s| s.success())
            .unwrap_or(false);
        let verdict = if ok {
            match (spec.find_credential)(&raw) {
                Some(credential) => {
                    // Known by value before anything else can print it.
                    crate::secrets::register(&credential);
                    match runtime.block_on(connected(&state, provider, &credential)) {
                        Ok((account, economy)) => {
                            eprintln!("sign-in: done, the account is connected");
                            FlowState::Done { account, economy }
                        }
                        Err(e) => {
                            eprintln!("sign-in: the account could not be stored: {e}");
                            FlowState::Failed(format!(
                                "the sign-in succeeded but the account could not be stored: {e}"
                            ))
                        }
                    }
                }
                // A clean exit without a credential is still a failure —
                // and the CLI's last words are the only clue anyone has.
                // SCRUBBED: "none was found" once meant "the scraper did
                // not recognize it", and the unrecognized token rode this
                // very message into the logs (27 Aug 2026).
                None => {
                    let why = format!(
                        "the CLI finished but no credential appeared in its output; its last words:\n{}",
                        tail(&scrub_secrets(&output), 400)
                    );
                    eprintln!("sign-in: {}", why.replace('\n', " · "));
                    FlowState::Failed(why)
                }
            }
        } else {
            let words = tail(&scrub_secrets(&output), 400);
            eprintln!(
                "sign-in: the CLI exited unsuccessfully: {}",
                words.replace('\n', " · ")
            );
            FlowState::Failed(words)
        };
        if let Ok(mut slot) = FLOW.lock()
            && let Some(f) = slot.as_mut()
            && still_mine(f, generation)
        {
            f.state = verdict;
        }
    });

    Ok(())
}

/// Fold one chunk of the CLI's output into the flow and move its state on.
fn read_chunk(f: &mut Flow, spec: &PasteBack, chunk: &str) {
    f.raw.push_str(chunk);
    f.output.push_str(&strip_ansi(chunk));
    if matches!(f.state, FlowState::Starting | FlowState::Restarting(_))
        && let Some(url) = find_url(&f.output[f.scan_from..], spec.is_auth_url)
    {
        eprintln!("sign-in: authorization URL ready");
        f.url = Some(url.clone());
        f.state = FlowState::UrlReady(url);
    }
    // An invalid code does not end the CLI — it re-prompts (same url), or
    // hits an OAuth error and offers "Press Enter to retry" (fresh url after
    // the Enter). Catch the words as they accumulate, or the interface spins
    // on "exchanging" forever.
    if matches!(f.state, FlowState::Exchanging) {
        match (spec.exchange_verdict)(&f.output[f.exchange_from..]) {
            ExchangeVerdict::Wait => {}
            ExchangeVerdict::Rejected => {
                eprintln!("sign-in: the CLI refused the code and is re-prompting on the same URL");
                f.state = FlowState::CodeRejected(tail(&scrub_secrets(&f.output), 200));
            }
            ExchangeVerdict::Restart => {
                // The CLI's own retry: give it the Enter it asked for, and
                // scrape only what it prints from here on — the old link is
                // dead.
                let why = tail(&scrub_secrets(&f.output[f.exchange_from..]), 200);
                eprintln!(
                    "sign-in: OAuth error from the CLI — pressing Enter for a fresh URL ({})",
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

/// The credential the CLI printed, made an account; and the economy, asked
/// once now so nobody has to ask it again on a poll.
async fn connected(
    state: &AppState,
    provider: &'static dyn Provider,
    credential: &str,
) -> Result<(serde_json::Value, serde_json::Value), crate::accounts::AccountError> {
    let label = crate::economy::label_for(state, provider, Kind::Subscription, credential).await;
    let account = crate::accounts::link(
        state,
        provider.vendor(),
        Kind::Subscription,
        credential,
        label.as_deref(),
    )
    .await?;
    let economy = crate::economy::detect_for(state).await;
    state.set_economy(economy.clone());
    Ok((
        serde_json::json!(account),
        crate::economy::as_json(&economy),
    ))
}

/// Forward the pasted authorization code to the CLI.
pub fn submit_code(code: &str) -> Result<(), String> {
    let (mut enter, generation) = {
        let mut slot = FLOW.lock().map_err(|_| "flow state poisoned".to_string())?;
        let Some(f) = slot.as_mut() else {
            return Err("no sign-in flow is running".into());
        };
        if matches!(f.state, FlowState::Done { .. }) {
            return Err("the sign-in already completed".into());
        }
        let Some(master) = f.master.as_mut() else {
            return Err("the flow is no longer accepting input".into());
        };
        eprintln!(
            "sign-in: forwarding a pasted code ({} chars) to the CLI",
            code.trim().chars().count()
        );
        master
            .write_all(code.trim().as_bytes())
            .and_then(|_| master.flush())
            .map_err(|e| format!("could not hand the code to the CLI: {e}"))?;
        let enter = master
            .try_clone()
            .map_err(|e| format!("could not keep the terminal for the Enter: {e}"))?;
        // The recognizers judge only what the CLI says from here on — an old
        // "Invalid code" further up the transcript must not condemn this one.
        f.exchange_from = f.output.len();
        f.state = FlowState::Exchanging;
        (enter, f.generation)
    };
    // The code and the Enter travel separately, with a beat between them.
    // The CLI's raw-mode input treats a rapid burst as one paste event, and a
    // carriage return glued to the end of the burst is swallowed *into* the
    // paste instead of registering as the Enter key -- measured live: the
    // prompt showed a full line of asterisks and then sat there.
    //
    // The beat is taken with the lock released: it used to be held across the
    // sleep, on a tokio worker, stalling every poll behind it.
    std::thread::sleep(Duration::from_millis(300));
    let still = FLOW
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|f| still_mine(f, generation)))
        .unwrap_or(false);
    if !still {
        return Err("the sign-in was replaced while the code was being sent".into());
    }
    enter
        .write_all(b"\r")
        .and_then(|_| enter.flush())
        .map_err(|e| format!("could not press Enter for the CLI: {e}"))
}

/// A snapshot for the interface. Reads what the flow learned; never runs
/// anything on behalf of a poll.
pub fn status() -> serde_json::Value {
    let snapshot = {
        let Ok(mut slot) = FLOW.lock() else {
            return serde_json::json!({ "state": "idle" });
        };
        let Some(f) = slot.as_mut() else {
            return serde_json::json!({ "state": "idle" });
        };
        // An abandoned flow ends here rather than at the next start.
        let finished = matches!(f.state, FlowState::Done { .. } | FlowState::Failed(_));
        if !finished && f.started.elapsed() > LIFETIME {
            if let Some(mut c) = f.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            f.master = None;
            f.state = FlowState::Failed(
                "the sign-in was left unfinished for ten minutes and has been stopped".into(),
            );
        }
        (
            f.provider,
            f.state.clone(),
            f.started.elapsed().as_secs(),
            // Scrubbed: the live tail streams the CLI's words to the
            // interface, and on the success path those words include the
            // credential.
            tail(&scrub_secrets(&f.output), 400),
            f.url.clone(),
            f.rejected_note.clone(),
            f.skew,
        )
    };
    let (provider, flow, secs, t, url, rejected, skew) = snapshot;
    let mut v = match flow {
        FlowState::Starting => {
            serde_json::json!({ "state": "starting", "seconds": secs, "tail": t })
        }
        FlowState::UrlReady(url) => {
            serde_json::json!({ "state": "url_ready", "url": url, "tail": t, "rejected": rejected })
        }
        FlowState::Exchanging => serde_json::json!({ "state": "exchanging", "tail": t }),
        FlowState::CodeRejected(why) => {
            serde_json::json!({ "state": "code_rejected", "tail": why, "url": url })
        }
        FlowState::Restarting(why) => serde_json::json!({ "state": "restarting", "tail": why }),
        FlowState::Done { account, economy } => {
            serde_json::json!({ "state": "done", "account": account, "economy": economy })
        }
        FlowState::Failed(t) => serde_json::json!({ "state": "failed", "tail": t }),
    };
    v["provider"] = provider.into();
    v["clock_skew_secs"] = skew.into();
    v
}

#[cfg(test)]
mod tests {
    use super::{find_url, skew_from};

    /// A machine minutes behind the world (a Docker Desktop VM woken from
    /// host sleep) must be measured as such from the `Date` header — and the
    /// timezone must not be able to lie: both sides are judged in UTC.
    #[test]
    fn a_frozen_clock_is_measured_and_a_true_clock_reads_zero() {
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
            find_url(&output[scan_from..], |u| u.contains("claude.ai")).as_deref(),
            Some("https://claude.ai/oauth/authorize?state=NEW")
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

#[cfg(test)]
mod what_the_scrubber_was_missing {
    //! Two ways a live credential reached the log in the branch written to
    //! keep it out. Both turn on the same assumption: that `sk-ant` and the
    //! token's body arrive in one unbroken run of token characters.

    use super::{scrub_secrets, tail};

    const TOKEN: &str = "sk-ant-oat01-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    /// Every site read `scrub_secrets(&tail(output, N))`, so the scrubber only
    /// ever saw the last N characters. A transcript long enough to push the
    /// `sk-ant-` prefix out of that window — while leaving the body inside —
    /// gave the scrubber a run with no marker in it, which it printed as-is.
    #[test]
    fn a_prefix_cut_off_by_the_tail_still_hides_the_body() {
        let transcript = format!("{}\n{TOKEN}\n", "noise line\n".repeat(50));
        let shown = scrub_secrets(&transcript);
        let shown = tail(&shown, 200);
        assert!(
            !shown.contains("AAAAAAAAAAAAAAAAAAAA"),
            "the token body survived: {shown}"
        );
    }

    /// `strip_ansi` turns a carriage return into a newline, and a TUI redraw
    /// puts one mid-token. That leaves a short run carrying the marker and a
    /// long run carrying the secret, neither of which the scrubber recognised
    /// — and rejoining the two lines gives the credential back.
    #[test]
    fn a_token_split_by_a_redraw_is_still_a_token() {
        let split = "sk-ant-oat01-AAAA\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let shown = scrub_secrets(split);
        assert!(
            !shown.contains("AAAAAAAAAAAAAAAAAAAA"),
            "the split halves rejoin into the credential: {shown}"
        );
    }

    /// What leaves this module carries no credential: the very failure
    /// message that once ferried an unrecognized token into the logs must
    /// blot it out, while ordinary words pass untouched.
    #[test]
    fn a_credential_is_scrubbed_from_outbound_text() {
        let text = "Your OAuth token:\nsk-ant-at01-3BrYJsbWQSRjWgqoWSud8cWu6nFBNIqo9F19xKrQsFBZ\nStore this token securely.";
        let scrubbed = scrub_secrets(text);
        assert!(!scrubbed.contains("3BrYJsbW"), "token survived: {scrubbed}");
        assert!(scrubbed.contains("sk-ant-…[redacted]"));
        assert!(scrubbed.contains("Store this token securely."));
        // A bare mention of the prefix is prose, not a credential.
        assert_eq!(
            scrub_secrets("set sk-ant-… as the key"),
            "set sk-ant-… as the key"
        );
    }

    /// And nothing else gets eaten: a diagnostic is why anybody reads this.
    #[test]
    fn ordinary_output_is_left_alone() {
        let said = "error: connection refused after 3 attempts (code 7)";
        assert_eq!(scrub_secrets(said), said);
    }
}
