import { KeyRound } from "lucide-react";
import type { Economy } from "../lib/api";
import { useT } from "../lib/i18n";

/**
 * The founder is told what is missing where they are looking (M22).
 *
 * Before this, an unsigned agent CLI was a fact the server knew
 * (`economy.unknown_kind === "not_signed_in"`) and the person discovered by
 * burning a failed turn. Shown only for that one kind: a custom adapter's
 * unknown is deliberate, and an unreadable probe is not an invitation to sign
 * in — warning about a remedy that may not apply is how people learn to
 * ignore warnings.
 */
export function SignInNotice({ economy }: { economy: Economy | null }) {
  const t = useT();
  if (!economy || economy.kind !== "unknown" || economy.unknown_kind !== "not_signed_in") {
    return null;
  }
  return (
    <div className="mx-auto w-full max-w-2xl px-4 pt-4">
      <div className="rounded-lg border border-border bg-card p-4 shadow-soft">
        <div className="flex items-start gap-3">
          <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <KeyRound className="h-4 w-4" />
          </span>
          <div className="min-w-0 space-y-2.5">
            <div>
              <p className="text-sm font-medium">{t("economy.notSignedIn")}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">{t("economy.notSignedInBody")}</p>
            </div>
            <div>
              <p className="text-xs font-medium">{t("economy.notSignedInKey")}</p>
              <code className="mt-1 block overflow-x-auto rounded bg-muted px-2 py-1 font-mono text-[11px]">
                export ANTHROPIC_API_KEY=sk-ant-…
              </code>
            </div>
            <div>
              <p className="text-xs font-medium">{t("economy.notSignedInPlan")}</p>
              <code className="mt-1 block overflow-x-auto rounded bg-muted px-2 py-1 font-mono text-[11px]">
                docker compose exec --user agent overmind claude setup-token
              </code>
              <p className="mt-1 text-[11px] text-muted-foreground">{t("economy.notSignedInPlanHost")}</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
