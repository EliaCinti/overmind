# Overmind — public website

The static website for **overmind.eliacinti.dev**. Plain HTML/CSS, minimal JS
(none, in fact), fully self-contained. Extends the single-page landing
(`docs/landing/index.html`) into a full site without redesigning the
Living Memory (轍) identity.

## Sitemap

| Page | File | What's on it |
|------|------|--------------|
| **Home** | `index.html` | Sharper hero + the memory-native pitch, the three pillars, teaser bands into every subpage, quickstart, footer. |
| **Features** | `features.html` | The eight essentials, each a one-line explainer + a small inline SVG: organizational memory, hash-chained audit, isolated worktrees, atomic budgets, approval gates, org chart, heartbeats & recovery, guided hiring. |
| **How it works** | `how-it-works.html` | The control-plane story with `architecture.svg`, expanded to four steps (recall → checkout → work → record), closing on the `proof-chain.svg` audit band. |
| **The app** | `app.html` | Faithful SVG mockups of the real `web/` UI — Kanban board, org chart, task-detail drawer, hire dialog — true to the app tokens (OKLCH violet, per-status hues, Inter + JetBrains Mono). |
| **Wadachi** | `wadachi.html` | Powered by Wadachi 轍, with the real Wadachi logomark; recall / store / why, how it's wired over MCP, and why it's always optional. |

## Assets (`assets/`)

Everything is bundled into the site's own web root — no repo-relative
`../../.github/...` paths, so it serves correctly from any document root.

- **Inherited** (copied from `.github/assets/`): `hero.svg`,
  `proof-chain.svg`, `architecture.svg`, `brand/{wordmark,logomark,palette-swatches}.svg`.
- **Wadachi logomark**: `wadachi-logomark.svg` (+ `-light` variant) — the real
  mark, brought in from branch `design/wadachi-real-logo`.
- **New for this site** (hand-authored SVG mockups of the product UI):
  `mock-board.svg`, `mock-org.svg`, `mock-task.svg`, `mock-hire.svg`.
- **Styles**: one shared `site.css`.

## Deploying behind nginx

Static files — copy `docs/site/` to the document root and serve. No build step,
no external CDNs, analytics, or remote fonts (system font stack, so it works
behind nginx + Cloudflare). Example server block:

```nginx
server {
    server_name overmind.eliacinti.dev;
    root /var/www/overmind-site;      # contents of docs/site/
    index index.html;
    location / { try_files $uri $uri/ =404; }
}
```

Fonts: the CSS asks for Inter / JetBrains Mono if the visitor already has them
and otherwise falls back to the native system stack — nothing is fetched
remotely. To pin the exact brand faces, self-host the woff2 files under
`assets/fonts/` and add `@font-face` rules to `site.css` (optional).
