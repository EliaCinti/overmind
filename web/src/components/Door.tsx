import { useState } from "react";
import { ArrowLeft, KeyRound, LogIn, UserPlus } from "lucide-react";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * The door (M24, ADR-0032), as the owner asked for it: a landing that
 * offers the two ways in, each on its own screen, nobody ever stuck --
 * a back arrow and a title on every step say where you are. Behind it,
 * a living background that never stops drifting (and stops entirely
 * under prefers-reduced-motion).
 *
 * Sign up writes to the one protected store this machine already has:
 * `overmind.sqlite`, names and argon2id hashes, nothing else, nowhere
 * else. The first account created owns the instance; everyone after is
 * a member, and today the only thing that changes is that billing is
 * the owner's.
 */
export function Door({
  mode,
  onEntered,
}: {
  mode: "unclaimed" | "locked";
  onEntered: () => void;
}) {
  const t = useT();
  const [screen, setScreen] = useState<"landing" | "login" | "signup">("landing");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [invite, setInvite] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reset = (next: "landing" | "login" | "signup") => {
    setScreen(next);
    setError(null);
    setPassword("");
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !password) return;
    setBusy(true);
    setError(null);
    try {
      if (screen === "signup") await api.authSignup(name.trim(), password, invite.trim());
      else await api.authLogin(name.trim(), password);
      onEntered();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(
        msg.includes("attempts") || msg.includes("tentativi")
          ? t("door.limited")
          : screen === "signup"
            ? msg.toLowerCase().includes("unauthorized")
              ? t("door.signupTaken")
              : msg
            : t("door.failed"),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="relative flex flex-1 items-center justify-center p-6">
      {/* The living background: three blobs of the app's own hues. */}
      <div aria-hidden className="door-bg">
        <div className="door-blob door-blob-a" />
        <div className="door-blob door-blob-b" />
        <div className="door-blob door-blob-c" />
      </div>

      <div className="relative w-full max-w-sm">
        {screen === "landing" ? (
          <div className="rounded-xl border border-border bg-card/85 p-6 shadow-soft backdrop-blur-sm">
            <span className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <KeyRound className="h-5 w-5" />
            </span>
            <h1 className="mt-4 text-xl font-semibold">{t("door.welcome")}</h1>
            <p className="mt-1 text-sm text-muted-foreground">{t("door.chooseBody")}</p>
            {mode === "unclaimed" && (
              <p className="mt-2 text-xs text-muted-foreground">{t("door.signupFirstHint")}</p>
            )}
            <div className="mt-6 grid gap-3">
              <Button className="w-full" onClick={() => reset("login")}>
                <LogIn className="h-4 w-4" /> {t("door.chooseLogin")}
              </Button>
              <Button variant="outline" className="w-full" onClick={() => reset("signup")}>
                <UserPlus className="h-4 w-4" /> {t("door.chooseSignup")}
              </Button>
            </div>
          </div>
        ) : (
          <div className="rounded-xl border border-border bg-card/85 p-6 shadow-soft backdrop-blur-sm">
            {/* Where you are, and the way back: nobody gets stuck here. */}
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="icon"
                onClick={() => reset("landing")}
                aria-label={t("door.back")}
              >
                <ArrowLeft className="h-4 w-4" />
              </Button>
              <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                {screen === "signup" ? t("door.whereSignup") : t("door.whereLogin")}
              </span>
            </div>
            <h1 className="mt-3 text-lg font-semibold">
              {screen === "signup" ? t("door.signupTitle") : t("door.loginTitle")}
            </h1>
            <p className="mt-1 text-sm text-muted-foreground">
              {screen === "signup" ? t("door.signupBody") : t("door.loginBody")}
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
                  autoComplete={screen === "signup" ? "new-password" : "current-password"}
                />
                {screen === "signup" && (
                  <p className="mt-1 text-xs text-muted-foreground">{t("door.passwordHint")}</p>
                )}
              </Field>
              {screen === "signup" && mode === "locked" && (
                <Field label={t("door.inviteCode")}>
                  <Input
                    value={invite}
                    onChange={(e) => setInvite(e.target.value)}
                    className="font-mono"
                  />
                  <p className="mt-1 text-xs text-muted-foreground">{t("door.inviteHint")}</p>
                </Field>
              )}
              {error && <p className="text-sm text-destructive">{error}</p>}
              <Button
                type="submit"
                className="w-full"
                disabled={busy || !name.trim() || !password}
              >
                {screen === "signup" ? t("door.signup") : t("door.login")}
              </Button>
            </form>
          </div>
        )}
      </div>
    </div>
  );
}
