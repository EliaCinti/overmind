# ADR-0036: Tools in the agent's hand — declared by the operator, granted per agent

- **Date:** 2026-08-23
- **Status:** accepted
- **Builds on:** [ADR-0005](0005-structured-agent-characterization.md) (structured, server-enforced traits), [ADR-0023](0023-os-level-sandboxing.md) (the cage), [ADR-0027](0027-agents-reach-memory-through-overmind.md) (the per-run MCP config and `--strict-mcp-config`).

## Context

The owner's first ask that needs a tool outside Overmind: a company that
turns a sketch of his new house into a Blender model — one agent driving
Blender through BlenderMCP, the others (engineer, architect, stylist,
furnisher) analysing, researching, proposing.

Today that is impossible by design. Every run gets a per-run MCP config
holding exactly one server — Overmind's own memory endpoint — and the CLI is
launched with `--strict-mcp-config`, so a caged agent cannot inherit whatever
MCP servers the machine's own configuration happens to hold (ADR-0027). That
was the right call and stays: an agent's tools must be something somebody
*granted*, never something it *found*.

Conversational turns have it even tighter: they get no MCP config at all.

## Decisions

1. **Tools are declared by whoever runs the box, not by whoever runs a
   company.** `OVERMIND_AGENT_TOOLS` names a JSON file in the CLI's own
   shape — `{"mcpServers": {"blender": {"command": "uvx", "args":
   ["blender-mcp"]}}}` — read once at startup. A tool is a *command the
   server will spawn*, and that makes it the operator's to declare, exactly
   like `OVERMIND_AGENT_CMD` and `OVERMIND_MEMORY_CMD`: a member of a
   company must not be able to make the server run a command of their
   choosing. The registry is listed at `GET /api/tools` by name and command,
   so the interface can offer what exists and nothing else.
   - An optional top-level `"descriptions"` map — `{"blender": "Blender,
     via BlenderMCP"}` — is the one addition to the CLI's shape. It is
     stripped before the per-run config is written; it exists so the hire
     dialog and the prompt can say what a tool *is* in one line.

2. **Tools are granted per agent, as a structured trait — at hire or afterwards.** `tools: ["blender"]`
   joins `AgentTraits` beside `permissions` and `model`, validated at every
   entry point against the registry — an unknown name is refused at the
   boundary with 400, never stored and handed to a run later (the rule
   ADR-0021 applies to models). It rides in the traits JSON, so granting or
   withdrawing a tool is a config revision like any other characterization
   change: versioned, roll-backable, audited. **Per agent and not per
   company, on purpose:** the owner's design has one modeler touching
   Blender and four colleagues who must not — one Blender, one socket, one
   hand on it.

3. **A run's MCP config is what Overmind wrote, and nothing else.** The
   per-run file now holds the memory endpoint (task runs, as before) plus
   the granted servers; `--strict-mcp-config` is unchanged. **Conversational
   turns get a per-run file too** — granted servers only, no memory
   endpoint, because a turn has no session and therefore no memory token —
   so the refinement the owner wants ("here is a photo of the sofa, move it
   under the window") reaches Blender from the chat. A custom
   `OVERMIND_AGENT_CMD` still receives nothing: it is not the Claude CLI and
   never signed this contract.

4. **The agent is told, in its own prompt, what it holds.** One line in the
   persona block — *"Tools granted to you: blender — Blender, via
   BlenderMCP."* — so a granted tool is something the agent is *meant* to
   use, not something it discovers in a menu.

5. **A granted tool is a door the operator opened, and the threat model says
   so.** The tool's process runs where the CLI runs — inside the cage on
   macOS, as the agent uid in the image — but what it *talks to* (Blender's
   socket, a database, GitHub) is outside any promise the cage makes.
   Granting is therefore a decision with two signatures: the operator's
   (the registry) and the company's (the trait). The threat-model table
   gains the row, named by its test.

## Alternatives rejected

- **Tools defined per company, from the UI** — rejected: it would let any
  member make the server spawn an arbitrary command. The cage would contain
  the process, but "a member can run commands on the box" is not a sentence
  this product should make true.
- **Dropping `--strict-mcp-config`** so agents inherit the machine's MCP
  servers — rejected for the reason ADR-0027 gave: an agent would quietly
  hold tools nobody granted.
- **Tools per company only** — rejected: it cannot express the owner's own
  design (one modeler, four advisers), and one Blender socket shared by five
  agents is a race.
- **A generic "capabilities" grant on the existing declared permissions**
  (`blender:write`) — rejected: declared permissions are compiled into the
  prompt and not policed (ADR-0005's honesty); a tool grant is *enforced* —
  the server writes the config or it does not — and deserves a field that
  says so.

## Consequences

- `Config.agent_tools` (registry, `OVERMIND_AGENT_TOOLS`), `GET /api/tools`,
  `AgentTraits.tools`, validation in `validate_traits`, the per-run config
  merge in `AgentMcpConfig::write` for tasks and a sibling for turns, the
  prompt line, the hire dialog's *Tools* choice, the threat-model row.
- On macOS the cage denies `$HOME`; a tool that needs a cache (`uvx` writes
  `~/.cache/uv`) needs `OVERMIND_SANDBOX_ALLOW` to name it — a decision made
  visibly, not the cage quietly giving way.
- The first company that uses it is the owner's house: the walk is the
  acceptance, with Blender on his desk, not mine.

## Addendum (2026-08-23): granted after the fact

The first live walk showed the gap at once: the CEO proposes the team and
hires it, and the proposal carries no tools — a grant is the owner's, not the
CEO's — so the modeler came into the world without Blender and there was no
way to hand it over short of the API. `POST /api/agents/{id}/tools` sets the
whole hand of an agent already hired, under the same rules as at hire
(registry-validated, a config revision, the trait the next run reads), and the
org chart's edit row offers the same chips as the hire dialog. The path the
owner wanted — *tell the CEO everything, then hand the one tool to the one
agent* — now exists.

