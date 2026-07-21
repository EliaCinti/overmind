# ADR-0015: Agent-facing memory tools — injection is the floor, tools are the ceiling

- **Date:** 2026-07-21
- **Status:** proposed (extends ADR-0013, informs M8)

## Context

ADR-0013 wired organizational memory as an **orchestrator-driven loop**: Overmind calls `get_context` when a task starts, injects the text into the agent's prompt, and calls `store_memory` when it finishes. The agent itself never talks to the brain — it only reads what was handed to it.

The obvious next step is to let agents call memory **themselves**, as MCP tools: `recall` with their own query mid-task, `why` to recover the rationale behind a decision, `related_memories` to walk the graph. The question this ADR answers is not *whether* to do that, but whether it **replaces** the injected loop or **sits on top of it**.

We now have field evidence, not speculation. We ran a real multi-agent company (a local Paperclip instance, agents on Claude Code / OpenCode+Kimi-K3 / Gemini CLI) with Wadachi connected as an MCP app and its tools installed on every agent. What we observed:

- **Tool availability does not imply tool use.** An engineer agent on Kimi K3, with the memory tools present in its runtime, spent a turn shell-probing the API trying to discover *how* memory was exposed instead of calling the tools it already had. It used them only when the task text named `get_context` and `recall` explicitly.
- **The failure mode is silent.** An agent that never calls `store_memory` emits no error. The task succeeds, the run is green, and the knowledge is simply gone. There is nothing to alert on.
- **Discipline is model-dependent.** The same instructions produce reliable tool use on Claude and unreliable use on cheaper/faster models — which are exactly the models we want to run high-volume coding work on (ADR budget rationale).
- **Tools are not free.** Installing the full Wadachi surface pushes ~30 tool definitions into the context of *every* run of *every* agent. Paperclip's own UI warns about this ("install only where it will actually be used").

## Decisions

1. **Hybrid, with distinct roles. Injection stays the deterministic floor; agent tools are a discretionary ceiling.** The ADR-0013 loop is *not* removed when tools arrive. It guarantees that every task starts with relevant memory and ends with a durable trace, regardless of the model driving it. Agent-side tools only ever *add* depth on top of that guarantee.
   The governing principle: **the reliability of organizational memory must never depend on model discretion**, because that failure is silent and unrecoverable.

2. **The tool surface is role-scoped, never all-or-nothing.** Wadachi exposes 31 tools; almost no agent should see all of them. The mapping is a property of the role, alongside `reports_to` and the model choice (ADR-0011):
   - *Every task agent (read):* `get_context`, `recall`, `expand_memory`, `why`, `related_memories`.
   - *Curator/librarian role only:* `consolidate`, `merge_memories`, `sleep`, `flag_stale`, `set_belief`, `review_beliefs`.
   - *Never exposed to a task agent:* `delete_memory`. Destructive memory operations are an operator action, not an agent capability.
   This is both a cost decision (context budget per run) and a safety decision (blast radius of a confused agent).

3. **Writes stay orchestrator-authoritative; agents propose.** The completion-time `store_memory` remains Overmind's, with the task/issue as provenance — that is what makes memory attributable and auditable (ADR-0009 audit chain stays independent). Agents that want to record something mid-task **propose** it; acceptance is a separate step. Wadachi already has exactly this shape (`reflect` → `proposed` insight → `accept_insight`), so this is a reuse, not a new mechanism. Free-form agent writes are rejected below.

4. **Agent-facing tools require a shared, long-lived brain endpoint.** ADR-0013's stdio pool (4 persistent connections, `OVERMIND_MEMORY_POOL`) is sized for the orchestrator's ~2 calls per task. Once N agents each call memory tens of times per task, per-orchestrator stdio no longer composes: the agent runtime is a *separate process* that needs its own path to the brain. The managed brain of M8 should therefore be reachable as a **long-lived per-company endpoint** (Wadachi's HTTP/streamable-HTTP transport) that agent runtimes connect to, while Overmind's own calls may keep using stdio. Two consequences follow: the endpoint needs an auth token per company, and the brain must be concurrency-safe — which Wadachi is as of v0.14.0 (WAL + `busy_timeout` + atomic writes), the requirement logged in ADR-0004.

5. **Overmind owns a per-runtime MCP config translation layer.** There is no common way to hand an MCP server to a coding CLI: each runtime has its own schema and its own injection point. These are verified empirically, not from documentation (the Gemini one is undocumented and was recovered by reading the shipped bundle):

   | Runtime | Config shape | Injection point |
   |---|---|---|
   | Claude Code | `mcpServers.<name> = { type: "http", url, headers }` | `--mcp-config <file>` + `--strict-mcp-config` |
   | OpenCode | `mcp.<name> = { type: "remote", url, enabled, headers }` | `opencode.json` under `XDG_CONFIG_HOME` |
   | Gemini CLI | `mcpServers.<name> = { httpUrl, headers }` | `GEMINI_CLI_SYSTEM_SETTINGS_PATH` |

   Two properties this layer must have, both learned the hard way: the written config carries a **bearer token**, so it must be per-run, mode `0600`, and deleted in a `finally` that covers its whole lifetime; and it must **not** be written into a config file shared by other agents (for Gemini this means the *system* settings path, which merges with — rather than overwrites — the shared user settings, leaving auth intact and avoiding cross-agent token leakage between concurrent runs).

## Alternatives considered

- **Pure agent-driven — drop the injected loop once tools exist.** Rejected: it converts a guaranteed behaviour into a probabilistic one, with a silent failure mode, precisely on the cheap models we run most work on. Observed directly (see Context).
- **Pure injection — never give agents tools.** Rejected as a permanent stance: it caps the agent at whatever the orchestrator guessed was relevant at task start, and forecloses the genuinely useful cases (targeted `recall` mid-task, `why` before contradicting a past decision).
- **Install the full tool surface on every agent.** Rejected: ~30 tool definitions in every run's context is a standing token tax on every agent, and it hands destructive operations to agents that have no business holding them.
- **Let agents write memories freely.** Rejected: it trades a clean corpus for duplicates and low-value entries. Wadachi has `consolidate`/`sleep` to clean up afterwards, but prevention is cheaper than curation, and unattributed writes weaken provenance.
- **One brain process per agent.** Rejected: N processes over one brain directory is exactly the concurrent-writer situation ADR-0004 flagged, and it wastes a cold start per agent. A shared endpoint with a concurrency-safe brain is the same cost model as the existing pool, just moved out of process.
- **Reusing the orchestrator's stdio pool for agent tool calls** (proxying agent calls through Overmind). Attractive for auditability, but it makes Overmind a synchronous middleman on every agent tool call and couples agent latency to orchestrator health. Deferred: revisit if we later want to *enforce* policy on agent memory calls, where a proxy is the natural enforcement point.

## Consequences

- M8's managed brain gains a second surface: besides `Memory::with_brain_dir` for Overmind's own stdio calls, it must expose a per-company authenticated endpoint for agent runtimes.
- The agent model grows a `memory_tools` (role → toolset) notion, which the runtime adapter translates into whichever config shape the target CLI expects.
- Memory reliability remains testable the way it is today: the injected loop is deterministic, so the existing no-provider and broken-provider tests keep their meaning. Agent tool use is, by construction, *not* something we assert on — which is the reason it is not allowed to be load-bearing.
- Wadachi's HTTP transport moves from optional to load-bearing for the multi-agent path. It exists and is verified end-to-end (agents on all three runtimes above reached a Wadachi HTTP endpoint and executed real tool calls), but note that some MCP clients issue one-shot `tools/list` calls with no `initialize` handshake, so the server must run in stateless mode to answer them.
