import { useEffect, useState } from "react";
import { Check, Copy, Plug, Trash2 } from "lucide-react";
import type { CompanyToken, IssuedToken } from "../lib/api";
import { api } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";
import { useT } from "../lib/i18n";
import { cn } from "../lib/utils";

/**
 * Connections — the credentials this company has handed to things outside
 * Overmind (ADR-0028): a Claude Code session in an editor, a script.
 *
 * Two things make this more than a token list.
 *
 * **What you actually need is the config, not the secret.** Nobody wants a
 * UUID; they want the four lines that make their editor able to file work here.
 * So the secret is shown once, already inside the MCP configuration it belongs
 * in, ready to copy.
 *
 * **A credential you cannot tell apart is one you will never revoke.** Hence
 * the label, and hence "never used" being stated rather than left blank: the
 * question people actually have when they find an old token is whether anything
 * still depends on it.
 */
export function ConnectionsDialog({
  open,
  onOpenChange,
  companyId,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  companyId: string;
}) {
  const t = useT();
  const [tokens, setTokens] = useState<CompanyToken[] | null>(null);
  const [label, setLabel] = useState("");
  const [issued, setIssued] = useState<IssuedToken | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = () =>
    api
      .listTokens(companyId)
      .then(setTokens)
      .catch(() => setTokens([]));

  useEffect(() => {
    if (!open) return;
    // The secret belongs to the moment it was created, not to the dialog.
    setIssued(null);
    setLabel("");
    setError(null);
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, companyId]);

  const create = async () => {
    if (!label.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setIssued(await api.createToken(companyId, label.trim()));
      setLabel("");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : t("common.failed"));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: string) => {
    await api.revokeToken(id).catch(() => {});
    await load();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("connections.title")}
      description={t("connections.desc")}
    >
      <div className="flex flex-col gap-5">
        {issued ? (
          <IssuedConfig issued={issued} onDone={() => setIssued(null)} />
        ) : (
          <Field label={t("connections.label")} hint={t("connections.labelHint")}>
            {/* The button rides with the input, not with the field: the hint
                sits under both, and an `items-end` row would push it below the
                line it belongs to. */}
            <div className="flex gap-2">
              <Input
                autoFocus
                className="flex-1"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder={t("connections.labelPlaceholder")}
                onKeyDown={(e) => e.key === "Enter" && create()}
              />
              <Button variant="primary" onClick={create} disabled={busy || !label.trim()}>
                {busy ? t("common.working") : t("connections.create")}
              </Button>
            </div>
          </Field>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}

        {tokens && tokens.length > 0 && (
          <div className="flex flex-col gap-1">
            {tokens.map((tok) => (
              <TokenRow key={tok.id} token={tok} onRevoke={() => revoke(tok.id)} />
            ))}
          </div>
        )}
        {tokens && tokens.length === 0 && !issued && (
          <p className="text-sm text-muted-foreground">{t("connections.empty")}</p>
        )}
      </div>
    </Dialog>
  );
}

function TokenRow({ token, onRevoke }: { token: CompanyToken; onRevoke: () => void }) {
  const t = useT();
  const revoked = token.revoked_at !== null;
  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-md border border-border px-3 py-2 text-sm",
        revoked && "opacity-50",
      )}
    >
      <Plug className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span className={cn("truncate", revoked && "line-through")}>{token.label}</span>
      <span className="ml-auto shrink-0 text-xs text-muted-foreground">
        {revoked
          ? t("connections.revoked")
          : token.last_used_at
            ? t("connections.lastUsed", { when: new Date(token.last_used_at).toLocaleString() })
            : t("connections.neverUsed")}
      </span>
      {!revoked && (
        <button
          onClick={onRevoke}
          aria-label={t("connections.revoke")}
          title={t("connections.revoke")}
          className="shrink-0 rounded-md p-1 text-muted-foreground transition hover:bg-destructive/10 hover:text-destructive"
        >
          <Trash2 className="h-4 w-4" />
        </button>
      )}
    </div>
  );
}

/** The secret, once — inside the configuration it is for. */
function IssuedConfig({ issued, onDone }: { issued: IssuedToken; onDone: () => void }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const config = JSON.stringify(
    {
      mcpServers: {
        overmind: {
          type: "http",
          url: `${window.location.origin}/mcp`,
          headers: { Authorization: `Bearer ${issued.token}` },
        },
      },
    },
    null,
    2,
  );

  const copy = async () => {
    await navigator.clipboard.writeText(config).catch(() => {});
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-primary/40 bg-primary/5 p-3">
      <p className="text-sm">
        <span className="font-medium">{issued.label}</span> — {t("connections.onceOnly")}
      </p>
      <pre className="mono max-h-52 overflow-auto rounded-md bg-card p-3 text-xs leading-relaxed">
        {config}
      </pre>
      <div className="flex items-center gap-2">
        <Button variant="primary" size="sm" onClick={copy}>
          {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
          {copied ? t("connections.copied") : t("connections.copy")}
        </Button>
        <Button variant="ghost" size="sm" onClick={onDone}>
          {t("connections.done")}
        </Button>
      </div>
    </div>
  );
}
