import { useState } from "react";
import { motion } from "motion/react";
import {
  Shield,
  Server,
  Palette,
  Eye,
  Search,
  FileText,
  Bot,
  Crown,
  UserPlus,
  Pencil,
  Check,
  X,
  Pause,
  Play,
  Ban,
  ShieldCheck,
} from "lucide-react";
import type { Agent, AgentBudget } from "../lib/api";
import { api } from "../lib/api";
import { AUTONOMY_LABEL } from "../lib/status";
import { Button } from "./ui/button";
import { Badge, Input } from "./ui/primitives";
import { cn, formatCents } from "../lib/utils";

const ICONS: Record<string, typeof Bot> = {
  "security-engineer": Shield,
  "backend-developer": Server,
  "frontend-developer": Palette,
  "code-reviewer": Eye,
  researcher: Search,
  "technical-writer": FileText,
};

export function OrgChart({
  agents,
  budgets,
  onChanged,
  onHireUnder,
}: {
  agents: Agent[];
  budgets: AgentBudget[];
  onChanged: () => void;
  onHireUnder: (managerId: string | null) => void;
}) {
  const active = agents.filter((a) => a.status !== "terminated");
  const childrenOf = (id: string | null) => active.filter((a) => (a.reports_to ?? null) === id);
  const budgetOf = (id: string) => budgets.find((b) => b.agent_id === id);

  return (
    <div className="flex-1 overflow-auto px-6 pb-8">
      <div className="mx-auto max-w-3xl">
        {/* The human owner is the root of the chart. */}
        <div className="mb-2 flex items-center gap-3 rounded-lg border border-border bg-card p-3.5 shadow-soft">
          <span className="flex h-9 w-9 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <Crown className="h-4.5 w-4.5" />
          </span>
          <div className="min-w-0">
            <p className="font-medium">You</p>
            <p className="text-xs text-muted-foreground">Owner · everyone ultimately reports here</p>
          </div>
          <Button
            size="sm"
            variant="outline"
            className="ml-auto"
            onClick={() => onHireUnder(null)}
          >
            <UserPlus className="h-4 w-4" />
            Hire
          </Button>
        </div>

        <Tree
          nodes={childrenOf(null)}
          childrenOf={childrenOf}
          agents={active}
          budgetOf={budgetOf}
          depth={0}
          onChanged={onChanged}
          onHireUnder={onHireUnder}
        />
        {active.length === 0 && (
          <p className="mt-6 text-center text-sm text-muted-foreground">
            No agents yet. Hire your first one to build the org.
          </p>
        )}
      </div>
    </div>
  );
}

function Tree({
  nodes,
  childrenOf,
  agents,
  budgetOf,
  depth,
  onChanged,
  onHireUnder,
}: {
  nodes: Agent[];
  childrenOf: (id: string) => Agent[];
  agents: Agent[];
  budgetOf: (id: string) => AgentBudget | undefined;
  depth: number;
  onChanged: () => void;
  onHireUnder: (managerId: string | null) => void;
}) {
  return (
    <div className={cn(depth > 0 && "ml-5 border-l border-border pl-4")}>
      {nodes.map((agent) => (
        <div key={agent.id} className="mt-2">
          <Node
            agent={agent}
            agents={agents}
            budget={budgetOf(agent.id)}
            onChanged={onChanged}
            onHireUnder={onHireUnder}
          />
          <Tree
            nodes={childrenOf(agent.id)}
            childrenOf={childrenOf}
            agents={agents}
            budgetOf={budgetOf}
            depth={depth + 1}
            onChanged={onChanged}
            onHireUnder={onHireUnder}
          />
        </div>
      ))}
    </div>
  );
}

function Node({
  agent,
  agents,
  budget,
  onChanged,
  onHireUnder,
}: {
  agent: Agent;
  agents: Agent[];
  budget: AgentBudget | undefined;
  onChanged: () => void;
  onHireUnder: (managerId: string | null) => void;
}) {
  const [editing, setEditing] = useState(false);
  const Icon = ICONS[agent.archetype] ?? Bot;
  const paused = agent.status === "paused";

  // Valid managers = anyone except self (server also rejects cycles).
  const managerOptions = agents.filter((a) => a.id !== agent.id);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      className={cn(
        "group rounded-lg border border-border bg-card p-3 transition hover:border-primary/40",
        paused && "opacity-70",
      )}
    >
      <div className="flex items-center gap-3">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
          <Icon className="h-4 w-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{agent.name}</span>
            {agent.title && <span className="text-sm text-muted-foreground">· {agent.title}</span>}
            {paused && (
              <Badge tone="var(--color-status-cancelled)">
                <Pause className="h-3 w-3" />
                paused
              </Badge>
            )}
            {agent.requires_approval && (
              <Badge tone="var(--color-status-in_review)">
                <ShieldCheck className="h-3 w-3" />
                approval
              </Badge>
            )}
          </div>
          <p className="text-xs text-muted-foreground">
            {agent.archetype} · {AUTONOMY_LABEL[agent.traits.autonomy]}
          </p>
        </div>
        <div className="flex items-center gap-1 opacity-0 transition group-hover:opacity-100">
          <Button size="icon" variant="ghost" onClick={() => setEditing((v) => !v)} title="Edit">
            <Pencil className="h-4 w-4" />
          </Button>
          <Button
            size="icon"
            variant="ghost"
            onClick={() => onHireUnder(agent.id)}
            title="Hire a report"
          >
            <UserPlus className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {budget && budget.budget_cents > 0 && <BudgetBar budget={budget} />}

      {editing && (
        <EditRow
          agent={agent}
          managerOptions={managerOptions}
          onDone={() => setEditing(false)}
          onChanged={onChanged}
        />
      )}
    </motion.div>
  );
}

/** Month-to-date spend (+ in-flight reservation) against the cap. */
function BudgetBar({ budget }: { budget: AgentBudget }) {
  const used = budget.spent_cents + budget.reserved_cents;
  const pct = Math.min(100, (used / budget.budget_cents) * 100);
  const tone =
    pct >= 100
      ? "var(--color-status-blocked)"
      : pct >= 80
        ? "var(--color-status-in_review)"
        : "var(--color-status-done)";
  return (
    <div className="mt-2.5 flex items-center gap-2">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
        <div className="h-full rounded-full transition-all" style={{ width: `${pct}%`, background: tone }} />
      </div>
      <span className="mono shrink-0 text-[11px] text-muted-foreground">
        {formatCents(used)}/{formatCents(budget.budget_cents)}
      </span>
    </div>
  );
}

function EditRow({
  agent,
  managerOptions,
  onDone,
  onChanged,
}: {
  agent: Agent;
  managerOptions: Agent[];
  onDone: () => void;
  onChanged: () => void;
}) {
  const [title, setTitle] = useState(agent.title ?? "");
  const [manager, setManager] = useState(agent.reports_to ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Run a governance action, then refresh; keeps the panel open so several
  // actions can be taken in a row.
  const run = async (p: Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await p;
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed");
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    await run(
      api.reassignAgent(agent.id, {
        reports_to: manager === "" ? null : manager,
        title,
      }),
    );
    onDone();
  };

  return (
    <motion.div
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: "auto" }}
      className="mt-3 flex flex-col gap-2 border-t border-border pt-3"
    >
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <label className="flex flex-col gap-1 text-xs text-muted-foreground">
          Title
          <Input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g. Senior Engineer" className="h-9" />
        </label>
        <label className="flex flex-col gap-1 text-xs text-muted-foreground">
          Reports to
          <select
            value={manager}
            onChange={(e) => setManager(e.target.value)}
            className="h-9 rounded-md border border-input bg-background px-2.5 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option value="">You (owner)</option>
            {managerOptions.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}

      {/* Governance actions */}
      <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
        <span className="text-xs font-medium text-muted-foreground">Governance</span>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() =>
            run(agent.status === "paused" ? api.resumeAgent(agent.id) : api.pauseAgent(agent.id))
          }
        >
          {agent.status === "paused" ? (
            <>
              <Play className="h-4 w-4" /> Resume
            </>
          ) : (
            <>
              <Pause className="h-4 w-4" /> Pause
            </>
          )}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => run(api.setApprovalGate(agent.id, !agent.requires_approval))}
        >
          <ShieldCheck className="h-4 w-4" />
          {agent.requires_approval ? "Drop approval gate" : "Require approval"}
        </Button>
        <Button
          size="sm"
          variant="destructive"
          disabled={busy}
          onClick={() => {
            if (confirm(`Terminate ${agent.name}? This is permanent.`))
              run(api.terminateAgent(agent.id));
          }}
        >
          <Ban className="h-4 w-4" />
          Terminate
        </Button>
      </div>

      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onDone}>
          <X className="h-4 w-4" />
          Cancel
        </Button>
        <Button size="sm" variant="primary" onClick={save} disabled={busy}>
          <Check className="h-4 w-4" />
          Save title / manager
        </Button>
      </div>
    </motion.div>
  );
}
