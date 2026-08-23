import { useEffect, useState } from "react";
import { CreditCard, Loader2 } from "lucide-react";
import type { Economy } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { useT } from "../lib/i18n";

/**
 * Who pays, asked rather than assumed (ADR-0037).
 *
 * Shown only in the one state where a person is being billed against their
 * expectation: a claude.ai login is signed in and an API key is winning. The
 * CLI says so in a log line; this is the same fact, where they are looking,
 * with the remedy as a button — Overmind keeps the key out of the agents'
 * environment and asks the CLI again. Nobody has to find a shell.
 *
 * Rendered on every page, next to the sign-in notice: the bill is not a
 * property of the org chart.
 */
export function PayerNotice({ economy, onChanged }: { economy: Economy | null; onChanged: () => void }) {
  const t = useT();
  const [flow, setFlow] = useState<
    | { step: "idle" }
    | { step: "working" }
    | { step: "done"; plan: string | null }
    | { step: "failed"; why: string }
    | { step: "dismissed" }
  >({ step: "idle" });

  // Success outlives its cause — once the plan pays, `economy` stops being a
  // key and this card would vanish before the person read the good news —
  // but not by much: a sentence that stays after it is read becomes
  // furniture, so it leaves on its own.
  useEffect(() => {
    if (flow.step !== "done") return;
    const id = window.setTimeout(() => setFlow({ step: "dismissed" }), 6000);
    return () => window.clearTimeout(id);
  }, [flow.step]);

  if (flow.step === "done") {
    return (
      <Shell>
        <p className="text-sm font-medium text-primary">
          {flow.plan
            ? t("economy.letPlanPayDoneWithPlan", { plan: flow.plan })
            : t("economy.letPlanPayDone")}
        </p>
      </Shell>
    );
  }
  if (flow.step === "dismissed") return null;
  if (!economy || economy.kind !== "key" || !economy.overrides_login) return null;

  const choose = async () => {
    setFlow({ step: "working" });
    try {
      const r = await api.payWith("plan");
      setFlow({ step: "done", plan: r.economy.kind === "subscription" ? r.economy.plan : null });
      onChanged();
    } catch (e) {
      setFlow({ step: "failed", why: e instanceof Error ? e.message : String(e) });
      onChanged();
    }
  };

  return (
    <Shell>
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-[var(--color-status-in_review)]/15 text-[var(--color-status-in_review)]">
          <CreditCard className="h-4 w-4" />
        </span>
        <div className="min-w-0 flex-1 space-y-2.5">
          <div>
            <p className="text-sm font-medium">{t("economy.keyOverridesLogin")}</p>
            <p className="mt-0.5 text-xs text-muted-foreground">{t("economy.letPlanPayBody")}</p>
          </div>
          {flow.step === "working" ? (
            <p className="flex items-center gap-2 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("economy.letPlanPayWorking")}
            </p>
          ) : (
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" onClick={choose}>
                {t("economy.letPlanPay")}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setFlow({ step: "dismissed" })}>
                {t("economy.keepKey")}
              </Button>
            </div>
          )}
          {flow.step === "failed" && (
            <div className="space-y-1 text-xs text-[var(--color-status-blocked)]">
              <p className="font-medium">{t("economy.letPlanPayFailed")}</p>
              <p className="break-words font-mono text-[11px]">{flow.why}</p>
            </div>
          )}
        </div>
      </div>
    </Shell>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="mx-auto w-full max-w-2xl px-4 pt-4">
      <div className="rounded-lg border border-border bg-card p-4 shadow-soft">{children}</div>
    </div>
  );
}
