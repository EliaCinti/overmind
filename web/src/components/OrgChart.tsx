import { useState } from "react";
import { motion } from "motion/react";
import { Crown, UserPlus, Pencil, Check, X, Pause, Play, Ban, ShieldCheck } from "lucide-react";
import type { Agent, AgentBudget, Economy, OrgProposal, PlanWindow } from "../lib/api";
import { PLAN_WINDOWS } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Badge, Input } from "./ui/primitives";
import { cn } from "../lib/utils";
import { useT, useFormats } from "../lib/i18n";
import { FALLBACK_ICON, FUNCTION_ICONS } from "../lib/catalog";
import { OrgProposalPanel, TwoRoads } from "./OrgProposal";

export function OrgChart({
  agents,
  budgets,
  economy,
  planWindows,
  proposal,
  onChanged,
  onHireUnder,
  onTalkToCeo,
}: {
  agents: Agent[];
  budgets: AgentBudget[];
  /** How the server pays, so a cap is read as what it is (ADR-0030). */
  economy: Economy | null;
  /** Where each of the plan's windows stands, as last reported. */
  planWindows: Record<string, PlanWindow>;
  /** A team the CEO drew up and you have not answered yet (M15). */
  proposal: OrgProposal | null;
  onChanged: () => void;
  onHireUnder: (managerId: string | null) => void;
  onTalkToCeo: () => void;
}) {
  const t = useT();
  const active = agents.filter((a) => a.status !== "terminated");
  // First run: the founding CEO and nobody else, nothing proposed yet.
  const ceo = active.find((a) => a.reports_to === null);
  const alone = active.length === 1 && !!ceo && !proposal;
  const childrenOf = (id: string | null) => active.filter((a) => (a.reports_to ?? null) === id);
  const budgetOf = (id: string) => budgets.find((b) => b.agent_id === id);

  return (
    <div className="flex-1 overflow-auto px-6 pb-8">
      <div className="mx-auto max-w-3xl">
        {alone && ceo && (
          <TwoRoads
            ceoName={ceo.name}
            onTalkToCeo={onTalkToCeo}
            onHire={() => onHireUnder(ceo.id)}
          />
        )}
        {proposal && <OrgProposalPanel proposal={proposal} onChanged={onChanged} />}

        {active.length > 0 && <EconomyNote economy={economy} planWindows={planWindows} />}

        {/* The human owner is the root of the chart. */}
        <div className="mb-2 flex items-center gap-3 rounded-lg border border-border bg-card p-3.5 shadow-soft">
          <span className="flex h-9 w-9 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <Crown className="h-4.5 w-4.5" />
          </span>
          <div className="min-w-0">
            <p className="font-medium">{t("org.you")}</p>
            <p className="text-xs text-muted-foreground">{t("org.ownerLine")}</p>
          </div>
          <Button size="sm" variant="outline" className="ml-auto" onClick={() => onHireUnder(null)}>
            <UserPlus className="h-4 w-4" />
            {t("nav.hire")}
          </Button>
        </div>

        <Tree
          nodes={childrenOf(null)}
          childrenOf={childrenOf}
          agents={active}
          budgetOf={budgetOf}
          economy={economy}
          depth={0}
          onChanged={onChanged}
          onHireUnder={onHireUnder}
        />
        {active.length === 0 && (
          <p className="mt-6 text-center text-sm text-muted-foreground">{t("org.empty")}</p>
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
  economy,
  depth,
  onChanged,
  onHireUnder,
}: {
  nodes: Agent[];
  childrenOf: (id: string) => Agent[];
  agents: Agent[];
  budgetOf: (id: string) => AgentBudget | undefined;
  economy: Economy | null;
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
            economy={economy}
            onChanged={onChanged}
            onHireUnder={onHireUnder}
          />
          <Tree
            nodes={childrenOf(agent.id)}
            childrenOf={childrenOf}
            agents={agents}
            budgetOf={budgetOf}
            economy={economy}
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
  economy,
  onChanged,
  onHireUnder,
}: {
  agent: Agent;
  agents: Agent[];
  budget: AgentBudget | undefined;
  economy: Economy | null;
  onChanged: () => void;
  onHireUnder: (managerId: string | null) => void;
}) {
  const t = useT();
  const [editing, setEditing] = useState(false);
  const Icon = FUNCTION_ICONS[agent.archetype] ?? FALLBACK_ICON;
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
                {t("org.paused")}
              </Badge>
            )}
            {agent.requires_approval && (
              <Badge tone="var(--color-status-in_review)">
                <ShieldCheck className="h-3 w-3" />
                {t("org.approvalBadge")}
              </Badge>
            )}
          </div>
          <p className="text-xs text-muted-foreground">
            {agent.archetype} · {t(`autonomy.${agent.traits.autonomy}`)}
          </p>
        </div>
        <div className="flex items-center gap-1 opacity-0 transition group-hover:opacity-100">
          <Button
            size="icon"
            variant="ghost"
            onClick={() => setEditing((v) => !v)}
            title={t("org.edit")}
          >
            <Pencil className="h-4 w-4" />
          </Button>
          <Button
            size="icon"
            variant="ghost"
            onClick={() => onHireUnder(agent.id)}
            title={t("org.hireReport")}
          >
            <UserPlus className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {budget && budget.budget_cents > 0 && <BudgetBar budget={budget} economy={economy} />}

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

/**
 * Month-to-date spend (+ in-flight reservation) against the cap — and how much
 * of it is left, which is the number a person actually steers by.
 *
 * What it says depends on how the work is paid for (ADR-0030). Under an API key
 * these are charges and the cap is a ceiling in money. Under a subscription they
 * are equivalents nobody will be billed for, so the amounts wear a `≈` and the
 * remaining percentage is of *Overmind's own cap* — never of the plan, whose
 * quota is not visible from here. Presenting one as the other would be the lie
 * this milestone exists to avoid.
 */
function BudgetBar({ budget, economy }: { budget: AgentBudget; economy: Economy | null }) {
  const t = useT();
  const { formatCents } = useFormats();
  const used = budget.spent_cents + budget.reserved_cents;
  const pct = Math.min(100, (used / budget.budget_cents) * 100);
  const left = Math.max(0, 100 - Math.round(pct));
  const tone =
    pct >= 100
      ? "var(--color-status-blocked)"
      : pct >= 80
        ? "var(--color-status-in_review)"
        : "var(--color-status-done)";
  const equivalent = economy?.kind === "subscription";
  const amounts = t(equivalent ? "economy.approxOfCap" : "economy.ofCap", {
    used: formatCents(used),
    cap: formatCents(budget.budget_cents),
  });
  const est = budget.estimates;
  return (
    <div className="mt-2.5">
      <div className="flex items-center gap-2" title={amounts}>
        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
          <div
            className="h-full rounded-full transition-all"
            style={{ width: `${pct}%`, background: tone }}
          />
        </div>
        <span className="mono shrink-0 text-[11px] text-muted-foreground">
          {t("economy.left", { pct: left })}
        </span>
      </div>
      {/* What the next run will reserve, priced from this agent's own ledger
          (M26) -- and, in the tooltip, on how much history it rests. */}
      {est && (
        <p
          className="mt-1 text-[11px] text-muted-foreground/80"
          title={t("economy.nextRunFrom", { task: est.task.samples, turn: est.turn.samples })}
        >
          {t("economy.nextRun", {
            task: formatCents(est.task.cents),
            turn: formatCents(est.turn.cents),
          })}
        </p>
      )}
    </div>
  );
}

/**
 * Said once, above the chart, rather than on every agent card.
 *
 * The meaning of a cap is a property of the whole server, so repeating it beside
 * each bar would be noise — and leaving it out entirely is how a number gets
 * read as a promise nobody made.
 */
function EconomyNote({
  economy,
  planWindows,
}: {
  economy: Economy | null;
  planWindows: Record<string, PlanWindow>;
}) {
  const t = useT();
  if (!economy) return null;
  const what =
    economy.kind === "key"
      ? t("economy.key")
      : economy.kind === "subscription"
        ? economy.plan
          ? t("economy.subscriptionWithPlan", { plan: economy.plan })
          : t("economy.subscription")
        : t("economy.unknown");
  const means =
    economy.kind === "key"
      ? t("economy.keyMeaning")
      : economy.kind === "subscription"
        ? t("economy.subscriptionMeaning")
        : t("economy.unknownMeaning");
  return (
    <div className="mb-2.5 space-y-1">
      <p className="text-[11px] text-muted-foreground">
        <span className="font-medium">{what}</span> · {means}
      </p>
      {economy.kind === "subscription" && <PlanLifeline windows={planWindows} />}
      {economy.kind === "key" && economy.overrides_login && (
        // Nothing is broken here, which is exactly why it goes unnoticed: the
        // work runs, the plan sits unused, and the bill arrives later. The CLI
        // warns in a log line; a log line is not being told.
        <p className="text-[11px] text-[var(--color-status-in_review)]">
          <span className="font-medium">{t("economy.keyOverridesLogin")}</span>{" "}
          {t("economy.keyOverridesLoginFix")}
        </p>
      )}
    </div>
  );
}

/**
 * The plan's own life-line: **both** windows, side by side (ADR-0030).
 *
 * A plan limits on two clocks at once — five hours and seven days — and they
 * run out at different moments, so collapsing them into "the plan" hides the
 * thing you are about to hit. Each is learned separately, because a run reports
 * whichever is governing it right then; one nobody has reported yet says so
 * rather than borrowing the other's state.
 *
 * No percentage, and that is not an oversight: `used_percentage` exists only in
 * the status line, which a headless run never invokes. What a run does report is
 * the window, when it resets, and whether we are still allowed inside it —
 * which is what a person waiting on it is actually asking.
 */
function PlanLifeline({ windows }: { windows: Record<string, PlanWindow> }) {
  const t = useT();
  const { timeUntil } = useFormats();
  const name = (w: string) =>
    w === "five_hour"
      ? t("economy.windowFiveHour")
      : w === "seven_day"
        ? t("economy.windowSevenDay")
        : t("economy.windowOther");
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px]">
      <span className="text-muted-foreground">{t("economy.planLifeline")}</span>
      {PLAN_WINDOWS.map((key) => {
        const w = windows[key];
        if (!w) {
          return (
            <span key={key} className="text-muted-foreground/60">
              {name(key)} · {t("economy.windowUnreported")}
            </span>
          );
        }
        const tone =
          w.health === "exhausted"
            ? "font-medium text-[var(--color-status-blocked)]"
            : w.health === "warning"
              ? "text-[var(--color-status-in_review)]"
              : "text-muted-foreground";
        const state =
          w.health === "exhausted"
            ? t("economy.planExhausted")
            : w.health === "warning"
              ? t("economy.planWarning")
              : t("economy.planAllowed");
        return (
          <span key={key} className={tone}>
            <span className="font-medium">{name(key)}</span> · {state} ·{" "}
            {t("economy.windowResets", { when: timeUntil(w.resets_at) })}
          </span>
        );
      })}
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
  const t = useT();
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
      setError(e instanceof Error ? e.message : t("common.failed"));
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
          {t("org.title")}
          <Input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={t("org.titlePlaceholder")}
            className="h-9"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs text-muted-foreground">
          {t("org.reportsTo")}
          <select
            value={manager}
            onChange={(e) => setManager(e.target.value)}
            className="h-9 rounded-md border border-input bg-background px-2.5 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option value="">{t("org.youOwner")}</option>
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
        <span className="text-xs font-medium text-muted-foreground">{t("org.governance")}</span>
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
              <Play className="h-4 w-4" /> {t("org.resume")}
            </>
          ) : (
            <>
              <Pause className="h-4 w-4" /> {t("org.pause")}
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
          {agent.requires_approval ? t("org.dropApproval") : t("org.requireApproval")}
        </Button>
        <Button
          size="sm"
          variant="destructive"
          disabled={busy}
          onClick={() => {
            if (confirm(t("org.terminateConfirm", { name: agent.name })))
              run(api.terminateAgent(agent.id));
          }}
        >
          <Ban className="h-4 w-4" />
          {t("org.terminate")}
        </Button>
      </div>

      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onDone}>
          <X className="h-4 w-4" />
          {t("common.cancel")}
        </Button>
        <Button size="sm" variant="primary" onClick={save} disabled={busy}>
          <Check className="h-4 w-4" />
          {t("org.saveEdits")}
        </Button>
      </div>
    </motion.div>
  );
}
