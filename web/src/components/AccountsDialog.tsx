import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  CreditCard,
  ExternalLink,
  KeyRound,
  Loader2,
  Trash2,
} from "lucide-react";
import type { Account, ProviderInfo } from "../lib/api";
import { api, ApiError } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

const KNOWN_PROVIDERS = ["anthropic", "openai", "google"] as const;

export function AccountsDialog({
  open,
  onOpenChange,
  initialProvider,
  onAccountsChanged,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialProvider?: string;
  onAccountsChanged?: () => void;
}) {
  const t = useT();
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actionBusy, setActionBusy] = useState<string | null>(null);

  // Disconnection confirmation
  const [confirmDisconnect, setConfirmDisconnect] = useState<Account | null>(null);

  // Active connection form per provider: { provider, mode: "key" | "subscription" }
  const [connecting, setConnecting] = useState<{
    provider: string;
    mode: "key" | "subscription";
  } | null>(null);

  // Key input state
  const [apiKey, setApiKey] = useState("");

  // Subscription OAuth sign-in flow state
  const [signInFlow, setSignInFlow] = useState<
    | { step: "idle" }
    | { step: "starting" }
    | { step: "url"; url: string; rejected?: string }
    | { step: "exchanging"; tail?: string }
    | { step: "restarting"; tail?: string }
    | { step: "done"; account?: Account }
    | { step: "failed"; tail: string }
  >({ step: "idle" });
  const [signInCode, setSignInCode] = useState("");
  const [clockSkew, setClockSkew] = useState<number | null>(null);
  const pollingRef = useRef<number | null>(null);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await api.listAccounts();
      setAccounts(res.accounts);
      setProviders(res.providers);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!open) {
      setConfirmDisconnect(null);
      setConnecting(null);
      setApiKey("");
      setSignInFlow({ step: "idle" });
      setSignInCode("");
      if (pollingRef.current) {
        window.clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
      return;
    }
    load();
    if (initialProvider) {
      // If initialProvider is specified, we can pre-focus or start connect
    }
  }, [open, initialProvider]);

  // Handle subscription polling
  const isSignInActive =
    signInFlow.step === "starting" ||
    signInFlow.step === "url" ||
    signInFlow.step === "exchanging" ||
    signInFlow.step === "restarting";

  useEffect(() => {
    if (!isSignInActive) {
      if (pollingRef.current) {
        window.clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
      return;
    }

    pollingRef.current = window.setInterval(async () => {
      try {
        const s = await api.signInStatus();
        if (typeof s.clock_skew_secs === "number") {
          setClockSkew(s.clock_skew_secs);
        }
        if (s.state === "url_ready") {
          setSignInFlow({ step: "url", url: s.url ?? "", rejected: s.rejected ?? undefined });
        } else if (s.state === "exchanging") {
          setSignInFlow({ step: "exchanging", tail: s.tail });
        } else if (s.state === "code_rejected") {
          setSignInFlow({ step: "url", url: s.url ?? "", rejected: s.tail });
        } else if (s.state === "restarting") {
          setSignInFlow({ step: "restarting", tail: s.tail });
        } else if (s.state === "done") {
          setSignInFlow({ step: "done", account: s.account });
          if (pollingRef.current) {
            window.clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          await load();
          onAccountsChanged?.();
        } else if (s.state === "failed") {
          setSignInFlow({ step: "failed", tail: s.tail });
          if (pollingRef.current) {
            window.clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
        }
      } catch {
        // Retry next tick
      }
    }, 2000);

    return () => {
      if (pollingRef.current) {
        window.clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
    };
  }, [isSignInActive, onAccountsChanged]);

  const handleActivate = async (account: Account) => {
    setActionBusy(account.id);
    setError(null);
    try {
      await api.activateAccount(account.id);
      await load();
      onAccountsChanged?.();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setActionBusy(null);
    }
  };

  const handleDisconnect = async (account: Account) => {
    setActionBusy(account.id);
    setError(null);
    try {
      await api.disconnectAccount(account.id);
      setConfirmDisconnect(null);
      await load();
      onAccountsChanged?.();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setActionBusy(null);
    }
  };

  const handleStartConnectKey = (provider: string) => {
    setConnecting({ provider, mode: "key" });
    setApiKey("");
    setError(null);
  };

  const handleSubmitKey = async (provider: string) => {
    if (!apiKey.trim()) return;
    setActionBusy("key");
    setError(null);
    try {
      await api.connectAccount(provider, apiKey.trim());
      setConnecting(null);
      setApiKey("");
      await load();
      onAccountsChanged?.();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setActionBusy(null);
    }
  };

  const handleStartConnectSubscription = async (provider: string) => {
    setConnecting({ provider, mode: "subscription" });
    setSignInFlow({ step: "starting" });
    setSignInCode("");
    setClockSkew(null);
    setError(null);
    try {
      await api.signInBegin(provider, "subscription");
    } catch (e) {
      setSignInFlow({
        step: "failed",
        tail: e instanceof ApiError ? e.message : String(e),
      });
    }
  };

  const handleSubmitSignInCode = async () => {
    if (!signInCode.trim()) return;
    setActionBusy("code");
    setError(null);
    try {
      await api.signInCode(signInCode.trim());
      setSignInFlow({ step: "exchanging" });
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setActionBusy(null);
    }
  };

  const providerTitle = (p: string) => {
    switch (p) {
      case "anthropic":
        return t("accounts.providerAnthropic");
      case "openai":
        return t("accounts.providerOpenAI");
      case "google":
        return t("accounts.providerGoogle");
      default:
        return p;
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("accounts.title")}
      description={t("accounts.body")}
      className="max-w-2xl"
    >
      <div className="flex flex-col gap-6">
        {error && (
          <div className="rounded-xl border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {error}
          </div>
        )}

        {loading && !accounts && (
          <div className="flex items-center justify-center py-8 text-muted-foreground">
            <Loader2 className="h-6 w-6 animate-spin" />
          </div>
        )}

        {accounts && (
          <div className="flex flex-col gap-6">
            {KNOWN_PROVIDERS.map((vendor) => {
              const providerAccounts = accounts.filter((a) => a.provider === vendor);
              const activeAccount = providerAccounts.find((a) => a.active);
              const providerInfo = providers.find((p) => p.provider === vendor);
              const isConnecting = connecting?.provider === vendor;

              const subscriptionShape = providerInfo?.kinds?.subscription?.shape;
              const keyShape = providerInfo?.kinds?.key?.shape;

              return (
                <section
                  key={vendor}
                  className="rounded-2xl border border-border/70 bg-card/60 p-4 shadow-sm"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 pb-3">
                    <div className="flex items-center gap-2.5">
                      <h3 className="text-base font-semibold">{providerTitle(vendor)}</h3>
                      {activeAccount ? (
                        <span className="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-0.5 text-xs font-medium text-primary">
                          <Check className="h-3 w-3" />
                          {t("accounts.activeBadge")}: {activeAccount.label}
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2.5 py-0.5 text-xs font-medium text-amber-500">
                          <AlertTriangle className="h-3 w-3" />
                          {t("accounts.noActiveNotice")}
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Connected accounts list */}
                  <div className="mt-3">
                    {providerAccounts.length === 0 ? (
                      <p className="py-2 text-xs text-muted-foreground">
                        {t("accounts.noAccounts", { provider: providerTitle(vendor) })}
                      </p>
                    ) : (
                      <ul className="flex flex-col divide-y divide-border/40">
                        {providerAccounts.map((a) => {
                          const isActionTarget = actionBusy === a.id;
                          const isConfirming = confirmDisconnect?.id === a.id;

                          return (
                            <li key={a.id} className="flex flex-col gap-2 py-2.5">
                              <div className="flex items-center justify-between gap-3">
                                <div className="min-w-0 flex-1">
                                  <div className="flex items-center gap-2">
                                    <span className="truncate text-sm font-medium">
                                      {a.label}
                                    </span>
                                    {a.active && (
                                      <span className="rounded bg-primary/15 px-1.5 py-0.5 text-[10px] font-semibold text-primary uppercase tracking-wider">
                                        {t("accounts.activeBadge")}
                                      </span>
                                    )}
                                    <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                      {a.kind === "subscription"
                                        ? t("accounts.kindSubscription")
                                        : t("accounts.kindKey")}
                                    </span>
                                    <span className="rounded bg-muted/60 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                      {a.source === "stored"
                                        ? t("accounts.sourceStored")
                                        : t("accounts.sourceEnv")}
                                    </span>
                                  </div>
                                  <div className="mt-0.5 text-xs text-muted-foreground">
                                    {a.kind === "key" && a.last_four
                                      ? t("accounts.keyEndingIn", { last_four: a.last_four })
                                      : null}
                                    {a.source === "environment" && (
                                      <span className="italic">
                                        {" "}
                                        · {t("accounts.sourceEnvHint")}
                                      </span>
                                    )}
                                  </div>
                                </div>

                                <div className="flex items-center gap-2">
                                  {!a.active && (
                                    <Button
                                      size="sm"
                                      variant="outline"
                                      disabled={!!actionBusy}
                                      onClick={() => handleActivate(a)}
                                    >
                                      {isActionTarget ? (
                                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                      ) : null}
                                      {t("accounts.makeActive")}
                                    </Button>
                                  )}

                                  {a.source === "stored" && (
                                    <Button
                                      size="sm"
                                      variant="ghost"
                                      disabled={!!actionBusy}
                                      onClick={() => setConfirmDisconnect(a)}
                                      className="text-muted-foreground hover:text-destructive"
                                      aria-label={t("accounts.disconnect")}
                                      title={t("accounts.disconnect")}
                                    >
                                      <Trash2 className="h-4 w-4" />
                                    </Button>
                                  )}
                                </div>
                              </div>

                              {/* Disconnect confirmation card */}
                              {isConfirming && (
                                <div className="mt-2 rounded-xl border border-destructive/40 bg-destructive/10 p-3 text-xs text-destructive">
                                  <p className="font-semibold">
                                    {t("accounts.disconnectConfirmPrompt")}
                                  </p>
                                  <p className="mt-1">
                                    {a.active
                                      ? t("accounts.disconnectLastActiveWarning", {
                                          provider: providerTitle(vendor),
                                        })
                                      : t("accounts.disconnectGenericConfirm")}
                                  </p>
                                  {a.kind === "subscription" && (
                                    <p className="mt-1 text-muted-foreground">
                                      {t("accounts.disconnectSubscriptionNotice", {
                                        provider: providerTitle(vendor),
                                      })}
                                    </p>
                                  )}
                                  <div className="mt-2.5 flex items-center gap-2">
                                    <Button
                                      size="sm"
                                      variant="destructive"
                                      disabled={!!actionBusy}
                                      onClick={() => handleDisconnect(a)}
                                    >
                                      {actionBusy === a.id ? (
                                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                      ) : null}
                                      {t("accounts.confirmDisconnect")}
                                    </Button>
                                    <Button
                                      size="sm"
                                      variant="ghost"
                                      onClick={() => setConfirmDisconnect(null)}
                                    >
                                      {t("accounts.cancel")}
                                    </Button>
                                  </div>
                                </div>
                              )}
                            </li>
                          );
                        })}
                      </ul>
                    )}
                  </div>

                  {/* Connect trigger buttons or active forms */}
                  <div className="mt-4 border-t border-border/40 pt-3">
                    {!isConnecting ? (
                      <div className="flex flex-wrap items-center gap-2">
                        {subscriptionShape === "paste_back" && (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => handleStartConnectSubscription(vendor)}
                          >
                            <CreditCard className="h-3.5 w-3.5" />
                            {t("accounts.connectSubscription")}
                          </Button>
                        )}
                        {keyShape === "key" && (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => handleStartConnectKey(vendor)}
                          >
                            <KeyRound className="h-3.5 w-3.5" />
                            {t("accounts.connectKey")}
                          </Button>
                        )}
                        {!subscriptionShape && !keyShape && (
                          <p className="text-xs text-muted-foreground">
                            {vendor === "openai"
                              ? t("accounts.unavailableCodex")
                              : vendor === "google"
                                ? t("accounts.unavailableGoogle")
                                : t("accounts.unavailableReason", { reason: "unsupported" })}
                          </p>
                        )}
                      </div>
                    ) : connecting.mode === "key" ? (
                      /* Connect key form */
                      <div className="flex flex-col gap-2 rounded-xl bg-muted/40 p-3">
                        <label className="text-xs font-medium">
                          {t("accounts.connectKey")} ({providerTitle(vendor)})
                        </label>
                        <div className="flex items-center gap-2">
                          <Input
                            type="password"
                            autoFocus
                            value={apiKey}
                            onChange={(e) => setApiKey(e.target.value)}
                            placeholder={t("accounts.keyPlaceholder")}
                            className="font-mono text-xs"
                          />
                          <Button
                            size="sm"
                            variant="primary"
                            disabled={actionBusy === "key" || !apiKey.trim()}
                            onClick={() => handleSubmitKey(vendor)}
                          >
                            {actionBusy === "key" ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : null}
                            {t("accounts.saveKey")}
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => {
                              setConnecting(null);
                              setApiKey("");
                            }}
                          >
                            {t("accounts.cancel")}
                          </Button>
                        </div>
                      </div>
                    ) : (
                      /* Connect subscription OAuth flow */
                      <div className="flex flex-col gap-3 rounded-xl bg-muted/40 p-3">
                        <div className="flex items-center justify-between">
                          <span className="text-xs font-semibold">
                            {t("accounts.connectSubscription")} ({providerTitle(vendor)})
                          </span>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => {
                              setConnecting(null);
                              setSignInFlow({ step: "idle" });
                            }}
                          >
                            {t("accounts.cancel")}
                          </Button>
                        </div>

                        {clockSkew !== null && Math.abs(clockSkew) > 120 && (
                          <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-2 text-xs text-amber-500">
                            {t("accounts.clockSkewWarning", {
                              n: String(Math.round(Math.abs(clockSkew) / 60)),
                            })}
                          </div>
                        )}

                        {signInFlow.step === "starting" && (
                          <div className="flex items-center gap-2 text-xs text-muted-foreground">
                            <Loader2 className="h-4 w-4 animate-spin text-primary" />
                            {t("accounts.startingSignIn")}
                          </div>
                        )}

                        {signInFlow.step === "url" && (
                          <div className="space-y-3">
                            {signInFlow.rejected && (
                              <p className="text-xs text-destructive">{signInFlow.rejected}</p>
                            )}
                            <div>
                              <p className="text-xs text-muted-foreground">
                                {t("accounts.stepAuthLink")}
                              </p>
                              <a
                                href={signInFlow.url}
                                target="_blank"
                                rel="noreferrer"
                                className="mt-1 inline-flex items-center gap-1.5 text-xs font-medium text-primary hover:underline"
                              >
                                {signInFlow.url}
                                <ExternalLink className="h-3 w-3" />
                              </a>
                            </div>

                            <div className="space-y-1.5">
                              <p className="text-xs text-muted-foreground">
                                {t("accounts.stepPasteCode")}
                              </p>
                              <div className="flex items-center gap-2">
                                <Input
                                  value={signInCode}
                                  onChange={(e) => setSignInCode(e.target.value)}
                                  placeholder={t("accounts.pasteCodePlaceholder")}
                                  className="font-mono text-xs"
                                  autoFocus
                                />
                                <Button
                                  size="sm"
                                  variant="primary"
                                  disabled={actionBusy === "code" || !signInCode.trim()}
                                  onClick={handleSubmitSignInCode}
                                >
                                  {actionBusy === "code" ? (
                                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                  ) : null}
                                  {t("accounts.submitCode")}
                                </Button>
                              </div>
                            </div>
                          </div>
                        )}

                        {signInFlow.step === "exchanging" && (
                          <div className="flex items-center gap-2 text-xs text-muted-foreground">
                            <Loader2 className="h-4 w-4 animate-spin text-primary" />
                            {t("accounts.checkingCode")}
                          </div>
                        )}

                        {signInFlow.step === "restarting" && (
                          <div className="flex items-center gap-2 text-xs text-amber-500">
                            <Loader2 className="h-4 w-4 animate-spin" />
                            {t("accounts.restartingFlow")}
                          </div>
                        )}

                        {signInFlow.step === "done" && (
                          <div className="flex items-center gap-2 text-xs text-primary font-medium">
                            <Check className="h-4 w-4" />
                            {t("accounts.signInDone")}
                          </div>
                        )}

                        {signInFlow.step === "failed" && (
                          <div className="space-y-2">
                            <p className="text-xs text-destructive">
                              {t("accounts.signInFailed")} {signInFlow.tail}
                            </p>
                            <Button
                              size="sm"
                              variant="outline"
                              onClick={() => handleStartConnectSubscription(vendor)}
                            >
                              {t("accounts.retry")}
                            </Button>
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                </section>
              );
            })}
          </div>
        )}
      </div>
    </Dialog>
  );
}
