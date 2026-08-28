# syntax=docker/dockerfile:1
#
# Overmind — single self-contained image: the Rust server serves the API, the
# live socket, and the built React UI (ADR-0014). Includes the toolchain agents
# typically need (git, gh, ripgrep, python3, node) so tasks can run in-container.
# Bookworm across build and runtime keeps glibc compatible for the binary.

# ── build the web UI ───────────────────────────────────────────────────────
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# ── build the server ───────────────────────────────────────────────────────
FROM rust:1-bookworm AS server
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --release --locked -p overmind-server

# ── runtime ────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gnupg \
    # GitHub CLI (agents open PRs) from its official apt repo
    && mkdir -p -m 755 /etc/apt/keyrings \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
         -o /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
         > /etc/apt/sources.list.d/github-cli.list \
    # Node 22 (npm-based agent CLIs) from NodeSource
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends \
         git ripgrep python3 python3-venv gh nodejs \
    && rm -rf /var/lib/apt/lists/*

# The agent CLI, which is the whole point of the image having a toolchain.
#
# Until now the image installed git, gh, ripgrep, python3 and node — everything
# an agent's *work* needs — and not the thing that does the work. The server's
# default adapter is `claude -p …`, so every task in a fresh container died at
# the spawn with `command not found`, and the fix was a comment in
# docker-compose.yml telling you to derive your own image.
#
# Pinned, and overridable at build time (`--build-arg CLAUDE_CODE_VERSION=…`):
# an image that installs "latest" is an image that changes under you between two
# builds of the same commit. `OVERMIND_AGENT_CMD` remains the way to point
# Overmind at a different adapter entirely (ADR-0003's genericity is about the
# *memory* provider, but the same escape hatch exists here).
ARG CLAUDE_CODE_VERSION=2.1.233
RUN npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
    && npm cache clean --force

# The toolchain is yours to extend (M23). Dogfooding found an agent trying to
# verify its own LaTeX deliverable with pdflatex the image does not carry --
# it shipped the sheet anyway, honestly, with the failure logged beside it.
# Baking every toolchain in would grow the image without bound; instead one
# build argument adds what YOUR agents need:
#
#   docker compose -f docker-compose.yml -f docker-compose.build.yml \
#     build --build-arg EXTRA_APT_PACKAGES="texlive-latex-base texlive-lang-italian"
#
ARG EXTRA_APT_PACKAGES=""
RUN if [ -n "$EXTRA_APT_PACKAGES" ]; then \
      apt-get update \
      && apt-get install -y --no-install-recommends $EXTRA_APT_PACKAGES \
      && rm -rf /var/lib/apt/lists/*; \
    fi

# The memory provider (ADR-0031), for the same reason as the CLI above: the
# server's memory contract is MCP over stdio, so an image with no provider to
# name in OVERMIND_MEMORY_CMD remembers nothing — and M19's acceptance run
# showed what that costs: a confident document about the wrong company.
#
# Its own venv because bookworm's Python is externally managed (PEP 668), and
# the semantic extra because keyword search is the fallback, not the product.
# Pinned like the CLI; Overmind still never imports Wadachi — the coupling is
# the protocol, and OVERMIND_MEMORY_CMD remains the way to swap providers.
ARG WADACHI_VERSION=0.15.0
# The brain must never sink the ship (measured 27 Aug 2026: a first-time
# installer's `compose build` died on this layer — transient registry/network
# trouble — and "the build failed" read as "Overmind is broken"). Degrade in
# steps, loudly: semantic → keyword-only → no memory at all. The server
# already survives a missing provider at runtime; the build now matches.
RUN python3 -m venv /opt/wadachi \
    && { /opt/wadachi/bin/pip install --no-cache-dir "wadachi[semantic]==${WADACHI_VERSION}" \
         || { echo "=========================================================="; \
              echo "WARNING: wadachi[semantic] failed to install (network?)."; \
              echo "Retrying WITHOUT semantic search (keyword-only recall)."; \
              echo "=========================================================="; \
              /opt/wadachi/bin/pip install --no-cache-dir "wadachi==${WADACHI_VERSION}"; } \
         || { echo "=========================================================="; \
              echo "WARNING: wadachi could not be installed at all."; \
              echo "The image will run WITHOUT organizational memory."; \
              echo "Rebuild later, or set OVERMIND_MEMORY_CMD to your own."; \
              echo "=========================================================="; }; } \
    && { [ -x /opt/wadachi/bin/wadachi ] && ln -s /opt/wadachi/bin/wadachi /usr/local/bin/wadachi || true; }

# The embedding model, baked at build time. fastembed's default cache is
# $TMPDIR/fastembed_cache — ephemeral here, so left alone the first recall of
# every fresh container would reach the network silently and re-download 67 MB
# after every restart, and an offline machine would degrade to keyword search
# without saying so. Baking is what pinning the CLI version was: no first run
# changes behaviour based on what the network happened to answer.
ENV FASTEMBED_CACHE_PATH=/opt/fastembed
# Best-effort for the same reason: this step reaches Hugging Face, the most
# commonly blocked host in the whole build. Without the bake, semantic recall
# downloads the model on first use (or degrades to keyword) — a slower first
# run, not a broken image.
RUN /opt/wadachi/bin/python -c \
    "from fastembed import TextEmbedding; TextEmbedding('BAAI/bge-small-en-v1.5')" \
    || echo "WARNING: embedding model could not be baked (Hugging Face unreachable?) — semantic recall will fetch it on first use or fall back to keyword search"

COPY --from=server /app/target/release/overmind-server /usr/local/bin/overmind-server
COPY --from=web /web/dist /app/web/dist

# The agent is not the server, and neither of them is you (ADR-0029).
#
# Two reasons, and either alone would be enough. The boundary: an agent that
# misreads its task, or a prompt injection inside a document someone handed it,
# sits next to `overmind.sqlite` and its audit chain, every company's brain and
# the per-run MCP tokens — all of which stay the server's, unreadable to this
# uid. And the plain mechanics: the adapter CLI refuses
# `--dangerously-skip-permissions` as root, so an image whose agents are root is
# an image whose agents cannot write a file, however good its cage is.
#
# The server therefore stays root and drops to this uid per spawn. Overriding
# `user:` in compose takes that ability away: the server then says so at startup
# and its agents are read-only, rather than failing at the first write.
RUN useradd --create-home --uid 10001 --shell /bin/sh agent
ENV OVERMIND_AGENT_UID=10001 \
    OVERMIND_AGENT_GID=10001 \
    OVERMIND_AGENT_HOME=/home/agent

# Sensible container defaults; override any via the environment.
#
# OVERMIND_MEMORY_CMD is the image's environment, not the code's default
# (ADR-0031): on a host, Overmind configures no memory it was not asked for.
# Here the provider is baked in, so unset would only mean "forget by default".
# Set it empty to disable memory deliberately.
ENV OVERMIND_WEB_DIR=/app/web/dist \
    OVERMIND_DB=sqlite:///data/overmind.sqlite \
    OVERMIND_DATA_DIR=/data \
    OVERMIND_ADDR=0.0.0.0:7070 \
    OVERMIND_REPOS_DIR=/repos \
    OVERMIND_MEMORY_CMD=wadachi

WORKDIR /app
# Modes and ownership are the server's to set at startup — it knows which of
# these hold runs and which hold its own shelves, and it has to do it on every
# boot anyway for volumes that predate this layout. `/repos` exists empty so
# that a container with nothing mounted can still say so.
RUN mkdir -p /data /repos
VOLUME /data
EXPOSE 7070

# A minimal healthcheck the orchestrator can read.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD curl -fsS http://127.0.0.1:7070/api/health || exit 1

CMD ["overmind-server"]
