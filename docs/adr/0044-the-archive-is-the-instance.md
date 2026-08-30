# ADR-0044: The archive is the instance

- **Date:** 2026-08-29
- **Status:** proposed
- **Builds on:** [ADR-0014](0014-docker-deployment.md) (one deployment image), [ADR-0024](0024-managed-per-company-brain.md) (a brain per company under `/data`), [ADR-0029](0029-the-cage-inside-the-container.md) (the agent's home on its own volume, the server's shelves `0700`), [ADR-0032](0032-authentication-the-boundary-moves-off-the-machine.md) (an unclaimed instance is open; the owner is claimed once), [ADR-0037](0037-who-pays-is-asked.md) (the product's sign-in token at `<data>/claude-oauth-token`, the `pay-with-plan` marker), [ADR-0043](0043-the-compose-file-is-the-installers.md) (the image is a download).

## Context

On 22 Aug an unplugged SSD took the Docker VM's disk with it, and the only
thing between the owner and total loss was a `cp -a` out of a volume that was
already reporting I/O errors. On the two-machine walk the friend's honest
question was the same in another form: *what happens to my data when I run
`docker compose down -v`?* — and the answer today is a warning in a comment.
`NEXT.md` ranks this second among the frictions of the walk and third among
the things 0.1 cannot do: **the data has no way out.**

What the data is — by what the code does, not by what a directory is called:

- **durable:** `overmind.sqlite` (the path comes from `OVERMIND_DB`, which is
  under `/data` only in the image — natively it defaults to the working
  directory), one brain per company under `<data>/companies/<id>/brain/`,
  `<data>/attachments/`, `<data>/artifacts/`, `<data>/meetings/`, and two
  small files that decide who pays: `pay-with-plan` and `claude-oauth-token`;
- **scratch:** `<data>/sessions/`, `<data>/chat/`, `<data>/worktrees/` — the
  runner and the CEO call them *throwaway scratch dirs*; a code task's branch
  lives in the workspace repository, a deliverable in `artifacts/`;
- **on the other volume:** the agent CLI's own login (`/home/agent/.claude`,
  `overmind-agent-home`), when the sign-in was done with `claude login`
  rather than from the product. It is not under `/data` and this ADR does
  not archive it.

On the owner's house instance the durable set is ~300 MB; the scratch dirs
are another ~700 MB.

Four decisions were the owner's, taken 29 Aug 2026 — the third on a wrong
premise, corrected here:

1. The subscription token goes into the archive, **sealed with a
   passphrase** — not left out (a restore should not cost a sign-in), not in
   the clear (an archive is what you copy to other disks).
2. An archive is both a **download from the browser** and a **file in a
   folder on the server**, so a scheduled backup has a place to land.
3. Everything durable goes in. The owner chose "all of `/data`" when
   `sessions/` and `chat/` were described to him as transcripts; they are
   scratch, and the archive takes what a restore can use.
4. Restore lands on an **empty instance only**; anything else is refused
   with the reason.

## Decision

**One archive, one verb each way, the owner's hand on both.**

### The archive

`overmind-<scope>-<UTC timestamp>.tar.gz`, assembled from a staging
directory (`<data>/export-<id>/`, so hashes are known before the tar is
written; the folder needs transient room for one more copy), entries in this
order:

1. `MANIFEST.json` — `format` (1), the Overmind version, created-at, `scope`
   (`instance`; `company:<id>` is a later slice), the SHA-256 of every entry
   that follows, the **chain report of the snapshot** — computed by opening
   the `VACUUM INTO` output, not the live pool: `valid`, `events_checked`,
   `first_invalid_seq` as `audit::verify` returns them, plus `last_seq` and
   `last_hash` read from the snapshot's `audit_events` — and, when a token
   is present, the KDF parameters, salt and nonce used to seal it.
2. `overmind.sqlite` — a **consistent snapshot** of the database named by
   `OVERMIND_DB`, taken with `VACUUM INTO` on the live pool. Never a file copy
   of a database in WAL mode. Two columns are **scrubbed in the snapshot**
   before hashing: `agent_task_sessions.mcp_token` (a per-run bearer; a run in
   flight at export time must not survive as a credential) and
   `company_tokens.token` (the editor's integration tokens — a restored
   instance does not honour tokens that lived in a file; they are marked
   revoked in the snapshot and the restore says so). Session hashes stay:
   they are hashes. The scrub runs with `secure_delete` on and ends in a
   `VACUUM`: an `UPDATE` alone leaves the old bytes in the page's free space,
   readable with `strings`.
3. `companies/<id>/brain/…` — the managed brain is Wadachi's (ADR-0024,
   ADR-0031): its `brain.db` is taken with `VACUUM INTO` through a read-only
   connection (Wadachi's own process may hold it open; WAL makes that safe),
   the rest of the directory copied after. With `OVERMIND_MEMORY_CMD` empty
   there is no brain and nothing to take; with another provider the
   directory is copied as files and the manifest says `brain: copied`, not
   `snapshot`.
4. `attachments/`, `artifacts/`, `pay-with-plan` — copied as files, each
   opened `O_NOFOLLOW`: `read_dir` says what a name *was*, `std::fs::copy`
   resolves it again, and a privileged copy that follows a link planted in
   between is how a credential gets into an archive that promises none.
   Every directory the export writes is `0700` and every file it writes
   `0600` — the staging tree included, and it lives inside the backup folder
   rather than beside the run shelves the cage leaves traversable.
5. `secrets/claude-oauth-token.enc` — the token, **sealed**: key =
   argon2id(passphrase, 16-byte salt; m = 64 MiB, t = 3, p = 1; the numbers
   written here because the door's `Argon2::default()` is not a spec);
   cipher = XChaCha20-Poly1305, 24-byte nonce, the archive's identity line
   (`overmind-backup/<format>/<scope>/<created_at>`) as associated data —
   not the manifest's bytes, which name this entry's hash and would make the
   two circular. Salt and nonce ride in the manifest; the key never
   exists outside the request. An export with a token present **requires** a
   passphrase; without a token none is asked.

Left out, by name: `sessions/`, `chat/`, `worktrees/`, `meetings/` (scratch —
and the meeting room in particular is `hand_over`-ed to the agent uid for
every turn and outlives the meeting, so copying it as the server would be a
privileged read of an agent-writable tree; what a meeting decided is in
`meeting_turns`, which rides in the database), the export/restore staging
dirs, and `backups/` (an archive does not contain the archives). The agent CLI's own login on the other volume is out by boundary,
and the compose comment says so beside the volume.

### Export — `POST /api/backup`

**Owner only, on a claimed instance.** `require_owner` today waves an
unclaimed instance through — right for claiming, wrong here: an unclaimed
instance can already hold companies and a product sign-in, and anyone on the
port could otherwise fill `backups/` and download it. Export, list and
download refuse with `409 unclaimed` until an owner exists.

Body: `{ "passphrase": "…" }` when a token is present; the handler never logs
a body. The archive lands in **`OVERMIND_BACKUP_DIR`** (default
`<data>/backups/`, created `0700`, archives `0600` — the agent uid in the
image must not read a plaintext database it cannot read anywhere else; the
container smoke test's probe is extended to the folder). `backup.exported`
goes on the chain with the actor, the name, the size and the chain report.
The answer names the file; `GET /api/backups` lists the folder;
`GET /api/backup/<name>` streams the file (`tower-http` serve, not a
`Vec<u8>`), and `<name>` must be a bare entry of the folder — no separator,
no `..`, no path that is not in the listing. Pruning is the owner's: the UI
shows date and size beside each archive and a delete that is audited.

### Restore — `POST /api/restore`

Accepted **only while the instance is empty**: no user, no company, no token
file — a predicate of its own (`state_of` gains it), not `require_owner`'s
"no user yet". That is the state a fresh `docker compose up` leaves you in,
and it is a state in which the API is already open by design (ADR-0032): **a
restore is a claim with a payload.** Whoever can reach the port of an empty
instance can claim it today; with this ADR they can also fill it from an
archive of their choosing. The threat model says so in as many words, and
the compose file's rule stands — loopback by default, claim before you share
the address.

The upload is multipart, streamed to disk (the route is exempt from the
128 MB body limit; nothing of the archive is held in memory), with an
optional `passphrase` or an explicit `skip_token: true`. The server:

1. unpacks into `<data>/restore-<id>/`, **never onto the live tree**;
2. reads the manifest: `format` must be known, the Overmind version must not
   be newer than the server's (`sqlx::migrate!` would refuse the database
   and the image would crash-loop under `restart: unless-stopped`);
3. checks every entry's hash; opens the restored database in staging and
   runs `verify()` — the chain must verify **and** match `events_checked`,
   `last_seq`, `last_hash` in the manifest;
4. unseals the token when a passphrase is given. **A wrong passphrase
   refuses the whole restore** — this is the moment a retry is free; after
   the swap the instance is claimed and it would not be. `skip_token: true`
   restores without the token *and without `pay-with-plan`*: a marker that
   says "the plan pays" with nothing that pays would leave every agent
   command credential-less, and the UI says the sign-in is yours to redo;
5. on any failure: deletes the staging directory, says why, touches nothing;
6. on success: writes `<data>/restore-pending` naming the staging directory
   and the manifest hash, **answers** — so the UI can say "restored,
   restarting" — and then exits 0 with a log line.

**The swap happens at boot, before the pool opens.** `main` looks for
`restore-pending` first: removes the live database's `-wal`/`-shm` sidecars
(a stale WAL replayed onto a restored file is a database with someone
else's last minute in it), moves the staged database to the `OVERMIND_DB`
path and the staged directories into `<data>`, deletes the marker, then
opens the pool as always and appends `backup.restored` — a system event
like the scheduler's, no actor, the manifest hash and the scope in the
payload; the archive's owner did not act and is not named as if they had.
Nothing is renamed under a live pool or a running scheduler; `/data` being a
mount point (renames within it, never across it) is the only atomicity the
design needs.

A restore on a non-empty instance is `409` with what makes it non-empty and
the way out (stop, empty, start, restore). There is no "restore over": the
owner asked for it not to exist.

### The UI

Under the instance's settings, owner only: **Export** (a passphrase field
when a token is present, with the sentence about what it seals), the list
of archives with download and delete, and — on an empty instance, on the
landing beside *claim the owner* — **Restore an archive**, with the
passphrase field and the "restore without the sign-in" choice spelled out.

### What this ADR does not decide

- **Scheduled backups.** The folder makes them possible; a schedule inside
  Overmind — and where its passphrase would live, which is the real
  question — is a later slice.
- **Company-scoped export/restore** (`company:<id>`): the manifest has the
  field; the row-filtering across thirty tables is its own slice, after an
  instance-level archive has been restored on another machine for real.
- **Off-site copies** (S3, rsync): the archive is a file; the copy is yours.
- **The agent CLI's own login**: on its own volume, by ADR-0029's boundary.
  The wiki keeps the raw volume copy for it, and for the day the product's
  export is not what you have.

## What the security review changed

The first cut of slice A was reviewed before it merged, and two findings held:

- **The staging tree was left at the umask's mercy** — `0755` under a data dir
  the cage deliberately keeps traversable, holding a `0644` snapshot of the
  whole database and every brain for the length of an export. On Docker
  Desktop, where Landlock is absent and the uid is the only boundary, that
  handed an agent everything the threat model says it cannot read. Staging
  moved inside the `0700` backup folder, is created `0700` explicitly, and the
  snapshots are `0600`.
- **`meetings/` was in the durable set and copied as the server.** The room is
  the agent's; `copy_tree` checked with `file_type()` and copied with
  `std::fs::copy`, which resolves the path again — a link swapped in between
  would have been followed as root. The room is now scratch, and every copy
  is `O_NOFOLLOW`.

Two more the code review added, before any of this shipped: the assembly —
every copy, every hash, the KDF and the gzip — runs in `spawn_blocking`, off
the runtime's workers, because a gigabyte of instance would otherwise hold one
for minutes while the socket and the scheduler wait; and a passphrase that
seals a token must be **at least twelve characters**, longer than the door's
password floor, because this credential is the one that leaves the machine and
nothing rate-limits an attempt on an archive sitting on somebody's disk. A
`OVERMIND_BACKUP_DIR` that holds the data directory is refused rather than
obeyed: forcing it `0700` would take away the path a caged agent walks to its
own run.

## Consequences

- Three new dependencies, named because the rulebook asks: `tar`, `flate2`
  (rust backend), `chacha20poly1305`. `argon2` is already at the door.
- The threat model gains its row **in the first slice, with the code** —
  not after: *an archive is the whole instance and leaves the box only by
  the owner's hand on a claimed instance; the subscription token in it is
  sealed by a passphrase the server never keeps; per-run and integration
  tokens are scrubbed from it; a restore is a claim with a payload, open
  exactly as wide as the claim is.* Held by tests that read the archive
  bytes and find no `sk-ant-`, no bearer from `agent_task_sessions`, no
  `company_tokens.token`.
- `docker-compose.yml`'s "until export/restore ships, copy the volume"
  comment retires; the wiki keeps the raw copy of both volumes as the last
  resort it is, and names what the export does not carry.
- Restore ends the process and the swap is a boot-time job: the honest cost
  of a database that is never renamed under its own pool.
- Restored instances re-mint their editor tokens; the restore notice says
  so.

## Rejected

- **Token in the clear** (as NEXT.md first wrote): an archive is what ends
  up on a USB stick. **Token left out**: a restore that costs a sign-in on a
  machine that may have no browser (the image).
- **Copying `overmind.sqlite` as a file**: a WAL database copied mid-write is
  a database with a story missing. `VACUUM INTO` is the consistent snapshot
  reachable from sqlx.
- **Restore over a live instance with a confirmation**: the owner chose the
  empty-instance rule; it also removes an entire class of bugs (running
  tasks, open sockets, the scheduler, a pool pointing at an unlinked inode)
  from the first slice.
- **Swapping the tree inside the request**: the pool would keep writing to
  the old inode, the audit event would land in the old file, the stale WAL
  would replay onto the new one. The marker-and-boot swap is one file more
  and none of that.
- **Restoring the rest when the passphrase is wrong**: an irreversible swap
  on a typo, with a marker that says the plan pays and nothing that does.
- **Only a browser download**: nowhere for a nightly backup to land, and a
  large file through a browser tab every evening is not a habit anyone
  keeps.
- **Archiving the scratch dirs because "all of `/data`" was said**: the
  choice was made on a mislabel; archiving 700 MB of throwaway directories
  under the name "transcripts" would have been the label's error, not the
  owner's.
