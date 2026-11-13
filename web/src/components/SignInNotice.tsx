import { KeyRound } from "lucide-react";
import type { Economy } from "../lib/api";
import { Button } from "./ui/button";
import { openAccountsDialog } from "../lib/events";
import { useT } from "../lib/i18n";

/**
 * The founder is told what is missing where they are looking (M22) — and can
 * fix it from here by opening the Accounts panel (M34, ADR-0051).
 *
 * Shown only for `unknown_kind === "not_signed_in"`: a custom adapter's
 * unknown is deliberate, and an unreadable probe is not an invitation to sign
 * in. This is the *urgent* case — nobody can pay at all, so it belongs at the
 * top of every page.
 */
export function SignInNotice({
  economy,
  onOpenAccounts,
}: {
  economy: Economy | null;
  onSignedIn?: () => void;
  onOpenAccounts?: () => void;
}) {
  const t = useT();

  const wanted =
    economy && economy.kind === "unknown" && economy.unknown_kind === "not_signed_in";
  if (!wanted) return null;

  const handleClick = () => {
    if (onOpenAccounts) {
      onOpenAccounts();
    } else {
      openAccountsDialog("anthropic");
    }
  };

  return (
    <div className="mx-auto w-full max-w-2xl px-4 pt-4">
      <div className="rounded-lg border border-border bg-card p-4 shadow-soft">
        <div className="flex items-start gap-3">
          <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <KeyRound className="h-4 w-4" />
          </span>
          <div className="min-w-0 flex-1 space-y-2.5">
            <div>
              <p className="text-sm font-medium">{t("economy.notSignedIn")}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {t("economy.notSignedInBody")}
              </p>
            </div>

            <div className="pt-1">
              <Button size="sm" variant="primary" onClick={handleClick}>
                <KeyRound className="h-4 w-4" />
                {t("economy.notSignedInAction")}
              </Button>
            </div>

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
          </div>
        </div>
      </div>
    </div>
  );
}
