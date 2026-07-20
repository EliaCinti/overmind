import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { X, Play, GitBranch, CircleDollarSign, Bot, ChevronRight } from "lucide-react";
import type { Agent, Session, Task, TaskSessionRef, TaskStatus } from "../lib/api";
import { api } from "../lib/api";
import { STATUS_LABEL, STATUS_VAR, STATUS_LABEL as SL, TRANSITIONS } from "../lib/status";
import { Button } from "./ui/button";
import { Badge, Dot, Spinner } from "./ui/primitives";
import { cn, formatCents, timeAgo } from "../lib/utils";

export function TaskDetail({
  task,
  agents,
  tick,
  onClose,
  onChanged,
}: {
  task: Task | null;
  agents: Agent[];
  tick: number;
  onClose: () => void;
  onChanged: () => void;
}) {
  return (
    <AnimatePresence>
      {task && (
        <>
          <motion.div
            className="fixed inset-0 z-40 bg-black/30"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            onClick={onClose}
          />
          <motion.aside
            className="fixed inset-y-0 right-0 z-40 flex w-full max-w-xl flex-col border-l border-border bg-background shadow-pop"
            initial={{ x: "100%" }}
            animate={{ x: 0 }}
            exit={{ x: "100%" }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
          >
            <Inner task={task} agents={agents} tick={tick} onClose={onClose} onChanged={onChanged} />
          </motion.aside>
        </>
      )}
    </AnimatePresence>
  );
}

function Inner({
  task,
  agents,
  tick,
  onClose,
  onChanged,
}: {
  task: Task;
  agents: Agent[];
  tick: number;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [sessions, setSessions] = useState<TaskSessionRef[]>([]);
  const [session, setSession] = useState<Session | null>(null);
  const [diff, setDiff] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pickAgent, setPickAgent] = useState(false);

  // (Re)load sessions whenever the task or a live tick changes.
  useEffect(() => {
    let alive = true;
    api.listTaskSessions(task.id).then((s) => {
      if (!alive) return;
      setSessions(s);
      if (s[0]) api.getSession(s[0].id).then((full) => alive && setSession(full));
      else setSession(null);
    });
    return () => {
      alive = false;
    };
  }, [task.id, tick]);

  const activeAgents = agents.filter((a) => a.status === "active");

  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Action failed");
    } finally {
      setBusy(false);
    }
  };

  const start = (agentId: string) => {
    setPickAgent(false);
    act(() => api.startTask(task.id, agentId));
  };

  const transition = (to: TaskStatus) => act(() => api.transitionTask(task.id, to));

  const loadDiff = async () => {
    if (!session) return;
    setDiff(await api.getSessionDiff(session.id));
  };

  const moves = TRANSITIONS[task.status];

  return (
    <>
      <header className="flex items-start justify-between gap-4 border-b border-border px-6 py-4">
        <div className="min-w-0">
          <div className="mb-1.5 flex items-center gap-2">
            <Badge tone={STATUS_VAR[task.status]}>
              <Dot tone={STATUS_VAR[task.status]} />
              {STATUS_LABEL[task.status]}
            </Badge>
            <span className="text-xs text-muted-foreground">{task.priority} priority</span>
          </div>
          <h2 className="text-lg font-semibold leading-tight">{task.title}</h2>
        </div>
        <button
          onClick={onClose}
          className="rounded-md p-1.5 text-muted-foreground transition hover:bg-muted hover:text-foreground"
        >
          <X className="h-4 w-4" />
        </button>
      </header>

      <div className="flex flex-1 flex-col gap-5 overflow-y-auto px-6 py-5">
        {/* Actions */}
        <section className="flex flex-col gap-3">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Actions
          </h3>
          <div className="flex flex-wrap gap-2">
            {task.status === "todo" && (
              <Button variant="primary" size="sm" onClick={() => setPickAgent((v) => !v)} disabled={busy}>
                <Play className="h-4 w-4" />
                Start with agent
              </Button>
            )}
            {moves.map((to) => (
              <Button
                key={to}
                size="sm"
                variant={to === "done" ? "primary" : to === "cancelled" ? "outline" : "secondary"}
                onClick={() => transition(to)}
                disabled={busy}
              >
                Move to {SL[to]}
              </Button>
            ))}
            {moves.length === 0 && task.status !== "todo" && (
              <span className="text-sm text-muted-foreground">Terminal state — no moves.</span>
            )}
          </div>

          <AnimatePresence>
            {pickAgent && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                className="overflow-hidden"
              >
                <div className="flex flex-col gap-1.5 rounded-md border border-border bg-muted/40 p-2">
                  {activeAgents.length === 0 && (
                    <span className="px-2 py-1 text-sm text-muted-foreground">
                      No active agents — hire one first.
                    </span>
                  )}
                  {activeAgents.map((a) => (
                    <button
                      key={a.id}
                      onClick={() => start(a.id)}
                      className="flex items-center gap-2 rounded px-2 py-1.5 text-left text-sm transition hover:bg-card"
                    >
                      <Bot className="h-4 w-4 text-primary" />
                      <span className="font-medium">{a.name}</span>
                      <span className="text-xs text-muted-foreground">{a.archetype}</span>
                      <ChevronRight className="ml-auto h-4 w-4 text-muted-foreground" />
                    </button>
                  ))}
                </div>
              </motion.div>
            )}
          </AnimatePresence>
          {error && <p className="text-sm text-destructive">{error}</p>}
        </section>

        {/* Session */}
        {session && (
          <section className="flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Latest run
              </h3>
              <SessionStatus status={session.status} />
            </div>

            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
              <span className="inline-flex items-center gap-1.5 mono">
                <GitBranch className="h-3.5 w-3.5" />
                {session.branch}
              </span>
              <span className="inline-flex items-center gap-1.5 mono">
                <CircleDollarSign className="h-3.5 w-3.5" />
                {formatCents(session.cost_cents)}
              </span>
              <span>{timeAgo(session.finished_at ?? session.started_at ?? session.created_at)}</span>
            </div>

            {session.last_error && (
              <p className="rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {session.last_error}
              </p>
            )}

            {session.output && (
              <pre className="mono max-h-72 overflow-auto rounded-md border border-border bg-muted/50 p-3 text-xs leading-relaxed whitespace-pre-wrap">
                {session.output}
              </pre>
            )}

            <div>
              {diff === null ? (
                <Button size="sm" variant="outline" onClick={loadDiff}>
                  <GitBranch className="h-4 w-4" />
                  View diff
                </Button>
              ) : (
                <DiffView diff={diff} />
              )}
            </div>
          </section>
        )}

        {sessions.length > 1 && (
          <p className="text-xs text-muted-foreground">
            {sessions.length} runs on this task.
          </p>
        )}

        {busy && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner className="h-4 w-4" /> Working…
          </div>
        )}
      </div>
    </>
  );
}

function SessionStatus({ status }: { status: string }) {
  const tone =
    status === "completed"
      ? "var(--color-status-done)"
      : status === "failed"
        ? "var(--color-status-blocked)"
        : "var(--color-status-in_progress)";
  return (
    <Badge tone={tone}>
      <Dot tone={tone} className={status === "running" ? "animate-pulse" : ""} />
      {status}
    </Badge>
  );
}

/** Minimal syntax coloring for a unified diff. */
function DiffView({ diff }: { diff: string }) {
  if (!diff.trim()) {
    return <p className="text-sm text-muted-foreground">No changes in the worktree.</p>;
  }
  return (
    <pre className="mono max-h-96 overflow-auto rounded-md border border-border bg-muted/50 p-3 text-xs leading-relaxed">
      {diff.split("\n").map((line, i) => (
        <div
          key={i}
          className={cn(
            "whitespace-pre",
            line.startsWith("+") && !line.startsWith("+++") && "text-status-done",
            line.startsWith("-") && !line.startsWith("---") && "text-status-blocked",
            (line.startsWith("@@") || line.startsWith("diff ")) && "text-primary font-medium",
          )}
        >
          {line || " "}
        </div>
      ))}
    </pre>
  );
}
