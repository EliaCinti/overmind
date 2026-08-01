import { useState } from "react";
import { connectRepo } from "../lib/repo";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * Connect a git repo to a company — the one thing `code` tasks need and
 * `knowledge` tasks don't (ADR-0017). Offered when you actually reach for it,
 * not as a toll gate at first run.
 *
 * Creates the project + primary workspace + the default goal that code tasks
 * attach to; the same three calls the first-run step makes.
 */
export function ConnectRepoDialog({
  open,
  onOpenChange,
  companyId,
  onConnected,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  companyId: string;
  onConnected: () => void;
}) {
  const t = useT();
  const [cwd, setCwd] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!cwd.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await connectRepo(companyId, cwd.trim());
      onConnected();
      onOpenChange(false);
      setCwd("");
    } catch (e) {
      setError(e instanceof Error ? e.message : t("repo.failed"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("repo.title")}
      description={t("repo.desc")}
    >
      <div className="flex flex-col gap-4">
        <Field label={t("repo.path")} hint={t("repo.pathHint")}>
          <Input
            autoFocus
            value={cwd}
            onChange={(e) => setCwd(e.target.value)}
            placeholder={t("repo.pathPlaceholder")}
            className="mono"
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </Field>
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" onClick={submit} disabled={busy || !cwd.trim()}>
            {busy ? t("repo.submitting") : t("repo.submit")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
