# ADR-0040: A conversation outgrows the turn, and is compacted

- **Date:** 2026-08-24
- **Status:** accepted
- **Builds on:** [ADR-0013](0013-memory-over-mcp.md) (memory over MCP), [ADR-0024](0024-managed-per-company-brain.md) (one brain per company), [ADR-0022](0022-the-price-of-a-turn.md) (every turn is budget-gated).

## Context

Every chat turn replays the whole thread into the prompt. After one real day
of use, the owner's CEO thread held days of full-length briefs; turns grew
slow and expensive, and the thread was on its way to an error nobody would
understand. The owner named both the problem and the shape of the fix:
*"cosa succede quando la chat diventa un po' lunga? Potremmo fare come fa
Anthropic"* — and, for the memory half: *"l'agente salva tutto il necessario
su wadachi e riparte ricordando tutto."*

## Decisions

1. **Compaction is a server decision, taken at the turn.** When the
   transcript that would ride into a turn exceeds
   `OVERMIND_CHAT_COMPACT_CHARS` (default 60 000 characters; `0` disables),
   the agent first writes a **handoff summary** of everything but the last
   six messages: decisions and who took them, open questions, numbers, names
   and constraints, in the conversation's own language. The summary is a
   budget-gated turn like any other (ADR-0022) — compaction is a spend, and
   it shows on the ledger.

2. **Append-only summaries, in their own table.** `conversation_summaries`
   (id, conversation_id, content, covers_until, created_at). A turn reads
   the latest summary plus every message after `covers_until`. A later
   compaction writes a new row covering more — it never rewrites one. The
   summary the CEO reads is the summary the audit can show.

3. **The messages are not deleted.** The thread stays whole for the person
   scrolling back and for the audit; what shrinks is the *turn's* view of
   it. Deleting would trade a UI convenience for a hole in the record.

4. **The summary goes to the brain too.** On compaction the summary is also
   `store_memory`d to the company brain (tags `chat-compaction`,
   `conversation`; category `context`) — so what a long thread learned is
   recallable from tasks and meetings, not only from the thread itself.
   Best-effort, like every memory write: a brainless company still compacts.

5. **The person is told, quietly.** A system chip lands in the thread:
   *"Conversazione compattata: N messaggi precedenti riassunti (e salvati in
   memoria). Il filo resta leggibile qui."* Audited as `chat.compacted`.

6. **A failed compaction never eats the turn.** The full transcript rides
   once more and the next turn tries again.

## Consequences

- A thread can now live for months: the turn's prompt is bounded by the
  summary plus six messages, whatever the thread's true length.
- `tests/compaction.rs`: below the threshold nothing changes; above it the
  summary rides, the fresh question rides verbatim, the chip lands, and the
  next turn does not re-compact.
- Meetings are already bounded by their turn cap; task prompts do not carry
  the thread. Chat was the one unbounded surface.
- Not done, deliberately: the owner's "delete the chat and open a new one".
  The turn's view resets exactly as if it had; the record keeps its history.
