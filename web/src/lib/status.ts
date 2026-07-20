import type { TaskStatus, TaskPriority } from "./api";

/** Board columns, in order. Terminal states share the "Done" column tail. */
export const BOARD_COLUMNS: TaskStatus[] = [
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "blocked",
  "done",
];

export const STATUS_LABEL: Record<TaskStatus, string> = {
  backlog: "Backlog",
  todo: "To do",
  in_progress: "In progress",
  in_review: "In review",
  blocked: "Blocked",
  done: "Done",
  cancelled: "Cancelled",
};

/** CSS var name for each status hue (defined in index.css status tier). */
export const STATUS_VAR: Record<TaskStatus, string> = {
  backlog: "var(--color-status-backlog)",
  todo: "var(--color-status-todo)",
  in_progress: "var(--color-status-in_progress)",
  in_review: "var(--color-status-in_review)",
  blocked: "var(--color-status-blocked)",
  done: "var(--color-status-done)",
  cancelled: "var(--color-status-cancelled)",
};

/**
 * The transition table, mirrored from the server (domain.rs). The UI only
 * offers valid moves; the server still enforces them, so this is convenience,
 * not trust.
 */
export const TRANSITIONS: Record<TaskStatus, TaskStatus[]> = {
  backlog: ["todo", "cancelled"],
  todo: ["in_progress", "blocked", "cancelled"],
  in_progress: ["in_review", "blocked", "cancelled"],
  in_review: ["in_progress", "done", "cancelled"],
  blocked: ["todo", "in_progress", "cancelled"],
  done: [],
  cancelled: [],
};

export const PRIORITY_LABEL: Record<TaskPriority, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  urgent: "Urgent",
};

export const AUTONOMY_LABEL: Record<string, string> = {
  propose_only: "Propose only",
  act_with_approval: "Act with approval",
  act_within_budget: "Act within budget",
};

export const STRICTNESS_LABEL: Record<string, string> = {
  lenient: "Lenient",
  standard: "Standard",
  strict: "Strict",
};

/** Human sentence describing what an agent may do, for the live hire preview. */
export function autonomySentence(autonomy: string): string {
  switch (autonomy) {
    case "propose_only":
      return "proposes changes but never acts without you";
    case "act_with_approval":
      return "acts on tasks once you approve each start";
    case "act_within_budget":
      return "picks up and runs tasks on its own, within budget";
    default:
      return autonomy;
  }
}
