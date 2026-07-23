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

## Branch & Pull Request Workflow

The `main` branch is **protected**. Direct pushes are not allowed. 

1. **Branch:** Create a new branch for your work (e.g., `feat/my-new-feature` or `docs/add-guide`).
2. **Commit:** Use conventional commits (e.g., `feat: ...`, `fix: ...`, `docs: ...`).
3. **Push:** Push your branch to the repository (or your fork).
4. **Open a PR:** Open a focused, descriptive Pull Request against `main`. 
5. **Review:** Wait for Elia to review. **Do not merge your own PR.**

## Questions?

If you get stuck or have questions about how a feature should be implemented, please open an **Issue** before spending too much time on code. We want this project to be welcoming, so please be respectful in all interactions.
