//! What Overmind needs from the CLI it drives ([ADR-0048](../../docs/adr/0048-a-provider-is-a-capability-not-a-name.md)).
//!
//! Until now there was no boundary: `runner.rs` built a literal `claude -p …`
//! and read Claude Code's own events back in seventy places, `economy.rs`
//! probed `claude auth status --json`, and `claude_auth.rs` drove
//! `setup-token` on a pty. A second provider would have touched every one of
//! them, and a third would have touched them again.
//!
//! This is that boundary, and it is deliberately about the **wire** first: the
//! shapes an adapter emits and Overmind has to read. The rest of ADR-0048's
//! members — how a turn is invoked, how a run is bounded, who pays, how a
//! sign-in is driven, which models exist — arrive in later slices, each
//! guarded by the same suite as this one.
//!
//! **What does not belong here.** A provider knows its own wire format and
//! nothing about Overmind's conventions. `draft_reply` stays in `runner.rs`
//! because the JSON plan with a `reply` key is *our* prompt's shape, which
//! every provider's agents answer in — reading it is not provider knowledge,
//! and moving it here would have each new provider reimplement Overmind's own
//! protocol.

use serde_json::Value;

/// What a turn cost, as the adapter reported it.
///
/// Money is the unit here because Claude Code reports money. A provider that
/// cannot — ADR-0048's second case — reports no cost at all rather than a
/// zero, and is bounded by a count of turns instead. Nothing derives one from
/// the other: a turn costs what its context costs.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCost {
    pub model: String,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub cost_cents: i64,
}

/// How a run is stopped before it costs more than it may.
///
/// One variant today because one provider ships today. ADR-0048's second case
/// — a provider that cannot price a turn but can count them — attaches here as
/// `Turns(u32)` when the provider that needs it arrives, rather than being
/// built now against nothing: an unexercised variant is a guess about a wire
/// format nobody has run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bound {
    /// Stop when this many cents have been spent.
    Money { cents: i64 },
}

/// Everything about one turn that changes the command, and nothing else.
///
/// A struct rather than four positional arguments because the list grows: each
/// provider added is a chance to pass `caged` where `mcp` was meant, and the
/// compiler cannot see the mistake when both are the same shape.
pub struct TurnSpec<'a> {
    /// Whether the run is inside the OS cage (ADR-0023). Some adapters take a
    /// flag that is only safe when something else is enforcing the boundary.
    pub caged: bool,
    /// The MCP servers this run may reach, and only those.
    pub mcp_config: Option<&'a std::path::Path>,
    /// What stops it, if anything does.
    pub bound: Option<Bound>,
}

/// One CLI Overmind knows how to drive.
///
/// Every method reads what the adapter emitted and answers `None` when this
/// output is not that kind of thing. None of them may panic on malformed
/// input: an adapter is a foreign program, its output is untrusted, and a
/// truncated line is ordinary rather than exceptional.
pub trait Provider: Send + Sync {
    /// The name this provider is configured and reported under.
    fn id(&self) -> &'static str;

    /// The shell command that runs one turn.
    ///
    /// Returned as a string because the cage runs it through a shell
    /// (ADR-0023), which is also why anything interpolated from a path must be
    /// quoted by the implementation — the caller cannot know which parts of
    /// the string are yours.
    ///
    /// Not consulted at all when `OVERMIND_AGENT_CMD` is set: that names a
    /// command Overmind was told to run and did not compose, the same reason
    /// `economy.rs` refuses to interrogate a custom adapter.
    fn invoke(&self, spec: &TurnSpec<'_>) -> String;

    /// The text this line adds to the answer being streamed, if it adds any.
    ///
    /// Called per line while a turn runs, so it must be cheap and must not
    /// allocate for lines it does not recognise.
    fn text_delta(&self, line: &str) -> Option<String>;

    /// What the agent is doing right now, from one line (ADR-0039).
    ///
    /// Structured, never an English sentence: the interface words it in the
    /// person's language, so a provider that returned prose would be
    /// untranslatable.
    fn activity(&self, line: &str) -> Option<Value>;

    /// What the whole turn cost, from its whole output.
    ///
    /// `None` means *not reported*, which is not the same as free. Callers
    /// must not substitute a zero — see [`ParsedCost`].
    fn cost(&self, output: &str) -> Option<ParsedCost>;

    /// Why the turn failed, in the adapter's **own, unbounded** words.
    ///
    /// `None` means the turn did not fail. The words are the adapter's
    /// wherever it gave any: "Credit balance is too low" tells somebody what
    /// to do, and "agent exited with code 1" does not.
    ///
    /// Implement this and never call it — [`Provider::failure`] is what the
    /// rest of Overmind uses, and it is the one that bounds the length.
    fn failure_words(&self, output: &str) -> Option<String>;

    /// Why the turn failed, bounded and fit to store or show.
    ///
    /// Provided rather than required, and that is the point: this text is a
    /// foreign program's, it reaches `agent_task_sessions.last_error` and the
    /// session drawer, and how long it may be is **Overmind's** policy rather
    /// than each adapter's. Left to implementors it would be applied by the
    /// first provider and forgotten by the second — which is exactly what had
    /// already happened *inside* one implementation, where the budget-ceiling
    /// branch returned unclamped while every other branch clamped.
    fn failure(&self, output: &str) -> Option<String> {
        self.failure_words(output)
            .map(|s| crate::ceo::clamp_agent_text(&s))
    }

    /// The command that asks this CLI who pays: the binary, then its
    /// arguments (ADR-0030).
    ///
    /// Given rather than run, because *how* it is run is Overmind's: as the
    /// agent and not as the server — in the image the server is root and the
    /// credentials live in the agent's home, so a probe run as the server
    /// answers confidently about the wrong home directory — with a timeout,
    /// and with a failure that becomes *unknown* rather than a guess.
    fn economy_probe(&self) -> (&'static str, &'static [&'static str]);

    /// What that command's answer means.
    ///
    /// Split from running it so the rule can be tested against payloads that
    /// were **observed**, rather than against shapes we imagined — which is
    /// how the difference between a key, a key over a login, a named plan and
    /// a `setup-token` was learned in the first place.
    fn read_economy(&self, status: &Value) -> crate::economy::Economy;

    /// Where the subscription stands, read from a run's own output.
    ///
    /// A plan's state rides along with work already being done rather than
    /// costing a call of its own. `None` from a provider that says nothing
    /// about it, which is most of them.
    fn plan_window(&self, output: &str) -> Option<crate::economy::PlanWindow>;

    /// The adapter's own id for this session, so a later turn can resume it.
    ///
    /// `None` where the adapter names no session, which is a provider that
    /// cannot resume rather than an error.
    fn session_id(&self, output: &str) -> Option<String>;
}

/// The provider this instance drives.
///
/// One, for now, and the signature is where that will change: ADR-0048's step
/// 3 gives an agent its own provider, so this becomes a lookup rather than a
/// constant. Returning a `&'static dyn` keeps every call site written the way
/// it will stay.
pub fn current() -> &'static dyn Provider {
    &claude_code::ClaudeCode
}

pub mod claude_code;
