import { useEffect, useState } from "react";
import { AlertTriangle, Bell, Check, CheckCheck, Gavel, Ban, Play, Users, X } from "lucide-react";
import type { Approval, Notification } from "../lib/api";
import { api } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { timeAgo } from "../lib/utils";
import { cn } from "../lib/utils";

/**
 * How the company reaches you (ADR-0020): one inbox for everything an agent
 * wants you to know or decide. Agents ask — to convene a meeting, to start a
 * gated task — and answer here, inline. The bell's badge is the count of what
 * you haven't seen.
 */
export function Inbox({
  companyId,
  tick,
  onDecided,
  openSignal,
  onOpenMeeting,
}: {
  companyId: string;
  tick: number;
  /** Called after a decision (with the approval id) or a read. */
  onDecided: (approvalId?: string) => void;
  /** Bumped when something elsewhere (a toast) asks for the inbox. */
  openSignal: number;
  onOpenMeeting: (meetingId: string) => void;
}) {
  const [items, setItems] = useState<Notification[]>([]);
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    Promise.all([api.listNotifications(companyId), api.listApprovals(companyId)])
      .then(([n, a]) => {
        if (!alive) return;
        setItems(n.notifications);
        setUnread(n.unread);
        setApprovals(a);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [companyId, tick]);

  useEffect(() => {
    if (openSignal > 0) setOpen(true);
  }, [openSignal]);

  const pendingApproval = (id: string | null) =>
    id ? approvals.find((a) => a.id === id && a.status === "pending") : undefined;

  const decide = async (n: Notification, decision: "approve" | "reject") => {
    if (!n.approval_id) return;
    setBusy(n.id);
    try {
      await api.decideApproval(n.approval_id, decision);
      await api.readNotification(n.id).catch(() => {});
      onDecided(n.approval_id);
    } finally {
      setBusy(null);
    }
  };

  const markAllRead = async () => {
    await api.readAllNotifications(companyId).catch(() => {});
    onDecided();
  };

  const waiting = items.filter((n) => pendingApproval(n.approval_id)).length;

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className="relative inline-flex h-9 w-9 items-center justify-center rounded-md text-muted-foreground transition hover:bg-muted hover:text-foreground"
        title={unread ? `${unread} unread` : "Inbox"}
      >
        <Bell className="h-4.5 w-4.5" />
        {unread > 0 && (
          <span
            className={cn(
              "absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-semibold",
              waiting > 0
                ? "bg-destructive text-destructive-foreground"
                : "bg-primary text-primary-foreground",
            )}
          >
            {unread}
          </span>
        )}
      </button>

      <Dialog
        open={open}
        onOpenChange={setOpen}
        title="Inbox"
        description={
          waiting
            ? `${waiting} waiting on your decision`
            : unread
              ? `${unread} unread`
              : "Nothing waiting on you."
        }
        className="max-w-xl"
      >
        <div className="flex flex-col gap-2">
          {items.length === 0 && (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Nothing yet. When an agent needs you — to convene a meeting, to start a gated task —
              it lands here.
            </p>
          )}

          {items.map((n) => {
            const approval = pendingApproval(n.approval_id);
            return (
              <div
                key={n.id}
                className={cn(
                  "rounded-md border p-3 transition",
                  approval
                    ? "border-primary/40 bg-primary/[0.04]"
                    : "border-border bg-card",
                  !n.read_at && !approval && "border-l-2 border-l-primary",
                )}
              >
                <div className="flex items-start gap-3">
                  <KindIcon kind={n.kind} />
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium">{n.title}</p>
                    <p className="mt-1 whitespace-pre-wrap text-xs leading-relaxed text-muted-foreground">
                      {n.body}
                    </p>
                    <p className="mt-1.5 text-[11px] text-muted-foreground/70">
                      {timeAgo(n.created_at)}
                    </p>
                  </div>
                </div>

                <div className="mt-2.5 flex items-center justify-end gap-1.5">
                  {n.subject_type === "meeting" && n.subject_id && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => {
                        setOpen(false);
                        onOpenMeeting(n.subject_id!);
                      }}
                    >
                      View meeting
                    </Button>
                  )}
                  {approval && (
                    <>
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={busy === n.id}
                        onClick={() => decide(n, "reject")}
                      >
                        <X className="h-4 w-4" />
                        Reject
                      </Button>
                      <Button
                        size="sm"
                        variant="primary"
                        disabled={busy === n.id}
                        onClick={() => decide(n, "approve")}
                      >
                        <Check className="h-4 w-4" />
                        Approve
                      </Button>
                    </>
                  )}
                </div>
              </div>
            );
          })}

          {unread > 0 && (
            <button
              onClick={markAllRead}
              className="mt-1 inline-flex items-center justify-center gap-1.5 self-end rounded-md px-2 py-1 text-xs text-muted-foreground transition hover:bg-muted hover:text-foreground"
            >
              <CheckCheck className="h-3.5 w-3.5" />
              Mark all read
            </button>
          )}
        </div>
      </Dialog>
    </>
  );
}

/** One glyph per notification kind, so the feed is scannable. */
function KindIcon({ kind }: { kind: string }) {
  const { Icon, tone } = iconFor(kind);
  return (
    <span
      className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md"
      style={{ color: tone, background: `color-mix(in oklch, ${tone} 12%, transparent)` }}
    >
      <Icon className="h-4 w-4" />
    </span>
  );
}

function iconFor(kind: string) {
  switch (kind) {
    case "meeting.requested":
      return { Icon: Users, tone: "var(--color-primary)" };
    case "meeting.decided":
      return { Icon: Gavel, tone: "var(--color-status-done)" };
    case "meeting.declined":
      return { Icon: Ban, tone: "var(--color-muted-foreground)" };
    case "meeting.failed":
      return { Icon: AlertTriangle, tone: "var(--color-destructive)" };
    case "approval.requested":
      return { Icon: Play, tone: "var(--color-primary)" };
    default:
      return { Icon: Bell, tone: "var(--color-muted-foreground)" };
  }
}
