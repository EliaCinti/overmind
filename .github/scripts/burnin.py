#!/usr/bin/env python3
"""Burn-in: many agents writing and reading one company brain at once.

Not a CI job -- a bench tool. Point it at a running Overmind whose adapter is
a cheap stub, and it founds a company, hires N agents, starts 2N knowledge
tasks simultaneously, and hammers the read endpoints (browse, semantic search,
budget, board) while the writes are in flight. Every response is counted;
anything that is not a 2xx is a finding.

Round one of this script found both SQLite defects M23 fixed: the missing
busy_timeout (reads and writes colliding) and the deferred-BEGIN checkout
(SQLITE_BUSY_SNAPSHOT under simultaneous starts -- 8 of 12 answered 500).
Round three, after the fixes: 24/24 starts, 1200/1200 reads, zero errors.

Usage:
    docker run -d --name burnin -p 127.0.0.1:7575:7070 \
      -v $PWD/stub:/stub:ro \
      -e OVERMIND_AGENT_CMD='sh /stub/agent.sh' \
      -e OVERMIND_SANDBOX_ALLOW=/stub  overmind:latest
    python3 .github/scripts/burnin.py [port] [agents]
"""
import json, sys, time, urllib.request, urllib.error
from concurrent.futures import ThreadPoolExecutor, as_completed

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 7575
N_AGENTS = int(sys.argv[2]) if len(sys.argv) > 2 else 12
N_TASKS = N_AGENTS * 2
API = f"http://127.0.0.1:{PORT}/api"
errors = []

def call(method, path, body=None, timeout=60):
    req = urllib.request.Request(API + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        errors.append(f"{method} {path} -> {e.code}")
        return e.code, None
    except Exception as e:
        errors.append(f"{method} {path} -> {e}")
        return 0, None

_, co = call("POST", "/companies", {"name": f"Burnin-{int(time.time())}", "language": "it"})
cid = co["id"]
agents = [call("POST", f"/companies/{cid}/agents",
               {"name": f"W{i}", "archetype": "builder"})[1]["id"] for i in range(N_AGENTS)]
tasks = []
for i in range(N_TASKS):
    _, t = call("POST", f"/companies/{cid}/tasks",
                {"title": f"Nota {i}", "description": "Scrivi NOTA.md.", "execution_kind": "knowledge"})
    call("POST", f"/tasks/{t['id']}/transition", {"to": "todo"})
    tasks.append(t["id"])

def start(i):
    s, r = call("POST", f"/tasks/{tasks[i]}/start", {"agent_id": agents[i % N_AGENTS]})
    return s, (r or {}).get("session_id")

def reader(rounds):
    ok = 0
    for _ in range(rounds):
        for p in [f"/companies/{cid}/memory/memories",
                  f"/companies/{cid}/memory/memories?q=nota",
                  f"/companies/{cid}/budget", f"/companies/{cid}/tasks", "/health"]:
            s, _ = call("GET", p)
            if s == 200: ok += 1
        time.sleep(0.2)
    return ok

t0 = time.time()
with ThreadPoolExecutor(max_workers=40) as ex:
    starts = [ex.submit(start, i) for i in range(N_TASKS)]
    readers = [ex.submit(reader, 30) for _ in range(8)]
    sessions = [f.result()[1] for f in as_completed(starts) if f.result()[1]]
    deadline = time.time() + 240
    done = set()
    while len(done) < len(sessions) and time.time() < deadline:
        for sid in sessions:
            if sid in done: continue
            _, r = call("GET", f"/sessions/{sid}")
            if r and r.get("status") in ("completed", "failed"):
                done.add(sid)
                if r["status"] != "completed":
                    errors.append(f"session {r.get('status')}: {r.get('last_error')}")
        time.sleep(2)
    reads_ok = sum(f.result() for f in readers)

expected_reads = 8 * 30 * 5
print(f"starts: {len(sessions)}/{N_TASKS} · completed: {len(done)} in {time.time()-t0:.0f}s")
print(f"concurrent reads: {reads_ok}/{expected_reads}")
_, mem = call("GET", f"/companies/{cid}/memory/memories")
print(f"memories: {len((mem or {}).get('items', []))}/{N_TASKS + 1} expected")
_, ver = call("GET", "/audit/verify")
print(f"audit: {'valid, ' + str(ver.get('events_checked')) + ' events' if ver and ver.get('valid') else 'BROKEN'}")
print(f"errors: {len(errors)}")
for e in errors[:10]:
    print(" -", e)
sys.exit(1 if errors or len(done) < len(sessions) else 0)
