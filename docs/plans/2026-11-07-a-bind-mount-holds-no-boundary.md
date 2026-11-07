# A bind mount holds no boundary

- **Date:** 2026-11-07
- **Decided in:** [ADR-0054](../adr/0054-a-bind-mount-holds-no-boundary.md) (accepted with this plan)
- **Status:** approved by the owner, 2026-11-07, who added at approval what is under "Useful to everyone"

## The problem

On Docker Desktop with the default compose file, since v0.2.4, an agent can read and write `overmind.sqlite`, every brain, the backups and the setup code, and read and write a mounted repository. The uid split rests on ownership, and VirtioFS bind mounts do not keep it. The measurements are in the ADR. Advisory GHSA-cr22-mp4p-vj89 is a private draft.

This replaces PR B of the [plan of 6 Nov](2026-11-06-what-the-cage-grants-on-the-host.md), which treated the compose file as a matter of wording. PR A (#116) and PR C are unaffected.

## The approach — one PR, several commits

The layout and the check ship together: the check alone would turn every Docker Desktop install read-only, and the layout alone would leave other layouts claiming a cage they do not have.

**Order:** after #115 and #116 are merged, from the `main` that results. The branch stays local until step 7.

1. **Tests first, seen failing.**
   - The check: a `0600` canary owned by the current user, with the agent uid set to the current user, is *reached*, so the uid does not count. That is the VirtioFS effect, reproduced without VirtioFS and without root.
   - `runs_dir` defaults to the data dir and takes `OVERMIND_RUNS_DIR`; run, chat and meeting directories are created under it, and shelves are not.
   - `container-smoke.sh`: new paired probes for listing `/data`, `/srv/hold`, `/repos` and `/proc/1/root`, with `-v` bind mounts on `/data` and `/repos` named exactly as the compose file names them.
2. **`OVERMIND_RUNS_DIR`.** `Config.runs_dir`, used where run paths are built (`runner.rs:1227`, `:1231`, `meeting.rs:752`, `ceo.rs:697`, `:927`) and by the deletion debris (`api.rs:1290`). `lay_out_data_dir` lays the `0711` parents under it and the `0700` shelves under the data dir.
3. **The check at startup.** After the layout, once, and before anything can start a run. The result goes where the binary already records what its probe found (`db.rs:298`). When it failed the uid stops counting as a mechanism, so `caged` answers false unless Landlock is held, and the startup line names the reason and the remedy. The refusal a task gets must be visible, not the silent success of the old bug.
4. **The image.** `/srv/hold` at `0700`, the two links, `/runs`, `OVERMIND_RUNS_DIR=/runs`, `VOLUME /runs`. Keep `VOLUME /data`, measured to land behind the hold. Read `install.sh` and `install.ps1`, which name these paths.
5. **Compose and docs.** A named volume for `/runs`, with the reason. Fold in what the worktree already has (compose comment, README, ROADMAP, CHANGELOG), rewriting the false sentence. THREAT-MODEL lines 116 and 154 and the `/repos` row, including that this was false from v0.2.4 to the fix. The amendment to ADR-0047 in the README. CHANGELOG with the advisory id. `docs/NEXT.md` left out of the commit if the owner's account block is still uncommitted there.
6. **Proof on Docker Desktop, by hand, before any push.** Throwaway instances only, a small notional cost on Haiku, stated before it runs:
   - a **real run** started by the server, whose probes from inside it include `/proc/self/fd`, the database, the setup code, `/repos`, the runs parent;
   - a `code` run on a repository in `/repos`: clone, commit, branch back;
   - **the upgrade:** an instance created by the current image, with a task, an attachment and a finished session, moved to the new image with the old compose file: its data is there, a new run works, the old session's diff says why it cannot open, and what happens to the anonymous `/runs` on a recreate and on `down` then `up`;
   - **the check firing:** `OVERMIND_DATA_DIR` on a plain bind mount: agents read-only, and the startup line says so;
   - `container-smoke.sh` run locally.
7. **Reviews, then public.** Checks from CLAUDE.md, `/code-review medium`, `/security-review` with `/data` and `/repos` as its question. I ask before pushing. The owner merges, tags, and publishes the advisory on the same day.
8. **After the release.** Restart `overmind` and `overmind-homeserver` on the new image, with the diff of each compose file shown first and a backup taken first.

## Useful to everyone

Prepared with the fix and published with the release, not before: everything here names the hole.

- **A check anyone can run.** One command that says whether an install is exposed, measured on both the old and the new image. It goes in the advisory, the README and the release notes.
- **README.** A short Security section, which it does not have today: what holds on which platform, what does not, how to check and update, where to report.
- **The site.** `docs/landing/index.html` says what the cage is per platform and links the threat model, with no claim the threat model does not make. Deployed by the owner's go-ahead at release.
- **GitHub.** The advisory filled in with patched versions and the check, and a CVE requested from it. Release notes that lead with this. SECURITY.md re-read against what is now true.

## What counts as done

- From inside a real run on Docker Desktop, with the mounts the compose file names: the database, setup code, brains, backups and repositories are refused, and the run still does its work.
- The same probes hold on the ubuntu runner, by the smoke script.
- An existing instance survives the upgrade with only an image change, and what it loses is what the ADR says it loses.
- A layout that exposes the data dir is refused the cage, and says so.
- The THREAT-MODEL is true again and says when it was not. The advisory is published with the release.

## Not in this

- Windows and gRPC-FUSE: not measured, covered by the check rather than by a claim. The advisory says so.
- Migrating old run directories, and cleaning up run directories: the second is already a debt.
- Isolating one run from another on Docker Desktop.
