import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { motion } from "motion/react";
import { SendHorizontal, Sparkles } from "lucide-react";
import type { Agent, Conversation, Message } from "../lib/api";
import { ApiError, api } from "../lib/api";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";

/**
 * The conversational surface (M12 / ADR-0018). The user talks to a CEO agent;
 * posting a message runs the CEO's turn server-side, which replies and opens
 * tasks. Replies and system notices arrive asynchronously over the live
 * channel, so this view refetches whenever the parent's `tick` changes.
 */
export function Chat({
  companyId,
  agents,
  tick,
  onChanged,
}: {
  companyId: string;
  agents: Agent[];
  tick: number;
  /** Bump the app-wide live tick (so the board reflects new CEO tasks at once). */
  onChanged: () => void;
}) {
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [pending, setPending] = useState(false); // CEO turn in flight

  // The CEO is the leader of the organization: the agent that reports to the
  // human (the top of the org chart). It's a property of the org — designated
  // in the Org view, not re-picked per conversation. Once a thread exists, its
  // CEO is whatever it was opened with.
  const candidates = useMemo(() => agents.filter((a) => a.status === "active"), [agents]);
  const leader = useMemo(
    () => candidates.find((a) => a.reports_to === null)?.id ?? candidates[0]?.id ?? null,
    [candidates],
  );
  const ceoId = conversation?.ceo_agent_id ?? leader;
  const ceo = candidates.find((a) => a.id === ceoId) ?? null;

  // (Re)load the thread on company switch and on every live tick.
  useEffect(() => {
    let cancelled = false;
    api
      .getConversation(companyId)
      .then((r) => {
        if (cancelled) return;
        setConversation(r.conversation);
        setMessages(r.messages);
        const last = r.messages[r.messages.length - 1];
        if (last && last.role !== "user") setPending(false);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [companyId, tick]);

  // Keep the thread pinned to the bottom as it grows / the CEO "types".
  const bottomRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [messages, pending]);

  async function send() {
    const content = draft.trim();
    if (!content || !ceoId || sending) return;
    setDraft("");
    setSending(true);
    setPending(true);
    // Optimistic: show the user's message immediately; the refetch reconciles.
    setMessages((m) => [
      ...m,
      { id: `tmp-${Date.now()}`, role: "user", content, created_at: new Date().toISOString() },
    ]);
    try {
      await api.postMessage(companyId, ceoId, content);
      onChanged();
    } catch (e) {
      setPending(false);
      const msg = e instanceof ApiError ? e.message : "Could not reach the CEO.";
      setMessages((m) => [
        ...m,
        { id: `err-${Date.now()}`, role: "system", content: msg, created_at: new Date().toISOString() },
      ]);
    } finally {
      setSending(false);
    }
  }

  const noCeo = candidates.length === 0;

  return (
    <div className="mx-auto flex w-full min-h-0 max-w-3xl flex-1 flex-col px-4">
      {ceo && (messages.length > 0 || pending) && (
        <div className="flex items-center gap-2.5 border-b border-border py-3">
          <Avatar name={ceo.name} />
          <div className="flex flex-col leading-tight">
            <span className="text-sm font-medium">{ceo.name}</span>
            <span className="text-xs text-muted-foreground">{ceo.title ?? "CEO"}</span>
          </div>
        </div>
      )}
      <div className="flex-1 space-y-5 overflow-y-auto py-6">
        {messages.length === 0 && !pending ? (
          <EmptyState ceoName={ceo?.name ?? null} />
        ) : (
          messages.map((m) => <MessageRow key={m.id} message={m} ceo={ceo} />)
        )}
        {pending && <Typing ceo={ceo} />}
        <div ref={bottomRef} />
      </div>

      <div className="border-t border-border py-3">
        <div className="flex items-end gap-2">
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
            rows={1}
            disabled={noCeo}
            placeholder={
              noCeo ? "Hire an agent to act as CEO first…" : `Message ${ceo?.name ?? "the CEO"}…`
            }
            className={cn(
              "max-h-40 min-h-11 flex-1 resize-none rounded-xl border border-input bg-background px-3.5 py-2.5 text-sm",
              "placeholder:text-muted-foreground/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              "disabled:opacity-60",
            )}
          />
          <Button
            variant="primary"
            size="icon"
            onClick={() => void send()}
            disabled={noCeo || !draft.trim() || sending}
            aria-label="Send message"
            className="h-11 w-11 rounded-xl"
          >
            <SendHorizontal className="h-4.5 w-4.5" />
          </Button>
        </div>
        <p className="mt-1.5 px-1 text-[11px] text-muted-foreground/70">
          The CEO decomposes what you ask into tasks and dispatches the team. Watch them land on
          the Board.
        </p>
      </div>
    </div>
  );
}

function EmptyState({ ceoName }: { ceoName: string | null }) {
  return (
    <div className="flex h-full flex-col items-center justify-center py-16 text-center">
      <span className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
        <Sparkles className="h-6 w-6" />
      </span>
      <h2 className="text-lg font-semibold tracking-tight">
        Talk to {ceoName ?? "your CEO"}
      </h2>
      <p className="mt-1.5 max-w-md text-sm text-muted-foreground">
        Describe what you want — a decision, a piece of research, a change to ship. The CEO breaks
        it down, opens the right tasks, and puts the team on it.
      </p>
    </div>
  );
}

function MessageRow({ message, ceo }: { message: Message; ceo: Agent | null }) {
  if (message.role === "system") {
    return (
      <div className="flex justify-center">
        <span className="rounded-full bg-muted px-3 py-1 text-xs text-muted-foreground">
          {message.content}
        </span>
      </div>
    );
  }

  const isUser = message.role === "user";
  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: "easeOut" }}
      className={cn("flex gap-3", isUser ? "justify-end" : "justify-start")}
    >
      {!isUser && <Avatar name={ceo?.name ?? "CEO"} className="mt-6" />}
      <div className={cn("flex max-w-[82%] flex-col gap-1", isUser && "items-end")}>
        {!isUser && (
          <span className="px-1 text-xs font-medium text-muted-foreground">
            {ceo?.name ?? "CEO"}
            {ceo?.title ? <span className="font-normal"> · {ceo.title}</span> : null}
          </span>
        )}
        <div
          className={cn(
            "whitespace-pre-wrap rounded-2xl px-3.5 py-2.5 text-sm leading-relaxed",
            isUser
              ? "rounded-br-sm bg-primary text-primary-foreground"
              : "rounded-tl-sm border border-border bg-card text-card-foreground",
          )}
        >
          {message.content}
        </div>
      </div>
    </motion.div>
  );
}

function Typing({ ceo }: { ceo: Agent | null }) {
  return (
    <div className="flex gap-3">
      <Avatar name={ceo?.name ?? "CEO"} />
      <div className="flex items-center gap-1.5 rounded-2xl rounded-tl-sm border border-border bg-card px-4 py-3.5">
        {[0, 1, 2].map((i) => (
          <motion.span
            key={i}
            className="h-1.5 w-1.5 rounded-full bg-muted-foreground/60"
            animate={{ opacity: [0.3, 1, 0.3] }}
            transition={{ duration: 1.1, repeat: Infinity, delay: i * 0.18 }}
          />
        ))}
      </div>
    </div>
  );
}

function Avatar({ name, className }: { name: string; className?: string }) {
  const initials = name
    .split(/\s+/)
    .slice(0, 2)
    .map((w) => w.charAt(0).toUpperCase())
    .join("");
  return (
    <span
      className={cn(
        "flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-xs font-semibold text-primary",
        className,
      )}
    >
      {initials || "C"}
    </span>
  );
}
