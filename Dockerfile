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
ENV OVERMIND_WEB_DIR=/app/web/dist \
    OVERMIND_DB=sqlite:///data/overmind.sqlite \
    OVERMIND_DATA_DIR=/data \
    OVERMIND_ADDR=0.0.0.0:7070

WORKDIR /app
# Modes are the server's to set at startup — it knows which of these hold runs
# and which hold its own shelves, and it has to do it on every boot anyway for
# a volume that predates this layout.
RUN mkdir -p /data
VOLUME /data
EXPOSE 7070

# A minimal healthcheck the orchestrator can read.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD curl -fsS http://127.0.0.1:7070/api/health || exit 1

CMD ["overmind-server"]
