# ADR-0053: The cage grants nothing the host runs

- **Date:** 2026-11-06
- **Status:** accepted (2026-11-06), with the owner's approval of the plan that implements it
- **Corrects:** `adapter_paths` in `sandbox.rs`, in force since [ADR-0023](0023-os-level-sandboxing.md)
- **Builds on:** [ADR-0023](0023-os-level-sandboxing.md) (deny by default), [ADR-0029](0029-the-cage-inside-the-container.md) (the cage is a set of mechanisms), [ADR-0052](0052-a-run-owns-its-repository.md) (the same hole, in the operator's repository), ADR-0051, in a later docs PR (the host's login is never used implicitly)
- **Plan:** [what the cage grants on the host](../plans/2026-11-06-what-the-cage-grants-on-the-host.md)
- **Amended by:** the [addendum](#addendum--uvx-needs-no-grant-on-localshareuv-2026-11-06) below, on what `uvx` needs

## Context

ADR-0023 chose deny by default over a blocklist, naming `~/.config/gh` as the kind of place a blocklist leaves open. The profile it produced grants that place. `adapter_paths` puts four paths of the server's `$HOME` in `writable_paths`, and on macOS a writable path is `(allow file*)`, reading included:

- `~/.config`, commented *"shared tool configuration a coding task legitimately reads"*, though the grant is for writing too;
- `~/.local`, for the CLI's binary;
- `~/.claude` and `~/.claude.json`, for the CLI's credentials, sessions and settings.

Measured on 6 Nov 2026 with `sandbox-exec` and the same rules, on the development Mac:

| From inside the cage | Result |
|---|---|
| read a file of API keys kept under `~/.config` | **allowed** |
| read `~/.config/gh/hosts.yml` | **allowed** (the token itself is in the Keychain on this machine) |
| write in `~/.config` | **allowed** |
| write in `~/.local/bin`, which holds `claude` and `wadachi` | **allowed** |
| write in `~/.claude` | **allowed** |
| read `~/.ssh/config`, `~/.zshrc`; write in `~/.ssh`, `~/` | denied |

The network is open by design (ADR-0023), so what an agent can read it can send. What it can write, the host later runs, outside any cage:

- `~/.config/git/config`, read by the server's own `git()` at the next `prepare_clone` and by the owner's git;
- `~/.local/bin/claude`, the next time the owner types `claude`;
- `~/.claude/settings.json`, whose hooks run at the owner's next Claude Code session.

This is ADR-0052's hole in another directory: a grant given so that a tool works, reaching a place something more privileged reads later. It applies wherever the server's `$HOME` is the owner's: the native instance on macOS, and a native Linux host under Landlock, which shares `writable_paths`. The image is not affected: there the agent's home is its own and the uid is the boundary.

The same day, the CLI answered on the host's login, with no key or token in its environment, with `~/.config` absent from the profile and `~/.local/bin` and `~/.local/share/claude` read-only. So, for the CLI alone, without an MCP server, those grants were not what it needed.

## Decision

**1. The rule.** A path enters the cage only if the agent's own run needs it, and it enters with the least access that run needs. A path the host executes, or reads as configuration, does not enter writable. A credential that is not the adapter's own does not enter at all.

**2. `~/.config` leaves the cage**, for reading and writing. A toolchain that needs a part of it asks through `OVERMIND_SANDBOX_ALLOW`, which is the operator's decision and visible in their configuration.

**3. `~/.local` becomes read-only and narrow:** `~/.local/bin` and `~/.local/share/claude`. `adapter_read_paths` also reaches Landlock, which today receives only `cage.readable`. The CLI can no longer update itself from inside a run, which is correct: replacing the binary the host runs is the host's job.

**4. `OVERMIND_SANDBOX_READ`** is added beside `OVERMIND_SANDBOX_ALLOW`: colon-separated, read-only. Without it, an operator whose tool has to read something can only grant it writable, as the native instance does today with `~/.local/share/uv`, where `wadachi` is installed.

**5. The server's git reads none of the host's configuration.** `git()` and the clone run with `GIT_CONFIG_SYSTEM=/dev/null`, a `GIT_CONFIG_GLOBAL` of Overmind's own (`/dev/null`, or the clone's `SafeDirectoryConfig`), `core.hooksPath=/dev/null` and `core.fsmonitor=false`. This holds even for what the operator set up themselves: a `core.hooksPath` or an `init.templateDir` in their `~/.gitconfig` is not meant for a clone the server makes for an agent.

**6. `~/.claude` and `~/.claude.json` stay writable, and the hole they leave is stated.** The CLI writes there on every run while it runs on the host's login. An agent can plant a hook that runs as the owner at their next Claude Code session, so on a native instance **the cage does not hold against a hostile agent until this is closed**. `docs/THREAT-MODEL.md` says so in those words from this change on.

It closes with the agent's own home on the native instance, in M34 step 4, together with removing the implicit host login (ADR-0051 decision 4). The two are one change: a private home has no Keychain login, so it needs an account Overmind holds, so it needs the panel. Closing this is added to that step's definition of done.

## Consequences

- **A toolchain that wrote under `~/.config` or `~/.local` fails loudly**, which is the direction ADR-0023 chose. The fix is one environment variable, and the startup line and README name it.
- **The native instance's `start.sh` changes:** `~/.local/share/uv` moves to `OVERMIND_SANDBOX_READ`. `~/.cache/uv` stays writable because `uvx` needs a cache. A poisoned cache could reach the host's `uv`, so that remains the operator's stated risk, recorded in THREAT-MODEL. No instance outside this repository's owner is known to rely on these grants, and the image is untouched.
- **The cage still does not protect against a hostile agent on a native instance**, until M34 step 4. It is narrower, and honest about the remainder.
- **What this does not change:** the network stays open, and the Keychain stays readable (ADR-0023's M23 finding).

## Rejected

- **Read-only `~/.config`.** Writes stop, but reading the owner's keys with the network open is the worse half.
- **Read-only `~/.local` as a whole tree.** One line simpler, but it is a blocklist's reasoning: every other tool's data in `~/.local/share` comes along, and the next credential file a tool invents with it.
- **Denying `settings.json`, `hooks/`, `plugins/` and the like inside `~/.claude`.** A list again, with the gap ADR-0052 rejected a list for. `~/.claude.json` carries `mcpServers` and has to stay writable, and the list has to follow every new extension point of a tool we do not own.
- **Closing `~/.claude` now, with a private home and a token placed by hand.** It works, but the owner chose on 28 Oct to have no manual stop-gap for accounts, and until the panel exists it would have to be redone on every instance. No run has started on the native instance since 12 Sep 2026.
- **Listing keys in `git_isolation` and relying on that for `git()`.** `git()` has the isolation's job only for configuration, not its other settings: it has to keep working on the operator's repository, with its remotes, so it does not take the agent's `GIT_SSH_COMMAND` or credential resets.

## Addendum — `uvx` needs no grant on `~/.local/share/uv` (2026-11-06)

The live check of this ADR's plan found two things wrong in decision 4 and in the first consequence. The rest stands.

**The reason given was wrong.** The grant on `~/.local/share/uv` is not for `wadachi`. The brain reaches an agent over HTTP, at the server's `/mcp`, and `wadachi` runs under the server, outside the cage. What runs in the cage and uses that directory is `uvx`, for a tool such as BlenderMCP. The grant was worse than described: `wadachi` is installed there, so an agent could replace a program that runs as the server.

**The fix given does not work.** Read-only, `uvx` does not start: it creates a temporary file in the directory it installs tools in. Measured on 6 Nov 2026, with the cage's rules and `uvx --from blender-mcp`:

| `~/.local/share/uv` | `UV_TOOL_DIR` | `uvx` | Plant beside `wadachi` |
|---|---|---|---|
| writable (the native instance until now) | unset | runs | **allowed** |
| read-only (this ADR, above) | unset | **fails**: `Could not create temporary file` | refused |
| not granted | under `~/.cache/uv` | runs | refused |

With the third row and `OVERMIND_SANDBOX_ALLOW="$HOME/.cache/uv"`, the real CLI reports the tool `connected`, through the real cage.

A Python that `uv` manages itself lives under the same directory, and installing one there is refused too. Measured with `UV_PYTHON_PREFERENCE=only-managed`: `failed to create directory ~/.local/share/uv/python`, and it runs with `UV_PYTHON_INSTALL_DIR` moved.

**Decision.** `~/.local/share/uv` is not granted at all. A `uvx` tool runs with `UV_TOOL_DIR` and `UV_PYTHON_INSTALL_DIR` under `~/.cache/uv`, set in the server's environment, and that is what the native instance's `start.sh`, the README and `docs/TOOLS.md` say. It does not reduce the risk this ADR states for `~/.cache/uv`, and it widens where that risk reaches: anything the host or the server launches through `uv` with these variables runs what an agent could have planted there. The documented memory server is plain `wadachi`, which does not.

`OVERMIND_SANDBOX_READ` stays. It is measured to read and not write, and the README keeps it as the way to fit a toolchain that only has to read.

**The rule also has to hold after an agent has been there.** What an agent may write it may replace. Measured from inside the cage: `~/.claude.json`, `~/.claude` and the run directory all took a link to `~/.config`, and the next build, resolving it, granted `~/.config` to read and write. So those paths are never followed if they are links. A run directory that is a link gets no profile, and a resume refuses it. An operator whose `~/.claude` is a link of their own names the real place in `OVERMIND_SANDBOX_ALLOW`.

**Decision 5 was also narrower than it read.** `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` do not cover `~/.config/git/ignore` and `attributes`, which git reads regardless. `core.excludesFile` and `core.attributesFile` now point at `/dev/null`, for the server's git and the agent's.
