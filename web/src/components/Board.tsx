import { motion } from "motion/react";
import { FolderGit2 } from "lucide-react";
import type { Agent, Task } from "../lib/api";
import { BOARD_COLUMNS, STATUS_VAR } from "../lib/status";
import { TaskCard } from "./TaskCard";
import { Button } from "./ui/button";
import { useT } from "../lib/i18n";

export function Board({
  tasks,
  agents,
  onOpenTask,
  hasRepo,
  onConnectRepo,
}: {
  tasks: Task[];
  agents: Agent[];
  onOpenTask: (t: Task) => void;
  hasRepo: boolean;
  onConnectRepo: () => void;
}) {
  const t = useT();
  const byStatus = (s: string) => tasks.filter((task) => task.status === s);

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* No repo is a normal state, not an error: say what still works. */}
      {!hasRepo && (
        <div className="mx-6 mb-3 flex items-center gap-3 rounded-lg border border-border bg-muted/40 px-4 py-2.5">
          <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
          <p className="flex-1 text-sm text-muted-foreground">{t("board.noRepo")}</p>
          <Button size="sm" variant="outline" onClick={onConnectRepo}>
            {t("common.connectRepo")}
          </Button>
        </div>
      )}
      <div className="flex flex-1 gap-4 overflow-x-auto px-6 pb-6">
        {BOARD_COLUMNS.map((col) => {
          const items = byStatus(col);
          const tone = STATUS_VAR[col];
          return (
            <div key={col} className="flex w-80 shrink-0 flex-col">
              <div className="mb-3 flex items-center gap-2 px-1">
                <span className="h-2.5 w-2.5 rounded-full" style={{ background: tone }} />
                <h2 className="text-sm font-semibold">{t(`status.${col}`)}</h2>
                <span className="mono text-xs text-muted-foreground">{items.length}</span>
              </div>
              <div className="flex flex-1 flex-col gap-2.5 rounded-lg bg-muted/40 p-2.5">
                {items.map((item) => (
                  <motion.div
                    key={item.id}
                    layout
                    initial={{ opacity: 0, y: 6 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.15 }}
                  >
                    <TaskCard task={item} agents={agents} onClick={() => onOpenTask(item)} />
                  </motion.div>
                ))}
                {items.length === 0 && (
                  <div className="flex h-16 items-center justify-center rounded-md border border-dashed border-border/60 text-xs text-muted-foreground/60">
                    {t("board.emptyColumn")}
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
