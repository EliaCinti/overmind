import { motion } from "motion/react";
import type { Agent, Task } from "../lib/api";
import { BOARD_COLUMNS, STATUS_LABEL, STATUS_VAR } from "../lib/status";
import { TaskCard } from "./TaskCard";

export function Board({
  tasks,
  agents,
  onOpenTask,
}: {
  tasks: Task[];
  agents: Agent[];
  onOpenTask: (t: Task) => void;
}) {
  const byStatus = (s: string) => tasks.filter((t) => t.status === s);

  return (
    <div className="flex h-full gap-4 overflow-x-auto px-6 pb-6">
      {BOARD_COLUMNS.map((col) => {
        const items = byStatus(col);
        const tone = STATUS_VAR[col];
        return (
          <div key={col} className="flex w-80 shrink-0 flex-col">
            <div className="mb-3 flex items-center gap-2 px-1">
              <span className="h-2.5 w-2.5 rounded-full" style={{ background: tone }} />
              <h2 className="text-sm font-semibold">{STATUS_LABEL[col]}</h2>
              <span className="mono text-xs text-muted-foreground">{items.length}</span>
            </div>
            <div className="flex flex-1 flex-col gap-2.5 rounded-lg bg-muted/40 p-2.5">
              {items.map((t) => (
                <motion.div
                  key={t.id}
                  layout
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.15 }}
                >
                  <TaskCard task={t} agents={agents} onClick={() => onOpenTask(t)} />
                </motion.div>
              ))}
              {items.length === 0 && (
                <div className="flex h-16 items-center justify-center rounded-md border border-dashed border-border/60 text-xs text-muted-foreground/60">
                  Nothing here
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
