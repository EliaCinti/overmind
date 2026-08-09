import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { X, Play, GitBranch, CircleDollarSign, Bot, ChevronRight, Paperclip } from "lucide-react";
import type {
  Agent,
  Artifact,
  Attachment,
  Session,
  Task,
  TaskSessionRef,
  TaskStatus,
} from "../lib/api";
import { api } from "../lib/api";
import { STATUS_VAR, TRANSITIONS } from "../lib/status";
import { Button } from "./ui/button";
import { Badge, Dot, Spinner } from "./ui/primitives";
import { Deliverables } from "./Deliverables";
import { formatBytes, iconFor } from "../lib/files";
import { useFormats, useT } from "../lib/i18n";
import { cn } from "../lib/utils";

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
            <Inner
              task={task}
              agents={agents}
              tick={tick}
              onClose={onClose}
              onChanged={onChanged}
            />
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
  const t = useT();
  const { formatCents, timeAgo } = useFormats();
  const [sessions, setSessions] = useState<TaskSessionRef[]>([]);
  const [session, setSession] = useState<Session | null>(null);
  const [diff, setDiff] = useState<string | null>(null);
  const [artifacts, setArtifacts] = useState<Artifact[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pickAgent, setPickAgent] = useState(false);

  // A knowledge task delivers documents instead of a diff (ADR-0017) — but
  // since M17 a code run can also hand back files, so artifacts are loaded
  // either way and the diff button is what depends on the kind.
  const isKnowledge = task.execution_kind === "knowledge";

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

  // Load whatever the latest run delivered.
  useEffect(() => {
    if (!session) {
      setArtifacts(null);
      return;
    }
    let alive = true;
    api.listTaskArtifacts(task.id).then((a) => alive && setArtifacts(a));
    return () => {
      alive = false;
    };
  }, [task.id, session?.id, session?.status]);

  const activeAgents = agents.filter((a) => a.status === "active");

  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : t("task.actionFailed"));
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
              {t(`status.${task.status}`)}
            </Badge>
            <span className="text-xs text-muted-foreground">
              {t("task.priorityLabel", { p: t(`priority.${task.priority}`) })}
            </span>
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
        <TaskInputs taskId={task.id} tick={tick} />

        {/* Actions */}
        <section className="flex flex-col gap-3">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {t("task.actions")}
          </h3>
          <div className="flex flex-wrap gap-2">
            {task.status === "todo" && (
              <Button
                variant="primary"
                size="sm"
                onClick={() => setPickAgent((v) => !v)}
                disabled={busy}
              >
                <Play className="h-4 w-4" />
                {t("task.startWithAgent")}
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
                {t("task.moveTo", { status: t(`status.${to}`) })}
              </Button>
            ))}
            {moves.length === 0 && task.status !== "todo" && (
              <span className="text-sm text-muted-foreground">{t("task.terminal")}</span>
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
                      {t("task.noActiveAgents")}
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
                {t("task.latestRun")}
              </h3>
              <SessionStatus status={session.status} />
            </div>

            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
              {session.branch && (
                <span className="inline-flex items-center gap-1.5 mono">
                  <GitBranch className="h-3.5 w-3.5" />
                  {session.branch}
                </span>
              )}
              <span className="inline-flex items-center gap-1.5 mono">
                <CircleDollarSign className="h-3.5 w-3.5" />
                {formatCents(session.cost_cents)}
              </span>
              <span>
                {timeAgo(session.finished_at ?? session.started_at ?? session.created_at)}
              </span>
            </div>

            {session.last_error && (
              <p className="rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {session.last_error}
              </p>
            )}

            {/* What the agent said comes first and in prose; the adapter's
                envelope is diagnostic, so it folds away. Showing the envelope
                as the report is how a wall of `ttft_ms` ended up being the
                thing a person read after a run. */}
            {session.said ? (
              <>
                <p className="text-sm leading-relaxed whitespace-pre-wrap">{session.said}</p>
                <details className="text-xs">
                  <summary className="cursor-pointer text-muted-foreground hover:text-foreground">
                    {t("task.rawOutput")}
                  </summary>
                  <pre className="mono mt-2 max-h-72 overflow-auto rounded-md border border-border bg-muted/50 p-3 text-xs leading-relaxed whitespace-pre-wrap">
                    {session.output}
                  </pre>
                </details>
              </>
            ) : (
              session.output && (
                <pre className="mono max-h-72 overflow-auto rounded-md border border-border bg-muted/50 p-3 text-xs leading-relaxed whitespace-pre-wrap">
                  {session.output}
                </pre>
              )
            )}

            {/* A code run's primary deliverable is its diff; a knowledge
                run has none. Either can also hand back files (M17). */}
            {!isKnowledge && (
              <div>
                {diff === null ? (
                  <Button size="sm" variant="outline" onClick={loadDiff}>
                    <GitBranch className="h-4 w-4" />
                    {t("task.viewDiff")}
                  </Button>
                ) : (
                  <DiffView diff={diff} />
                )}
              </div>
            )}
            {(isKnowledge || (artifacts !== null && artifacts.length > 0)) && (
              <div className="flex flex-col gap-2">
                <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  {t("task.deliverables")}
                </h3>
                <Deliverables artifacts={artifacts} />
              </div>
            )}
          </section>
        )}

        {sessions.length > 1 && (
          <p className="text-xs text-muted-foreground">{t("task.runs", { n: sessions.length })}</p>
        )}

        {busy && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner className="h-4 w-4" /> {t("common.working")}
          </div>
        )}
      </div>
    </>
  );
}

function SessionStatus({ status }: { status: string }) {
  const t = useT();
  const tone =
    status === "completed"
      ? "var(--color-status-done)"
      : status === "failed"
        ? "var(--color-status-blocked)"
        : "var(--color-status-in_progress)";
  return (
    <Badge tone={tone}>
      <Dot tone={tone} className={status === "running" ? "animate-pulse" : ""} />
      {status === "running" || status === "completed" || status === "failed"
        ? t(`sessionStatus.${status}`)
        : status}
    </Badge>
  );
}

/** Minimal syntax coloring for a unified diff. */
function DiffView({ diff }: { diff: string }) {
  const t = useT();
  if (!diff.trim()) {
    return <p className="text-sm text-muted-foreground">{t("task.noChanges")}</p>;
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

/**
 * Files handed to the task (M17).
 *
 * The point of a task attachment is that it survives the conversation: an
 * agent that picks this task up in an hour still gets the spreadsheet. So it
 * lives on the task, above the actions — it is part of the brief, not an
 * afterthought — and the panel stays visible when empty, because "you can
 * attach things here" is the thing most people would not otherwise guess.
 */
function TaskInputs({ taskId, tick }: { taskId: string; tick: number }) {
  const t = useT();
  const { locale } = useFormats();
  const [files, setFiles] = useState<Attachment[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let alive = true;
    api
      .listTaskAttachments(taskId)
      .then((a) => alive && setFiles(a))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [taskId, tick]);

  const add = async (picked: FileList | null) => {
    if (!picked || picked.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      for (const file of Array.from(picked)) {
        await api.uploadTaskAttachment(taskId, file);
      }
      setFiles(await api.listTaskAttachments(taskId));
    } catch (e) {
      setError(e instanceof Error ? e.message : t("task.attachFailed"));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: string) => {
    await api.removeTaskAttachment(taskId, id).catch(() => {});
    setFiles((current) => current.filter((f) => f.id !== id));
  };

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {t("task.inputs")}
        </h3>
        <input
          ref={inputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => {
            void add(e.target.files);
            e.target.value = "";
          }}
        />
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => inputRef.current?.click()}
        >
          <Paperclip className="h-4 w-4" />
          {busy ? t("task.attaching") : t("task.attach")}
        </Button>
      </div>
      {files.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("task.inputsHint")}</p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {files.map((f) => {
            const Icon = iconFor(f.mime);
            return (
              <div
                key={f.id}
                className="group flex items-center gap-2 rounded-md border border-border bg-muted/40 px-3 py-1.5"
              >
                <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-sm">{f.filename}</span>
                <span className="mono shrink-0 text-[11px] text-muted-foreground">
                  {formatBytes(f.size_bytes, locale)}
                </span>
                <button
                  type="button"
                  onClick={() => void remove(f.id)}
                  aria-label={t("task.detach", { name: f.filename })}
                  className="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition hover:bg-muted hover:text-foreground group-hover:opacity-100"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            );
          })}
        </div>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </section>
  );
}
