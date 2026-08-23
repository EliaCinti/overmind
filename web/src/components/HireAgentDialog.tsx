import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Bot, ChevronRight, Sparkles } from "lucide-react";
import type {
  Agent,
  Archetype,
  AgentTraits,
  Autonomy,
  Domain,
  Model,
  ReviewStrictness,
  Tool,
} from "../lib/api";
import { api } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Field, Input, Textarea } from "./ui/primitives";
import { Chip, Segmented } from "./ui/controls";
import { useCatalogText, useFormats, useT } from "../lib/i18n";
import { DOMAIN_ICONS, FALLBACK_ICON, FUNCTION_ICONS } from "../lib/catalog";
import { cn } from "../lib/utils";

type Level = "pick" | "field" | "tune" | "expert";

/**
 * What a domain adds on top of a function's defaults, for the preview.
 *
 * Mirrors `AgentTraits::with_domain` on the server, which stays the authority —
 * it recomposes from the catalog on every hire. This exists so the tune step
 * shows the traits you are actually about to create, rather than the function's
 * bare defaults with the field's contribution appearing only after the fact.
 */
function withDomain(base: AgentTraits, domain: Domain | null): AgentTraits {
  if (!domain) return { ...base, focus_areas: [...base.focus_areas] };
  const patch = domain.traits_patch;
  const focus = [...base.focus_areas];
  for (const f of patch.focus_areas) if (!focus.includes(f)) focus.push(f);
  const permissions = [...base.permissions];
  for (const p of patch.permissions) {
    // A field never decides which kinds of task an agent may take.
    if (p === "task:code" || p === "task:knowledge") continue;
    if (!permissions.includes(p)) permissions.push(p);
  }
  return {
    ...base,
    focus_areas: focus,
    permissions,
    multimodal: base.multimodal || patch.multimodal,
  };
}

export function HireAgentDialog({
  open,
  onOpenChange,
  companyId,
  archetypes,
  domains,
  models,
  agents,
  defaultManager,
  onHired,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  companyId: string;
  archetypes: Archetype[];
  domains: Domain[];
  models: Model[];
  agents: Agent[];
  defaultManager: string | null;
  onHired: () => void;
}) {
  const t = useT();
  const catalog = useCatalogText();
  const { formatCents } = useFormats();
  const [level, setLevel] = useState<Level>("pick");
  const [picked, setPicked] = useState<Archetype | null>(null);
  const [field, setField] = useState<Domain | null>(null);
  const [name, setName] = useState("");
  const [title, setTitle] = useState("");
  const [manager, setManager] = useState<string>("");
  const [traits, setTraits] = useState<AgentTraits | null>(null);
  const [brief, setBrief] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The operator's tool registry (ADR-0036), asked for when the dialog opens:
  // empty is the ordinary case, and then the field simply is not there.
  const [tools, setTools] = useState<Tool[]>([]);
  useEffect(() => {
    if (open) api.listTools().then(setTools).catch(() => setTools([]));
  }, [open]);

  const reset = () => {
    setLevel("pick");
    setPicked(null);
    setField(null);
    setName("");
    setTitle("");
    setManager(defaultManager ?? "");
    setTraits(null);
    setBrief("");
    setError(null);
  };

  const chooseFunction = (a: Archetype) => {
    setPicked(a);
    setName(a.name);
    setManager(defaultManager ?? "");
    setLevel("field");
  };

  const chooseField = (d: Domain) => {
    if (!picked) return;
    setField(d);
    setTraits(withDomain(picked.default_traits, d));
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
        domain: field?.slug ?? null,
        traits,
        custom_brief: brief.trim() || null,
        title: title.trim() || null,
        reports_to: manager || null,
      });
      onHired();
      onOpenChange(false);
      setTimeout(reset, 200);
    } catch (e) {
      setError(e instanceof Error ? e.message : t("hire.failed"));
    } finally {
      setBusy(false);
    }
  };

  // Focus-area suggestions = what the function and the field between them
  // suggest, so tuning stays click-first even in a field nobody typed.
  const focusOptions = useMemo(
    () => (picked ? withDomain(picked.default_traits, field).focus_areas : []),
    [picked, field],
  );

  // A vision-less model cannot carry a multimodal agent; the server refuses it,
  // so the dialog should not offer the combination in the first place.
  const modelHasVision = models.find((m) => m.id === traits?.model)?.vision ?? true;

  const functionName = picked
    ? catalog("archetype", picked.slug, { name: picked.name, description: picked.description }).name
    : "";

  const description = () => {
    if (level === "pick") return t("hire.pickFunctionDesc");
    if (level === "field") return t("hire.fieldOf", { function: functionName });
    const step = level === "tune" ? t("hire.tune") : t("hire.expert");
    const where = field
      ? catalog("domain", field.slug, { name: field.name, description: field.description }).name
      : "";
    return `${functionName}${where ? ` · ${where}` : ""} · ${step}`;
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        onOpenChange(o);
        if (!o) setTimeout(reset, 200);
      }}
      title={t("hire.title")}
      description={description()}
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
            {archetypes.map((a) => (
              <CatalogCard
                key={a.id}
                icon={FUNCTION_ICONS[a.slug] ?? FALLBACK_ICON}
                {...catalog("archetype", a.slug, { name: a.name, description: a.description })}
                onClick={() => chooseFunction(a)}
              />
            ))}
          </motion.div>
        )}

        {level === "field" && (
          <motion.div
            key="field"
            initial={{ opacity: 0, x: 8 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -8 }}
            transition={{ duration: 0.15 }}
            className="flex flex-col gap-3"
          >
            <p className="text-sm text-muted-foreground">{t("hire.pickDomainDesc")}</p>
            <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
              {domains.map((d) => (
                <CatalogCard
                  key={d.id}
                  icon={DOMAIN_ICONS[d.slug] ?? FALLBACK_ICON}
                  {...catalog("domain", d.slug, { name: d.name, description: d.description })}
                  onClick={() => chooseField(d)}
                />
              ))}
            </div>
            <div className="pt-1">
              <Button variant="ghost" onClick={() => setLevel("pick")}>
                {t("common.back")}
              </Button>
            </div>
          </motion.div>
        )}

        {(level === "tune" || level === "expert") && traits && picked && (
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
                  <Field label={t("hire.name")}>
                    <Input value={name} onChange={(e) => setName(e.target.value)} />
                  </Field>
                  <Field label={t("hire.jobTitle")} hint={t("hire.jobTitleHint")}>
                    <Input
                      value={title}
                      onChange={(e) => setTitle(e.target.value)}
                      placeholder={t("hire.jobTitlePlaceholder")}
                    />
                  </Field>
                </div>

                <Field label={t("hire.reportsTo")} hint={t("hire.reportsToHint")}>
                  <select
                    value={manager}
                    onChange={(e) => setManager(e.target.value)}
                    className="h-10 rounded-md border border-input bg-background px-3 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <option value="">{t("hire.youOwner")}</option>
                    {managerOptions.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.name}
                        {m.title ? ` · ${m.title}` : ""}
                      </option>
                    ))}
                  </select>
                </Field>

                <Field label={t("hire.focus")} hint={t("hire.focusHint")}>
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
                  <Field label={t("hire.autonomy")}>
                    <Segmented<Autonomy>
                      value={traits.autonomy}
                      onChange={(v) => setTraits({ ...traits, autonomy: v })}
                      options={(
                        ["propose_only", "act_with_approval", "act_within_budget"] as Autonomy[]
                      ).map((v) => ({ value: v, label: t(`autonomy.${v}`) }))}
                    />
                  </Field>
                  <Field label={t("hire.strictness")}>
                    <Segmented<ReviewStrictness>
                      value={traits.review_strictness}
                      onChange={(v) => setTraits({ ...traits, review_strictness: v })}
                      options={(["lenient", "standard", "strict"] as ReviewStrictness[]).map(
                        (v) => ({ value: v, label: t(`strictness.${v}`) }),
                      )}
                    />
                  </Field>
                </div>

                <Field
                  label={t("hire.budget", { amount: formatCents(traits.monthly_budget_cents) })}
                >
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

                <Field label={t("hire.model")}>
                  <Segmented
                    value={traits.model}
                    onChange={(v) =>
                      setTraits({
                        ...traits,
                        model: v,
                        multimodal:
                          traits.multimodal && (models.find((m) => m.id === v)?.vision ?? true),
                      })
                    }
                    options={models.map((m) => ({ value: m.id, label: m.display_name }))}
                  />
                </Field>

                <Field label={t("hire.multimodal")} hint={t("hire.multimodalHint")}>
                  <Chip
                    active={traits.multimodal}
                    onClick={() =>
                      modelHasVision && setTraits({ ...traits, multimodal: !traits.multimodal })
                    }
                  >
                    {t("hire.multimodal")}
                  </Chip>
                </Field>

                {/* Tools (ADR-0036): what the operator declared, granted per
                    agent. The field is absent when nothing is declared, so a
                    box with no tools never promises one. */}
                {tools.length > 0 && (
                  <Field label={t("hire.tools")} hint={t("hire.toolsHint")}>
                    <div className="flex flex-wrap gap-2">
                      {tools.map((tool) => {
                        const held = (traits.tools ?? []).includes(tool.name);
                        return (
                          <Chip
                            key={tool.name}
                            active={held}
                            onClick={() =>
                              setTraits({
                                ...traits,
                                tools: held
                                  ? (traits.tools ?? []).filter((x) => x !== tool.name)
                                  : [...(traits.tools ?? []), tool.name],
                              })
                            }
                          >
                            <span title={tool.description ?? tool.command}>{tool.name}</span>
                          </Chip>
                        );
                      })}
                    </div>
                  </Field>
                )}
              </>
            ) : (
              <Field label={t("hire.brief")} hint={t("hire.briefHint")}>
                <Textarea
                  value={brief}
                  onChange={(e) => setBrief(e.target.value)}
                  placeholder={t("hire.briefPlaceholder")}
                  className="min-h-32"
                />
              </Field>
            )}

            <LivePreview
              traits={traits}
              name={name || picked.name}
              hasBrief={brief.trim().length > 0}
            />

            {error && <p className="text-sm text-destructive">{error}</p>}

            <div className="flex items-center justify-between gap-2 pt-1">
              <Button
                variant="ghost"
                onClick={() => setLevel(level === "expert" ? "tune" : "field")}
              >
                {t("common.back")}
              </Button>
              <div className="flex gap-2">
                {level === "tune" && (
                  <Button variant="outline" onClick={() => setLevel("expert")}>
                    <Sparkles className="h-4 w-4" />
                    {t("hire.expertMode")}
                  </Button>
                )}
                <Button variant="primary" onClick={submit} disabled={busy}>
                  {busy ? t("hire.submitting") : t("hire.submit")}
                </Button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </Dialog>
  );
}

/** One pickable catalog row — a function or a field, drawn identically. */
function CatalogCard({
  icon: Icon,
  name,
  description,
  onClick,
}: {
  icon: typeof Bot;
  name: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group flex flex-col gap-2 rounded-lg border border-border bg-card p-4 text-left transition hover:border-primary/50 hover:shadow-soft cursor-pointer"
    >
      <div className="flex items-center gap-2.5">
        <span className="flex h-9 w-9 items-center justify-center rounded-md bg-primary/10 text-primary">
          <Icon className="h-4.5 w-4.5" />
        </span>
        <span className="font-medium">{name}</span>
        <ChevronRight className="ml-auto h-4 w-4 text-muted-foreground opacity-0 transition group-hover:opacity-100" />
      </div>
      <p className="text-sm leading-snug text-muted-foreground">{description}</p>
    </button>
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
  const t = useT();
  const { formatCents } = useFormats();
  return (
    <div className={cn("rounded-md border border-border bg-muted/40 p-3.5 text-sm")}>
      <p className="leading-relaxed">
        <span className="font-medium">{name}</span> {t(`autonomySays.${traits.autonomy}`)}
        {t("hire.previewReviewing")}
        <span className="font-medium">{t(`strictness.${traits.review_strictness}`)}</span>
        {t("hire.previewStrictness")}
        <span className="font-medium">
          {traits.focus_areas.length ? traits.focus_areas.join(", ") : t("hire.previewNoFocus")}
        </span>
        {t("hire.previewCapped")}
        <span className="mono">{formatCents(traits.monthly_budget_cents)}</span>
        {t("hire.previewPerMonth")}
        <span className="mono">{traits.model}</span>.{traits.multimodal && t("hire.previewLooks")}
        {(traits.tools ?? []).length > 0 &&
          t("hire.previewTools", { tools: (traits.tools ?? []).join(", ") })}
        {hasBrief && t("hire.previewBrief")}
      </p>
    </div>
  );
}
