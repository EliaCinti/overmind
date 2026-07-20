import { useEffect, useState } from "react";
import { Bell, Check, X } from "lucide-react";
import type { Approval } from "../lib/api";
import { api } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { timeAgo } from "../lib/utils";

/** Bell button with a pending-count badge; opens the approvals inbox. */
export function ApprovalsInbox({
  companyId,
  tick,
  onDecided,
}: {
  companyId: string;
  tick: number;
  onDecided: () => void;
}) {
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .listApprovals(companyId)
      .then((a) => alive && setApprovals(a))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [companyId, tick]);

  const pending = approvals.filter((a) => a.status === "pending");

  const decide = async (id: string, decision: "approve" | "reject") => {
    setBusy(id);
    try {
      await api.decideApproval(id, decision);
      onDecided();
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className="relative inline-flex h-9 w-9 items-center justify-center rounded-md text-muted-foreground transition hover:bg-muted hover:text-foreground"
        title="Approvals"
      >
        <Bell className="h-4.5 w-4.5" />
        {pending.length > 0 && (
          <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-[10px] font-semibold text-destructive-foreground">
            {pending.length}
          </span>
        )}
      </button>

      <Dialog
        open={open}
        onOpenChange={setOpen}
        title="Approvals"
        description={
          pending.length ? `${pending.length} waiting on you` : "Nothing waiting on you."
        }
      >
        <div className="flex flex-col gap-2">
          {approvals.length === 0 && (
            <p className="py-6 text-center text-sm text-muted-foreground">No approval requests.</p>
          )}
          {approvals.map((a) => (
            <div
              key={a.id}
              className="flex items-center gap-3 rounded-md border border-border bg-card p-3"
            >
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{a.summary || a.type}</p>
                <p className="text-xs text-muted-foreground">
                  {a.status === "pending" ? `requested ${timeAgo(a.created_at)}` : a.status}
                </p>
              </div>
              {a.status === "pending" ? (
                <div className="flex gap-1.5">
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy === a.id}
                    onClick={() => decide(a.id, "reject")}
                  >
                    <X className="h-4 w-4" />
                    Reject
                  </Button>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={busy === a.id}
                    onClick={() => decide(a.id, "approve")}
                  >
                    <Check className="h-4 w-4" />
                    Approve
                  </Button>
                </div>
              ) : (
                <span
                  className="text-xs font-medium"
                  style={{
                    color:
                      a.status === "approved"
                        ? "var(--color-status-done)"
                        : "var(--color-status-cancelled)",
                  }}
                >
                  {a.status}
                </span>
              )}
            </div>
          ))}
        </div>
      </Dialog>
    </>
  );
}
