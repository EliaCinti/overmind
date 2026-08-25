import { useEffect, useState } from "react";
import { AlertTriangle, Bell, Check, CheckCheck, Gavel, Ban, Play, Users, X } from "lucide-react";
import type { Approval, Notification } from "../lib/api";
import { api } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { useFormats, useNotificationText, useT } from "../lib/i18n";
import { Markdown } from "./Markdown";
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
  onOpenOrg,
}: {
  companyId: string;
  tick: number;
  /** Called after a decision (with the approval id) or a read. */
  onDecided: (approvalId?: string) => void;
  /** Bumped when something elsewhere (a toast) asks for the inbox. */
  openSignal: number;
  onOpenMeeting: (meetingId: string) => void;
  /** Jump to the org view — where a proposed team is actually read. */
  onOpenOrg: () => void;
}) {
  const t = useT();
  const { timeAgo } = useFormats();
  const notificationText = useNotificationText();
  const [items, setItems] = useState<Notification[]>([]);
  const [approvals, setApprovals] = useState<Approval[]>([]);
  // What waits on you stands alone (measured: a new start approval landed on
  // top of the already-decided team proposal and the two read as one pile).
  // Everything decided or merely informative is still the record (ADR-0020)
  // — one toggle away, and shown outright when nothing is waiting.
  const [showHistory, setShowHistory] = useState(false);
  /** Long bodies fold at ~6 lines; ids the person opened stay open. */
  const [expanded, setExpanded] = useState<Record<string, true>>({});
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

  // Looking is reading (measured 25 Aug 2026: the badge kept counting
  // informational items — a dropped meeting, a budget notice — that the
  // person had already seen). Opening the inbox marks everything read
  // EXCEPT the asks still waiting on a decision: those stay unread until
  // decided, because the badge's other job is "something waits on you".
  useEffect(() => {
    if (!open || items.length === 0) return;
    const seen = items.filter((n) => !n.read_at && !pendingApproval(n.approval_id));
    if (seen.length === 0) return;
    Promise.all(seen.map((n) => api.readNotification(n.id).catch(() => {}))).then(() => {
      setUnread((u) => Math.max(0, u - seen.length));
      setItems((cur) =>
        cur.map((n) =>
          seen.some((x) => x.id === n.id) ? { ...n, read_at: new Date().toISOString() } : n,
        ),
      );
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, items.length, approvals.length]);

  const pendingApproval = (id: string | null) =>
    id ? approvals.find((a) => a.id === id && a.status === "pending") : undefined;
  // Who beside what (M25): once decided, the item says by whom -- a name read
  // off the audit chain, where the actor has lived since M24.
  const decidedApproval = (id: string | null) =>
    id ? approvals.find((a) => a.id === id && a.status !== "pending") : undefined;

  const decide = async (n: Notification, decision: "approve" | "reject") => {
    if (!n.approval_id) return;
    setBusy(n.id);
    try {
      await api.decideApproval(n.approval_id, decision);
      await api.readNotification(n.id).catch(() => {});
      onDecided(n.approval_id);
      // Deciding the last thing that waited closes the inbox: what remains
      // is history, and history is not why the dialog was opened (measured:
      // after an approval the decided pile popped up, half clipped).
      const stillWaiting = items.some(
        (x) =>
          x.id !== n.id &&
          x.approval_id &&
          x.approval_id !== n.approval_id &&
          approvals.some((a) => a.id === x.approval_id && a.status === "pending"),
      );
      if (!stillWaiting) setOpen(false);
    } finally {
      setBusy(null);
    }
  };

  const markAllRead = async () => {
    await api.readAllNotifications(companyId).catch(() => {});
    onDecided();
  };

  const renderItem = (n: Notification) => {
    const approval = pendingApproval(n.approval_id);
    const text = notificationText(n);
    return (
              <div
                key={n.id}
                className={cn(
                  "rounded-md border p-3 transition",
                  approval ? "border-primary/40 bg-primary/[0.04]" : "border-border bg-card",
                  !n.read_at && !approval && "border-l-2 border-l-primary",
                )}
              >
                <div className="flex items-start gap-3">
                  <KindIcon kind={n.kind} />
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium">{text.title}</p>
                    {(() => {
                      // Rendered as the Markdown it is written in (a team
                      // proposal arrives as a list), folded past ~6 lines so
                      // a long request never hides the buttons — "Show all"
                      // opens it in place (measured: a 1,000-character
                      // proposal was unreadable in the card).
                      const long = text.body.length > 360 || text.body.split("\n").length > 6;
                      const open = !!expanded[n.id] || !long;
                      return (
                        <div className="mt-1 text-xs leading-relaxed text-muted-foreground">
                          <div className={open ? "" : "line-clamp-6"}>
                            <Markdown text={text.body} />
                          </div>
                          {long && (
                            <button
                              type="button"
                              className="mt-1 text-[11px] text-primary underline underline-offset-2 hover:opacity-80"
                              onClick={() =>
                                setExpanded((e) => {
                                  const next = { ...e };
                                  if (next[n.id]) delete next[n.id];
                                  else next[n.id] = true;
                                  return next;
                                })
                              }
                            >
                              {open ? t("inbox.showLess") : t("inbox.showAll")}
                            </button>
                          )}
                        </div>
                      );
                    })()}
                    <p className="mt-1.5 text-[11px] text-muted-foreground/70">
                      {timeAgo(n.created_at)}
                      {(() => {
                        const d = decidedApproval(n.approval_id);
                        return d?.decided_by
                          ? ` · ${t(d.status === "approved" ? "inbox.approvedBy" : "inbox.rejectedBy", { name: d.decided_by })}`
                          : null;
                      })()}
                    </p>
                  </div>
                </div>

                <div className="mt-2.5 flex items-center justify-end gap-1.5">
                  {/* The jump is only offered while there is somewhere to
                      jump: a decided proposal no longer renders in the Org
                      view, so its button led to a page with nothing on it
                      and looked broken (measured). */}
                  {n.subject_type === "org_proposal" && pendingApproval(n.approval_id) && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => {
                        setOpen(false);
                        onOpenOrg();
                      }}
                    >
                      {t("inbox.viewProposal")}
                    </Button>
                  )}
                  {n.subject_type === "meeting" && n.subject_id && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => {
                        setOpen(false);
                        onOpenMeeting(n.subject_id!);
                      }}
                    >
                      {t("common.viewMeeting")}
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
                        {t("common.reject")}
                      </Button>
                      <Button
                        size="sm"
                        variant="primary"
                        disabled={busy === n.id}
                        onClick={() => decide(n, "approve")}
                      >
                        <Check className="h-4 w-4" />
                        {t("common.approve")}
                      </Button>
                    </>
                  )}
                </div>
              </div>
    );
  };

  const waiting = items.filter((n) => pendingApproval(n.approval_id)).length;

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className="relative inline-flex h-9 w-9 items-center justify-center rounded-full text-muted-foreground transition hover:bg-muted hover:text-foreground"
        title={unread ? t("nav.unread", { n: unread }) : t("nav.inbox")}
        aria-label={t("nav.inbox")}
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
        title={t("nav.inbox")}
        description={
          waiting
            ? t("nav.waitingOnYou", { n: waiting })
            : unread
              ? t("nav.unread", { n: unread })
              : t("nav.nothingWaiting")
        }
        className="max-w-xl"
      >
        <div className="flex max-h-[65vh] flex-col gap-2 overflow-y-auto pr-1">
          {items.length === 0 && (
            <p className="py-8 text-center text-sm text-muted-foreground">{t("inbox.empty")}</p>
          )}

          {(() => {
            const waitingItems = items.filter((n) => pendingApproval(n.approval_id));
            const earlier = items.filter((n) => !pendingApproval(n.approval_id));
            const historyOpen = waitingItems.length === 0 || showHistory;
            return (
              <>
                {waitingItems.length > 0 && earlier.length > 0 && (
                  <p className="px-0.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("inbox.waitingSection")}
                  </p>
                )}
                {waitingItems.map(renderItem)}
                {waitingItems.length > 0 && earlier.length > 0 && (
                  <button
                    type="button"
                    onClick={() => setShowHistory((v) => !v)}
                    className="mt-1 self-start rounded-md px-1 py-1 text-xs text-muted-foreground transition hover:bg-muted hover:text-foreground"
                  >
                    {historyOpen
                      ? t("inbox.hideHistory")
                      : t("inbox.showHistory", { n: earlier.length })}
                  </button>
                )}
                {historyOpen && earlier.map(renderItem)}
              </>
            );
          })()}

          {unread > 0 && (
            <button
              onClick={markAllRead}
              className="mt-1 inline-flex items-center justify-center gap-1.5 self-end rounded-md px-2 py-1 text-xs text-muted-foreground transition hover:bg-muted hover:text-foreground"
            >
              <CheckCheck className="h-3.5 w-3.5" />
              {t("common.markAllRead")}
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
