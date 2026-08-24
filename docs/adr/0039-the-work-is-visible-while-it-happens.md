# ADR-0039: The work is visible while it happens

- **Date:** 2026-08-24
- **Status:** accepted
- **Builds on:** [ADR-0030](0030-how-you-pay-is-a-first-class-fact.md) (the same stream already carries the plan window), [ADR-0038](0038-from-the-ceos-plan-to-a-running-task.md) (`answering` on the conversation).

## Context

The owner, watching a twenty-minute Blender run with nothing moving on
screen: *"the user must know what is happening — and above all that
something is happening."* And of the chat: *"could we do what Claude does —
show what it is doing, thinking?"*

The material was already flowing. The adapter runs with `--output-format
stream-json --verbose`: one JSON event per line, naming every tool call and
every assistant message as it happens. Overmind collected that stream into a
buffer and read it **once, at the end** — the narration existed and was
thrown away until the run was over, which is exactly when nobody needs it
anymore.

## Decisions

1. **The stream is read as it arrives.** `runner::drain_narrating` replaces
   `wait_with_output` on both spawn paths (task runs and conversational
   turns): stdout is read line by line into the same buffer the callers
   always got — same timeout, same stderr courtesy, same envelope parsing at
   the end — and each line is glanced at for what it says the agent is doing.

2. **Activity is structured, never an English sentence.** An `assistant`
   event yields `{kind:"tool", tool, server?}` (MCP names `mcp__srv__tool`
   are split; underscores become spaces) or `{kind:"text", preview}` (first
   120 chars). The interface words it in the person's language — a server
   that shipped sentences would have shipped them in one language forever.

3. **Kept in memory, keyed by what is watching.** Chat turns narrate under
   their conversation id, meeting turns under the meeting id, task runs under
   the session id — one `AppState` map, latest line wins, cleared when the
   run ends however it ends. A restart forgets it, correctly: the narration
   of a process that did not survive should not survive either.

4. **Exposed where the watcher already looks.** `GET …/conversation` carries
   `activity` beside `answering`; `GET /sessions/{id}` carries `activity`
   beside `status`. The chat shows the line inside the typing bubble ("Sta
   usando execute blender code (blender)…"); the task detail shows it beside
   the running badge. Both poll gently (2s) while something runs — the
   narration has no websocket event of its own, deliberately: a line per tool
   call is signal, a push per line is noise.

## Consequences

- A Blender run that thinks for four minutes now *says* it is thinking, and
  names the tool the moment it acts. The chat's dots gained words.
- Two same-day inbox fixes ride along, same complaint ("the user must know"):
  deciding the last waiting item closes the inbox instead of surfacing the
  decided pile (clipped, at that — the list now scrolls), and what waits on
  you stands alone.
- `tests/activity.rs` holds both surfaces with a narrating stub adapter.
- Meetings narrate under their meeting id; the meeting view does not show it
  yet — the room already renders turns as they land, which is narration of a
  coarser grain.
