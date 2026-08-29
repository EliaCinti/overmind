# ADR-0043: The compose file is the installer's

- **Date:** 2026-08-29
- **Status:** accepted
- **Builds on:** [ADR-0014](0014-docker-deployment.md) (one deployment image), [ADR-0029](0029-the-cage-inside-the-container.md) (the agent's credentials on their own volume). Supersedes ADR-0014's "run anywhere with `docker compose up --build`".

## Context

On the two-machine walk a friend on a fresh machine ran `docker compose pull`
to get 0.2.1 and saw no new version land. The compose declared both
`image: ghcr.io/eliacinti/overmind:latest` and `build: .`; an earlier
`up --build` on that machine had tagged a from-source image with the published
image's name, and what Compose does with a service that is both buildable and
pullable varies by version and by flag. The fix was "delete everything and
download again" — a sledgehammer, and the install itself still opened with
`git clone`, which no installer of an image should need.

A tracked `docker-compose.override.yml` (an M23 dogfood mount of
`/tmp/m23-repos:/repos`) added a second ambiguity: Compose merges it silently
into every plain `docker compose` run in a clone and into none run elsewhere.

## Decision

1. **`docker-compose.yml` names only the published image**, declares
   `name: overmind` so the container and volumes carry the same names wherever
   the file lives, and `docker compose up -d --pull always` is the one command —
   to start and to update. Offline, plain `up -d` starts the image already
   present.
2. **Building from source is a separate file, `docker-compose.build.yml`**,
   layered with `-f` (or `COMPOSE_FILE`). It tags the build `overmind:local`,
   never the published name; `pull_policy: build` keeps `--pull always` from
   asking a registry for it. `EXTRA_APT_PACKAGES` and the dogfood `/repos` mount
   live there. The tracked override file is gone.
3. **The compose file ships with every release as an asset**, and the
   Quickstart downloads that one file instead of cloning. The asset names
   `latest`: it is where you get the file, not a pin — a pinned asset would
   break the one update line, and pinning stays a deliberate edit of `image:`.

## Consequences

- In a clone, `docker compose up --build` builds nothing; the compose header,
  README, CONTRIBUTING and the Dockerfile say where the build went.
- A developer carries the two `-f` flags (or `COMPOSE_FILE`) into every later
  command: a plain `up -d --pull always` puts the published image back, with
  none of what was built in — said in the overlay and in the README.
- Existing installs keep their data: a clone's project name was already
  `overmind`, so `name: overmind` names the same volumes.
- The README's `curl` points at raw `main` until a release carries the asset;
  after that it should point at `releases/latest/download/docker-compose.yml`
  (noted in NEXT.md).

## Rejected

- **`docker-compose.override.yml` for the build**, auto-merged: it is the
  ambiguity itself — the same command means two things in two directories.
- **Both keys with `pull_policy`**: the built image would still claim the
  published name locally.
- **A version-pinned asset per release**: "file belongs to version" reads well
  and breaks "the same line updates".
