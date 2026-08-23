import type { TaskStatus } from "./api";

/**
 * The *structure* of task state: which columns exist, in what order, which
 * moves are legal, what colour each state wears.
 *
 * The **labels** used to live here too. They moved to the dictionary (M16):
 * a status renders as `t(`status.${task.status}`)`, which type-checks against
 * the dictionary — add a status server-side and the build fails until every
 * language names it.
 */
export const BOARD_COLUMNS: TaskStatus[] = [
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "blocked",
  "done",
];

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
  in_progress: ["todo", "in_review", "blocked", "cancelled"],
  in_review: ["todo", "in_progress", "done", "cancelled"],
  blocked: ["todo", "in_progress", "cancelled"],
  done: [],
  cancelled: [],
};
