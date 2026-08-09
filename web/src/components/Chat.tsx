import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { motion } from "motion/react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { ArrowUpRight, Check, ChevronDown, Paperclip, SendHorizontal, Sparkles, X } from "lucide-react";
import type { Agent, Attachment, Message } from "../lib/api";
import { ApiError, api } from "../lib/api";
import { Button } from "./ui/button";
import { useT } from "../lib/i18n";
import { cn } from "../lib/utils";

/**
 * The conversational surface (ADR-0018/0019). The user talks to any agent in
 * its role — the CEO (the org leader) or a teammate. Posting a message runs
 * that agent's turn server-side: it replies, opens tasks (optionally assigned
 * to a teammate — the ripple), and can escalate to the CEO. Files/images
 * attached to a message reach the agent's working directory. Replies arrive
 * asynchronously over the live channel, so this view refetches on `tick`.
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
  /** Bump the app-wide live tick (so the board reflects new tasks at once). */
  onChanged: () => void;
}) {
  const t = useT();
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState("");
  const [files, setFiles] = useState<File[]>([]);
  const [sending, setSending] = useState(false);
  const [pending, setPending] = useState(false); // agent turn in flight
  const [activeId, setActiveId] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  // Who you can talk to; the CEO is the org leader (reports to the human).
  const candidates = useMemo(() => agents.filter((a) => a.status === "active"), [agents]);
  const leader = useMemo(
    () => candidates.find((a) => a.reports_to === null)?.id ?? candidates[0]?.id ?? null,
    [candidates],
  );
  const currentId = activeId ?? leader;
  const agent = candidates.find((a) => a.id === currentId) ?? null;

  // (Re)load the active agent's thread on switch / company change / live tick.
  useEffect(() => {
    if (!currentId) {
      setMessages([]);
      return;
    }
    let cancelled = false;
    api
      .getConversation(companyId, currentId)
      .then((r) => {
        if (cancelled) return;
        setMessages(r.messages);
        const last = r.messages[r.messages.length - 1];
        if (last && last.role !== "user") setPending(false);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [companyId, currentId, tick]);

  // Keep the thread pinned to the bottom as it grows / the agent "types".
  const bottomRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [messages, pending]);

  function switchTo(id: string) {
    if (id === currentId) return;
    setActiveId(id);
    setMessages([]);
    setPending(false);
    setDraft("");
    setFiles([]);
  }

  async function send() {
    const content = draft.trim();
    const toSend = files;
    if ((!content && toSend.length === 0) || !currentId || sending) return;
    setDraft("");
    setFiles([]);
    setSending(true);
    setPending(true);
    setMessages((m) => [
      ...m,
      { id: `tmp-${Date.now()}`, role: "user", content, created_at: new Date().toISOString() },
    ]);
    try {
      const ids: string[] = [];
      for (const f of toSend) {
        const a = await api.uploadAttachment(companyId, currentId, f);
        ids.push(a.id);
      }
      await api.postMessage(companyId, currentId, content, ids);
      onChanged();
    } catch (e) {
      setPending(false);
      const msg = e instanceof ApiError ? e.message : t("chat.unreachable");
      setMessages((m) => [
        ...m,
        {
          id: `err-${Date.now()}`,
          role: "system",
          content: msg,
          created_at: new Date().toISOString(),
        },
      ]);
    } finally {
      setSending(false);
    }
  }

  const noAgents = candidates.length === 0;
  const canSend = !noAgents && (draft.trim().length > 0 || files.length > 0) && !sending;

  return (
    <div className="mx-auto flex w-full min-h-0 max-w-3xl flex-1 flex-col px-4">
      {/* agent picker — "You're talking to [agent]" above the thread (Claude-style dropdown) */}
      {candidates.length > 0 && (
        <div className="flex items-center gap-1 border-b border-border py-2.5">
          <span className="pl-1 text-sm text-muted-foreground">{t("chat.talkingTo")}</span>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button
                type="button"
                className="press flex items-center gap-2 rounded-lg px-2 py-1.5 text-sm hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="flex h-6 w-6 items-center justify-center rounded-md bg-primary/10 text-[10px] font-semibold text-primary">
                  {initials(agent?.name ?? "?")}
                </span>
                <span className="font-medium">{agent?.name ?? t("chat.selectAgent")}</span>
                {agent?.title && <span className="text-muted-foreground">· {agent.title}</span>}
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              </button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content
                align="start"
                sideOffset={6}
                className="z-50 min-w-[248px] rounded-xl border border-border bg-card p-1.5 text-card-foreground shadow-lg"
              >
                <DropdownMenu.Label className="px-2.5 pb-1 pt-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  {t("chat.talkTo")}
                </DropdownMenu.Label>
                {candidates.map((a) => (
                  <DropdownMenu.Item
                    key={a.id}
                    onSelect={() => switchTo(a.id)}
                    className="flex cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm outline-none data-[highlighted]:bg-muted"
                  >
                    <span className="flex h-7 w-7 items-center justify-center rounded-md bg-primary/10 text-[11px] font-semibold text-primary">
                      {initials(a.name)}
                    </span>
                    <span className="flex min-w-0 flex-col">
                      <span className="flex items-center gap-1.5 font-medium">
                        {a.name}
                        {a.reports_to === null && (
                          <span className="rounded bg-primary/15 px-1 text-[9px] font-semibold uppercase tracking-wide text-primary">
                            CEO
                          </span>
                        )}
                      </span>
                      {a.title && (
                        <span className="truncate text-xs text-muted-foreground">{a.title}</span>
                      )}
                    </span>
                    {a.id === currentId && (
                      <Check className="ml-auto h-4 w-4 shrink-0 text-primary" />
                    )}
                  </DropdownMenu.Item>
                ))}
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </div>
      )}

      <div className="flex-1 space-y-5 overflow-y-auto py-6">
        {messages.length === 0 && !pending ? (
          <EmptyState agent={agent} isLeader={agent?.reports_to === null} />
        ) : (
          messages.map((m) => (
            <MessageRow key={m.id} message={m} agent={agent} companyId={companyId} />
          ))
        )}
        {pending && <Typing agent={agent} />}
        <div ref={bottomRef} />
      </div>

      <div className="border-t border-border py-3">
        {files.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-2">
            {files.map((f, i) => (
              <span
                key={`${f.name}-${i}`}
                className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-muted/50 py-1 pl-2 pr-1 text-xs"
              >
                <Paperclip className="h-3 w-3 text-muted-foreground" />
                <span className="max-w-[160px] truncate">{f.name}</span>
                <button
                  type="button"
                  onClick={() => setFiles((prev) => prev.filter((_, j) => j !== i))}
                  className="rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                  aria-label={t("chat.remove", { name: f.name })}
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            ))}
          </div>
        )}
        <div className="flex items-end gap-2">
          <input
            ref={fileRef}
            type="file"
            multiple
            className="hidden"
            onChange={(e) => {
              const picked = Array.from(e.target.files ?? []);
              if (picked.length) setFiles((prev) => [...prev, ...picked]);
              e.target.value = "";
            }}
          />
          <Button
            variant="ghost"
            size="icon"
            onClick={() => fileRef.current?.click()}
            disabled={noAgents}
            aria-label={t("chat.attach")}
            className="h-11 w-11 rounded-xl"
          >
            <Paperclip className="h-4.5 w-4.5" />
          </Button>
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
            disabled={noAgents}
            placeholder={
              noAgents
                ? t("chat.placeholderNoAgents")
                : t("chat.placeholder", { name: agent?.name ?? t("chat.theAgent") })
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
            disabled={!canSend}
            aria-label={t("chat.send")}
            className="h-11 w-11 rounded-xl"
          >
            <SendHorizontal className="h-4.5 w-4.5" />
          </Button>
        </div>
        <p className="mt-1.5 px-1 text-[11px] text-muted-foreground/70">
          {agent?.reports_to === null
            ? t("chat.hintLeader")
            : t("chat.hintTeammate", { name: agent?.name ?? t("chat.thisAgent") })}
        </p>
      </div>
    </div>
  );
}

function EmptyState({ agent, isLeader }: { agent: Agent | null; isLeader?: boolean }) {
  const t = useT();
  return (
    <div className="flex h-full flex-col items-center justify-center py-16 text-center">
      <span className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
        <Sparkles className="h-6 w-6" />
      </span>
      <h2 className="text-lg font-semibold tracking-tight">
        {t("chat.emptyTitle", { name: agent?.name ?? t("chat.emptyTeam") })}
        {agent?.title ? <span className="text-muted-foreground"> · {agent.title}</span> : null}
      </h2>
      <p className="mt-1.5 max-w-md text-sm text-muted-foreground">
        {isLeader
          ? t("chat.emptyLeader")
          : t("chat.emptyTeammate", { name: agent?.name ?? t("chat.thisAgent") })}
      </p>
    </div>
  );
}

function MessageRow({
  message,
  agent,
  companyId,
}: {
  message: Message;
  agent: Agent | null;
  companyId: string;
}) {
  const t = useT();
  const fallbackAgentName = t("chat.agent");
  if (message.role === "system") {
    return (
      <div className="flex justify-center">
        <span className="rounded-full bg-muted px-3 py-1 text-xs text-muted-foreground">
          {message.content}
        </span>
      </div>
    );
  }

  // An escalation is an agent speaking, not Overmind. It used to be written
  // with the `system` role, which gave agent-authored text the system's voice
  // — and an agent could use that voice to tell you something the system never
  // said (M10 slice 4). Labelled, and never styled as a system notice.
  if (message.role === "escalation") {
    return (
      <div className="flex justify-center">
        <span className="flex items-center gap-1.5 rounded-full border border-border bg-card px-3 py-1 text-xs text-muted-foreground">
          <ArrowUpRight className="h-3 w-3" />
          <span className="font-medium">{t("chat.escalation")}</span>
          {message.content}
        </span>
      </div>
    );
  }

  const isUser = message.role === "user";
  const attachments = message.attachments ?? [];
  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: "easeOut" }}
      className={cn("flex gap-3", isUser ? "justify-end" : "justify-start")}
    >
      {!isUser && <Avatar name={agent?.name ?? fallbackAgentName} className="mt-6" />}
      <div className={cn("flex max-w-[82%] flex-col gap-1", isUser && "items-end")}>
        {!isUser && (
          <span className="px-1 text-xs font-medium text-muted-foreground">
            {agent?.name ?? fallbackAgentName}
            {agent?.title ? <span className="font-normal"> · {agent.title}</span> : null}
          </span>
        )}
        {attachments.length > 0 && (
          <div className={cn("flex flex-wrap gap-2", isUser && "justify-end")}>
            {attachments.map((a) => (
              <AttachmentView key={a.id} companyId={companyId} att={a} />
            ))}
          </div>
        )}
        {message.content && (
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
        )}
      </div>
    </motion.div>
  );
}

function AttachmentView({ companyId, att }: { companyId: string; att: Attachment }) {
  const url = api.attachmentUrl(companyId, att.id);
  if (att.mime.startsWith("image/")) {
    return (
      <a
        href={url}
        target="_blank"
        rel="noopener"
        className="block overflow-hidden rounded-xl border border-border"
        title={att.filename}
      >
        <img src={url} alt={att.filename} className="max-h-56 max-w-[240px] object-cover" />
      </a>
    );
  }
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener"
      className="inline-flex items-center gap-2 rounded-lg border border-border bg-card px-3 py-2 text-xs hover:bg-muted"
    >
      <Paperclip className="h-3.5 w-3.5 text-muted-foreground" />
      <span className="max-w-[200px] truncate">{att.filename}</span>
    </a>
  );
}

function Typing({ agent }: { agent: Agent | null }) {
  const fallbackAgentName = useT()("chat.agent");
  return (
    <div className="flex gap-3">
      <Avatar name={agent?.name ?? fallbackAgentName} />
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

function initials(name: string) {
  return (
    name
      .split(/\s+/)
      .slice(0, 2)
      .map((w) => w.charAt(0).toUpperCase())
      .join("") || "A"
  );
}

function Avatar({ name, className }: { name: string; className?: string }) {
  return (
    <span
      className={cn(
        "flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-xs font-semibold text-primary",
        className,
      )}
    >
      {initials(name)}
    </span>
  );
}
