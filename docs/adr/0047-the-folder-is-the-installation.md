# ADR-0047: The folder is the installation

- **Date:** 2026-09-02
- **Status:** proposed
- **Supersedes** ADR-0043's choice of Docker named volumes (the rest of ADR-0043 — one image, one command to start and update, the compose shipped as a release asset — stands).

## Context

`docker-compose.yml` kept everything in two Docker named volumes,
`overmind-data` and `overmind-agent-home`, and ADR-0043 chose them for a
specific reason: the compose declares `name: overmind`, so the volumes carry
the same names wherever the file is, and *an update run from another directory
finds your data*. That was written after the two-machine walk, where exactly
that had gone wrong.

Two things pushed back, both from the owner running it for real:

1. **"If I tear the container down I lose everything."** That is not true —
   named volumes survive `docker compose down`; only `-v` removes them, which
   the file says in capitals — and it was demonstrated on his own machine
   before anything was changed. But the belief is reasonable, because a named
   volume is invisible: the data is somewhere under `/var/lib/docker/volumes`,
   under a name you did not choose, and nothing about it looks like *yours*.
2. **The priority is persistence, not convenience.** Asked to weigh it, the
   owner was explicit: `docker compose down -v` must not be able to destroy the
   instance, and on Linux the data being root-owned — readable only with
   `sudo` — is a **wanted** property rather than a cost.

A first draft reached for a fixed absolute path, `/opt/overmind/data` on Linux.
It does not work: **Docker Desktop on macOS does not share `/opt`** ("Mounts
denied: the path is not shared from the host"), so that default would fail on
the first command on the machine Overmind is developed on. A `${HOME}`-based
default works everywhere but puts the data somewhere the person did not choose,
which is the complaint again in a different place.

## Decision

**The folder that holds `docker-compose.yml` is the installation.**

```yaml
    volumes:
      - ${OVERMIND_DATA:-./data}:/data
      - ${OVERMIND_AGENT_HOME:-./agent}:/home/agent/.claude
```

Docker creates both on the first start. There is no path to choose and none to
document per platform: you download the file into a folder, and that folder is
your Overmind. To put the data elsewhere on purpose — `/opt/overmind/data`, an
external disk — set `OVERMIND_DATA` to an absolute path, in a `.env` beside the
file or in the shell.

And **`container_name: overmind`**, so it is `docker logs overmind` rather than
`overmind-overmind-1`, which Compose builds from `<project>-<service>-<index>`
and which reads like a mistake.

## Consequences

- **`docker compose down -v` becomes harmless.** A bind mount is not a named
  volume; the flag cannot reach it. Verified end to end against the 0.2.3
  image: claim, found a company, `down -v`, restart — the owner and the company
  are still there.
- **The data is in plain sight**, `ls`-able, `cp`-able, and part of any ordinary
  backup. The setup code in particular is now `cat data/setup-code` rather than
  a grep through `docker compose logs`, which removes a step from every first
  install until M32's script exists.
- **The whole install moves by moving the folder** — compose, data, sign-in.
- **What we give up, and it is the thing ADR-0043 bought:** running
  `docker compose` from another directory now yields an empty instance, because
  `./data` is somewhere else too. The mitigation is one sentence repeated in the
  compose, the README and the wiki — *the folder is the installation* — and one
  property that the named-volume version did not have: **the failure is
  visible.** An empty `./data` beside the file is understood in seconds; data
  living under an unexpected volume name in a system directory nobody opens is
  what cost a day in August.
- On Linux the files are owned by root, because the server runs as root inside
  the container (only the agent is uid 10001). Reading them from the host takes
  `sudo`. Deliberate: whoever cannot `sudo` on the box cannot read the
  instance's data.
- A checkout is not an installation: `/data/` and `/agent/` join `.gitignore`,
  because the installer's own file now writes beside itself and a developer
  running it from the repository must not commit an instance.

## Rejected

- **A fixed `/opt/overmind/data`.** Right for Linux (FHS), and denied by Docker
  Desktop on macOS out of the box. A default that fails on the first command on
  one of the three supported platforms is not a default.
- **`${HOME}/.overmind/data`.** Portable, and it works — but it answers "where
  are my data" with a path the person did not pick, which is the original
  complaint wearing different clothes.
- **Shipping a tarball whose extraction point becomes the install.** The idea
  the owner proposed, and its whole virtue survives here: a single file
  downloaded into a folder already *is* an extraction. A tar would add a step
  and a format without adding a property.
- **Keeping named volumes and documenting that `down` is safe.** True, and it
  had already been explained once; a design that needs the same reassurance
  twice is answering the wrong question.
