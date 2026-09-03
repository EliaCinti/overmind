import { useState } from "react";
import { KeyRound } from "lucide-react";
import type { Economy } from "../lib/api";
import { ConnectPlan } from "./ConnectPlan";
import { useT } from "../lib/i18n";

/**
 * The founder is told what is missing where they are looking (M22) — and can
 * fix it from here (M23), through [`ConnectPlan`](./ConnectPlan.tsx).
 *
 * Shown only for `unknown_kind === "not_signed_in"`: a custom adapter's
 * unknown is deliberate, and an unreadable probe is not an invitation to sign
 * in. This is the *urgent* case — nobody can pay at all, so it belongs at the
 * top of every page. Choosing between two payers that both work is not urgent
 * and lives in the org view instead, next to who is being billed.
 *
 * Once a flow has started the card stays regardless of the economy: the
 * moment the sign-in lands, `economy` stops being "not signed in", and a card
 * that unmounted there would take the good news with it — measured, the first
 * time someone completed the flow and asked where the success message went.
 */
export function SignInNotice({
  economy,
  onSignedIn,
}: {
  economy: Economy | null;
  onSignedIn: () => void;
}) {
  const t = useT();
  const [engaged, setEngaged] = useState(false);

  const wanted =
    economy && economy.kind === "unknown" && economy.unknown_kind === "not_signed_in";
  if (!engaged && !wanted) return null;

  return (
    <div className="mx-auto w-full max-w-2xl px-4 pt-4">
      <div className="rounded-lg border border-border bg-card p-4 shadow-soft">
        <div className="flex items-start gap-3">
          <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <KeyRound className="h-4 w-4" />
          </span>
          <div className="min-w-0 flex-1 space-y-2.5">
            {/* Only while it is still true. Once the sign-in lands the economy
                stops being "nobody can pay", and leaving the heading up would
                print "the agents cannot pay yet" directly above the line
                saying they can. */}
            {wanted && (
              <div>
                <p className="text-sm font-medium">{t("economy.notSignedIn")}</p>
                <p className="mt-0.5 text-xs text-muted-foreground">
                  {t("economy.notSignedInBody")}
                </p>
              </div>
            )}

            {/* The guided way: the subscription, from the product. */}
            <ConnectPlan onSignedIn={onSignedIn} onEngaged={() => setEngaged(true)} />

            {/* The manual ways stay, quieter, below the guided one. */}
            {wanted && (
              <details className="pt-1">
                <summary className="cursor-pointer text-[11px] text-muted-foreground">
                  {t("economy.notSignedInKey")}
                </summary>
                <code className="mt-1 block overflow-x-auto rounded bg-muted px-2 py-1 font-mono text-[11px]">
                  export ANTHROPIC_API_KEY=sk-ant-…
                </code>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  {t("economy.notSignedInPlanHost")}
                </p>
              </details>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
