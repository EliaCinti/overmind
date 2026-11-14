# ADR-0055: Agents share a studio, and a level widens the studio, never the cage

- **Date:** 2026-11-13
- **Status:** accepted (the owner, 13 Nov 2026)
- **Builds on:** [ADR-0023](0023-os-level-sandboxing.md) and [ADR-0053](0053-the-cage-grants-nothing-the-host-runs.md) (the cage), [ADR-0027](0027-agents-reach-memory-through-overmind.md) (agents talk to Overmind over MCP, with a per-run token), [ADR-0028](0028-overmind-as-an-mcp-server-for-outside-callers.md) (the credential decides the grant), [ADR-0036](0036-tools-in-the-agents-hand.md) (a tool is a door the operator opened), [ADR-0039](0039-the-work-is-visible-while-it-happens.md) (activity is structured), [ADR-0026](0026-change-awareness-across-concurrent-agents.md) (awareness rides along with work), [ADR-0050](0050-a-loop-that-converges-and-a-stop-that-is-not-a-crash.md) (a stop that winds down).

## Context

The owner's ask, 13 Nov 2026: stop keeping every agent in a cage of its own.
He wants one place where several agents work on his request at the same time
and under his eyes, each with a pointer he can tell apart, every change one
makes visible to the others at once so they build on it, and three depths he
chooses between: inside Overmind only, then named folders with a terminal and
authorised tools, then the whole computer. It has to look like a team at work,
and he has to be able to step in or take over.

What exists today points the other way, on purpose.

- **A run is alone.** Each run works in its own directory, and the cage denies
  everything else, another run's directory included. Agents meet through
  tasks, dependencies, meetings and memory, never through a shared surface.
- **A run has no pointer.** An agent is a headless CLI. What it does is a
  stream of tool calls, of which Overmind keeps the latest line per run in
  memory, polled every two seconds (ADR-0039).
- **The live channel is coarse.** `/ws` says "this company changed" and the
  client refetches (ADR-0010). Nothing carries a position or a keystroke.

Three facts decide the design before any preference does.

1. **The agents do not have mice.** Below the third level a pointer on screen
   can only be a drawing of something the agent did. If the drawing is not
   tied to a real action, the screen shows a team at work whether or not one
   is working.
2. **An operating system has one pointer and one keyboard focus.** "Several
   agents using the whole computer as real users in parallel" cannot mean
   several system pointers. On macOS it can mean several agents acting on
   different applications through the accessibility interface, with one of
   them at a time holding the real pointer. How far that goes is not known
   and has to be measured.
3. **The cage is the only thing between a prompt injection and the host.**
   Every level the owner describes sounds like widening it.

## Decision

**1. A studio is a shared place that the server owns.** A studio belongs to a
company, has a title, a reach (decision 3) and a set of *surfaces*: files
first, then a browser, a terminal, the desktop. Its bytes live under
`<data>/studios/<id>/`, written by the server only. Several runs are attached
to one studio at the same time.

**2. Agents reach the studio through Overmind, over MCP, and the cage does not
move.** The per-run token that already resolves a run to its company
(ADR-0027) also resolves it to its studio and its agent. The studio's tools
(`studio_look`, `studio_read`, `studio_edit`, …) are served by Overmind's own
MCP endpoint. The agent's process stays in the cage it has today, at every
level. This is the rule the owner fixed on 6 Nov for the working folder,
generalised: *the place does not enter the cage; the server writes it.*

It follows that every action on a studio passes through one point that knows
who is acting. Attribution, the journal, the live view, the gate that holds an
agent and the level check are the same code path, not five mechanisms kept in
agreement.

**3. Reach is a property of the studio, with three values, enforced by which
tools exist.**

| reach | what the studio contains | where it runs |
|---|---|---|
| `inside` | its own files, a browser Overmind hosts | native and the image |
| `granted` | plus folders the owner named, a terminal confined to them, tools the operator declared | native and the image (folders must be mounted) |
| `computer` | plus the real desktop, through the hand (decision 9) | native macOS first |

`tools/list` answers by reach: a tool above the studio's reach is not refused,
it is absent (ADR-0028). Opening a studio at `inside` is a member's verb.
Raising it to `granted` names paths on the machine and is the instance
owner's, like declaring a tool (ADR-0036). Arming `computer` is the instance
owner's, for a bounded time, and ends by itself. No agent, the CEO included,
can raise a reach or propose a studio above `inside`.

**4. A pointer is a fact.** A pointer is drawn where a call the server
witnessed acts: the lines an edit changed, the element a click hit, the
terminal a command went to. Motion between two facts is presentation and may
be paced; nothing is drawn that no call produced. A tool Overmind does not
host (an operator's MCP server such as Blender) has a name on the stage and no
pointer, because Overmind does not see where it acts.

**5. Two grains: a journal that lasts and a presence that does not.**

- The **journal** is one append-only table per studio: every action that
  changed something or decided something, in order, each row hashed over the
  previous one. The company's audit chain records the studio's opening, every
  change of reach, every hold and takeover, and the closing with the
  journal's final hash. The chain stays small; the journal cannot be edited
  without the chain saying so.
- **Presence** is where each actor is and what it is doing now. Kept in
  memory, latest wins, gone at restart, never on the chain.

A new socket per studio, `/ws/studios/{id}`, carries both: a snapshot on
connect, then actions with their sequence number and presence frames. A gap
in the sequence is repaired from the journal over HTTP. The company channel
stays coarse.

**6. The server orders the writes.** Every file has a revision. An edit names
the revision it read; if the region it touches has changed since, the edit is
refused and answered with the current text and who changed it. An agent can
claim a target with a stated intent; a claim is shown to everybody, refuses a
second hand by name, and expires. One hand at a time on a browser tab, a
terminal, the real pointer.

**7. Awareness rides along with the work.** Every answer to a studio call
carries a bounded digest of what others changed since this agent last looked.
`studio_look` asks for it outright. An agent can leave a short note pinned to
a place. Nothing is pushed into an agent's context between its own calls. A
note reaches another agent's prompt delimited and clamped, as every
agent-written text has since M10.

**8. The person's hand always wins.** A person on the studio has a pointer of
their own, can pin a note, can hold one agent or all of them (no call of a
held agent changes anything; a hold that lasts winds the run down with a
handoff, ADR-0050), can take a surface and work on it, and can close the
studio, which stops every run attached to it. At `computer`, input from the
physical mouse or keyboard holds every hand at once.

**9. The third level goes through a hand on the host, and promises what it can
keep.** The desktop is reached by a separate native program, the *hand*, that
the owner starts and to which macOS itself asks him to grant Accessibility and
Screen Recording. The agent's process is still caged; it asks the server, the
server asks the hand. The hand is not in the image and cannot be: a container
has no desktop of the host's to act on.

At this level there is no containment to claim. An agent that follows an
injected instruction acts with the owner's session. What stands is: arming by
the owner for a bounded time, applications excluded by default, named
applications held behind an approval, the owner's physical input stopping
everything, a recording in the journal, and the brakes of M36a. The threat
model says this in these words.

**10. Joining a studio is a declared capability of the provider.** A provider
whose CLI cannot be given an MCP configuration cannot join a studio, and the
hire dialog says so rather than a run failing to teach it (ADR-0048).

## Alternatives considered

- **One directory granted writable in every run's cage, a file watcher to see
  the changes.** The agents would use their own file tools, which are faster
  and cost fewer tokens than calls to Overmind. Rejected: a watcher sees that
  a file changed and not who changed it, runs of one uid cannot be told apart,
  and each provider's stream would have to be parsed for intent. It also
  reverses ADR-0053's rule a week after it was written. To be reconsidered
  only on evidence that the mediated path is too slow for real work.
- **Pointers animated from the activity line.** Cheap, and a lie in the sense
  of fact 1: an idle agent would still look busy.
- **A CRDT for shared text (Yjs, Automerge).** It merges keystrokes from
  simultaneous typists. Agents act in whole calls; revisions and refusal are
  enough and add no dependency. To be reconsidered if people typing beside
  agents becomes common.
- **Every action on the audit chain.** A studio produces hundreds of actions
  an hour; the chain of a week of real use holds about nine hundred events.
  The anchored journal keeps both properties.
- **Widening the cage level by level.** It is what the three levels sound
  like. Rejected because it makes the agent's own process the thing trusted
  with the folder, the terminal and the desktop, and removes the one point
  where an action can be attributed, shown, held and refused.
- **One virtual machine per agent for the third level.** True parallelism and
  real isolation, and not what was asked: the owner's applications and files
  are not in it. Stays possible later as a different kind of studio.
- **Lifting the browser and the desktop from tools each agent starts for
  itself** (a Playwright or computer-use MCP server per agent, ADR-0036).
  Works today and shares nothing: each agent gets its own browser, and
  Overmind sees a tool name.

## Consequences

- A new module and its tables (`studios`, members, files, journal, claims,
  notes), a third kind of grant in `mcp_server.rs`, a second socket, a new
  view in the web app. Each later surface adds tools and a tile, not a
  mechanism.
- Work done through the studio costs more tokens per edit than native file
  tools. This is the price of decision 2 and it is paid knowingly.
- The per-run MCP token becomes an identity shown on screen. The known debt
  that a run can read the token of a run beside it
  ([NEXT](../NEXT.md), *Debts from the cage*) has to be paid before the first
  slice, not after.
- Holding and closing need to reach a running process, which is the open debt
  *Moving a task does not stop its run*.
- The threat model gains a section per level. Rows land with the slice that
  makes them true, each naming its test.
- Chromium, a terminal emulator in the web app and, at the third level, a
  second binary are new dependencies. Each is named in the milestone that
  brings it, with its measured cost.
- Open, to be measured before the milestone that depends on it: whether
  Chromium runs under `sandbox-exec` and under Docker's default seccomp
  profile with its own sandbox on; how many agents can act on macOS
  applications in parallel without the system pointer, and on which kinds of
  application.
