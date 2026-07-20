# ADR-0010: Frontend stack (best-in-class graphical tech) and coarse live updates

- **Date:** 2026-07-20
- **Status:** accepted

## Context

M4 is the first UI. Elia's directive: **use the absolute best graphical technology** — beauty is a pillar (UX.md #7), not an afterthought. My initial instinct was to hand-roll CSS to keep the dependency surface small; the directive overrides that. Separately, the board must update live as agents work.

## Decisions

### Stack

- **Vite + React + TypeScript** (per ADR-0002), **Tailwind CSS v4** (`@tailwindcss/vite`), **Radix UI** primitives (accessible dialogs/controls), **Motion** (`motion/react`) for animation, **Lucide** icons, **clsx + tailwind-merge**. Fonts **self-hosted** via Fontsource (Inter + JetBrains Mono) — no CDN, works offline (matches the self-hosted, no-account pillar).
- This is also what Paperclip's UI uses (Tailwind v4 + shadcn/Radix), so "best tech" and "follow Paperclip" converge.
- **Design tokens** are the single source of visual values (Paperclip DESIGN.md #2): OKLCH semantic tier (background/foreground/card/muted/primary/accent/destructive/border) + a **status tier** with one hue per task lifecycle state, defined once in `index.css`, light + dark. Machine values (ids, costs, branches) render monospace (#6).

### Live updates: coarse-by-design

The server broadcasts a minimal `{ type: "changed", company_id }` over a `/ws` WebSocket whenever board state changes (a `broadcast::Sender` in `AppState`, `notify()` called after each mutating commit). The client **refetches the affected scope** rather than applying deltas. `hello` on connect / on lag tells it to resync wholesale.

### API surface

API nested under **`/api`**; `/ws` for the socket; the built SPA served at the root with SPA history fallback (`ServeDir` + `ServeFile`), only when `OVERMIND_WEB_DIR` exists (dev uses Vite's proxy). Two read endpoints added for the UI: `GET /companies/{id}/projects` (nested goals + workspaces) and `GET /tasks/{id}/sessions`.

## Alternatives considered

- **Hand-rolled CSS design system** — leanest, but forgoes the animation/accessibility quality of Radix+Motion and contradicts the "best graphical tech" directive. Rejected.
- **Delta-based live updates** (send the exact changed rows) — less refetching, but the wire format can desync from server truth and is fiddly to keep correct across every mutation. Rejected: coarse refetch is trivially correct, and SQLite reads are cheap for a self-hosted single node.
- **Server-Sent Events instead of WebSocket** — simpler, but we already want a bidirectional channel for future streaming; WS keeps that open.

## Consequences

- Two languages/build systems in the repo (Rust + a Vite frontend); `web/dist` is the build artifact the server serves.
- Progressive-disclosure hiring (ADR-0005's UX) ships now in M4, ahead of the M5 org UI, because it's the most compelling demo of the whole system.
- Diff review is currently read-only (syntax-colored unified diff). Inline diff *comments* and merge/commit actions belong to a dedicated review milestone.
- Coarse refetch means a burst of changes yields a burst of refetches; fine at self-hosted scale, revisit only if it ever isn't.
