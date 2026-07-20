# ADR-0014: Docker deployment image; agent sandboxing deferred to M10 (containers over sandbox-exec)

- **Date:** 2026-07-20
- **Status:** accepted

## Context

Paperclip ships Docker for two distinct jobs: **deployment** (one image, `docker compose` quickstart, ECS task definitions) and **agent sandboxing** (running agents — especially untrusted code review — in isolated containers). Elia asked to adopt Docker; we separate the two concerns.

## Decisions

1. **A single deployment image now.** A multi-stage `Dockerfile` builds the web UI (Node) and the Rust server, then a slim Bookworm runtime serves both (the server already serves `web/dist` at the root). Bookworm across build and runtime keeps glibc compatible. The runtime bundles the toolchain agents typically need — git, gh, ripgrep, python3(+venv), node — so tasks can run in-container; the agent CLI itself is configured via `OVERMIND_AGENT_CMD` (adapter-agnostic, not baked in). `docker-compose.yml` gives a one-command quickstart with a named volume for `/data` (DB, worktrees, managed brains) and a documented mount for host repos.
2. **No Wadachi in the image.** Keeping the two-project separation (ADR-0004): memory stays optional via `OVERMIND_MEMORY_CMD`; the image doesn't vendor or bundle Wadachi. Users point it at a Wadachi install or a sidecar.
3. **Agent-in-container sandboxing → M10 (security), and Docker supersedes sandbox-exec there.** ARCHITECTURE.md's v0 note said "OS-level sandboxing (sandbox-exec on macOS first, Linux later)". Running each agent in its own container is the cross-platform, stronger isolation and the natural home for the security pillar. It's deferred to the security milestone, not folded into this deployment image.

## Alternatives considered

- **Embed the SPA in the Rust binary** (rust-embed) for a single static binary, no web build stage — neat, but the image still needs the agent toolchain, and serving from `web/dist` is already how dev and prod work. Rejected as unnecessary.
- **Bundle Wadachi and a specific agent CLI in the base image** — turnkey, but couples the image to one memory server / one agent and breaks the adapter-agnostic, two-project separation. Rejected; documented as user-extended layers instead.
- **Do container-per-agent sandboxing now** — that's the security milestone's work (spawning agents in constrained containers, filesystem/network limits), not a deployment image. Deferred to M10.

## Consequences

- "Run anywhere" with `docker compose up --build`; the healthcheck hits `/api/health`.
- The build wasn't verified on the machine that authored it (Docker daemon was down); it must be validated with a real `docker build` before relying on it. [To verify: `docker build -t overmind .`]
- M10 gains a concrete direction: agent isolation via containers, and ARCHITECTURE.md's sandbox-exec note is superseded by this ADR.
