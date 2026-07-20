import { useState } from "react";
import type { TaskPriority } from "../lib/api";
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
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  companyId: string;
  goalId: string | null;
  onCreated: () => void;
}) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [priority, setPriority] = useState<TaskPriority>("medium");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!title.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await api.createTask(companyId, {
        title: title.trim(),
        description: description.trim(),
        goal_id: goalId ?? undefined,
        priority,
      });
      onCreated();
      onOpenChange(false);
      setTitle("");
      setDescription("");
      setPriority("medium");
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
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={busy || !title.trim()}>
            {busy ? "Creating…" : "Create task"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
