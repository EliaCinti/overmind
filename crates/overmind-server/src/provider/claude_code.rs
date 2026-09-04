//! Claude Code, the CLI Overmind was built around ([ADR-0048](../../../docs/adr/0048-a-provider-is-a-capability-not-a-name.md)).
//!
//! Everything here is knowledge of one program's output format, moved out of
//! `runner.rs` unchanged. It was written against payloads that were
//! **observed** — the comments naming a date and a measurement are the record
//! of that, and they travel with the code because a wire format nobody
//! recorded is a wire format somebody guessed.
//!
//! The shapes read here: `stream_event` wrapping the API's SSE deltas,
//! `assistant` messages for what the agent is doing, and a final JSON object
//! carrying `total_cost_usd`, `usage`, `session_id`, and — when a turn failed
//! — `is_error` with `subtype`, `result` or `errors`.

use serde_json::{Value, json};

use super::{ParsedCost, Provider};

/// The provider itself holds nothing: every answer is a pure function of the
/// bytes the CLI produced.
pub struct ClaudeCode;

impl Provider for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn text_delta(&self, line: &str) -> Option<String> {
        text_delta_in(line)
    }

    fn activity(&self, line: &str) -> Option<Value> {
        activity_in(line)
    }

    fn cost(&self, output: &str) -> Option<ParsedCost> {
        parse_cost(output)
    }

    fn failure(&self, output: &str) -> Option<String> {
        adapter_failure(output)
    }

    fn session_id(&self, output: &str) -> Option<String> {
        parse_adapter_session_id(output)
    }
}

/// The text delta inside a partial-message event, if this line is one.
/// `--include-partial-messages` makes the CLI emit `stream_event` lines that
/// wrap the API's own SSE events; the reply grows one delta at a time.
pub(crate) fn text_delta_in(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("stream_event") {
        return None;
    }
    let event = v.get("event")?;
    if event.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
        return None;
    }
    delta
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn activity_in(line: &str) -> Option<serde_json::Value> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let content = v.get("message")?.get("content")?.as_array()?;
    // The last block wins: a message that says a word and then calls a tool
    // is doing the tool.
    for block in content.iter().rev() {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                // MCP names arrive as `mcp__server__tool_name`.
                let (server, tool) = match name.strip_prefix("mcp__") {
                    Some(rest) => match rest.split_once("__") {
                        Some((srv, t)) => (Some(srv.to_string()), t.replace('_', " ")),
                        None => (None, rest.replace('_', " ")),
                    },
                    None => (None, name.replace('_', " ")),
                };
                let mut out = serde_json::json!({ "kind": "tool", "tool": tool });
                if let Some(srv) = server {
                    out["server"] = serde_json::json!(srv);
                }
                return Some(out);
            }
            Some("text") => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                let preview: String = text.trim().chars().take(120).collect();
                if !preview.is_empty() {
                    return Some(serde_json::json!({ "kind": "text", "preview": preview }));
                }
            }
            _ => {}
        }
    }
    None
}

/// What the adapter said went wrong, when it said anything.
///
/// "agent exited with code 1" is true and useless. The Claude Code CLI puts the
/// reason in its result envelope — `"Credit balance is too low"` is the one that
/// stopped the smoke run, and a person reading the drawer had to find it inside
/// a wall of JSON to learn their account was empty. Errors a human can act on
/// are worth more than exit codes.
pub(crate) fn adapter_failure(output: &str) -> Option<String> {
    let envelope: Value = output
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line.trim()).ok())?;
    if envelope.get("is_error").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    // The ceiling we handed it (ADR-0030). Worth naming rather than passing
    // through, because this failure is *ours*: the agent did not break, it
    // reached the cap this Overmind gave it, and the person reading has a
    // specific thing they can do about that. The envelope carries no `result`
    // in this case, so without this the whole event would read "agent exited
    // with code 1".
    if envelope.get("subtype").and_then(Value::as_str) == Some("error_max_budget_usd") {
        let said = envelope
            .get("errors")
            .and_then(Value::as_array)
            .and_then(|e| e.first())
            .and_then(Value::as_str)
            .unwrap_or("the run reached its budget ceiling");
        return Some(format!(
            "stopped at this agent's budget cap — {said}. Raise the cap to let it continue."
        ));
    }
    // Otherwise whatever the adapter said, preferring its prose and falling
    // back to its error list — `Credit balance is too low` arrived in the first
    // and a ceiling arrives in the second, and both beat an exit code.
    let said = envelope
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            envelope
                .get("errors")
                .and_then(Value::as_array)
                .and_then(|e| e.first())
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })?;
    Some(crate::ceo::clamp_agent_text(said))
}

fn last_json_object(output: &str) -> Option<Value> {
    for line in output.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            return Some(v);
        }
    }
    None
}

/// The adapter's own session id (e.g. Claude Code's), used for `--resume`.
fn parse_adapter_session_id(output: &str) -> Option<String> {
    last_json_object(output)?
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Find the last line of output that is a JSON object carrying
/// `total_cost_usd`, and extract cost + usage from it.
pub(crate) fn parse_cost(output: &str) -> Option<ParsedCost> {
    let v = last_json_object(output)?;
    let usd = v.get("total_cost_usd").and_then(Value::as_f64)?;
    let usage = v.get("usage").cloned().unwrap_or_else(|| json!({}));
    let tok = |key: &str| usage.get(key).and_then(Value::as_i64).unwrap_or(0);
    Some(ParsedCost {
        model: billed_model(&v),
        input_tokens: tok("input_tokens"),
        cached_input_tokens: tok("cache_read_input_tokens"),
        output_tokens: tok("output_tokens"),
        cost_cents: cost_cents(usd),
    })
}

/// Which model this run should be attributed to.
///
/// The real Claude Code envelope has **no top-level `model`** — measured
/// against the live CLI, not assumed — so the old `unwrap_or("unknown")` meant
/// every real cost event was filed under "unknown" while the stubs, which do
/// emit `model`, looked fine. It does carry `modelUsage`, a map of model to
/// per-model cost, and a single run can touch more than one: the CLI bills a
/// small model for its own bookkeeping alongside the one doing the work. We
/// attribute the run to whichever cost the most, which is the one the operator
/// chose and the one worth seeing in the ledger.
fn billed_model(v: &Value) -> String {
    let by_cost = v
        .get("modelUsage")
        .and_then(Value::as_object)
        .and_then(|m| {
            m.iter()
                .max_by(|(_, a), (_, b)| {
                    let cost = |u: &Value| u.get("costUSD").and_then(Value::as_f64).unwrap_or(0.0);
                    cost(a).total_cmp(&cost(b))
                })
                .map(|(name, usage)| {
                    // The canonical name where the CLI gives one, so a dated
                    // snapshot and its alias do not read as different models.
                    usage
                        .get("canonicalModel")
                        .and_then(Value::as_str)
                        .unwrap_or(name)
                        .to_string()
                })
        });
    by_cost
        .or_else(|| v.get("model").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Dollars to cents, never losing a run that cost money.
///
/// Plain rounding sends anything under half a cent to zero, and a cheap turn
/// is genuinely that cheap — a small-model chat turn measured at $0.004 would
/// have recorded as **0**. Spend that records as nothing is spend the cap never
/// sees, which is the whole failure M18 existed to fix, arriving by a different
/// door. A run that cost money costs at least a cent.
///
/// The bias is upward by design: the budget is a cap, reserved in flat 50-cent
/// estimates, so sub-cent precision would be false precision — and erring
/// toward the cap being respected is the safe direction for a limit.
fn cost_cents(usd: f64) -> i64 {
    if usd <= 0.0 {
        return 0;
    }
    ((usd * 100.0).round() as i64).max(1)
}

#[cfg(test)]
mod tests {
    use super::{parse_adapter_session_id, parse_cost};

    /// A real result envelope from the Claude Code CLI, captured live during
    /// M10's smoke run. Stubs emit a tidy `{"total_cost_usd":…,"model":…}`;
    /// the real thing has no top-level `model` at all, which is how the ledger
    /// came to file every real run under "unknown" while every test passed.
    const REAL_ENVELOPE: &str = include_str!("../../tests/fixtures/claude-code-result.json");

    #[test]
    fn the_real_envelope_is_attributed_to_the_model_that_did_the_work() {
        let cost = parse_cost(REAL_ENVELOPE).expect("the real CLI reports cost");
        assert_eq!(
            cost.model, "claude-haiku-4-5",
            "not `unknown`, and not the bookkeeping model the CLI bills alongside it"
        );
        assert!(cost.input_tokens > 0 || cost.cached_input_tokens > 0);
        assert!(
            cost.cost_cents >= 1,
            "the run cost money: {}",
            cost.cost_cents
        );
    }

    #[test]
    fn a_run_that_cost_money_never_records_as_free() {
        // Measured shape of a cheap small-model turn. Plain rounding sent this
        // to zero, and spend that records as nothing is spend no cap can see.
        assert_eq!(super::cost_cents(0.004), 1);
        assert_eq!(super::cost_cents(0.0001), 1);
        // Nothing is still nothing, and ordinary amounts are unchanged.
        assert_eq!(super::cost_cents(0.0), 0);
        assert_eq!(super::cost_cents(0.0558), 6);
        assert_eq!(super::cost_cents(1.20), 120);
    }

    #[test]
    fn parses_cost_from_final_json_line() {
        let output = "doing work...\n{\"model\":\"claude-sonnet\",\"session_id\":\"abc-123\",\"total_cost_usd\":0.0525,\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":10,\"output_tokens\":50}}";
        let cost = parse_cost(output).expect("cost parsed");
        assert_eq!(cost.cost_cents, 5);
        assert_eq!(cost.input_tokens, 100);
        assert_eq!(cost.cached_input_tokens, 10);
        assert_eq!(cost.output_tokens, 50);
        assert_eq!(cost.model, "claude-sonnet");
        assert_eq!(parse_adapter_session_id(output).as_deref(), Some("abc-123"));
    }

    #[test]
    fn no_cost_json_is_none() {
        assert!(parse_cost("plain output, no json").is_none());
        assert!(parse_cost("{\"no_cost\":true}").is_none());
        assert!(parse_adapter_session_id("no json").is_none());
    }
}

#[cfg(test)]
mod failure_tests {
    use super::adapter_failure;

    /// The envelope that ended the live smoke run, trimmed to the fields that
    /// matter. Kept verbatim in shape because this is the one thing a stub
    /// cannot teach us — see `tests/fixtures/`.
    const CREDIT_EXHAUSTED: &str = r#"{"is_error":true,"subtype":"success","result":"Credit balance is too low","terminal_reason":"api_error","total_cost_usd":0}"#;

    #[test]
    fn a_failed_run_reports_what_the_adapter_said() {
        assert_eq!(
            adapter_failure(CREDIT_EXHAUSTED).as_deref(),
            Some("Credit balance is too low"),
            "the reason was in the envelope all along"
        );
    }

    #[test]
    fn a_run_that_failed_without_saying_why_falls_back_to_the_exit_code() {
        // Non-JSON output, and a well-formed envelope that is not an error:
        // neither has a message worth showing instead of the exit code.
        assert_eq!(adapter_failure("Segmentation fault"), None);
        assert_eq!(
            adapter_failure(r#"{"is_error":false,"result":"all good"}"#),
            None
        );
        assert_eq!(adapter_failure(r#"{"is_error":true,"result":"  "}"#), None);
    }
}
