# ADR-0044: The archive is the instance

- **Date:** 2026-08-29
- **Status:** proposed
- **Builds on:** [ADR-0014](0014-docker-deployment.md) (everything on one volume), [ADR-0024](0024-managed-per-company-brain.md) (a brain per company under `/data`), [ADR-0029](0029-the-cage-inside-the-container.md) (the token lives in `<data>/claude-oauth-token`), [ADR-0032](0032-authentication-the-boundary-moves-off-the-machine.md) (an unclaimed instance is open; the owner is claimed once), [ADR-0043](0043-the-compose-file-is-the-installers.md) (the image is disposable).

## Context

On 22 Aug an unplugged SSD took the Docker VM's disk with it, and the only
thing between the owner and total loss was a `cp -a` out of a volume that was
already reporting I/O errors. On the two-machine walk the friend's honest
question was the same in another form: *what happens to my data when I run
`docker compose down -v`?* — and the answer today is a warning in a comment.
`NEXT.md` ranks this second among the frictions of the walk and first among
the things the product cannot do: **the data has no way out.**

What the data is: `overmind.sqlite` (the companies, the org, the tasks, the
hash-chained audit), one Wadachi brain per company under
`companies/<id>/brain/`, `attachments/`, `artifacts/`, the run transcripts
under `sessions/` and `chat/`, `meetings/`, two small files that decide who
pays (`pay-with-plan`, `claude-oauth-token`), and scratch (`run/`,
`worktrees/`). On the owner's house instance that is ~1 GB, two thirds of it
transcripts.

Four decisions were the owner's, taken 29 Aug 2026:

1. The subscription token goes into the archive, **sealed with a passphrase**
   — not left out (a restore should not cost a sign-in), not in the clear (an
   archive is what you copy to other disks).
2. An archive is both a **download from the browser** and a **file in a
   folder on the server**, so a scheduled backup has a place to land.
3. **All of `/data`** goes in — transcripts included: a restore reproduces the
   instance, not a summary of it.
4. Restore lands on an **empty instance only**; anything else is refused with
   the reason.

## Decision

**One archive, one verb each way, the owner's hand on both.**

### The archive

`overmind-<scope>-<UTC timestamp>.tar.gz`, entries in this order:

1. `MANIFEST.json` — format version, Overmind version, created-at, scope
   (`instance`; `company:<id>` is a later slice), the **audit chain report**
   (`verify()` run before sealing: event count, last hash, verified), the
   SHA-256 of every entry that follows, and — when a token is present — the
   KDF parameters and nonce used to seal it. The manifest is the archive's
   word for itself; restore checks every hash against it.
2. `overmind.sqlite` — a **consistent snapshot**, taken with `VACUUM INTO`
   on the live pool (SQLite 3.27+; the image and the native build both carry a
   current library). No file copy of a database in WAL mode, ever.
3. `companies/<id>/brain/…` — each brain's `brain.db` taken the same way
   (`VACUUM INTO` through a read-only connection to it; Wadachi's own
   process may hold it open and WAL makes that safe), the rest of the brain
   directory copied after the database.
4. `attachments/`, `artifacts/`, `sessions/`, `chat/`, `meetings/`,
   `pay-with-plan` — copied as files.
5. `secrets/claude-oauth-token.enc` — the token, **sealed**: key =
   argon2id(passphrase, fresh salt) with the same parameters the door uses for
   passwords; cipher = XChaCha20-Poly1305 with a fresh nonce; the manifest
   carries salt and nonce, never the key. An export with a token present
   **requires** a passphrase; without a token none is asked.

Left out, by name: `run/` and `worktrees/` (scratch of runs in flight;
a code task's branch lives in the workspace repository, not here) and
`backups/` (an archive does not contain the archives).

### Export — `POST /api/backup`

Owner only (`require_owner`), like billing: an export is everyone's data.
Body: `{ "passphrase": "…" }` when a token is present. The server writes the
archive to **`OVERMIND_BACKUP_DIR`** (default `<data>/backups/`), appends
`backup.exported` to the audit chain with the actor, the file name, the size
and the chain report, and answers with the file's name and `GET
/api/backup/<name>` — which streams it to the browser, owner only. The
folder is listable (`GET /api/backups`) and pruned by nobody: what to keep is
the owner's decision, said in the UI beside each file's date and size.

The passphrase is never logged and never stored; the request body is scrubbed
like everything the sign-in flow emits.

### Restore — `POST /api/restore`

Accepted **only while the instance is unclaimed** — no owner, no companies,
the state a fresh `docker compose up` leaves you in. That is the "empty
instance" rule made mechanical, and it is the state in which the API is
already open by design (ADR-0032): restore needs no door because the door
does not exist yet, and the archive brings the owner with it. Multipart: the
archive, an optional passphrase. The server:

1. unpacks into `<data>/restore-<id>/`, **never onto the live tree**;
2. checks every entry's hash against the manifest, opens the database and
   runs `verify()` — the chain must verify **and** match the report the
   manifest carries;
3. unseals the token if a passphrase was given; a wrong passphrase refuses
   the token only (`the sign-in stays yours to redo`) and restores the rest;
   no passphrase skips the token with the same note;
4. on any failure: deletes the staging directory, says why, touches nothing;
5. on success: swaps the staging tree into place, appends
   `backup.restored` to the restored chain (actor: the archive's owner
   account, marked as restored-by-archive; the manifest hash inside the
   payload), and **exits 0 with a log line saying so** — the image's
   `restart: unless-stopped` brings it back on the restored data; natively
   the person restarts, and the UI says exactly that.

A restore on a claimed instance is `409` with the reason and the way out
(stop, empty, start, restore). There is no "restore over": the owner asked
for it not to exist.

### The UI

Under the instance's settings, owner only: **Export** (a passphrase field
appears when a token is present, with the sentence about what it seals), the
list of archives in the folder with a download link each, and — on an
unclaimed instance, on the landing beside *claim the owner* — **Restore an
archive**.

### What this ADR does not decide

- **Scheduled backups.** The folder makes them possible (`POST /api/backup`
  from a cron with an owner token); a schedule inside Overmind is a later
  slice.
- **Company-scoped export/restore** (`company:<id>`): the manifest has the
  field; the row-filtering across thirty tables is its own slice, after
  instance-level export has been restored on another machine for real.
- **Off-site copies** (S3, rsync): the archive is a file; the copy is yours.

## Consequences

- Three new dependencies, named because the rulebook asks: `tar`, `flate2`
  (rust backend), `chacha20poly1305`. `argon2` is already at the door.
- The threat model gains a row: *an archive is the whole instance; it leaves
  the box only by the owner's hand, and the only secret in it is sealed by a
  passphrase the server never keeps* — held by tests that read the archive
  bytes and find no `sk-ant-`.
- `docker-compose.yml`'s "until export/restore ships, copy the volume"
  comment retires; the wiki page keeps the raw volume copy as the last
  resort it is.
- Restore ends the process. Honest cost of not re-opening a pool and a
  scheduler mid-flight; the alternative was a restore that half-lives until
  the next restart anyway.

## Rejected

- **Token in the clear** (as NEXT.md first wrote): an archive is what ends
  up on a USB stick. **Token left out**: a restore that costs a sign-in on a
  machine that may have no browser (the image).
- **Copying `overmind.sqlite` as a file**: a WAL database copied mid-write is
  a database with a story missing. `VACUUM INTO` is the SQLite backup API
  reachable from sqlx.
- **Restore over a live instance with a confirmation**: the owner chose the
  empty-instance rule; it also removes an entire class of bugs (what happens
  to running tasks, open sockets, the scheduler) from the first slice.
- **Only a browser download**: nowhere for a nightly backup to land, and 1 GB
  through a browser tab every evening is not a habit anyone keeps.
