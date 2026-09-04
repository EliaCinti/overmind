# ADR-0048: A provider is a capability, not a name

- **Date:** 2026-09-04
- **Status:** proposed
- **Builds on:** [ADR-0030](0030-how-you-pay-is-a-first-class-fact.md) (the economy is detected, never configured), [ADR-0037](0037-who-pays-is-asked.md) (who pays is asked, and can be chosen), [ADR-0023](0023-os-level-sandboxing.md) (every agent run is caged)

## Context

The owner wants agents on models other than Claude's — added with an API key, and where the provider allows it, with a subscription. Two questions had to be answered before any code: who actually permits a subscription, and what it costs to support a second one at all.

**Who permits a subscription, checked 4 Sep 2026.** All three major agent CLIs do, which means Overmind's existing shape — drive a CLI, ask it who pays — is right for all three rather than a Claude-only trick:

| Provider | Subscription | API key |
|---|---|---|
| Claude Code | Pro / Max, via `setup-token` | yes — **both, today** |
| OpenAI Codex CLI | ChatGPT Plus, Pro, Business, Edu, Enterprise (browser login or `codex login --device-auth`) | yes |
| Gemini CLI | a personal Google account (free Code Assist licence) or a paid Code Assist subscription; `oauth-personal` is its default | yes |
| DeepSeek, Mistral, xAI, local models | none | key only, through an OpenAI-shaped endpoint |

One detail worth carrying rather than rediscovering: Codex's ChatGPT sign-in **auto-creates an API key** in the selected org, which can then quietly become the payer. That is the same deception Overmind already names for Anthropic — a key silently overriding a login — so `overrides_login` generalises. But it must be **asked of each CLI**, never assumed: the fact is real, its shape is not shared.

**What it costs, measured rather than estimated.** `OVERMIND_AGENT_CMD` looks like an adapter seam and is not one. Counted 4 Sep 2026, in production code only:

- `runner.rs` — **70 places** read Claude Code's wire shape. It builds a literal `claude -p "$OVERMIND_TASK_PROMPT" --model … --output-format stream-json --verbose --include-partial-messages`, adds `--max-budget-usd`, `--mcp-config --strict-mcp-config`, `--dangerously-skip-permissions` and `--resume`, then reads Claude Code's own events back: `type: "stream_event"` for a text delta, `type: "assistant"` for a draft reply, a final object carrying `total_cost_usd` and `usage`, `is_error` / `terminal_reason` / `subtype` for a failure, `session_id` for a resume.
- `economy.rs` — probes `claude auth status --json` and reads `apiKeySource`, `authMethod`, `subscriptionType`. It **refuses to probe a custom adapter at all**, on the stated grounds that `auth status` is not a contract anybody else signed. That refusal is correct and becomes the model for the whole design.
- `claude_auth.rs` — drives `claude setup-token` on a pty, scraping a URL and a code out of a terminal transcript.
- `model.rs` — `Model { id, display_name, vision }`. Nothing about who can reach a model.

Four files, five distinct kinds of knowledge, and none of it behind a boundary.

**And the danger is not the plumbing.** `runner.rs:2141` says, in a comment that is correct today and fatal tomorrow:

> Missing/unparseable cost is not an error — the session already carries the full output.

A provider that cannot report what a turn cost therefore does not fail. It runs, the ledger never moves, and **the budget cap silently stops applying to that agent**. The same is true of every other guarantee Overmind makes: the cap, the cage, the audit chain and the brain are all built on things Claude Code happens to provide. A second provider that provides less does not announce it — it just quietly makes a promise untrue.

That is the decision this ADR exists to make. The interface is the easy half.

## Decision

**1. A provider is a trait, and its first implementation is the one that already exists.** `Provider` has eight members — `NEXT.md` had estimated six before anybody counted, and the survey above is where the other two came from:

| Member | What it answers | Where it lives today |
|---|---|---|
| `invoke` | how a turn is started, as an argv | `runner.rs:1169` |
| `stream` | how a text delta and a draft reply are read | `text_delta_in`, `draft_reply` |
| `cost` | what the turn cost, and in what tokens | `parse_cost` |
| `failure` | whether the turn failed, and in whose words | `adapter_failure` |
| `session` | how a session is named, for resuming it | `parse_adapter_session_id` |
| `economy` | who pays, asked of the CLI | `economy.rs::detect` |
| `sign_in` | how a subscription is connected, if at all | `claude_auth.rs` |
| `models` | which models, and which economies reach them | `model.rs` |

Claude Code becomes an implementation of this and **nothing else changes**. No new provider, no new behaviour, and the existing suite is the proof that nothing moved. That is the whole risk of this milestone, taken once, against tests that already exist.

**2. Capabilities are declared, and Overmind refuses what it cannot govern.** Each provider states what it can do, and the parts of Overmind that depend on a capability check for it rather than assuming it:

- **`cost` is not optional.** An agent cannot be hired onto a provider that cannot report what a turn cost, and the refusal says why. The budget cap is a promise made in real money ([ADR-0030](0030-how-you-pay-is-a-first-class-fact.md)); a provider that cannot price a turn cannot be governed, and a cap that silently stops applying is worse than a cap that was never offered. This is the rule the comment above forced.
- **`sign_in` is optional**, and its absence is a fact the interface states: this provider takes an API key, and that is the only way it pays. Most providers are here.
- **`economy` may answer *unknown*,** and unknown stays unknown. [ADR-0030](0030-how-you-pay-is-a-first-class-fact.md)'s rule is unchanged and now applies per provider: Overmind does not invent an answer it could not obtain.
- **`session` is optional.** A provider that cannot resume gets a fresh turn, and a conversation that would have resumed says so rather than silently starting over.

A capability is not a feature flag. It is a claim the provider makes, which Overmind checks **before** offering a person something that would not work.

**3. Adding a provider is one file and one conformance suite.** This is what the owner asked to be engineered well, so it is stated as an acceptance test rather than a hope. To add a provider, a contributor writes one module implementing the trait, declares its capabilities, and makes `tests/provider_conformance.rs` pass. That suite is written **once**, parameterised over every registered provider, and asserts what Overmind actually depends on:

- a turn invoked with a prompt and a model produces the prompt's answer;
- a text delta arrives before the turn ends (the UI streams, or it is not streaming);
- a completed turn reports a cost, or the provider declared it cannot and is refused at hire time;
- a failed turn is recognised as failed, and the reason is the provider's own words, not ours;
- the economy probe answers with one of the four states and never guesses;
- every model the catalogue offers for this provider can actually be invoked under at least one economy it declares.

No test spawns a real CLI, exactly as today: each provider ships a stub adapter that speaks its wire shape, which is also the artefact proving the wire shape was **observed** rather than imagined. A provider whose stub was written from documentation rather than from a recorded run is a provider that will fail on somebody's machine instead of in CI.

**4. The order is Claude Code, then Codex, then the payer, then entitlement, then Gemini.** Each step is worth having before the next exists:

1. the trait, with Claude Code as its only implementation;
2. Codex CLI, complete — first because its subscription is the one most Overmind owners already pay for, and because its auto-created key exercises the payer logic hardest;
3. the payer per provider, reusing the org view's control from [ADR-0037](0037-who-pays-is-asked.md): a company may reasonably run Claude on a plan and something else on a key;
4. entitlement in the catalogue, then a company default — an agent cannot be hired onto a model this instance cannot run, and a company sets one default instead of ten decisions;
5. Gemini CLI, once the trait has survived two.

**5. A provider is configured, never guessed.** Which providers exist on this instance is instance-level configuration, like the economy: named in the compose file, probed at boot, and reported by `/api/health`. Overmind does not scan `$PATH` hoping to find a CLI it recognises — a binary that happens to be present is not a decision somebody made.

## Alternatives considered

**`if provider == "codex"` where the knowledge already is.** Four files, seventy places. It is faster for exactly one provider and then never again: the third provider touches the same seventy places a third time, and no reading of any one file tells you what a provider must do. Rejected because the cost is paid per provider forever rather than once.

**A generic OpenAI-shaped HTTP client, no CLIs.** Tempting — one wire format, dozens of providers, no subprocess. It loses the two things Overmind is built on. The cage ([ADR-0023](0023-os-level-sandboxing.md)) cages a *process*; an HTTP call from the server is not caged and runs with the server's own reach. And a subscription is a CLI's login: there is no API endpoint that bills a ChatGPT Plus plan. This road reaches key-only providers and abandons every subscription, which is the half the owner asked for. Kept as a *possible ninth capability* later — a provider whose `invoke` is an HTTP call rather than an argv — but not as the design.

**Make `OVERMIND_AGENT_CMD` the contract: any CLI that speaks Claude Code's `stream-json`.** Zero code, and it already half-exists. But it makes every other CLI wrong by definition and puts the adaptation in a shell wrapper somebody writes alone, untested, with no way to declare that it cannot report cost. `economy.rs` already refuses to trust such an adapter; this would extend that distrust to the budget and the audit chain while pretending otherwise.

**Let a provider that cannot report cost run anyway, with a warning.** Rejected on the strength of the comment in `runner.rs`. A warning at hire time is read once; a cap that does not apply is discovered at the invoice. Overmind's budget is the reason an owner lets agents run unattended, and a promise that quietly stops holding is worse than one never made.

## Consequences

- **Adding a provider becomes a bounded task**: one module, one declaration, one suite to pass. The first one costs the abstraction; that is why this is a milestone and not a patch.
- **Some providers will be refused**, and will say why. That is the point, and it is a change in posture: Overmind currently accepts any `OVERMIND_AGENT_CMD` and hopes.
- **`OVERMIND_AGENT_CMD` keeps working** and keeps meaning what it means today — an adapter Overmind cannot interrogate. It becomes a provider whose capabilities are all *unknown*, which is what the code already believes about it.
- **The threat model grows a row per provider.** Each brings its own credential, its own storage, and its own sign-in; `docs/THREAT-MODEL.md` names the test that holds each, as it does for Claude's.
- **The economy stops being singular.** `GET /api/health` reports a payer per configured provider, and the org view names them all. Existing single-provider instances read exactly as they do now.
- **Tests**: `tests/provider_conformance.rs`, parameterised over every registered provider and run against its recorded stub — plus the existing suites unchanged, which is the acceptance criterion for step 1.
