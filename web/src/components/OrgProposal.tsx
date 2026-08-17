import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Check, CornerDownRight, RotateCcw, Sparkles, UserMinus, UserPlus, X } from "lucide-react";
import type { OrgProposal as Proposal, OrgProposalMember } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";
import { useCatalogText, useT } from "../lib/i18n";
import { FALLBACK_ICON, FUNCTION_ICONS } from "../lib/catalog";

/**
 * The team the CEO drew up, before it exists.
 *
 * Deliberately the same shape as the real org chart below it — same indent,
 * same node rhythm — but drawn provisionally: dashed outlines, and every hire
 * carries the reason it is there. Dropping someone does not remove them; they
 * go quiet and can come back, so you can always see what you are refusing.
 */
export function OrgProposalPanel({
  proposal,
  onChanged,
}: {
  proposal: Proposal;
  onChanged: () => void;
}) {
  const t = useT();
  const [busy, setBusy] = useState<string | null>(null);
  const [deciding, setDeciding] = useState(false);

  const kept = proposal.members.filter((m) => !m.excluded);
  const roots = proposal.members.filter(
    (m) => !m.reports_to || !proposal.members.some((o) => o.name === m.reports_to),
  );
  const childrenOf = (name: string) => proposal.members.filter((m) => m.reports_to === name);

  const toggle = async (m: OrgProposalMember) => {
    setBusy(m.id);
    try {
      await api.setProposalMemberExcluded(proposal.id, m.id, !m.excluded);
      onChanged();
    } finally {
      setBusy(null);
    }
  };

  const decide = async (decision: "approve" | "reject") => {
    if (!proposal.approval_id) return;
    setDeciding(true);
    try {
      await api.decideApproval(proposal.approval_id, decision);
      onChanged();
    } finally {
      setDeciding(false);
    }
  };

  const who = proposal.proposed_by_name ?? "The CEO";

  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.35, ease: [0.16, 1, 0.3, 1] }}
      className="mb-6 overflow-hidden rounded-xl border border-primary/35 bg-primary/[0.03]"
      aria-labelledby="proposal-heading"
    >
      <header className="px-5 pt-5">
        <div className="flex items-start gap-3">
          <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/12 text-primary">
            <Sparkles className="h-4 w-4" />
          </span>
          <div className="min-w-0">
            <h2 id="proposal-heading" className="text-base font-semibold">
              {t("proposal.heading", { ceo: who, n: kept.length })}
            </h2>
            <p className="mt-1.5 max-w-[68ch] text-sm leading-relaxed text-muted-foreground">
              {proposal.summary}
            </p>
          </div>
        </div>
      </header>

      <div className="px-5 py-4">
        <ProposedTree
          nodes={roots}
          childrenOf={childrenOf}
          depth={0}
          busy={busy}
          onToggle={toggle}
        />
      </div>

      <footer className="flex flex-wrap items-center gap-3 border-t border-primary/20 bg-primary/[0.04] px-5 py-3.5">
        <p className="min-w-0 flex-1 text-sm text-muted-foreground">
          {kept.length === 0 ? (
            <span className="text-destructive">{t("proposal.allDropped")}</span>
          ) : proposal.members.length === kept.length ? (
            t("proposal.hires", { n: kept.length })
          ) : (
            t("proposal.hiresAndSkips", {
              n: kept.length,
              skipped: proposal.members.length - kept.length,
            })
          )}
        </p>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" disabled={deciding} onClick={() => decide("reject")}>
            <X className="h-4 w-4" />
            {t("common.decline")}
          </Button>
          <Button
            variant="primary"
            size="sm"
            disabled={deciding || kept.length === 0}
            onClick={() => decide("approve")}
          >
            <Check className="h-4 w-4" />
            {deciding ? t("proposal.hiring") : t("proposal.hire", { n: kept.length })}
          </Button>
        </div>
      </footer>
    </motion.section>
  );
}

function ProposedTree({
  nodes,
  childrenOf,
  depth,
  busy,
  onToggle,
}: {
  nodes: OrgProposalMember[];
  childrenOf: (name: string) => OrgProposalMember[];
  depth: number;
  busy: string | null;
  onToggle: (m: OrgProposalMember) => void;
}) {
  return (
    <div className={cn(depth > 0 && "ml-5 border-l border-primary/25 pl-4")}>
      {nodes.map((m) => (
        <div key={m.id} className={cn(depth > 0 || nodes[0] !== m ? "mt-2" : undefined)}>
          <ProposedNode member={m} busy={busy === m.id} onToggle={() => onToggle(m)} />
          <ProposedTree
            nodes={childrenOf(m.name)}
            childrenOf={childrenOf}
            depth={depth + 1}
            busy={busy}
            onToggle={onToggle}
          />
        </div>
      ))}
    </div>
  );
}

function ProposedNode({
  member,
  busy,
  onToggle,
}: {
  member: OrgProposalMember;
  busy: boolean;
  onToggle: () => void;
}) {
  const t = useT();
  const catalog = useCatalogText();
  const Icon = FUNCTION_ICONS[member.archetype] ?? FALLBACK_ICON;
  const out = member.excluded;

  return (
    <motion.div
      layout
      transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
      className={cn(
        "group rounded-lg border border-dashed p-3 transition-colors",
        out ? "border-border bg-transparent" : "border-primary/40 bg-card hover:border-primary/60",
      )}
    >
      <div className="flex items-start gap-3">
        <span
          className={cn(
            "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md transition-colors",
            out ? "bg-muted text-muted-foreground/60" : "bg-primary/10 text-primary",
          )}
        >
          <Icon className="h-4 w-4" />
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <span
              className={cn(
                "font-medium",
                out && "text-muted-foreground/70 line-through decoration-1",
              )}
            >
              {member.name}
            </span>
            {member.title && (
              <span
                className={cn(
                  "text-sm",
                  out ? "text-muted-foreground/60" : "text-muted-foreground",
                )}
              >
                · {member.title}
              </span>
            )}
            {/* Both axes, in words (ADR-0021): `reviewer` alone does not tell
                you whether this hire grades projectors or pull requests. */}
            <span
              className={cn(
                "rounded px-1.5 py-0.5 text-[11px]",
                out ? "text-muted-foreground/50" : "bg-muted text-muted-foreground",
              )}
            >
              {catalog("archetype", member.archetype, { name: member.archetype, description: "" })
                .name}
              {member.domain && member.domain !== "general"
                ? ` · ${catalog("domain", member.domain, { name: member.domain, description: "" }).name}`
                : ""}
            </span>
          </div>

          <AnimatePresence initial={false}>
            {!out && member.rationale && (
              <motion.p
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.18 }}
                className="mt-1 max-w-[70ch] overflow-hidden text-sm leading-relaxed text-muted-foreground"
              >
                {member.rationale}
              </motion.p>
            )}
          </AnimatePresence>

          {out && (
            <p className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
              <CornerDownRight className="h-3.5 w-3.5" />
              {t("proposal.skipped")}
            </p>
          )}
        </div>

        <button
          onClick={onToggle}
          disabled={busy}
          aria-label={
            out
              ? t("proposal.putBack", { name: member.name })
              : t("proposal.skipOne", { name: member.name })
          }
          className={cn(
            "shrink-0 rounded-md p-1.5 transition disabled:opacity-40",
            "text-muted-foreground/70 hover:bg-muted hover:text-foreground",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            !out && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
          )}
          title={out ? "Put back" : "Skip this hire"}
        >
          {out ? <RotateCcw className="h-4 w-4" /> : <UserMinus className="h-4 w-4" />}
        </button>
      </div>
    </motion.div>
  );
}

/**
 * First run: the company has its CEO and nobody else. Two roads, deliberately
 * not symmetric — the CEO exists to do this for you, so that path leads; doing
 * it yourself is offered plainly underneath, not hidden and not competing.
 */
export function TwoRoads({
  ceoName,
  onTalkToCeo,
  onHire,
}: {
  ceoName: string;
  onTalkToCeo: () => void;
  onHire: () => void;
}) {
  const t = useT();
  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.35, ease: [0.16, 1, 0.3, 1] }}
      className="mb-6 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex items-start gap-4">
        <span className="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Sparkles className="h-5 w-5" />
        </span>
        <div className="min-w-0">
          <h2 className="text-base font-semibold">{t("org.twoRoadsTitle", { ceo: ceoName })}</h2>
          <p className="mt-1.5 max-w-[64ch] text-sm leading-relaxed text-muted-foreground">
            {t("org.twoRoadsBody", { ceo: ceoName })}
          </p>
          <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2">
            <Button variant="primary" onClick={onTalkToCeo}>
              {t("org.tellTheIdea", { ceo: ceoName })}
            </Button>
            <button
              onClick={onHire}
              className="inline-flex items-center gap-1.5 rounded-md px-1 py-1 text-sm text-muted-foreground underline-offset-4 transition hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <UserPlus className="h-4 w-4" />
              {t("org.orBuildYourself")}
            </button>
          </div>
        </div>
      </div>
    </motion.section>
  );
}
