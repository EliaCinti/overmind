import { useEffect, useState } from "react";
import { Archive, Download, Loader2, Trash2 } from "lucide-react";
import type { Archive as ArchiveFile } from "../lib/api";
import { api, ApiError } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * Backups — the way out for everything this instance is (M31, ADR-0044).
 *
 * Two things shape this screen.
 *
 * **The passphrase is asked only when there is something to seal.** The server
 * says whether a sign-in would travel; when none would, the field is not shown
 * at all, because a field nobody needs is a field nobody should have to reason
 * about (UX.md).
 *
 * **An archive is the whole instance**, so the list says how big and how old
 * each one is — the two facts you need to decide what to keep — and deleting
 * one asks once, because the file may be the only copy of a company's memory.
 */
export function BackupDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
}) {
  const t = useT();
  const [archives, setArchives] = useState<ArchiveFile[] | null>(null);
  const [sealed, setSealed] = useState(false);
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [justMade, setJustMade] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);

  const load = () =>
    api
      .backups()
      .then((r) => {
        setArchives(r.archives);
        setSealed(r.sign_in_travels);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));

  useEffect(() => {
    if (open) load();
  }, [open]);

  const close = (o: boolean) => {
    onOpenChange(o);
    if (!o) {
      setPassphrase("");
      setError(null);
      setJustMade(null);
      setConfirming(null);
    }
  };

  const exportNow = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setJustMade(null);
    try {
      const made = await api.backupCreate(sealed ? passphrase : undefined);
      setJustMade(made.name);
      setPassphrase("");
      await load();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (name: string) => {
    setBusy(true);
    setError(null);
    try {
      await api.backupDelete(name);
      setConfirming(null);
      if (justMade === name) setJustMade(null);
      await load();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const armed = !sealed || passphrase.trim().length >= 12;

  return (
    <Dialog
      open={open}
      onOpenChange={close}
      title={t("backup.title")}
      description={t("backup.body")}
    >
      <div className="flex flex-col gap-4">
        {sealed && (
          <Field label={t("backup.passphrase")} hint={t("backup.passphraseHint")}>
            <Input
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              placeholder={t("backup.passphrasePlaceholder")}
              autoComplete="new-password"
            />
          </Field>
        )}

        <div className="flex items-center gap-2">
          <Button onClick={exportNow} disabled={busy || !armed}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Archive className="h-4 w-4" />}
            {t("backup.export")}
          </Button>
          {justMade && (
            <a
              href={api.backupHref(justMade)}
              download={justMade}
              className="inline-flex items-center gap-1.5 text-sm text-primary underline-offset-4 hover:underline"
            >
              <Download className="h-4 w-4" />
              {t("backup.download")}
            </a>
          )}
        </div>

        {error && <p className="text-sm text-destructive">{error}</p>}

        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("backup.kept")}</span>
          {archives === null ? (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          ) : archives.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("backup.none")}</p>
          ) : (
            <ul className="flex flex-col divide-y divide-border/60 rounded-2xl border border-border/60">
              {archives.map((a) => (
                <li key={a.name} className="flex items-center gap-3 px-3 py-2">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm">{a.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {size(a.bytes)}
                      {a.created_at ? ` · ${when(a.created_at)}` : ""}
                    </p>
                  </div>
                  <a
                    href={api.backupHref(a.name)}
                    download={a.name}
                    className="text-muted-foreground hover:text-foreground"
                    aria-label={t("backup.download")}
                    title={t("backup.download")}
                  >
                    <Download className="h-4 w-4" />
                  </a>
                  {confirming === a.name ? (
                    <div className="flex items-center gap-1.5">
                      <Button
                        size="sm"
                        variant="destructive"
                        disabled={busy}
                        onClick={() => remove(a.name)}
                      >
                        {t("backup.deleteConfirm")}
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setConfirming(null)}>
                        {t("common.cancel")}
                      </Button>
                    </div>
                  ) : (
                    <button
                      type="button"
                      className="text-muted-foreground hover:text-destructive"
                      aria-label={t("backup.delete")}
                      title={t("backup.delete")}
                      onClick={() => setConfirming(a.name)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
          <p className="text-xs text-muted-foreground">{t("backup.restoreHint")}</p>
        </div>
      </div>
    </Dialog>
  );
}

function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

function when(iso: string): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? iso : at.toLocaleString();
}
