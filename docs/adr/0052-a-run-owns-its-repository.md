# ADR-0052: A run owns its repository

- **Date:** 2026-10-29
- **Status:** accepted (2026-10-29), with the owner's approval of [the plan that implements it](../plans/2026-10-29-the-run-owns-a-clone.md)
- **Supersedes:** the worktree in [ADR-0008](0008-execution-sessions-and-atomic-checkout.md) decision 3, and the line of [ADR-0029](0029-the-cage-inside-the-container.md) that put `repos_dir` in the cage's writable set
- **Builds on:** [ADR-0023](0023-os-level-sandboxing.md) (deny by default), [ADR-0029](0029-the-cage-inside-the-container.md) (the cage is a set of mechanisms)

## Context

A `code` run works in a `git worktree` of the operator's own repository. A worktree keeps its metadata in `<repo>/.git/worktrees/<name>`, inside the repository and not inside the run dir. So that the agent can commit, ADR-0029 added `repos_dir` to the cage's writable set — the whole of it.

That one grant opens a class of holes, not a single one. Found on 29 October 2026, in a `/security-review` of #113 and two `/code-review`s of its fix:

- The agent can write `<repo>/.git/config`, `.git/hooks/`, `info/attributes`, and its own worktree's `commondir` and `config.worktree`. Any git the server runs over that repository afterwards honours what the agent planted: `core.fsmonitor`, a `clean` or `process` filter, a `textconv` driver, a `post-checkout` hook, `core.sshCommand`. No list of keys closes this; the list has no end.
- The server runs that git **as the server**, which in the image is root: `session_diff` whenever someone opens a diff, `handoff_text`'s `git status` at every run stopped at a bound, and `git worktree add` at the start of the **next** run on the same repository. "Before the agent starts, the state is still the server's" is false. The previous run has already had it.
- The repositories are mounted from the host. A hook planted in `<repo>/.git/hooks` runs **as the owner, on the host**, the next time they use git in their own repository. That is out of the container entirely.
- On a machine without `repos_dir`, the opposite failure: the macOS profile grants nothing in the repository, so the agent's own git and any caged git cannot read the worktree's metadata at all.

Running the server's git in the agent's cage (plan 2026-10-29, "git runs in the cage") closes the first two for the commands that run after the agent. It cannot close the third: the owner's own git is not ours to cage.

## Decision

**A run gets a repository of its own, and the operator's repository is never in the cage.**

1. **A clone, not a worktree.** At the start of a `code` run the server runs `git clone --no-hardlinks <repo> <run dir>` from the operator's repository and checks out `overmind/task-<id8>-<sess8>` at the workspace's `default_ref`. It then removes the clone's `origin`, so nothing in the run dir names the operator's path. The clone's whole `.git` is inside the run dir. The server's git touches it before the agent runs, when it is still the server's.
2. **The cage stops granting `repos_dir`**, neither to write nor to read. A run reaches the operator's repository only through what the server copied for it.
3. **The work comes back as a bundle, never as a fetch from the agent's repository.** When the run ends, `git bundle create` runs **in the cage**, in the clone, for the run's branch from `base_sha`. The server then fetches that one file into the operator's repository with `transfer.fsckObjects=true`, into `refs/heads/overmind/…` only. Fetching from a bundle reads no configuration but the operator's own. A fetch from the clone would start an `upload-pack` in a repository the agent configured. The owner sees the branch when the run ends, not while it runs.
4. **Every git the server runs over the clone after the agent has had it runs in the run's cage:** the diff, the handoff's `git status`, the bundle. A planted command runs with the agent's reach and nothing more, which is nothing the agent could not do in its own run.
5. **What comes back is what was committed**, as today: the branch in the operator's repository carries the agent's commits; uncommitted work stays in the run dir, where the diff still shows it.

## Consequences

- The guarantee is narrow and true. The operator's repository contains nothing an agent wrote except the objects and the `overmind/` ref of a bundle the server checked. The server's git over a run's files runs no wider than the agent.
- **Disk and time.** A clone without hardlinks copies the history: a cost in the size of the repository at every run. Worktrees were never cleaned up either (ADR-0008, "not done yet"), so clones accumulate faster. A cleanup policy becomes a debt with a size; it is written in NEXT.md, not solved here.
- **Sessions from before** keep their worktrees. Their diffs run in the cage with a **read-only** grant on that repository's git directory, the only grant in the operator's repository that remains. Reading does not widen what an agent can plant.
- The agent loses the operator's other branches and remotes. A run that needs another ref is a task that names it; the server puts it in the clone.
- With no worktrees, the race inside `.git/worktrees/` measured in M27 is gone, and `WORKTREE_LOCKS` with it.

## Rejected

- **Narrowing the grant** to `.git/worktrees/<id>`, `objects/` and `refs/` while denying `config`, `hooks/` and `info/`. That is a list again, with the same gap as a list of keys: `commondir` and `config.worktree` stay writable and redirect git elsewhere. It also has to be held identically by three mechanisms, and in the image it collides with who owns a mounted file.
- **`clone --shared` or `--reference`** (alternates into the operator's object store). That needs a read grant on the operator's repository for every run. It is also fragile: a `git gc` on the owner's side can prune objects a clone still depends on.
- **Hardlinked clones.** An object file shared by inode is the operator's file: the agent can `chmod` and rewrite it, and in the image handing the run dir to the agent's uid would `chown` the operator's objects.
- **Fetching the branch straight from the clone with a caged `upload-pack`.** It is possible, but the cage would have to be spelled inside `--upload-pack`'s shell string. A bundle is one file, and the trusted side reads it with its own config alone.
- **Committing the agent's uncommitted work on the way back.** It changes what a run delivers. It is a separate decision, if it is ever wanted.
