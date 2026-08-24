# Tools in the agent's hand — the complete guide

Overmind agents can hold **MCP tools**: real software driven through the
[Model Context Protocol](https://modelcontextprotocol.io). The first was
Blender — one modeler agent building a house in the owner's open Blender
window while the rest of the company analysed and proposed. Anything with an
MCP server can be attached the same way: a browser, a database, a filesystem,
GitHub, your home automation.

This is the whole manual: what the pieces are, how to add a tool, how to
grant it, what the security model promises, and what to do when it breaks.
The short version lives in the [README](../README.md#tools-in-the-agents-hand);
the decisions behind the design in [ADR-0036](adr/0036-tools-in-the-agents-hand.md)
(+ addenda) and [ADR-0038](adr/0038-from-the-ceos-plan-to-a-running-task.md).

---

## The model: two roles, deliberately separate

| Role | Who | What they do | Where |
|---|---|---|---|
| **Declare** | The **operator** — whoever runs the Overmind process | Names which MCP servers *exist on this machine* and how to launch them | A JSON file on disk, `OVERMIND_AGENT_TOOLS` |
| **Grant** | The **owner/member** — whoever runs a company | Puts a declared tool in a *specific agent's* hand | The product: hire dialog, or Org → Edit |

The separation is a security boundary, not a convenience: **a tool is a
command the server will spawn**, so a member of a company must never be able
to make the server run a command of their choosing. Declaring takes filesystem
access to the machine; granting takes a session. Neither alone is enough.

What an agent holds is **enforced**, not suggested: the granted servers are
written into that run's own MCP configuration and the CLI is launched with
`--strict-mcp-config` — the agent gets exactly what was granted, never
something it found in the machine's own MCP config. Conversational turns get
the same treatment, so an agent can use its tool from the chat too.

---

## 1 · Declaring tools (operator)

Write a JSON file in the Claude CLI's own `mcpServers` shape, plus two
Overmind-specific keys:

```json
{
  "mcpServers": {
    "blender": { "command": "uvx", "args": ["blender-mcp"] },
    "files":   { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/Users/you/Projects/shared"] }
  },
  "exclusive": ["blender"],
  "descriptions": {
    "blender": "Blender, via BlenderMCP: inspect the open scene, run Python in it, take viewport screenshots.",
    "files":   "Read and write files under ~/Projects/shared."
  }
}
```

- **`mcpServers`** — exactly what the CLI accepts: `command` + `args` (stdio
  servers), or `url` for HTTP servers. An `env` map is passed through if the
  server needs one.
- **`descriptions`** *(recommended)* — one line per tool. It is shown in the
  hire dialog's chip, spoken into the agent's own prompt (*"Tools granted to
  you: blender — Blender, via BlenderMCP…"*), and told to the CEO about its
  teammates — write the **operating requirements** into it ("Blender must be
  open with the addon serving"), because this line is where people and agents
  learn them.
- **`exclusive`** *(optional)* — tools that fit **one hand at a time**.
  Granting one to a second active agent is refused with the holder's name.
  Use it for anything with a single socket or a single mutable target:
  Blender (one scene), a serial port, a printer. Leave shareable things
  (a read-only browser, a search API) off the list.

Then point Overmind at the file and restart — the registry is read once at
startup:

```sh
OVERMIND_AGENT_TOOLS=/path/to/agent-tools.json cargo run
# or in docker-compose.yml:
#   - OVERMIND_AGENT_TOOLS=/data/agent-tools.json   (mount the file into the container)
```

Ready-made registries to copy from live in [`docs/examples/`](examples/):
one file per common tool, requirements in the description.

### Editing the registry

The file is read at startup: **add or change a tool → restart Overmind.**
Grants survive the restart (they are a trait on the agent); a grant whose
tool disappeared from the registry simply stops being written into runs.

---

## 2 · Granting tools (owner)

- **At hire** — the *Tools* field appears in the hire dialog whenever the
  registry declares anything. Chips, one per tool; the preview sentence says
  what the agent will hold.
- **After hire** — Org view → *Edit* on the agent → the same chips. A click
  saves immediately (`POST /api/agents/{id}/tools`); the card reads
  *Holds: blender*.
- **The CEO's team proposal hires without tools** — a grant is the owner's
  decision, not the CEO's. Flow that works well: tell the CEO the idea, approve
  the org it proposes, then hand the tool to the one agent who needs it.

Every grant is validated against the registry (an unknown name is refused and
never stored), recorded as a **config revision** on the agent (visible in its
history, rollback-able), and — for `exclusive` tools — refused when another
active agent already holds it, naming the holder.

The agent knows what it holds: its prompt names each tool with its
description, and the CEO's prompt names what each teammate holds, so plans are
made *with* the tools instead of around them.

---

## 3 · What the security model promises (and what it does not)

- **The tool's process runs where the agent runs** — same cage (macOS
  `sandbox-exec` / the image's unprivileged uid + Landlock), same rules. If
  the tool needs paths the cage denies (a package cache, a socket file), open
  exactly those with `OVERMIND_SANDBOX_ALLOW` — colon-separated paths — rather
  than turning the cage off. For `uvx`-launched tools:
  `OVERMIND_SANDBOX_ALLOW="$HOME/.cache/uv:$HOME/.local/share/uv"`.
- **What the tool talks to is a door you opened.** BlenderMCP reaches your
  open Blender; a filesystem server reaches the directory you named. That
  reach is the point of declaring it — the cage confines the *process*, not
  the *purpose*. Declare with the same care you would install software.
- **Nothing is inherited.** Without a grant, an agent's runs carry only
  Overmind's own memory endpoint. A custom `OVERMIND_AGENT_CMD` receives no
  MCP config at all — Overmind cannot know what another adapter would do
  with one.
- **Failures are honest, not silent.** If the tool's target is not there
  (Blender closed, port not listening), the tool call fails, the agent sees
  the error and reports it in its own words. Nothing retries against your
  machine behind your back.

---

## 4 · Worked example: Blender

1. Install [Blender](https://www.blender.org) and `uv` (`brew install uv`).
2. In Blender, install the [BlenderMCP](https://github.com/ahujasid/blender-mcp)
   addon (`addon.py`) and start its server from the sidebar (default port
   9876). Keep Blender open while agents work.
3. Declare it — [`docs/examples/agent-tools.blender.json`](examples/agent-tools.blender.json)
   is ready: `blender` via `uvx blender-mcp`, marked `exclusive`.
4. Start Overmind with the registry and the uv cache allowed through the cage:

   ```sh
   OVERMIND_AGENT_TOOLS=docs/examples/agent-tools.blender.json \
   OVERMIND_SANDBOX_ALLOW="$HOME/.cache/uv:$HOME/.local/share/uv" \
   cargo run
   ```
5. Grant `blender` to **one** modeler agent. Give tasks through the CEO;
   attach sketches and photos to the chat — a task born from a conversation
   carries that conversation's files into the run.

What to expect: the agent connects to your open scene, runs Python in it,
takes viewport screenshots, and hands back a written report. Iterate — the
first pass will be a draft; corrections by chat are the workflow, not a
failure of it.

### Other tools people attach

- **A filesystem** — `@modelcontextprotocol/server-filesystem` with the
  directory as an argument. Shareable. Mind the cage: the directory must be
  writable inside it (`OVERMIND_SANDBOX_ALLOW`).
- **A real browser** — [Playwright MCP](https://github.com/microsoft/playwright-mcp)
  (`npx -y @playwright/mcp@latest`): navigate, read pages, screenshot.
- **GitHub, databases, everything else** — any server from the MCP ecosystem
  works if it runs as a command or an HTTP endpoint. Check the server's own
  README for the launch command and required environment (tokens go in the
  entry's `env` map — remember the registry file then contains a secret:
  keep its permissions tight).

---

## 5 · Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| The *Tools* field does not appear in the hire dialog | Nothing declared, or the registry failed to parse | Check the server's startup log: it names the file and the parse error. Restart after fixing. |
| Grant refused: *"unknown tool `x`"* | The name is not in `mcpServers` | Match the key exactly; restart if you just added it. |
| Grant refused naming another agent | The tool is in `exclusive` | Take it out of that agent's hand first (Org → Edit), then grant. |
| The agent reports the tool "could not connect" | The tool's target is not up (Blender closed, addon not serving, DB down) | Start the target, re-run the task. Overmind cannot start GUI applications for you. |
| `uvx`/`npx` "not found" or dies instantly in a run | The launcher is not installed, or its cache is denied by the cage | Install it (`brew install uv` / Node), and allow its cache: `OVERMIND_SANDBOX_ALLOW`. |
| Tool works in a task but not in chat | You are on a pre-M28 build | Update: since ADR-0036 conversational turns carry the granted servers too. |
| Registry edited but nothing changed | The file is read at startup | Restart Overmind. |

Still stuck: the run's session log (task → latest run) carries the adapter's
stderr, which is where MCP servers complain.
