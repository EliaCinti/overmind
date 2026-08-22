import { useState } from "react";
import { api, ApiError } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * Deleting a company is the one verb in the app with no undo (ADR-0034):
 * the rows, the brain and the debris on disk all go. So the confirmation
 * is not a click but a copy — type the company's name, exactly. A running
 * session holds the door (409), and the dialog says so instead of failing
 * wordlessly.
 */
export function DeleteCompanyDialog({
  open,
  onOpenChange,
  companyId,
  companyName,
  onDeleted,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  companyId: string;
  companyName: string;
  onDeleted: () => void;
}) {
  const t = useT();
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const armed = typed.trim() === companyName;

  const close = (o: boolean) => {
    onOpenChange(o);
    if (!o) {
      setTyped("");
      setError(null);
    }
  };

  const destroy = async () => {
    if (!armed || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.deleteCompany(companyId);
      close(false);
      onDeleted();
    } catch (e) {
      setError(
        e instanceof ApiError && e.status === 409 ? t("nav.deleteCompanyBusy") : String(e),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={close} title={t("nav.deleteCompany")}>
      <div className="space-y-3 p-4">
        <p className="text-sm text-muted-foreground">
          {t("nav.deleteCompanyWarning", { name: companyName })}
        </p>
        <Input
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          placeholder={companyName}
          aria-label={t("nav.deleteCompanyType")}
          onKeyDown={(e) => {
            if (e.key === "Enter") void destroy();
          }}
        />
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={() => close(false)}>
            {t("common.cancel")}
          </Button>
          <Button variant="destructive" disabled={!armed || busy} onClick={destroy}>
            {t("nav.deleteCompanyConfirm")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
