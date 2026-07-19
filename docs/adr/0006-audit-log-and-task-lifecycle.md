# ADR-0006: Audit log design (hash chain over stored representation) and task lifecycle

- **Date:** 2026-07-19
- **Status:** accepted

## Context

M1 needs the accountability spine: an audit log that makes history tamper-evident, and a validated task state machine. Several design choices had to be made.

## Decisions

### Audit log

1. **Single global chain.** One append-only `audit_events` table with a monotonic `seq` (SQLite AUTOINCREMENT) and one SHA-256 hash chain across all organizations, `org_id` as a filter column. Genesis `prev_hash` is 64 zeros.
2. **Hash over the stored representation.** The hash covers `seq`, `prev_hash`, `kind`, `org_id`, `task_id`, `created_at` and the payload **exactly as stored** (TEXT), with `0x1F` separators between fields. Hashing what is stored — not a re-canonicalized JSON — means verification can never diverge due to serialization differences.
3. **Defense in depth: triggers + chain.** SQLite `BEFORE UPDATE/DELETE` triggers make the table append-only through the SQL surface (mutation *impossible* casually); the hash chain makes any bypass (raw file edit) *detectable*, pointing at the exact first invalid `seq` via `GET /audit/verify`.
4. **Atomic with the domain write.** `audit::append` takes the caller's transaction: a state change and its audit event commit together or not at all.

### Task lifecycle

`open → in_progress → in_review → done`, with `in_review → in_progress` (review rejected) and cancellation from any non-terminal state. `done` and `cancelled` are terminal. The transition table lives in one place (`TaskState::can_transition`) and is enforced server-side; invalid transitions are 400s.

## Alternatives considered

- **Per-organization chains** — parallel appends scale better, but verification and implementation get more complex; premature for self-hosted single-node. Revisit if event volume demands it (the schema doesn't preclude it).
- **Merkle tree / signatures** — stronger proofs (inclusion, non-repudiation with keys), but overkill for M1. Signatures may return in M10 (security hardening).
- **Hashing canonicalized JSON** — rejected: any canonicalization drift between writer and verifier produces false tampering alarms.

## Consequences

- Every handler that mutates domain state must append its event in the same transaction — this is a code-review invariant from now on.
- SQLite's single-writer model serializes appends, so `MAX(seq)+1` inside a transaction is race-free by construction.
- A tampering attacker with file access can be *detected*, not *prevented* — prevention (e.g. periodic anchoring of the head hash elsewhere) is future work noted for M10.
