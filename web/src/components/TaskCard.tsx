import { ArrowUp, Minus, ChevronsUp, Bot } from "lucide-react";
import type { Agent, Task, TaskPriority } from "../lib/api";
import { STATUS_VAR } from "../lib/status";
import { Card } from "./ui/primitives";
import { timeAgo } from "../lib/utils";

const PRIORITY_ICON: Record<TaskPriority, typeof Minus> = {
  low: Minus,
  medium: Minus,
  high: ArrowUp,
  urgent: ChevronsUp,
};
const PRIORITY_TONE: Record<TaskPriority, string> = {
  low: "var(--color-muted-foreground)",
  medium: "var(--color-muted-foreground)",
  high: "var(--color-status-in_review)",
  urgent: "var(--color-status-blocked)",
};

export function TaskCard({
  task,
  agents,
  onClick,
}: {
  task: Task;
  agents: Agent[];
  onClick: () => void;
}) {
  const PIcon = PRIORITY_ICON[task.priority];
  const assignee = agents.find((a) => a.id === task.assignee_agent_id);
  return (
    <Card
      onClick={onClick}
      className="group relative cursor-pointer overflow-hidden p-3 transition hover:border-primary/40 hover:shadow-soft"
    >
      <span
        className="absolute inset-y-0 left-0 w-1"
        style={{ background: STATUS_VAR[task.status] }}
      />
      <div className="flex flex-col gap-2 pl-1.5">
        <p className="text-sm font-medium leading-snug">{task.title}</p>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1" style={{ color: PRIORITY_TONE[task.priority] }}>
            <PIcon className="h-3.5 w-3.5" />
            {task.priority}
          </span>
          <span aria-hidden>·</span>
          <span>{timeAgo(task.updated_at)}</span>
          {assignee && (
            <span className="ml-auto inline-flex items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-primary">
              <Bot className="h-3 w-3" />
              {assignee.name}
            </span>
          )}
        </div>
      </div>
    </Card>
  );
}
