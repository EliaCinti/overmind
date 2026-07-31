import { useEffect, useState } from "react";
import { motion } from "motion/react";
import { AlertTriangle, Ban, Check, Gavel, Loader2, Users, X } from "lucide-react";
import type { Meeting, MeetingDetail, MeetingStatus } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Badge } from "./ui/primitives";
import { cn, timeAgo } from "../lib/utils";

/**
 * Meetings (ADR-0020): agents ask for a room when a call needs more than one
 * of them. You allow it here, then watch the transcript reach a decision.
 */
export function Meetings({
  companyId,
  tick,
  onChanged,
  selectedId,
  onSelect,
}: {
  companyId: string;
  tick: number;
  /** Called after a decision, with the approval id that was resolved. */
  onChanged: (approvalId?: string) => void;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}) {
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    api
      .listMeetings(companyId)
      .then((m) => {
        if (!alive) return;
        setMeetings(m);
        // Land on something: whatever is waiting on you, else the newest.
        if (!selectedId && m.length) {
          onSelect((m.find((x) => x.status === "requested") ?? m[0]).id);
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [companyId, tick]);

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      return;
    }
    let alive = true;
    api
      .getMeeting(selectedId)
      .then((d) => alive && setDetail(d))
      .catch(() => alive && setDetail(null));
    return () => {
      alive = false;
    };
  }, [selectedId, tick]);

  const decide = async (approvalId: string, decision: "approve" | "reject") => {
    setBusy(true);
    try {
      await api.decideApproval(approvalId, decision);
      onChanged(approvalId);
    } finally {
      setBusy(false);
    }
  };

  if (meetings.length === 0) {
    return (
      <div className="flex h-full items-center justify-center px-6 pb-6">
        <div className="max-w-sm text-center">
          <Users className="mx-auto h-8 w-8 text-muted-foreground/40" />
          <p className="mt-3 text-sm font-medium">No meetings yet</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Agents ask for one when they hit a call none of them should make alone. You approve it
            first — nothing runs before that.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full gap-4 overflow-hidden px-6 pb-6">
      {/* the list */}
      <div className="flex w-72 shrink-0 flex-col gap-2 overflow-y-auto pr-1">
        {meetings.map((m) => (
          <button
            key={m.id}
            onClick={() => onSelect(m.id)}
            className={cn(
              "rounded-lg border p-3 text-left transition",
              m.id === selectedId
                ? "border-primary/50 bg-primary/[0.05]"
                : "border-border bg-card hover:bg-muted/50",
            )}
          >
            <div className="flex items-start justify-between gap-2">
              <p className="min-w-0 flex-1 truncate text-sm font-medium">{m.topic}</p>
              <StatusPill status={m.status} />
            </div>
            <p className="mt-1 truncate text-xs text-muted-foreground">
              {m.convener_name ? `${m.convener_name} asked` : "You convened"} ·{" "}
              {timeAgo(m.created_at)}
            </p>
          </button>
        ))}
      </div>

      {/* the room */}
      <div className="flex min-w-0 flex-1 flex-col overflow-y-auto rounded-lg border border-border bg-card">
        {!detail ? (
          <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
            Select a meeting
          </div>
        ) : (
          <>
            <div className="border-b border-border px-5 py-4">
              <div className="flex items-start justify-between gap-3">
                <h2 className="text-base font-semibold">{detail.meeting.topic}</h2>
                <StatusPill status={detail.meeting.status} />
              </div>
              {detail.meeting.reason && (
                <p className="mt-2 whitespace-pre-wrap text-sm text-muted-foreground">
                  <span className="font-medium text-foreground">Why: </span>
                  {detail.meeting.reason}
                </p>
              )}
              <div className="mt-3 flex flex-wrap items-center gap-1.5">
                {detail.participants.map((p) => (
                  <Badge key={p.id} tone="var(--color-primary)">
                    {p.name}
                    {p.title && <span className="opacity-60">· {p.title}</span>}
                  </Badge>
                ))}
                <span className="mono ml-1 text-xs text-muted-foreground">
                  cap {detail.meeting.turn_cap}
                </span>
              </div>

              {detail.meeting.status === "requested" && detail.meeting.approval_id && (
                <div className="mt-4 flex items-center gap-2 rounded-md border border-primary/40 bg-primary/[0.04] p-3">
                  <p className="flex-1 text-sm">
                    {detail.meeting.convener_name ?? "An agent"} is waiting on you. Nothing has run
                    yet.
                  </p>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy}
                    onClick={() => decide(detail.meeting.approval_id!, "reject")}
                  >
                    <X className="h-4 w-4" />
                    Reject
                  </Button>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={busy}
                    onClick={() => decide(detail.meeting.approval_id!, "approve")}
                  >
                    <Check className="h-4 w-4" />
                    Approve
                  </Button>
                </div>
              )}
            </div>

            {/* transcript */}
            <div className="flex flex-1 flex-col gap-3 px-5 py-4">
              {detail.turns.length === 0 && detail.meeting.status !== "requested" && (
                <p className="text-sm text-muted-foreground">No turns yet.</p>
              )}
              {detail.turns.map((t) => (
                <motion.div
                  key={t.id}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.18 }}
                  className="flex gap-3"
                >
                  <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-primary/10 text-[10px] font-semibold text-primary">
                    {initials(t.agent_name)}
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="text-xs font-medium text-muted-foreground">{t.agent_name}</p>
                    <p className="whitespace-pre-wrap text-sm leading-relaxed">{t.content}</p>
                  </div>
                </motion.div>
              ))}

              {detail.meeting.status === "open" && (
                <div className="flex items-center gap-2 pl-10 text-xs text-muted-foreground">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  deliberating…
                </div>
              )}
            </div>

            {detail.meeting.decision && (
              <div className="border-t border-border bg-muted/30 px-5 py-4">
                <div className="flex items-center gap-2">
                  <Gavel className="h-4 w-4 text-status-done" />
                  <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Decision
                  </p>
                </div>
                <p className="mt-1.5 whitespace-pre-wrap text-sm">{detail.meeting.decision}</p>
                <p className="mt-2 text-xs text-muted-foreground">
                  Everyone in the room carries this into their work · {""}
                  {timeAgo(detail.meeting.decided_at)}
                </p>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function StatusPill({ status }: { status: MeetingStatus }) {
  const map: Record<MeetingStatus, { label: string; tone: string; Icon?: typeof Users }> = {
    requested: { label: "Waiting on you", tone: "var(--color-status-todo)", Icon: Users },
    open: { label: "In session", tone: "var(--color-status-in_progress)", Icon: Loader2 },
    decided: { label: "Decided", tone: "var(--color-status-done)", Icon: Gavel },
    declined: { label: "Declined", tone: "var(--color-muted-foreground)", Icon: Ban },
    failed: { label: "Failed", tone: "var(--color-destructive)", Icon: AlertTriangle },
  };
  const { label, tone, Icon } = map[status];
  return (
    <Badge tone={tone} className="shrink-0">
      {Icon && <Icon className={cn("h-3 w-3", status === "open" && "animate-spin")} />}
      {label}
    </Badge>
  );
}

function initials(name: string) {
  return name
    .split(/\s+/)
    .slice(0, 2)
    .map((p) => p[0]?.toUpperCase() ?? "")
    .join("");
}
