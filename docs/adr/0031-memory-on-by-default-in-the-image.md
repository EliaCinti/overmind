# ADR-0031: Memory is on by default in the image

- **Date:** 2026-08-20
- **Status:** accepted
- **Supersedes:** the second decision of [ADR-0014](0014-docker-deployment.md) ("No Wadachi in the image"), the same way M19 superseded its stance on the agent CLI.

## Context

M19's acceptance run produced a confident, well-written document about the
wrong Overmind — DarthSim's Procfile process manager. The mechanism was
flawless: the cage held, the CLI wrote a real file, the cost reached the
ledger. The *context* was empty. The agent had no company memory configured,
so it answered from world knowledge, and nothing told it otherwise. M19
recorded that as "a fact about running with `OVERMIND_MEMORY_CMD` unset, and
an argument for M8's managed brain being on by default in the image."

Everything above the provider already exists and is tested. ADR-0024 gives
each company its own brain under the data dir, provisioned by a
`create_dir_all` and a `BRAIN_DIR`; ADR-0025 records provenance; ADR-0026
carries watermarks; ADR-0027 hands agents `recall`/`why`/`changed_since`
through Overmind. The one missing piece is the first one: in the image,
`OVERMIND_MEMORY_CMD` is unset, and the provider it would name is not there
to be named. The feature that distinguishes the product is off in the
artifact most people will run.

ADR-0014 decided against bundling Wadachi to keep the two-project separation
(ADR-0003/0004: Overmind speaks MCP to *a* memory server, never imports one).
That reasoning was about **coupling**, and it still holds — nothing in
Overmind's code knows Wadachi exists. But the image is not the code: it is a
default *configuration* of it, and M19 already crossed the same line for the
same reason when it baked in `@anthropic-ai/claude-code`. "The server's
default adapter is `claude -p …`, so an image without the CLI dies at the
spawn" has an exact analogue: the server's memory contract is MCP over stdio,
so an image without any provider remembers nothing.

**Measured before deciding** (2026-08-20, wadachi 0.15.0 on PyPI):

| | cost | what it buys |
|---|---|---|
| `wadachi` (core) | 27 MB of wheels (numpy-dominated) | keyword search — a deliberate fallback, and the reply says so: `"search_mode": "keyword (…)"` |
| `[semantic]` extra | +35 MB of wheels; **292 MB as an installed layer** (onnxruntime 58, numpy 71 with its bundled libs, pillow 16 — measured in the built image, where wheels roughly double on install) | semantic recall — the thing "memory" means in practice |
| the model | +67 MB, `qdrant/bge-small-en-v1.5-onnx-q` | downloaded lazily on first embed |

Two facts weigh more than the megabytes:

1. **fastembed's default cache is `$TMPDIR/fastembed_cache`** — ephemeral in a
   container. Left alone, the first `recall` of every fresh container reaches
   the network silently and re-downloads 67 MB after every restart. On an
   offline machine it degrades to keyword search without saying so.
2. **A brain written without embeddings is not damaged.** Wadachi leaves
   `embedding = NULL` and backfills at the first semantic search, so the
   choice of what ships is reversible either way.

## Decisions

1. **The image carries Wadachi with the semantic extra, pinned.** Installed
   into its own venv (`/opt/wadachi`, bookworm's Python is PEP 668
   externally-managed) and symlinked onto `PATH`. Pinned and overridable at
   build time (`--build-arg WADACHI_VERSION=…`), for the same reason the
   agent CLI is: an image that installs "latest" changes under you between
   two builds of the same commit.
2. **The model is baked at build time, not downloaded on first use.** One
   `TextEmbedding(…)` call during the build populates `/opt/fastembed`, and
   `FASTEMBED_CACHE_PATH` points there. The image works offline, builds
   reproducibly, and no first recall reaches the network silently — the exact
   reasoning that pinned the CLI version. The spawned memory server inherits
   the server's environment, so the variable arrives without code changes.
3. **`OVERMIND_MEMORY_CMD=wadachi` is set by the image's environment, not by
   the code.** The compiled-in default stays `None`: on a host, Overmind
   configures nothing it was not asked for, and inventing a memory provider
   on somebody's machine is not ours to do — the same shape as
   `OVERMIND_AGENT_UID`, set by our image and unset everywhere else. Opting
   out in the container is explicit: `OVERMIND_MEMORY_CMD=""` disables memory
   visibly, and `OVERMIND_MANAGED_BRAIN=off` still shares one brain.
4. **Overmind still never imports Wadachi.** The coupling remains the
   protocol. Any conforming MCP memory server dropped into
   `OVERMIND_MEMORY_CMD` replaces it, exactly as `OVERMIND_AGENT_CMD`
   replaces the CLI.

## Alternatives considered

- **Core only, `[semantic]` as a build arg** — a 27 MB image bump instead of
  ~360 MB. Rejected: the default experience would be the degraded one, and
  this milestone exists because the default experience is the product. The
  megabytes sit in an image that already carries node, python, gh and the
  agent CLI.
- **Semantic wheels in the image, model on the volume** — 67 MB smaller, but
  the first recall of a fresh install needs the network once, silently, and
  an offline machine stays on keyword search without saying so. Rejected:
  "no first run reaches the network silently" is the whole point of baking.
- **A Wadachi sidecar container** — cleaner-looking separation, but Wadachi
  is stdio-spawned per call with `BRAIN_DIR` set per company (ADR-0024);
  a sidecar would need a TCP transport Wadachi does not speak (`wadachi
  serve-http` does not exist — measured, 2026-08-13) and a per-company
  routing story. Rejected as machinery invented to avoid a `pip install`.

## Consequences

- `docker compose up` produces a company whose agents remember — the M19
  acceptance defect cannot recur in the default configuration.
- The image grows by **~360 MB of layers** (venv 292 MB + model 67 MB,
  measured on the built image — the wheel sizes above roughly double once
  installed). Accepted and worded in the Dockerfile.
- The threat model's "a malicious MCP memory server is a command *you*
  configured" weakens honestly to: in the image, the default is a command
  *we* pinned. The pin is the mitigation, and the sentence is updated rather
  than left true-by-omission.
- CI's container smoke can now check memory end-to-end through the real
  provider: found a company, and its founding memory is readable back through
  `GET /companies/{id}/memory/memories` (ADR-0025) — no stub in the loop.
