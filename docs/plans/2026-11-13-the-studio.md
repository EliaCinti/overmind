# The studio

*Plan, 13 Nov 2026. Carries out [ADR-0055](../adr/0055-agents-share-a-studio.md).
It fixes the contracts of M37 in the detail an implementer needs, and the
shape of M38 to M40. Each of those three opens with measurements, an ADR of
its own where a boundary moves, and a plan of its own: what is written about
them here is the direction, not the specification.*

## The problem

The owner wants several agents working in one place, at the same time, where
he can see each of them, where each sees what the others do, at one of three
depths he chooses, and where he can step in. Today a run is alone in its
directory, has no position on any surface, and is seen through one line of
activity polled every two seconds.

## The approach

The studio lives in the server. Agents act on it through Overmind's MCP
endpoint with the token their run already holds. The cage is not touched.
One code path receives every action, and from it come the journal, the live
view, the hold and the level check. A level adds surfaces to the studio.

Four milestones, each usable on its own:

| | adds | reach |
|---|---|---|
| **M37** | the studio, shared files, pointers, the person's hand, replay | `inside` |
| **M38** | a browser Overmind hosts | `inside` |
| **M39** | named folders, a terminal, operator's tools on the stage | `granted` |
| **M40** | the real desktop, through the hand | `computer` |

## Before M37: two debts

Both are in [NEXT](../NEXT.md) already. They stop being debts here.

1. **A run can read the MCP token of a run beside it** (*Debts from the
   cage*). In a studio the token is who the pointer belongs to. Fix as
   written there: a directory private to each run, outside its run dir.
   Test: `a_run_cannot_read_another_runs_mcp_token`.
2. **Moving a task does not stop its run** (*Debts from the attempts cap*).
   Hold and close must reach the process: the handle in `state.running` and
   the cage's process group. Test: `a_cancelled_task_stops_its_run`.

Each is a PR of its own, with `/security-review` (both touch the cage).

## M37, slice by slice

Every slice: test first, the four checks, `/code-review medium`,
`/security-review` where marked, a `CHANGELOG.md` line, one focused PR.

### Slice A: the studio and its journal *(security review)*

New module `crates/overmind-server/src/studio.rs`. One migration, next free
number.

```sql
studios(id, company_id, title, reach CHECK (reach IN ('inside','granted','computer')),
        status CHECK (status IN ('open','held','closed')),
        goal_id NULL, conversation_id NULL,
        opened_by, opened_at, closed_at NULL, armed_until NULL,
        journal_seq INTEGER NOT NULL DEFAULT 0, journal_head TEXT NOT NULL)
studio_members(studio_id, agent_id, color INTEGER, held INTEGER DEFAULT 0, joined_at,
               PRIMARY KEY (studio_id, agent_id))
studio_files(studio_id, path, rev INTEGER, sha256, size, updated_by, updated_at,
             PRIMARY KEY (studio_id, path))
studio_journal(studio_id, seq, at, actor_kind CHECK (actor_kind IN ('agent','person','system','outside')),
               actor_id, session_id NULL, surface, verb, target, detail TEXT,
               prev_hash, hash, PRIMARY KEY (studio_id, seq))   -- append-only by trigger, as audit_events
studio_claims(studio_id, target, agent_id, intent, expires_at, PRIMARY KEY (studio_id, target))
studio_notes(id, studio_id, target, author_kind, author_id, body, at, seen_by TEXT)
tasks.studio_id NULL
```

- A task with `studio_id` set runs as its `execution_kind` says, with the
  studio's tools added to its per-run MCP config. Its agent is enrolled as a
  member at checkout.
- File bytes: `<data>/studios/<id>/files/`, `0700`, server only. Paths are
  relative, normalised, walked with `O_NOFOLLOW` from a directory descriptor
  (the walk `read_own_handoff` already does). Limits: 2 MiB a text file,
  2000 files a studio, both refused by name and never truncated.
- `colour`: an index into a fixed palette of eight, the lowest one free at
  joining, kept for the studio's life.
- The journal row's hash is the audit chain's formula over
  `(prev_hash, seq, at, actor, surface, verb, target, detail)`. The chain
  gets `studio.opened`, `studio.reach_changed`, `studio.held`,
  `studio.released`, `studio.taken_over`, `studio.closed{journal_head, seq}`.
  `GET /api/studios/{id}/journal/verify` recomputes it and names the first
  bad row.

HTTP, all behind the door and membership (add `studios` to the bare-id
table in `auth.rs`):

```
POST /api/companies/{id}/studios        {title, goal_id?}        -> studio (reach: inside)
GET  /api/companies/{id}/studios
GET  /api/studios/{id}                  studio, members, files index, claims
GET  /api/studios/{id}/files/{*path}    bytes at head, or ?rev=
GET  /api/studios/{id}/journal?after=   rows after a sequence number, max 500
POST /api/studios/{id}/close
```

MCP, in `mcp_server.rs`: `Caller::Run` gains `studio: Option<StudioSeat>`
(studio id, agent id, session id, reach), resolved from the same token. Tools
listed only when the seat exists:

| tool | does | journaled |
|---|---|---|
| `studio_look` | who is here, where, with what intent; surfaces; the digest | no |
| `studio_list {path?}` | names, sizes, revisions, who last touched | no |
| `studio_read {path, from?, to?}` | text, its `rev` | no (presence only) |
| `studio_write {path, content, base_rev?}` | create, or replace when `base_rev` is the head | yes |
| `studio_edit {path, old, new, base_rev}` | replace one exact occurrence | yes |
| `studio_move {from, to}` · `studio_delete {path, base_rev}` | | yes |
| `studio_claim {target, intent, minutes?}` · `studio_release {target}` | default 5 min, max 30 | yes |
| `studio_note {target, text}` | 500 chars, clamped at the boundary | yes |

- **Refusals are answers.** A stale `base_rev` whose region changed returns
  `{refused: "changed", by, rev, text}` with the current text of the region.
  A stale `base_rev` whose region did not change is applied (the edit is
  rebased by the server). A claimed target returns `{refused: "claimed", by,
  intent, until}`. A held agent gets `{refused: "held", by, note}` on every
  mutating call.
- **The digest.** Every answer ends with `since`: up to ten entries of what
  others did after this session's last call (`who, verb, target, lines`),
  then a count of the rest. Notes addressed to a place this agent touches
  ride in it, delimited as M10 slice 4 delimits turns, with the author's
  role named (`person` or `agent`), so a note cannot pass for the system or
  for the owner.
- **The prompt.** One block, through `i18n.rs`: this task is worked in a
  studio with these teammates; the deliverable is what is in the studio;
  look before writing, claim what you are about to change, read the digest.
- **Empty-handed** (M19): a studio run that reported `completed` with no
  journaled action and no artifact fails as a knowledge run with nothing
  does.

Tests (`tests/studio.rs`, stub adapter that calls the MCP endpoint with the
token from its per-run config):

- `two_agents_write_one_studio_and_each_sees_the_other`
- `an_edit_on_a_changed_region_is_refused_with_the_current_text`
- `an_edit_beside_someone_elses_change_still_lands`
- `a_claimed_file_refuses_a_second_hand_and_names_the_holder`
- `a_claim_expires`
- `a_run_outside_the_studio_does_not_see_its_tools`
- `a_run_cannot_reach_another_companys_studio`
- `a_path_cannot_leave_the_studio` (`..`, absolute, a planted link)
- `a_note_cannot_forge_a_turn`
- `the_journal_is_anchored_in_the_chain_and_a_changed_row_is_named`
- `a_studio_run_that_touched_nothing_is_not_completed`
- `a_studio_works_with_memory_off`

Threat-model rows land here: the seat comes from the token; the path cannot
leave; the journal is anchored; a note is delimited.

### Slice B: the live channel *(security review)*

`GET /ws/studios/{id}`: the origin guard and the session wall of `/ws`, plus
membership checked at the upgrade and again when the studio's members change.

```jsonc
{"type":"snapshot","studio":{…},"members":[…],"files":[…],"claims":[…],"presence":[…],"seq":412}
{"type":"action","seq":413,"at":"…","actor":{"kind":"agent","id":"…"},"surface":"files",
 "verb":"edit","target":"brief/plan.md","detail":{"rev":7,"from":12,"to":18,"added":"…","removed":"…"}}
{"type":"presence","actor":{…},"surface":"files","target":"brief/plan.md",
 "anchor":{"line":12,"col":0},"state":"reading|writing|waiting|held|idle","intent":"…"}
{"type":"hold","agent_id":"…","by":"…"}  {"type":"closed"}
```

- Presence is set when a call starts and when it ends; `idle` after 20 s
  with no call. Kept in an `AppState` map like `activity`, cleared at the
  run's end. At most ten presence frames a second per actor; the server
  keeps the latest.
- A client that sees a gap in `seq` fetches the journal after its last
  number. A lagged receiver gets a fresh snapshot.
- `detail` of an edit is clamped to 4 KiB; a larger one carries line numbers
  and the client fetches the file.

Tests: `the_studio_socket_refuses_a_non_member`,
`a_late_watcher_gets_the_whole_state_in_one_frame`,
`an_action_reaches_the_socket_with_the_journals_number`,
`presence_is_forgotten_when_the_run_ends`.

### Slice C: the stage

`web/src/components/Studio.tsx` and `web/src/components/studio/`
(`Stage`, `FileSurface`, `Pointer`, `Roster`), `web/src/lib/studio.ts` (the
socket and a store). A sixth view beside board, org, chat, meetings, memory.

- **The file surface.** A tree and an open file. Proposed dependency:
  CodeMirror 6, for decorations (other actors' selections and carets) and,
  in slice E, editing. Named in the PR, as the rulebook asks.
- **A pointer** is the agent's colour, its name on a pill, and under it the
  intent of its claim or the verb of its last call, worded by the client
  from the structured frame (M16). It sits at the anchor: a line in the open
  file, a row of the tree when the file is not open.
- **Motion.** A spring of 200 to 350 ms between anchors. An edit is revealed
  as typing, at most 600 ms an action. When more than three actions wait,
  they apply at once: the view is never more than a second behind the
  journal. Reduced motion: pointers jump, text appears.
- **Simultaneity, and its real cadence.** Each agent is its own run, so the
  pointers move at the same time, independently. Each one moves at the pace
  of its agent's calls, which is seconds apart while the model thinks, not
  the continuous motion of a hand on a mouse. Between calls a pointer stays
  where it last acted and shows the agent's state (`thinking`, `waiting` on a
  claim, `held`), drawn from the activity stream ADR-0039 already reads.
- **The edit as it is being written.** The adapter streams with
  `--include-partial-messages`, so the arguments of a tool call arrive
  piece by piece before the call is made. When the call is a `studio_edit`
  or `studio_write`, the stage draws the text being written at its target,
  in the agent's colour and marked as not yet applied, and replaces it with
  the journaled change when the call lands, or removes it when the call is
  refused. This is what makes several agents look like they are typing at
  once, and it is still a fact: the model is producing that text. **Verify
  first** that the real stream carries the tool input deltas
  (`input_json_delta`) through the CLI, with a captured fixture, as M10 did
  for the envelope. If it does not, the pointer moves per call and nothing
  is invented to fill the gap. The provider trait gains an optional
  `tool_input_delta`; a provider without it gets per-call motion.
- **The roster.** One row an agent: colour, name, title, state, what it holds
  claimed, its budget line. Clicking a row follows that agent: the stage
  opens whatever it acts on.
- **Changes land visibly.** A changed line carries its author's colour for a
  few seconds; the tree marks who touched a file last.
- Dark and light, Italian and English, by the same rules as every surface.
  On a narrow screen the roster and the last actions, no stage.

Verified in the browser against the real server with the stub adapter.

### Slice D: from the CEO's plan to the studio

- In the chat composer, one toggle beside the attach button: *work on this
  together, where I can watch*. The turn then carries `studio: true`.
- The CEO's plan gains `"studio": true` on a set of tasks. The server opens
  one studio (`inside`) for them, puts the conversation's attachments in its
  files, and offers each task by its agent's autonomy, as ADR-0038 does.
  Tasks of one studio start together, each its own run, each under its own
  budget.
- Without the toggle, a plan asking for a studio becomes an approval, as a
  meeting request does (ADR-0020). An agent cannot ask for more than
  `inside`.
- `after` (M30) still orders work inside a studio. What a studio removes is
  the need for it when two agents only have to see each other's files.
- At closing, the studio's files become artifacts of its goal, so the board
  and the deliverables drawer show them as they show any work.

Tests (`tests/studio_floor.rs`): `the_ceos_plan_puts_three_agents_in_one_studio`,
`a_studio_asked_for_unprompted_waits_for_approval`,
`a_plan_cannot_ask_for_more_than_inside`,
`a_closed_studios_files_are_the_goals_artifacts`.

### Slice E: the person's hand *(security review)*

- **A pointer of your own.** The web client sends its own presence on the
  socket; other watchers and `studio_look` see it.
- **A note where you point.** `POST /api/studios/{id}/notes`. It reaches the
  agents that touch that target in their next digest, marked as the
  person's.
- **Hold.** `POST /api/studios/{id}/hold {agent_id?}` and `/release`. A held
  agent's mutating calls are refused with the reason; after five minutes
  held, the run winds down with a handoff (ADR-0050) and the task returns to
  the queue. Holding everything sets the studio `held`.
- **Take over.** Editing a file as a person goes through the same write path
  with `actor_kind: person`; a claim an agent holds on that file is released
  and the agent is told in its digest.
- **Close** stops every attached run (debt 2) and then anchors the journal.

Tests: `a_held_agent_changes_nothing_and_is_told_why`,
`a_long_hold_winds_the_run_down_with_a_handoff`,
`a_persons_edit_releases_the_agents_claim_and_tells_it`,
`closing_a_studio_stops_its_runs`,
`a_member_of_another_company_cannot_hold_or_write`.

### Slice F: replay

A timeline under the stage, drawn from the journal: scrub to any sequence
number and the files surface shows the studio as it was, with the pointers
where their last action before that point was. A file at a past revision is
rebuilt by applying the journal; keep a full copy every fifty revisions so
this stays cheap. Read-only, and labelled as the past wherever it shows.

Tests: `a_file_at_a_past_revision_is_what_the_journal_says`.

### M37 is done when

The owner asks, from the chat, for something three agents can make together.
One studio opens. He sees three pointers working at once and can tell whose
each is. One agent builds on a file another wrote a minute before, with no
dependency declared between their tasks. He holds one agent, changes a
paragraph himself, releases it, and its next action takes his change into
account. He scrubs back to the first minute. The journal verifies and the
chain names its head.

## M38: a browser on the studio

**Measure first, and write the numbers in the roadmap entry.**

1. Chromium in the image: size of the layer on amd64 and arm64; whether it
   starts as uid 10001 with its own sandbox under Docker's default seccomp
   profile (bubblewrap did not, ADR-0029), and what running it with
   `--no-sandbox` under the uid would leave unprotected.
2. Chromium under `sandbox-exec` on macOS: whether its own sandbox
   initialises inside ours.
3. `Page.startScreencast` over CDP: frames a second and CPU at 1280×800.
4. The CDP client: a crate or the protocol spoken directly. A new
   dependency either way.

**Shape.** One browser per studio, started by the server, in the cage, with a
profile under the studio's directory that is destroyed at closing. Tabs are
shared; one hand a tab at a time, any number of tabs at once. Tools:
`browser_open`, `browser_look` (the accessibility tree with references, a
screenshot on request), `browser_click`, `browser_type`, `browser_key`,
`browser_tabs`, `browser_close`. The stage shows the screencast with each
pointer at the box of the element its agent acted on. A person can take a
tab and use it. Downloads land in the studio's files, capped; uploads come
only from them.

**The boundary this milestone owns, in an ADR of its own.** A browser the
agent drives, running on the owner's machine, can be pointed at
`127.0.0.1:7070`, at the tailnet, at the router. Navigation to loopback,
link-local, private and CGNAT ranges is refused after resolution, not by
matching the name, so a rebinding DNS answer does not pass. A page is
untrusted text: what `browser_look` returns is delimited as such. Optional in
the image behind a build argument, as `EXTRA_APT_PACKAGES` is: a box without
a browser does not offer one.

**Done when** two agents research in two tabs of one browser while a third
writes from what they find, the owner watches all three, takes a tab to log
in somewhere by hand, and an agent told to open Overmind's own address is
refused, by a test.

## M39: named folders, a terminal, the operator's tools

**Measure first.** A PTY shell inside a `Cage` built for a studio, on macOS,
on Linux with Landlock and in the image with the uid alone; who owns what
the server writes into a mounted folder (the known debt of ADR-0052); how
fast a watcher reports a change made from outside on each platform.

**Shape.**

- **The operator names roots, the owner picks folders under them.**
  `OVERMIND_STUDIO_ROOTS`; in the image, a mount. The same refusals the
  working-folder design lists: the home itself, `~/.claude`, `~/.config`,
  the data directory, system paths. Raising a studio to `granted` is the
  instance owner's verb, shows the exact paths before it is confirmed, and
  is on the chain.
- **Folders are mounts on the files surface.** The same tools, the same
  revisions and claims; the server reads and writes on the agents' behalf. A
  change made from outside (the owner's editor, a command) is journaled with
  `actor_kind: outside` and reaches the digests.
- **The terminal.** A PTY the server holds (`openpty`, as `sign_in.rs` does),
  its shell confined to the studio's folders, as the agent uid in the image,
  with no provider credential in its environment. Tools: `terminal_open`,
  `terminal_run {id, command, seconds}`, `terminal_send`, `terminal_read`.
  One hand a terminal at a time. Output passes `secrets::scrub` and is
  clamped. The command line, its exit code and a hash of its output are
  journaled. The stage shows it with a terminal emulator (a new dependency);
  a person can take it and type. Closing kills its process group.
- **Operator's tools on the stage.** Granted per agent as today (ADR-0036).
  The stage shows the tool's name on the agent's row and who holds an
  exclusive one. No pointer, by ADR-0055 decision 4.

**Said plainly in the threat model:** the cage around the terminal permits
the network, as every cage here does; in the image without Landlock the
terminal is separated from other runs by nothing but being the same uid's.

**Done when** the owner raises a studio to `granted` on one project folder,
two agents edit it while a third runs its tests in the terminal, he sees the
command and its output as they happen, saves a file from his own editor and
sees an agent take it into account, and a terminal told to read `~/.ssh` is
refused, by a paired test.

## M40: the whole computer

**Not before** M35 and M36b (a run that is checked, approval by
reversibility), a week of real use of M38 and M39, and the measurement of
whether a caged process can start one outside the cage through `mach-lookup`
(*Debts from the cage*).

**Measure first, as `docs/analysis/…-agents-on-the-desktop.md`.** On macOS:
which actions the accessibility interface performs on a window that is not
frontmost, without the system pointer, and on which kinds of application
(AppKit, Electron, a browser, a game); whether events posted to one process
reach it in the background; a screenshot of one window with ScreenCaptureKit;
a transparent window above everything, ignoring input, for the pointers; how
the hand learns that the owner touched the mouse or the keyboard; what the
Accessibility and Screen Recording grants cost a person to give. The answer
decides how parallel the third level is. The expectation to test: several
agents at once through accessibility on different applications, one at a time
for whatever needs the real pointer.

**Shape.**

- **The hand.** A second crate and binary, macOS first, started by the owner,
  paired with the server by a one-time code on loopback. It holds the two
  grants. It is the only thing that touches the desktop.
- **Tools:** `desktop_look {app|window}` (the accessibility tree, a
  screenshot of that window), `desktop_act` (press, type, select, through
  accessibility), `desktop_open {app}`, `desktop_pointer` (the real pointer;
  needs the baton, which one agent holds at a time).
- **Pointers on the real screen,** drawn by the hand above everything, in
  each agent's colour, and mirrored on the stage in Overmind.
- **The guards.** Armed by the instance owner for a time (thirty minutes by
  default), ended by itself, shown at all times on screen. Applications
  excluded unless the owner removes the exclusion: password managers,
  Keychain Access, System Settings, the terminal and browser profile he
  uses, Overmind itself. Applications he names are held behind an approval
  an action. The hand cannot know that a click pays or sends, and the
  interface does not say it can. Physical input holds every hand until he
  releases. A global key ends the arming. Nothing acts on a locked screen.
- **The journal** records each action with a hash of the window's image
  before and after; the images are kept for the studio's life.

**Done when** the owner arms a studio for fifteen minutes, three agents work
in three applications at once with three pointers on his screen, he moves the
mouse and all three stop, he releases them, the time ends and the tools are
gone from `tools/list`, and an agent sent to an excluded application is
refused, by a test.

## Out of scope

- A cage widened for the agent's own process, at any level.
- Windows and Linux desktops at the third level; a virtual desktop inside the
  image. Written in the roadmap's icebox.
- A proxy in front of operator's MCP tools so that they get a pointer.
- People typing in one file at the same moment with merging by character.
