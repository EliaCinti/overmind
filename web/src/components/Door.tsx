import { useState } from "react";
import { KeyRound, LogIn } from "lucide-react";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * The door (M24, ADR-0032). Two faces of one screen: claim, when no owner
 * exists yet, and login, every time after. Rendered instead of the app, not
 * on top of it -- nothing behind the door is fetched before a session
 * exists.
 */
export function Door({
  mode,
  onEntered,
}: {
  mode: "unclaimed" | "locked";
  onEntered: () => void;
}) {
  const t = useT();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const claim = mode === "unclaimed";

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !password) return;
    setBusy(true);
    setError(null);
    try {
      if (claim) await api.authClaim(name.trim(), password);
      else await api.authLogin(name.trim(), password);
      onEntered();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg.includes("attempts") || msg.includes("tentativi")
        ? t("door.limited")
        : claim
          ? msg
          : t("door.failed"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <div className="w-full max-w-sm">
        <div className="rounded-xl border border-border bg-card p-6 shadow-soft">
          <span className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
            {claim ? <KeyRound className="h-5 w-5" /> : <LogIn className="h-5 w-5" />}
          </span>
          <h1 className="mt-4 text-lg font-semibold">
            {claim ? t("door.claimTitle") : t("door.loginTitle")}
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {claim ? t("door.claimBody") : t("door.loginBody")}
          </p>
          <form className="mt-5 space-y-4" onSubmit={submit}>
            <Field label={t("door.name")}>
              <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                autoFocus
                autoComplete="username"
              />
            </Field>
            <Field label={t("door.password")}>
              <Input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete={claim ? "new-password" : "current-password"}
              />
              {claim && (
                <p className="mt-1 text-xs text-muted-foreground">{t("door.passwordHint")}</p>
              )}
            </Field>
            {error && <p className="text-sm text-destructive">{error}</p>}
            <Button type="submit" className="w-full" disabled={busy || !name.trim() || !password}>
              {claim ? t("door.claim") : t("door.login")}
            </Button>
          </form>
        </div>
      </div>
    </div>
  );
}
