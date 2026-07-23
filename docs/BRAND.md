<!-- markdownlint-disable MD033 MD041 -->
<p align="center">
  <img src="../.github/assets/brand/wordmark.svg" alt="Overmind" width="560">
</p>

# Overmind — Brand Guide

Overmind is the mind that runs your agent company. The identity should feel the
same way the product does: **calm, precise, and quietly powerful** — a modern
developer tool, not a toy. This guide is the source of truth for palette,
typography, logo usage, and voice. When in doubt, match the assets already in
[`.github/assets/`](../.github/assets/) rather than inventing something new.

> The identity is **dark-first and violet-forward.** Every brand asset is a
> self-contained dark card so it renders identically in light and dark GitHub
> themes. No pure black or pure white fills — ever.

---

## Palette

<p align="center">
  <img src="../.github/assets/brand/palette-swatches.svg" alt="Overmind palette" width="820">
</p>

One accent, a stack of near-black surfaces, a tuned set of text tints, and three
semantic colors. That is the whole system — resist adding a second accent.

### Accent — violet

| Token        | Hex       | Role |
| ------------ | --------- | ---- |
| `accent-500` | `#7c5cff` | Primary accent: links, focus rings, primary buttons, active nodes. The brand color. |
| `accent-400` | `#9d7bff` | Gradient top-stop and hover state. Pair with `accent-500` for the signature violet gradient. |
| `accent-200` | `#c9bcff` | Accent tint on dark: satellite nodes, quiet highlights, secondary accent text. |

> **Contrast note.** `#7c5cff` on `#1a1523` is ~3.8:1 — fine for large text,
> icons, borders, and UI chrome, but **below 4.5:1 for body copy.** For accent
> text at body size, step up to `accent-200`.

### Surface — near-black

| Token         | Hex       | Role |
| ------------- | --------- | ---- |
| `ink-950`     | `#150f1f` | Deepest shade; base of background gradients. |
| `canvas-900`  | `#1a1523` | **Canonical background.** The default page/canvas color. |
| `surface-800` | `#221a33` | Cards, panels, the logomark tile. |
| `raised-700`  | `#2a2140` | Raised surfaces: popovers, hovered rows, nested panels. |

### Text — on dark

| Token        | Hex       | Role |
| ------------ | --------- | ---- |
| `text-hi`    | `#f2eeff` | High-emphasis text, headings, the bright center of the mark. Use in place of pure white. |
| `text-body`  | `#e7e1f7` | Default body copy. |
| `text-muted` | `#a99cd6` | Secondary text, captions, labels. |
| `text-faint` | `#6d6386` | Disabled text, separators, bullet dots. |

### Semantic

| Token     | Hex       | Role |
| --------- | --------- | ---- |
| `success` | `#46cf9c` | Passing checks, green budget, healthy runs. |
| `warning` | `#f5b73d` | Approaching a limit, needs attention. |
| `danger`  | `#ff6b7a` | Failed run, blown budget, broken audit chain. |

Semantic colors are for **state**, never decoration — if it isn't communicating
success/warning/danger, it should be violet or a surface tone.

---

## Typography

Overmind ships no bundled brand font; it uses a clean system sans so the identity
renders crisply everywhere with zero loading cost. This is what the wordmark and
hero already use.

**Display / UI stack**

```
'Segoe UI', 'Helvetica Neue', Arial, sans-serif
```

**Monospace** (code, hashes, budgets, IDs)

```
'SFMono-Regular', 'JetBrains Mono', Menlo, Consolas, monospace
```

| Style       | Weight | Tracking | Use |
| ----------- | ------ | -------- | --- |
| Wordmark    | 700    | `-1.5`   | The "Overmind" lockup only. |
| Heading     | 700    | `-0.5`   | Section titles. |
| Body        | 400–500 | `0`     | Running text. |
| Label / eyebrow | 600–700 | `+1.5`, uppercase | Column headers, tags, pills. |

Tighten tracking as size grows; the wordmark is deliberately tight (`-1.5`).
Never letter-space body copy.

---

## Logo

Overmind's mark is an **orchestration hub**: a bright central mind wiring four
satellite worker nodes — the company of agents, and the overmind that runs them.

- **Logomark** — [`.github/assets/brand/logomark.svg`](../.github/assets/brand/logomark.svg):
  the mark in its rounded violet-stroked tile. Use where space is square or tight
  (avatars, favicons, app chrome).
- **Wordmark** — [`.github/assets/brand/wordmark.svg`](../.github/assets/brand/wordmark.svg):
  logomark + "Overmind" lockup. Use in READMEs, docs headers, and slides.

### Do

- Keep clear space around the mark equal to the height of the central node.
- Place the mark on `canvas-900`, `surface-800`, or its own dark tile.
- Scale the whole asset uniformly; the mark is built to hold up small.
- Use the provided SVGs as-is — they are already theme-safe.

### Don't

- ❌ Recolor the mark, re-stroke the tile, or swap the violet for another hue.
- ❌ Put the mark on a pure-white or busy photographic background.
- ❌ Stretch, skew, rotate, or add drop shadows / outer glows.
- ❌ Rebuild the wordmark in a different typeface or re-letter-space it.
- ❌ Crop the tile or detach the nodes from the central mind.

---

## Voice & tone

Overmind talks like a **senior engineer who respects your time**: precise,
confident, and free of hype. It explains the mechanism, not the magic.

- **Clear over clever.** Say what a thing does. "Isolated worktrees," not
  "next-gen agent sandboxing."
- **Concrete over grand.** Lead with the invariant — atomic budgets,
  hash-chained audit, memory-native — not adjectives.
- **Calm, not loud.** No exclamation walls, no emoji confetti. One sharp line
  beats three excited ones.
- **Honest about edges.** Say when something is optional (Wadachi always is),
  experimental, or not done yet.
- **Vocabulary is canonical.** company / project / goal / task — never
  ticket / issue / mission. See [`PAPERCLIP-ALIGNMENT.md`](PAPERCLIP-ALIGNMENT.md).

**Tagline:** *the mind that runs your agent company.*

One-liner: *Overmind orchestrates a company of AI agents — isolated worktrees,
atomic budgets, a hash-chained audit log, and organizational memory.*

---

## Assets

| File | What |
| ---- | ---- |
| [`.github/assets/brand/logomark.svg`](../.github/assets/brand/logomark.svg) | Standalone mark in its rounded violet tile. |
| [`.github/assets/brand/wordmark.svg`](../.github/assets/brand/wordmark.svg) | Logomark + "Overmind" lockup. |
| [`.github/assets/brand/palette-swatches.svg`](../.github/assets/brand/palette-swatches.svg) | The palette, rendered. |

*License: MIT, same as the project. Reuse the marks to refer to Overmind; don't
imply endorsement or modify them.*
