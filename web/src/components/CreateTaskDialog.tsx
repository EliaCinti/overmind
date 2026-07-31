import { useState } from "react";
import type { ExecutionKind, TaskPriority } from "../lib/api";
import { api } from "../lib/api";
import { PRIORITY_LABEL } from "../lib/status";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Field, Input, Textarea } from "./ui/primitives";
import { Segmented } from "./ui/controls";

export function CreateTaskDialog({
  open,
  onOpenChange,
  companyId,
  goalId,
  onCreated,
  onConnectRepo,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  companyId: string;
  /** The goal code tasks attach to — null when no repo is connected. */
  goalId: string | null;
  onCreated: () => void;
  onConnectRepo: () => void;
}) {
  // Knowledge work needs no repo (ADR-0017), so that is the sane default when
  // there is none; a code task without a worktree could never run.
  const hasRepo = goalId !== null;
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [priority, setPriority] = useState<TaskPriority>("medium");
  const [executionKind, setExecutionKind] = useState<ExecutionKind>(hasRepo ? "code" : "knowledge");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const blocked = executionKind === "code" && !hasRepo;

  const submit = async () => {
    if (!title.trim() || blocked) return;
    setBusy(true);
    setError(null);
    try {
      await api.createTask(companyId, {
        title: title.trim(),
        description: description.trim(),
        goal_id: goalId ?? undefined,
        priority,
        execution_kind: executionKind,
      });
      onCreated();
      onOpenChange(false);
      setTitle("");
      setDescription("");
      setPriority("medium");
      setExecutionKind(hasRepo ? "code" : "knowledge");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create task");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title="New task"
      description="Describe the work. An agent can pick it up once it's in To do."
    >
      <div className="flex flex-col gap-4">
        <Field label="Title">
          <Input
            autoFocus
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="e.g. Add a health-check endpoint"
            onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && submit()}
          />
        </Field>
        <Field label="Description" hint="What the agent should do, and any constraints.">
          <Textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Return 200 with { status: ok } at GET /health…"
          />
        </Field>
        <Field label="Priority">
          <Segmented<TaskPriority>
            value={priority}
            onChange={setPriority}
            options={(["low", "medium", "high", "urgent"] as TaskPriority[]).map((p) => ({
              value: p,
              label: PRIORITY_LABEL[p],
            }))}
          />
        </Field>
        <Field
          label="Kind"
          hint="Code produces a diff in a git worktree; Knowledge produces documents."
        >
          <Segmented<ExecutionKind>
            value={executionKind}
            onChange={setExecutionKind}
            options={[
              { value: "code", label: "Code" },
              { value: "knowledge", label: "Knowledge" },
            ]}
          />
        </Field>
        {blocked && (
          <div className="flex items-center gap-3 rounded-md border border-border bg-muted/40 p-3">
            <p className="flex-1 text-sm text-muted-foreground">
              A code task needs a git repo to branch a worktree from. Knowledge tasks don't.
            </p>
            <Button size="sm" variant="outline" onClick={onConnectRepo}>
              Connect a repo
            </Button>
          </div>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={busy || !title.trim() || blocked}>
            {busy ? "Creating…" : "Create task"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
