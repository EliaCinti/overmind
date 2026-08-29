# Overmind — notes for an agent working in this repository

Overmind is the mind that runs a company of AI agents: a Rust server
(`crates/overmind-server`, axum + SQLite) and a React + TypeScript UI (`web/`),
with organizational memory over MCP (Wadachi by default). Read `README.md` for
what it is, `docs/ARCHITECTURE.md` for how it is shaped, `docs/ROADMAP.md` for
what shipped and how each milestone was accepted, `docs/NEXT.md` for what comes
next and why. `CONTRIBUTING.md` is the rulebook; this file is the short form.

## Before you write code

- **A boundary, a contract or a default changes → an ADR first** in `docs/adr/`
  (copy `0000-template.md`, next number, link it from the roadmap entry). The
  ADR says what was decided, why, and what was rejected. Code follows the ADR,
  never the other way round.
- **Test first.** Write the test, watch it fail for the reason you expect, then
  make it pass. Name the behaviour, not the function
  (`a_cheap_agent_is_priced_by_its_own_ledger_not_a_flat_guess`).
- **Tests never spawn the real agent CLI.** The door suite runs with
  `agent_cmd: Some("/usr/bin/true")`, the runner suites with a stub script.
- **Security is held by tests.** Every claim in `docs/THREAT-MODEL.md` names the
  test that holds it; a change to the door (`auth.rs`), the cage (`sandbox.rs`,
  `landlock.rs`), credentials (`claude_auth.rs`, `scrub_secrets`), the audit
  chain (`audit.rs`) or anything that moves data out of `/data` updates both the
  test and the threat model.
- **UX is a pillar** (`docs/UX.md`): progressive disclosure, click first / type
  last, every Level 1/2 option maps to server-enforced configuration. A
  user-facing string is a choice, not a text field, whenever it can be.
- **Memory is optional and first-class.** Anything touching the brain must work
  with `OVERMIND_MEMORY_CMD` set and with it empty.
- The company has a language (`i18n.rs`, M16): every agent prompt carries it,
  and the prose the server composes for a person (start summaries, compaction
  notices, meeting requests) goes through that module's helpers — not a
  hardcoded English string. UI chrome is the web app's dictionary.

## The checks (exactly CI's, `.github/workflows/ci.yml`)

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings   # unwrap_used = deny outside tests
cargo test --workspace
cd web && npm run lint && npm run build
```

Image: `docker compose -f docker-compose.yml -f docker-compose.build.yml build`,
then `IMAGE=overmind:local bash .github/scripts/container-smoke.sh`. Do not
invent other checks; do not skip these.

## Running it

- Native (macOS, the platform it is developed on): build the UI once
  (`cd web && npm install && npm run build`), then `cargo run` → :7070. Every
  run is caged with `sandbox-exec`.
- The image: `docker compose up -d --pull always` is the one command, to start
  and to update. `docker-compose.yml` names only the published image on
  purpose; the build lives in `docker-compose.build.yml`.

## Branches, commits, releases

- `main` is protected. Branch (`feat/…`, `fix/…`, `docs/…` — or a name that says
  what the change *is*), open a focused PR, **never merge it yourself**.
- A commit title is a sentence that says what changed and why it matters
  (`A token the CLI printed is a token the flow keeps`), a `docs(next): …` or
  `fix: …` prefix when the change is that plain. The body carries the story.
- Every change a person can notice gets a line in `CHANGELOG.md` under
  `[Unreleased]`, in the changelog's voice (bold claim, then the reason).
  Milestone status lives in `docs/ROADMAP.md`; frictions and debts in
  `docs/NEXT.md`. Keep all three true — they are read, not archived.
- Releases are tags: CHANGELOG entry first, `Cargo.toml` workspace version
  matches, `git tag vX.Y.Z && git push --tags`. The workflow builds the
  multi-arch image, pushes it to `ghcr.io/eliacinti/overmind` and publishes
  the release with the changelog entry as its notes.

## What not to do

- No `unwrap()`/`expect()` outside tests; no panics on user input.
- No credential in a log line, an error message or a UI tail — everything the
  sign-in flow emits passes `scrub_secrets`.
- No `0.0.0.0` binding, no widening of the cage, no new path out of `/data`
  without an ADR and a threat-model line.
- No new check, tool or dependency "for convenience" without saying so in the
  PR.
