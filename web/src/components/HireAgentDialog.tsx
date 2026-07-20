import { useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Shield,
  Server,
  Palette,
  Eye,
  Search,
  FileText,
  Bot,
  ChevronRight,
  Sparkles,
} from "lucide-react";
import type { Agent, Archetype, AgentTraits, Autonomy, ReviewStrictness } from "../lib/api";
import { api } from "../lib/api";
import { AUTONOMY_LABEL, STRICTNESS_LABEL, autonomySentence } from "../lib/status";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Field, Input, Textarea } from "./ui/primitives";
import { Chip, Segmented } from "./ui/controls";
import { cn, formatCents } from "../lib/utils";

const ICONS: Record<string, typeof Bot> = {
  "security-engineer": Shield,
  "backend-developer": Server,
  "frontend-developer": Palette,
  "code-reviewer": Eye,
  researcher: Search,
  "technical-writer": FileText,
};

const MODELS = ["claude-sonnet", "claude-opus", "claude-haiku"];

type Level = "pick" | "tune" | "expert";

export function HireAgentDialog({
  open,
  onOpenChange,
  companyId,
  archetypes,
  agents,
  defaultManager,
  onHired,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  companyId: string;
  archetypes: Archetype[];
  agents: Agent[];
  defaultManager: string | null;
  onHired: () => void;
}) {
  const [level, setLevel] = useState<Level>("pick");
  const [picked, setPicked] = useState<Archetype | null>(null);
  const [name, setName] = useState("");
  const [title, setTitle] = useState("");
  const [manager, setManager] = useState<string>("");
  const [traits, setTraits] = useState<AgentTraits | null>(null);
  const [brief, setBrief] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setLevel("pick");
    setPicked(null);
    setName("");
    setTitle("");
    setManager(defaultManager ?? "");
    setTraits(null);
    setBrief("");
    setError(null);
  };

  const choose = (a: Archetype) => {
    setPicked(a);
    setTraits({ ...a.default_traits, focus_areas: [...a.default_traits.focus_areas] });
    setName(a.name);
    setManager(defaultManager ?? "");
    setLevel("tune");
  };

  const managerOptions = agents.filter((a) => a.status !== "terminated");

  const toggleFocus = (f: string) => {
    if (!traits) return;
    const has = traits.focus_areas.includes(f);
    setTraits({
      ...traits,
      focus_areas: has ? traits.focus_areas.filter((x) => x !== f) : [...traits.focus_areas, f],
    });
  };

  const submit = async () => {
    if (!picked || !traits) return;
    setBusy(true);
    setError(null);
    try {
      await api.hireAgent(companyId, {
        name: name.trim() || picked.name,
        archetype: picked.slug,
        traits,
        custom_brief: brief.trim() || null,
        title: title.trim() || null,
        reports_to: manager || null,
      });
      onHired();
      onOpenChange(false);
      setTimeout(reset, 200);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to hire");
    } finally {
      setBusy(false);
    }
  };

  // Focus-area suggestions = the archetype's defaults, so tuning stays click-first.
  const focusOptions = useMemo(
    () => picked?.default_traits.focus_areas ?? [],
    [picked],
  );

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        onOpenChange(o);
        if (!o) setTimeout(reset, 200);
      }}
      title="Hire an agent"
      description={
        level === "pick"
          ? "Pick a role to start — everything is preconfigured."
          : `${picked?.name} · ${level === "tune" ? "tune the details" : "expert mode"}`
      }
      className="max-w-2xl"
    >
      <AnimatePresence mode="wait">
        {level === "pick" && (
          <motion.div
            key="pick"
            initial={{ opacity: 0, x: -8 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -8 }}
            transition={{ duration: 0.15 }}
            className="grid grid-cols-1 gap-2.5 sm:grid-cols-2"
          >
            {archetypes.map((a) => {
              const Icon = ICONS[a.slug] ?? Bot;
              return (
                <button
                  key={a.id}
                  type="button"
                  onClick={() => choose(a)}
                  className="group flex flex-col gap-2 rounded-lg border border-border bg-card p-4 text-left transition hover:border-primary/50 hover:shadow-soft cursor-pointer"
                >
                  <div className="flex items-center gap-2.5">
                    <span className="flex h-9 w-9 items-center justify-center rounded-md bg-primary/10 text-primary">
                      <Icon className="h-4.5 w-4.5" />
                    </span>
                    <span className="font-medium">{a.name}</span>
                    <ChevronRight className="ml-auto h-4 w-4 text-muted-foreground opacity-0 transition group-hover:opacity-100" />
                  </div>
                  <p className="text-sm leading-snug text-muted-foreground">{a.description}</p>
                </button>
              );
            })}
          </motion.div>
        )}

        {level !== "pick" && traits && picked && (
          <motion.div
            key="config"
            initial={{ opacity: 0, x: 8 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 8 }}
            transition={{ duration: 0.15 }}
            className="flex flex-col gap-5"
          >
            {level === "tune" ? (
              <>
                <div className="grid grid-cols-1 gap-5 sm:grid-cols-2">
                  <Field label="Name">
                    <Input value={name} onChange={(e) => setName(e.target.value)} />
                  </Field>
                  <Field label="Title" hint="Optional job title.">
                    <Input
                      value={title}
                      onChange={(e) => setTitle(e.target.value)}
                      placeholder="e.g. Senior Engineer"
                    />
                  </Field>
                </div>

                <Field label="Reports to" hint="Where this agent sits in the org.">
                  <select
                    value={manager}
                    onChange={(e) => setManager(e.target.value)}
                    className="h-10 rounded-md border border-input bg-background px-3 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <option value="">You (owner)</option>
                    {managerOptions.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.name}
                        {m.title ? ` · ${m.title}` : ""}
                      </option>
                    ))}
                  </select>
                </Field>

                <Field label="Focus areas" hint="What this agent pays attention to.">
                  <div className="flex flex-wrap gap-2">
                    {focusOptions.map((f) => (
                      <Chip
                        key={f}
                        active={traits.focus_areas.includes(f)}
                        onClick={() => toggleFocus(f)}
                      >
                        {f}
                      </Chip>
                    ))}
                  </div>
                </Field>

                <div className="grid grid-cols-1 gap-5 sm:grid-cols-2">
                  <Field label="Autonomy">
                    <Segmented<Autonomy>
                      value={traits.autonomy}
                      onChange={(v) => setTraits({ ...traits, autonomy: v })}
                      options={(
                        ["propose_only", "act_with_approval", "act_within_budget"] as Autonomy[]
                      ).map((v) => ({ value: v, label: AUTONOMY_LABEL[v] }))}
                    />
                  </Field>
                  <Field label="Review strictness">
                    <Segmented<ReviewStrictness>
                      value={traits.review_strictness}
                      onChange={(v) => setTraits({ ...traits, review_strictness: v })}
                      options={(
                        ["lenient", "standard", "strict"] as ReviewStrictness[]
                      ).map((v) => ({ value: v, label: STRICTNESS_LABEL[v] }))}
                    />
                  </Field>
                </div>

                <Field label={`Monthly budget · ${formatCents(traits.monthly_budget_cents)}`}>
                  <input
                    type="range"
                    min={500}
                    max={50000}
                    step={500}
                    value={traits.monthly_budget_cents}
                    onChange={(e) =>
                      setTraits({ ...traits, monthly_budget_cents: Number(e.target.value) })
                    }
                    className="w-full accent-[var(--color-primary)]"
                  />
                </Field>

                <Field label="Model">
                  <Segmented
                    value={traits.model}
                    onChange={(v) => setTraits({ ...traits, model: v })}
                    options={MODELS.map((m) => ({ value: m, label: m.replace("claude-", "") }))}
                  />
                </Field>
              </>
            ) : (
              <Field
                label="Custom brief"
                hint="Added on top of the structured config. It can add guidance but never override the enforced limits above."
              >
                <Textarea
                  value={brief}
                  onChange={(e) => setBrief(e.target.value)}
                  placeholder="e.g. Pay special attention to our authentication module and flag any use of deprecated crypto."
                  className="min-h-32"
                />
              </Field>
            )}

            <LivePreview traits={traits} name={name || picked.name} hasBrief={brief.trim().length > 0} />

            {error && <p className="text-sm text-destructive">{error}</p>}

            <div className="flex items-center justify-between gap-2 pt-1">
              <Button
                variant="ghost"
                onClick={() => setLevel(level === "expert" ? "tune" : "pick")}
              >
                Back
              </Button>
              <div className="flex gap-2">
                {level === "tune" && (
                  <Button variant="outline" onClick={() => setLevel("expert")}>
                    <Sparkles className="h-4 w-4" />
                    Expert mode
                  </Button>
                )}
                <Button variant="primary" onClick={submit} disabled={busy}>
                  {busy ? "Hiring…" : "Hire agent"}
                </Button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </Dialog>
  );
}

/** Plain-language restatement of what the configured agent will do. */
function LivePreview({
  traits,
  name,
  hasBrief,
}: {
  traits: AgentTraits;
  name: string;
  hasBrief: boolean;
}) {
  return (
    <div className={cn("rounded-md border border-border bg-muted/40 p-3.5 text-sm")}>
      <p className="leading-relaxed">
        <span className="font-medium">{name}</span> {autonomySentence(traits.autonomy)}, reviewing
        with <span className="font-medium">{STRICTNESS_LABEL[traits.review_strictness]}</span>{" "}
        strictness on{" "}
        <span className="font-medium">
          {traits.focus_areas.length ? traits.focus_areas.join(", ") : "no specific focus"}
        </span>
        . Capped at{" "}
        <span className="mono">{formatCents(traits.monthly_budget_cents)}</span>/mo on{" "}
        <span className="mono">{traits.model}</span>.
        {hasBrief && " Plus your custom brief."}
      </p>
    </div>
  );
}
