import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { api } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { useT } from "../lib/i18n";

/**
 * The owner mints a one-time invite (M25, ADR-0033). The raw code exists
 * only in this dialog and in the hand it is copied into: the server stores
 * its hash and nothing else, so closing without copying discards it forever
 * -- mint another.
 */
export function InviteDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const t = useT();
  const [code, setCode] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);

  const mint = async () => {
    setBusy(true);
    try {
      const r = await api.authMintInvite();
      setCode(r.invite);
      setCopied(false);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        onOpenChange(o);
        if (!o) {
          setCode(null);
          setCopied(false);
        }
      }}
      title={t("door.inviteMint")}
    >
      <div className="space-y-3 p-4">
        {code === null ? (
          <Button onClick={mint} disabled={busy}>
            {t("door.inviteMint")}
          </Button>
        ) : (
          <>
            <p className="text-sm text-muted-foreground">{t("door.inviteMinted")}</p>
            <div className="flex items-center gap-2">
              <code className="flex-1 overflow-x-auto rounded bg-muted px-2 py-1.5 font-mono text-xs">
                {code}
              </code>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  navigator.clipboard.writeText(code).then(() => setCopied(true));
                }}
              >
                {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                {copied ? t("door.inviteCopied") : t("door.inviteCopy")}
              </Button>
            </div>
          </>
        )}
      </div>
    </Dialog>
  );
}
