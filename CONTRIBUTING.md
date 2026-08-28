# Contributing to Overmind

Thanks for your interest in Overmind! 

Overmind is an open-source orchestration tool for teams of AI agents. It consists of a Rust server (axum + SQLite) and a React + TypeScript frontend.

## Philosophy

Before you start, keep these core principles in mind:

1. **Quality is non-negotiable:** All tests must pass, clippy and lints must be clean. Every architectural decision requires an Architecture Decision Record (ADR) in `docs/adr/`.
2. **Correctness-first stack:** We rely on the Rust compiler for concurrency-critical parts. We strictly enforce `unwrap_used = "deny"` in non-test code to prevent panics.
3. **UX matters:** Any user-facing feature should follow our UX principles outlined in `docs/UX.md` — progressive disclosure, click-first, and structured options enforced by the server.
4. **Memory is optional but first-class:** Any feature touching the organizational memory (Wadachi via MCP) must work both with and without a memory provider configured.

## Dev Setup

### Prerequisites

- **Rust:** Managed automatically via `rust-toolchain.toml`.
- **Node.js & npm:** Required for the frontend in the `web/` directory.

### 1. Build the UI

The Rust server serves the compiled frontend. Build it first:

```bash
cd web
npm install
npm run build
cd ..
```

### 2. Run the Server

Start the backend:

```bash
cargo run
```

The server will be available at `http://127.0.0.1:7070`, serving both the API, the WebSocket, and the built UI.

### 3. Frontend Dev (Hot Reload)

If you are working on the UI, start the Vite dev server. It will automatically proxy `/api` and `/ws` requests to your running Rust server:

```bash
cd web
npm run dev
```

## Testing & Checks

Before opening a Pull Request, verify that your changes pass all checks. These are the exact commands executed by our CI (`.github/workflows/ci.yml`). Do not invent new ones.

### Rust (Backend)

```bash
# Format code
cargo fmt --all

# Run linter (must be warning-free)
cargo clippy --workspace --all-targets -- -D warnings

# Run all tests
cargo test --workspace
```

### React/TypeScript (Frontend)

```bash
cd web

# Run oxlint
npm run lint

# Check types and build
npm run build
```

### The image

CI also builds the Docker image and runs a real task inside it (`.github/scripts/container-smoke.sh`) — "does it build" is not the bar, "an agent can do a day's work in it" is. Locally: `docker compose -f docker-compose.yml -f docker-compose.build.yml build` then `IMAGE=overmind:local bash .github/scripts/container-smoke.sh` (the default `docker-compose.yml` names only the published image; the build lives in the override).

### Writing tests

- **Test first.** Watch it fail for the reason you expect, then make it pass. A test that passed the moment it was written has proved nothing yet.
- Name the behaviour, not the function: `a_cheap_agent_is_priced_by_its_own_ledger_not_a_flat_guess`, not `test_estimate`.
- Tests never touch the real agent CLI: the door suite runs with `agent_cmd: Some("/usr/bin/true")`, the runner suites with a stub script. A test that spawned the real CLI on a machine that has one once opened the owner's browser a dozen times a day — it is in the roadmap as a lesson.
- Every claim in `docs/THREAT-MODEL.md` names the test that holds it; a security change updates both.

## Architecture decisions

Anything that changes a boundary, a contract or a default is an ADR in `docs/adr/` (copy `0000-template.md`), written **before** the code and linked from the roadmap entry. The ADR says what was decided, why, and what was rejected; the roadmap says what shipped and how it was accepted.

## Releasing

Releases are tags. `CHANGELOG.md` gets its entry first (the GitHub Release notes are extracted from it — the workflow fails if the entry is missing), the workspace version in `Cargo.toml` matches, then `git tag vX.Y.Z && git push --tags`: `.github/workflows/release.yml` builds the image, pushes it to `ghcr.io/eliacinti/overmind` (`X.Y.Z`, `X.Y`, `latest`) and publishes the release.

## Branch & Pull Request Workflow

The `main` branch is **protected**. Direct pushes are not allowed. 

1. **Branch:** Create a new branch for your work (e.g., `feat/my-new-feature` or `docs/add-guide`).
2. **Commit:** Use conventional commits (e.g., `feat: ...`, `fix: ...`, `docs: ...`).
3. **Push:** Push your branch to the repository (or your fork).
4. **Open a PR:** Open a focused, descriptive Pull Request against `main`. 
5. **Review:** Wait for Elia to review. **Do not merge your own PR.**

## Questions?

If you get stuck or have questions about how a feature should be implemented, please open an **Issue** before spending too much time on code. We want this project to be welcoming, so please be respectful in all interactions.
