import { useState } from "react";
import { CreditCard } from "lucide-react";
import type { Economy } from "../lib/api";
import { Button } from "./ui/button";
import { openAccountsDialog } from "../lib/events";
import { useT } from "../lib/i18n";

/**
 * Who pays, asked rather than assumed (ADR-0037, ADR-0051).
 *
 * Shown only in the one state where a person is being billed against their
 * expectation: a subscription exists and an API key is winning.
 * Points to the accounts dialog to choose which account is active.
 */
export function PayerNotice({
  economy,
  onOpenAccounts,
}: {
  economy: Economy | null;
  onChanged?: () => void;
  onOpenAccounts?: () => void;
}) {
  const t = useT();
  const [dismissed, setDismissed] = useState(false);

  if (dismissed) return null;
  if (!economy || economy.kind !== "key" || !economy.overrides_login) return null;

  const handleOpen = () => {
    if (onOpenAccounts) {
      onOpenAccounts();
    } else {
      openAccountsDialog("anthropic");
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
          <div className="flex flex-wrap items-center gap-2">
            <Button size="sm" onClick={handleOpen}>
              {t("economy.manageAccounts")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setDismissed(true)}>
              {t("economy.keepKey")}
            </Button>
          </div>
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
