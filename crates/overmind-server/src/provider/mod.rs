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

/// One CLI Overmind knows how to drive.
///
/// Every method reads what the adapter emitted and answers `None` when this
/// output is not that kind of thing. None of them may panic on malformed
/// input: an adapter is a foreign program, its output is untrusted, and a
/// truncated line is ordinary rather than exceptional.
pub trait Provider: Send + Sync {
    /// The name this provider is configured and reported under.
    fn id(&self) -> &'static str;

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

    /// Why the turn failed, in words worth showing a person.
    ///
    /// `None` means the turn did not fail. The words are the adapter's own
    /// wherever it gave any: "Credit balance is too low" tells somebody what
    /// to do, and "agent exited with code 1" does not.
    fn failure(&self, output: &str) -> Option<String>;

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
