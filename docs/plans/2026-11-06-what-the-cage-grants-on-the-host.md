# What the cage grants on the host

- **Date:** 2026-11-06
- **Decided in:** [ADR-0053](../adr/0053-the-cage-grants-nothing-the-host-runs.md) (accepted with this plan)
- **Status:** approved by the owner, 2026-11-06

## The problem

The security review of #114 left two debts: an agent on macOS can plant `~/.config/git/config`, and `docker-compose.yml` recommends a setting ADR-0052 made dangerous. The review of #115 added a third: `MEETING_REQUEST.json` is read following links.

Reading the code for the first debt found it wider than it was written. `~/.config`, `~/.local`, `~/.claude` and `~/.claude.json` are all granted for reading and writing, and the measurements are in the ADR. So this plan does not narrow one git setting. It removes what the cage does not need and states what remains.

Nothing here touches *TravelAgency*, or any company's data. No run has started on the native instance since 12 Sep 2026.

## The approach — three PRs, each from `main`

Every PR: tests first and seen failing, CLAUDE.md checks, `/code-review medium`, CHANGELOG, and the owner merges.

### PR A — the cage and the home (`/security-review` required)

1. `adapter_paths` keeps only `~/.claude` and `~/.claude.json`. Both take `home` as an argument, so the grants are testable without changing `HOME` for the whole test process.
2. `adapter_read_paths` gains `~/.local/bin` and `~/.local/share/claude`, and `build_landlock` starts using it.
3. `OVERMIND_SANDBOX_READ` in `Config`: read-only for both mechanisms, documented in the README beside `OVERMIND_SANDBOX_ALLOW`.
4. `git()` and the clone use the configuration of ADR-0053 decision 5.
5. THREAT-MODEL: a row for what closed, and the open hole of `~/.claude` in the ADR's words, including `~/.cache/uv`. If `sandbox_allow` or `~/.claude` is reported at startup, the line says so.

**Tests first:**

- `the_cage_grants_nothing_under_config`: neither stanza of the profile names it.
- `the_adapters_install_is_readable_and_never_writable`
- `a_read_only_allow_is_never_writable`, for the profile and for Landlock's lists.
- `an_agent_cannot_read_or_plant_in_the_hosts_config`: real cage, skipped where none exists. A temp home; the run reads a planted key file and writes beside it, and both are refused.
- `the_servers_git_does_not_read_the_hosts_config`: a `core.hooksPath` and an `init.templateDir` in a `.gitconfig` at the `HOME` of a spawned command; a run's clone does not run them. It spawns git through the helper with its own `HOME`, so the test process's environment stays alone.

**Live check, on a temporary instance, never on Casa's data:** the real CLI through the real cage, one `knowledge` task and one `code` task on Haiku, with the brain's MCP connected. It passes if both finish and the brain answers; the first measurement covered the CLI alone, with no MCP. Cost: a few cents on the subscription.

**After the merge:** `start.sh` moves `~/.local/share/uv` to `OVERMIND_SANDBOX_READ`, and Casa is rebuilt with a backup. `start.sh` is outside the repository, so this is a local change, and I will show the diff first.

### PR B — the compose stops recommending the hole (docs only)

- Remove the advice at `docker-compose.yml:105-109` and the commented variables at `:131-133`. A run works in its own clone (ADR-0052), so the agent no longer has to write a mounted repository, and an agent uid equal to the host's, without Landlock, reopens `/repos`.
- Correct README `:469` and ROADMAP `:264`. ADR-0029 stays as it is: it records what was thought then.
- **Measure in the image:** can uid 10001 read a world-readable file under `/repos`? THREAT-MODEL says the operator's repository is out of the cage *to write or to read*. On Docker Desktop there is no Landlock and the uid is the only boundary, so I expect the reading half to be false for ordinary permissions. If it is, the row is corrected and that becomes a debt in NEXT. Docker's daemon has to be up: if it is not, CI's `the image works` job gets a one-off step, or this waits and the PR says so.

### PR C — `MEETING_REQUEST.json` is read as `HANDOFF.md` is (`/security-review` required)

Generalise `read_own_handoff` to any file a run owns and read the request through it. Test first: `a_meeting_request_planted_as_a_symlink_is_not_followed`. This one starts **after #115 is merged**, since `finalize` changes there.

## Order, and what waits

1. #115: the owner's merge, then Casa rebuilt with a backup (34 → 35).
2. PR A, PR B, then PR C.
3. PR docs: ADR-0051, the plan of 28 Oct, NEXT, ROADMAP (M36 still reads `todo` with M36a on `main`).
4. M34, on the owner's word. The `~/.claude` hole is added to step 4's definition of done.

M36a's live acceptance is paused by the owner's choice (6 Nov) and is not lost: it needs real work on *TravelAgency*, and it resumes with that.

## Not in this plan

- The agent's own home on the native instance: M34 step 4, per the ADR.
- `.cache/uv`: stated as a risk, not closed.
- Stopping a run when its task is moved, and the other debts in NEXT.

## Done when

- A cage on macOS reads and writes nothing under `~/.config` and writes nothing under `~/.local`, shown by the tests above running on `macos-latest`.
- The live check passes on a temporary instance, with the CLI, the brain and a `code` run working.
- THREAT-MODEL states the `~/.claude` hole in plain words.
- PRs A, B and C are merged by the owner, Casa is rebuilt and answers, and its `start.sh` no longer grants `~/.local/share/uv` writable.
- NEXT's two #114 debts and the `MEETING_REQUEST.json` debt are gone or restated as what the measurements found.
