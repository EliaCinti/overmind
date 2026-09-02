import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ArrowLeft, Archive, KeyRound, Loader2, Lock, LogIn, Ticket, Upload, User, UserPlus } from "lucide-react";
import type { StagedRestore } from "../lib/api";
import { api, ApiError } from "../lib/api";
import { useT } from "../lib/i18n";
import { cn } from "../lib/utils";
import { DoorBackground } from "./DoorBackground";

/**
 * The door (M24/M25), third pass at the owner's word: a complex living
 * background (the constellation -- the product's own shape), and forms that
 * answer back -- round, glassy, animated between screens, shaking on a
 * refusal. Navigation unchanged: a back arrow and a location label on every
 * step, nobody stuck.
 */

/** A round input with its icon living inside. */
function RoundInput({
  icon: Icon,
  ...props
}: { icon: React.ComponentType<{ className?: string }> } & React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <div className="relative">
      <Icon className="pointer-events-none absolute left-4 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground/70" />
      <input
        {...props}
        className={cn(
          "h-12 w-full rounded-full border border-input bg-background/70 pl-11 pr-4 text-sm",
          "placeholder:text-muted-foreground/50 outline-none",
          "transition-all duration-200",
          "focus:border-primary focus:bg-background focus:shadow-[0_0_0_4px_var(--ring-soft,rgba(124,92,255,0.15))]",
          props.className,
        )}
      />
    </div>
  );
}

/** A round button that physically answers the hand. */
function RoundButton({
  variant = "primary",
  className,
  children,
  ...props
}: {
  variant?: "primary" | "outline" | "ghost";
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <motion.button
      whileHover={{ scale: props.disabled ? 1 : 1.02 }}
      whileTap={{ scale: props.disabled ? 1 : 0.97 }}
      transition={{ type: "spring", stiffness: 500, damping: 28 }}
      {...(props as object)}
      className={cn(
        "inline-flex h-12 w-full cursor-pointer items-center justify-center gap-2 rounded-full text-sm font-semibold",
        "transition-colors duration-200 outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        "disabled:pointer-events-none disabled:opacity-50",
        variant === "primary" &&
          "bg-primary text-primary-foreground shadow-[0_8px_24px_-10px_var(--ring-soft,rgba(124,92,255,0.6))] hover:brightness-110",
        variant === "outline" &&
          "border border-border bg-background/60 hover:border-primary/60 hover:bg-background",
        variant === "ghost" && "w-auto px-3 hover:bg-muted",
        className,
      )}
    >
      {children}
    </motion.button>
  );
}

const slide = {
  initial: { opacity: 0, x: 28, filter: "blur(4px)" },
  animate: { opacity: 1, x: 0, filter: "blur(0px)" },
  exit: { opacity: 0, x: -28, filter: "blur(4px)" },
  transition: { duration: 0.22, ease: [0.16, 1, 0.3, 1] as const },
};

export function Door({
  mode,
  onEntered,
}: {
  mode: "unclaimed" | "locked";
  onEntered: () => void;
}) {
  const t = useT();
  const [screen, setScreen] = useState<"landing" | "login" | "signup" | "restore">("landing");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [invite, setInvite] = useState("");
  const [setupCode, setSetupCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const [busy, setBusy] = useState(false);

  const [restoreFile, setRestoreFile] = useState<File | null>(null);
  const [restorePassphrase, setRestorePassphrase] = useState("");
  const [restoreNeedsPassphrase, setRestoreNeedsPassphrase] = useState(false);
  const [restoreSkipToken, setRestoreSkipToken] = useState(false);
  const [restoreBusy, setRestoreBusy] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [restoreDone, setRestoreDone] = useState<StagedRestore | null>(null);

  const go = (next: "landing" | "login" | "signup" | "restore") => {
    setScreen(next);
    setError(null);
    setPassword("");
  };

  const runRestore = async () => {
    if (!restoreFile || restoreBusy) return;
    setRestoreBusy(true);
    setRestoreError(null);
    try {
      const staged = await api.restore(
        restoreFile,
        setupCode.trim(),
        restoreNeedsPassphrase ? restorePassphrase.trim() || undefined : undefined,
        restoreSkipToken,
      );
      setRestoreDone(staged);
    } catch (e) {
      // The server only learns whether the archive is sealed by opening it,
      // so the first attempt is what asks: this turns that refusal into the
      // passphrase field appearing, instead of a dead end.
      if (e instanceof ApiError && !restoreNeedsPassphrase && /passphrase/i.test(e.message)) {
        setRestoreNeedsPassphrase(true);
      } else {
        setRestoreError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      setRestoreBusy(false);
    }
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !password) return;
    setBusy(true);
    setError(null);
    try {
      if (screen === "signup") {
        // On an instance nobody owns, the first account is the *claim*, and it
        // costs the setup code. Signing up here used to make you the owner
        // without one (ADR-0045).
        if (mode === "unclaimed") await api.authClaim(name.trim(), password, setupCode.trim());
        else await api.authSignup(name.trim(), password, invite.trim());
      } else await api.authLogin(name.trim(), password);
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
      setShake((n) => n + 1);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="relative flex flex-1 items-center justify-center p-6">
      <DoorBackground />

      <motion.div
        key={shake}
        animate={shake ? { x: [0, -8, 8, -5, 5, -2, 0] } : {}}
        transition={{ duration: 0.4 }}
        className="relative w-full max-w-sm"
      >
        <div className="overflow-hidden rounded-[2rem] border border-border/70 bg-card/75 p-7 shadow-pop backdrop-blur-xl">
          <AnimatePresence mode="wait" initial={false}>
            {screen === "landing" ? (
              <motion.div key="landing" {...slide}>
                <motion.span
                  className="flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10 text-primary"
                  initial={{ scale: 0.8, opacity: 0 }}
                  animate={{ scale: 1, opacity: 1 }}
                  transition={{ type: "spring", stiffness: 300, damping: 20 }}
                >
                  <KeyRound className="h-5.5 w-5.5" />
                </motion.span>
                <h1 className="mt-4 text-2xl font-semibold tracking-tight">{t("door.welcome")}</h1>
                <p className="mt-1.5 text-sm text-muted-foreground">{t("door.chooseBody")}</p>
                {mode === "unclaimed" && (
                  <p className="mt-2 rounded-2xl bg-primary/5 px-3 py-2 text-xs text-muted-foreground">
                    {t("door.signupFirstHint")}
                  </p>
                )}
                <div className="mt-6 grid gap-3">
                  <RoundButton onClick={() => go("login")}>
                    <LogIn className="h-4 w-4" /> {t("door.chooseLogin")}
                  </RoundButton>
                  <RoundButton variant="outline" onClick={() => go("signup")}>
                    <UserPlus className="h-4 w-4" /> {t("door.chooseSignup")}
                  </RoundButton>
                </div>
                {mode === "unclaimed" && (
                  <button
                    type="button"
                    onClick={() => go("restore")}
                    className="mt-4 w-full text-center text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                  >
                    {t("door.restoreOffer")}
                  </button>
                )}
              </motion.div>
            ) : screen === "restore" ? (
              <motion.div key="restore" {...slide}>
                <div className="flex items-center gap-2">
                  <RoundButton
                    type="button"
                    variant="ghost"
                    onClick={() => go("landing")}
                    aria-label={t("door.back")}
                    className="h-9"
                  >
                    <ArrowLeft className="h-4 w-4" />
                  </RoundButton>
                  <span className="text-xs font-medium text-muted-foreground">
                    {t("backup.restoreTitle")}
                  </span>
                </div>

                {restoreDone ? (
                  <div className="mt-5 flex flex-col items-center gap-3 text-center">
                    <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
                      <Archive className="h-5.5 w-5.5" />
                    </span>
                    <p className="text-sm">{t("backup.restoreDone")}</p>
                  </div>
                ) : (
                  <div className="mt-5 flex flex-col gap-4">
                    <p className="text-sm text-muted-foreground">{t("backup.restoreBody")}</p>

                    <label className="flex h-24 cursor-pointer flex-col items-center justify-center gap-1.5 rounded-2xl border border-dashed border-input bg-background/50 text-center transition-colors hover:border-primary/60">
                      <Upload className="h-5 w-5 text-muted-foreground" />
                      <span className="px-3 text-xs text-muted-foreground">
                        {restoreFile
                          ? t("backup.restoreChosen", { name: restoreFile.name })
                          : t("backup.restoreChoose")}
                      </span>
                      <input
                        type="file"
                        accept=".tar.gz,application/gzip"
                        className="hidden"
                        onChange={(e) => {
                          setRestoreFile(e.target.files?.[0] ?? null);
                          setRestoreError(null);
                          setRestoreNeedsPassphrase(false);
                        }}
                      />
                    </label>

                    <div className="flex flex-col gap-2">
                      <RoundInput
                        icon={KeyRound}
                        value={setupCode}
                        onChange={(e) => setSetupCode(e.target.value)}
                        placeholder={t("door.setupCode")}
                        className="font-mono"
                        aria-label={t("door.setupCode")}
                      />
                      <p className="pl-1 text-xs text-muted-foreground">{t("door.setupHint")}</p>
                    </div>

                    {restoreNeedsPassphrase && (
                      <div className="flex flex-col gap-2">
                        <p className="pl-1 text-xs text-muted-foreground">
                          {t("backup.restoreSealed")}
                        </p>
                        <RoundInput
                          icon={KeyRound}
                          type="password"
                          value={restorePassphrase}
                          onChange={(e) => setRestorePassphrase(e.target.value)}
                          placeholder={t("backup.passphrase")}
                          aria-label={t("backup.passphrase")}
                          disabled={restoreSkipToken}
                        />
                        <label className="flex items-center gap-2 pl-1 text-xs text-muted-foreground">
                          <input
                            type="checkbox"
                            checked={restoreSkipToken}
                            onChange={(e) => setRestoreSkipToken(e.target.checked)}
                          />
                          {t("backup.restoreSkipToken")}
                        </label>
                      </div>
                    )}

                    {restoreError && <p className="pl-1 text-sm text-destructive">{restoreError}</p>}

                    <RoundButton
                      type="button"
                      onClick={runRestore}
                      disabled={
                        !restoreFile ||
                        restoreBusy ||
                        (restoreNeedsPassphrase &&
                          !restoreSkipToken &&
                          restorePassphrase.trim().length < 12)
                      }
                    >
                      {restoreBusy ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <Archive className="h-4 w-4" />
                      )}
                      {restoreBusy ? t("backup.restoreWorking") : t("backup.restoreGo")}
                    </RoundButton>
                  </div>
                )}
              </motion.div>
            ) : (
              <motion.div key={screen} {...slide}>
                <div className="flex items-center gap-2">
                  <RoundButton
                    type="button"
                    variant="ghost"
                    onClick={() => go("landing")}
                    aria-label={t("door.back")}
                    className="h-9"
                  >
                    <ArrowLeft className="h-4 w-4" />
                  </RoundButton>
                  <span className="text-[11px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                    {screen === "signup" ? t("door.whereSignup") : t("door.whereLogin")}
                  </span>
                </div>
                <h1 className="mt-3 text-xl font-semibold tracking-tight">
                  {screen === "signup" ? t("door.signupTitle") : t("door.loginTitle")}
                </h1>
                <p className="mt-1 text-sm text-muted-foreground">
                  {screen === "signup" ? t("door.signupBody") : t("door.loginBody")}
                </p>
                <form className="mt-5 space-y-3.5" onSubmit={submit}>
                  <RoundInput
                    icon={User}
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder={t("door.name")}
                    autoFocus
                    autoComplete="username"
                    aria-label={t("door.name")}
                  />
                  <div>
                    <RoundInput
                      icon={Lock}
                      type="password"
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      placeholder={t("door.password")}
                      autoComplete={screen === "signup" ? "new-password" : "current-password"}
                      aria-label={t("door.password")}
                    />
                    {screen === "signup" && (
                      <p className="mt-1.5 pl-4 text-xs text-muted-foreground">
                        {t("door.passwordHint")}
                      </p>
                    )}
                  </div>
                  {screen === "signup" && mode === "unclaimed" && (
                    <div>
                      <RoundInput
                        icon={KeyRound}
                        value={setupCode}
                        onChange={(e) => setSetupCode(e.target.value)}
                        placeholder={t("door.setupCode")}
                        className="font-mono"
                        aria-label={t("door.setupCode")}
                      />
                      <p className="mt-1.5 pl-4 text-xs text-muted-foreground">
                        {t("door.setupHint")}
                      </p>
                    </div>
                  )}
                  {screen === "signup" && mode === "locked" && (
                    <div>
                      <RoundInput
                        icon={Ticket}
                        value={invite}
                        onChange={(e) => setInvite(e.target.value)}
                        placeholder={t("door.inviteCode")}
                        className="font-mono"
                        aria-label={t("door.inviteCode")}
                      />
                      <p className="mt-1.5 pl-4 text-xs text-muted-foreground">
                        {t("door.inviteHint")}
                      </p>
                    </div>
                  )}
                  <AnimatePresence>
                    {error && (
                      <motion.p
                        initial={{ opacity: 0, height: 0 }}
                        animate={{ opacity: 1, height: "auto" }}
                        exit={{ opacity: 0, height: 0 }}
                        className="pl-4 text-sm text-destructive"
                      >
                        {error}
                      </motion.p>
                    )}
                  </AnimatePresence>
                  <RoundButton type="submit" disabled={busy || !name.trim() || !password}>
                    {screen === "signup" ? t("door.signup") : t("door.login")}
                  </RoundButton>
                </form>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </motion.div>
    </div>
  );
}
