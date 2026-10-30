# Git in the agent's worktree runs as the server

*Plan, 29 Oct 2026. Found by `/security-review` on M36a slice 4 (#113). The defect is older than that branch: it has been in `session_diff` since M2.*

## The problem, in the code as it is

The server runs git inside a code run's worktree **after the agent has had it**. One place does this today: `runner::session_diff` (`git add --intent-to-add --all`, then `git diff <base>`). It runs when a run finalizes, and again every time someone opens the diff in the UI. #113 adds a second place, the handoff's `git status --porcelain`.

A worktree made by `git worktree add` holds a `.git` **file** (`gitdir: <repo>/.git/worktrees/<id>`), and that file sits in the run directory, which the agent owns. The agent can rewrite it to point at a git directory of its own, whose `config` can carry:

- `core.fsmonitor = <command>`, which `git add` and `git status` run;
- a `filter.<name>.clean` driver plus a `.gitattributes` entry, which `git add` runs on every matching file;
- a `diff.<name>.textconv` or `diff.external`, which `git diff` runs.

Every one of these executes **as the server**: outside the cage on macOS, and as root in the image. Both [THREAT-MODEL.md](../THREAT-MODEL.md) and ADR-0023/0029 promise that cannot happen. `git_isolation()` does not cover it, because it neutralises the global and system config, not the repository's own. `-c core.fsmonitor=false` would close one door and leave the drivers open, since their names are arbitrary.

## The approach

**Do not let the worktree say where its git directory is.** Every git call the server makes in a run dir after the agent has started passes `GIT_DIR` and `GIT_WORK_TREE` explicitly:

- `GIT_DIR` is the worktree's real administrative directory, `<common-dir>/worktrees/<id>`, which lives in the operator's repository **outside** the cage;
- `GIT_WORK_TREE` is the run dir.

With both set, git never reads the `.git` file the agent controls. The config it does read, the repository's and the worktree's own, is not writable from inside the cage.

How the real git dir is found, without trusting the run dir:
1. **At creation, before the agent runs**, read it once: `git rev-parse --absolute-git-dir` in the fresh worktree.
2. **Store it on the session** (`agent_task_sessions.git_dir`, one migration). `session_diff` and the handoff read it from there, never from disk.
3. **Sessions created before the migration** have no stored value. Their git dir is recovered from the trusted side: scan `<common-dir>/worktrees/*/gitdir` for the entry naming this worktree. If none matches, the diff is refused with a sentence rather than run against the agent's `.git`.

On top of that, as defence in depth, the same calls carry `-c core.fsmonitor=false -c core.hooksPath=/dev/null`, and `git diff` carries `--no-ext-diff --no-textconv`. These cost nothing, and they also cover a hostile operator repository config (out of scope by the threat model, but free).

## Tests first

- `a_planted_gitdir_does_not_run_as_the_server`: a stub agent rewrites `.git` to point at a git dir it made, whose config sets `core.fsmonitor` to a script that writes a marker file outside the run dir. After the run, `session_diff` is called. The marker must not exist, and the diff must still show the agent's real change.
- `a_textconv_driver_in_the_worktree_does_not_run`: the same shape, with `.gitattributes` and a `diff.*.textconv`.
- `a_session_from_before_the_migration_finds_its_git_dir_from_the_repository`: a session row without `git_dir` still diffs, and a worktree that matches no admin entry is refused.

## Done when

- The three tests pass.
- A threat-model row names them.
- The checks in CLAUDE.md are green, `/code-review medium` and `/security-review` have run.
- The PR is separate from #111/#112/#113. #113's `git status` then rides the same helper, either in a follow-up commit on #113 or here once #113 is merged, whichever lands second.

## Not in this plan

- Knowledge runs: the server never runs git in them.
- `exclude_from_git` and the `rev-parse HEAD` at creation: both run before the agent starts, against a worktree the server has just made.
