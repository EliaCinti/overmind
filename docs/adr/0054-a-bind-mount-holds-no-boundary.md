# ADR-0054: A bind mount holds no boundary

- **Date:** 2026-11-07
- **Status:** accepted (2026-11-07), with the owner's approval of the plan that implements it
- **Corrects:** the claim of [ADR-0029](0029-the-cage-inside-the-container.md) that the uid split "works identically on Docker Desktop", which stopped being true for the default install with [ADR-0047](0047-the-folder-is-the-installation.md)
- **Builds on:** [ADR-0029](0029-the-cage-inside-the-container.md) (the cage is a set of mechanisms), [ADR-0047](0047-the-folder-is-the-installation.md) (the folder is the installation), [ADR-0052](0052-a-run-owns-its-repository.md) (a run owns its repository)
- **Plan:** [a bind mount holds no boundary](../plans/2026-11-07-a-bind-mount-holds-no-boundary.md)
- **Advisory:** GHSA-cr22-mp4p-vj89, private until the release that closes it

## Context

In the image, the uid split is the whole cage on Docker Desktop, whose kernel has no Landlock. It rests on one assumption: that a file owned by root at `0600` is refused to uid 10001.

Docker Desktop's VirtioFS bind mounts break that assumption. A file on one appears to be owned by whichever uid is looking at it. ADR-0029 was measured on a named volume, where the assumption holds. ADR-0047 then moved the default to the bind mount `./data:/data`, released in v0.2.4, and nothing re-measured the boundary. CI could not have seen it: it runs on Linux, without Docker Desktop.

Measured on 7 Nov 2026 on Docker Desktop for macOS (Docker 28.0.4, kernel 6.10.14-linuxkit), with `overmind:main`, as uid 10001, by `docker exec` in a throwaway instance:

| As uid 10001 | Bind at `/data` | Volume at `/data` |
|---|---|---|
| read `overmind.sqlite`, its WAL, `setup-code` | **reached** | refused |
| open `overmind.sqlite` for writing | **reached** | refused |
| create a file in `/data` | **reached** | refused |
| list and create in `companies`, `backups`, `artifacts`, `attachments` | **reached** | refused |
| list `worktrees`, `sessions`, `chat`, `meetings` | **reached** | refused |
| list `/data` itself (`0755`) | reached | reached |

The result is the same on the internal disk and on an external one. A mounted repository is reachable the same way: read, create, modify, and write `.git/hooks/post-checkout`, seen from the host. So on the default install, `docs/THREAT-MODEL.md` is false where it says an agent stays out of Overmind's own data, and where it says the repository is out of the cage.

The compose file already warns that exFAT, NTFS and network shares do not honour ownership. It did not know Docker Desktop belongs on that list.

## Decision

**1. Nothing the agent must not reach is reachable by a path it can walk.** A permission on a file behind a bind mount means nothing, but a permission on a directory in the container's own layer is real, and the kernel checks it on the way to the mount. The image creates `/srv/hold` at `0700` root, and `/data` and `/repos` become links to `/srv/hold/data` and `/srv/hold/repos`.

Docker follows the link when it mounts, so a compose file already on someone's disk, naming `/data` and `/repos`, lands behind the hold with an image update and no edit. Paths stored in the database stay valid, and on the host the folder is still the installation.

**2. Runs leave `/data`.** An agent has to reach its run directory, so run directories cannot stand behind the hold, and a bind mount would not keep them apart anyway. `OVERMIND_RUNS_DIR` holds `worktrees`, `sessions`, `chat` and `meetings`. It defaults to the data dir, so a native install does not change. The image sets it to `/runs`, which is a volume and therefore honours ownership.

**3. The compose file names that volume.** A compose file from before this change gets the image's anonymous volume instead, which works and is less durable (see Consequences).

**4. The server measures the boundary at startup instead of assuming it.** Where there is an agent uid, the server writes a `0600` canary in the data dir and tries, as that uid, to read it and to list the repositories dir. If either is reached, **the uid stops counting as a mechanism**. Where Landlock is held it still stands, since its ruleset is deny by default for reads too. Where nothing else is held, as on Docker Desktop, no run is caged: agents are read-only and the startup line names the reason. This is what catches the layouts decisions 1 to 3 do not: `OVERMIND_DATA_DIR` pointed at another bind mount, a hand-written mount, a filesystem nobody measured. It is asked once, in line with `sandbox::caged`.

**5. The advice to set `OVERMIND_AGENT_UID` to your own uid is withdrawn.** It answered a need ADR-0052 removed. The text drafted for this before the measurement, saying the uid is what stops an agent writing a repository, is false and is not used.

## Measured before deciding

An image derived from `overmind:main` with decision 1 and a `/runs` volume, bind mounts on an external disk, targets `/data` and `/repos`:

| Check | Result |
|---|---|
| Where Docker mounted them | `/srv/hold/data`, `/srv/hold/repos` |
| On the host | the instance's files, in the folder |
| As root, via `/data` and via the hold | database and setup code read, repository listed |
| As 10001: read or open the database, setup code | refused, by both paths |
| As 10001: list `/data`, `/srv/hold`, `companies`; `cd` into the hold; create in `/data` | refused |
| As 10001: `/proc/1/root`, `/proc/1/cwd` | refused |
| As 10001: list `/repos`, read a file, write a hook, `git log` | refused |
| As root: clone from `/repos` into `/runs` | works |
| As 10001 in its run dir on `/runs`: write, commit | works |
| As 10001: list the runs parent, create beside its run dir | refused |
| After `docker restart` | same, and the run dir survived |
| With no mount at all | the image's `VOLUME /data` lands on `/srv/hold/data` |

**Not measured yet,** and required by the plan: a real run started by the server, including whether it inherits a descriptor on the database; Docker on native Linux; Windows; gRPC-FUSE. The server does not have `OVERMIND_RUNS_DIR` yet, so the run side above was done by hand.

## Rejected

- **A named volume for all of `/data`.** Measured and holding, with little code. It undoes ADR-0047: the data stops being in plain sight, and `down -v` can delete an instance again. It also reaches only those who replace their compose file, so every existing install stays open.
- **Changing only the mount targets in the compose file.** Same problem: existing installs stay open, and stored paths change.
- **A second bind mount for the run directories.** It keeps them in the folder, and on VirtioFS gives every run listing and writing over its siblings. The threat model admits a run can reach a sibling, not that it can enumerate one.
- **Warning without closing.** The startup check alone, on the current layout, would make every Docker Desktop install read-only.
- **Namespaces, or a uid per run.** Already rejected in ADR-0029: blocked by Docker's default seccomp profile, and neither touches this hole on its own.

## Consequences

- **Run directories are no longer the folder.** Moving the folder, or `down -v`, loses them, and with them the diffs of past sessions. What a run committed has already come back as a branch, deliverables are in `artifacts`, and archives do not carry run directories. This is a real amendment to ADR-0047, and the README says so.
- **An old compose file gets an anonymous volume for `/runs`.** It is expected to survive a recreate and not a `down` followed by `up`; the plan measures this. Upgrading the compose file fixes it, and the boundary holds either way.
- **Sessions from before the upgrade lose their diff.** Their run directories stay in `./data`, behind the hold, and the server reads them only as the agent uid, so it is refused. They are not migrated. The session says why rather than failing blank, and the disk stays used until run dirs are cleaned up, which is already a debt in [NEXT](../NEXT.md).
- **The grant that let old sessions' diffs read a repository's git dir stops working in the image**, for the same reason and with the same message.
- **A run can still reach a sibling's directory** on Docker Desktop, as the threat model already states. This ADR does not change that.
- **The check can turn a working install read-only**, on a layout that was in fact exposed and has no Landlock. That is the intent, and the message has to say what to do.

## Disclosure

The repository is public and SECURITY.md puts this in scope. ADR, plan and branch stay local until the fix is ready. The public PR, the merge, the tagged release and publishing the advisory happen together, by the owner's hand. The cost is that the Linux half of CI only sees the change when the PR opens, so the smoke script runs by hand on Docker Desktop first.
