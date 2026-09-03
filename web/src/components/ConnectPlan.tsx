import { useEffect, useRef, useState } from "react";
import { ArrowUpRight, Loader2 } from "lucide-react";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * The subscription sign-in, driven from the product (M23).
 *
 * The server drives the CLI's own OAuth flow and this follows it, so nobody
 * has to know a terminal command exists. Extracted from the sign-in notice on
 * 3 Sep 2026 because it was needed in a second place — the org view, where a
 * person paying with a key can now reach the plan — and a flow this long is
 * not something to keep two copies of.
 *
 * It owns the whole conversation and never renders nothing: idle is the
 * button. What it does *not* own is whether the surrounding card should be
 * there at all; it tells the parent, through `onEngaged`, that a flow has
 * started, because a card that vanishes the moment the economy changes takes
 * the good news with it.
 */
export function ConnectPlan({
  onSignedIn,
  onEngaged,
}: {
  onSignedIn: () => void;
  /** Called once, when the person starts the flow. */
  onEngaged?: () => void;
}) {
  const t = useT();
  const [flow, setFlow] = useState<
    | { step: "idle" }
    | { step: "starting" }
    | { step: "url"; url: string; rejected?: string }
    | { step: "exchanging"; tail?: string }
    | { step: "restarting"; tail?: string }
    | { step: "done" }
    | { step: "failed"; tail: string }
  >({ step: "idle" });
  const [code, setCode] = useState("");
  // Seconds this machine's clock is off the world's, once the server has
  // measured it. Minutes of skew (a Docker VM woken from host sleep) refuse
  // every OAuth code before it is pasted — worth its own loud line.
  const [skew, setSkew] = useState<number | null>(null);
  const polling = useRef<number | null>(null);

  const active =
    flow.step === "starting" ||
    flow.step === "url" ||
    flow.step === "exchanging" ||
    flow.step === "restarting";

  // While a flow runs, follow the server's view of it.
  useEffect(() => {
    if (!active) return;
    polling.current = window.setInterval(async () => {
      try {
        const s = await api.claudeAuthStatus();
        if (typeof s.clock_skew_secs === "number") setSkew(s.clock_skew_secs);
        if (s.state === "url_ready")
          // `rejected` survives a CLI restart: the fresh URL arrives with
          // the note that the previous code was refused.
          setFlow({ step: "url", url: s.url, rejected: s.rejected ?? undefined });
        else if (s.state === "exchanging") setFlow({ step: "exchanging", tail: s.tail });
        else if (s.state === "code_rejected")
          // The CLI said no and is prompting again (27 Aug 2026: without
          // this, an invalid code meant an eternal spinner): back to the
          // paste box, same URL, with the CLI's words shown.
          setFlow({ step: "url", url: s.url ?? "", rejected: s.tail });
        else if (s.state === "restarting")
          // An OAuth error (a 400 on the exchange) restarts the CLI's flow:
          // the old link is dead, a fresh one is being minted — say so
          // instead of leaving a paste box pointed at a dead URL.
          setFlow({ step: "restarting", tail: s.tail });
        else if (s.state === "done") {
          setFlow({ step: "done" });
          onSignedIn();
        } else if (s.state === "failed") setFlow({ step: "failed", tail: s.tail });
      } catch {
        // A poll that failed is a poll that will run again in two seconds.
      }
    }, 2000);
    return () => {
      if (polling.current) window.clearInterval(polling.current);
    };
  }, [active, onSignedIn]);

  const begin = async () => {
    onEngaged?.();
    setFlow({ step: "starting" });
    try {
      await api.claudeAuthStart();
    } catch (e) {
      setFlow({ step: "failed", tail: e instanceof Error ? e.message : String(e) });
    }
  };

  if (flow.step === "done") {
    return <p className="text-sm font-medium text-primary">{t("economy.connectPlanDone")}</p>;
  }

  return (
    <div className="space-y-2.5">
      {/* A clock minutes off the world refuses every code before it is
          pasted — named here, where the person is about to paste one. */}
      {active && skew !== null && Math.abs(skew) > 120 && (
        <p className="rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
          {t("economy.connectPlanClockSkew", { n: Math.round(Math.abs(skew) / 60) })}
        </p>
      )}

      {flow.step === "idle" && (
        <Button size="sm" onClick={begin}>
          {t("economy.connectPlan")}
        </Button>
      )}
      {flow.step === "starting" && (
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("economy.connectPlanStarting")}
        </p>
      )}
      {flow.step === "url" && (
        <div className="space-y-2">
          <p className="text-xs font-medium">{t("economy.connectPlanOpen")}</p>
          <a
            href={flow.url}
            target="_blank"
            rel="noopener"
            className="inline-flex items-center gap-1 break-all text-xs text-primary underline underline-offset-2"
          >
            {flow.url} <ArrowUpRight className="h-3 w-3 shrink-0" />
          </a>
          {flow.rejected && (
            <p className="rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
              {t("economy.connectPlanRejected")}
            </p>
          )}
          <p className="text-xs font-medium">{t("economy.connectPlanPaste")}</p>
          <form
            className="flex gap-2"
            onSubmit={async (e) => {
              e.preventDefault();
              if (!code.trim()) return;
              try {
                await api.claudeAuthCode(code);
                setFlow({ step: "exchanging" });
              } catch (err) {
                setFlow({
                  step: "failed",
                  tail: err instanceof Error ? err.message : String(err),
                });
              }
            }}
          >
            <Input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              className="h-8 font-mono text-xs"
              autoFocus
            />
            <Button size="sm" type="submit" disabled={!code.trim()}>
              {t("economy.connectPlanSubmit")}
            </Button>
          </form>
        </div>
      )}
      {flow.step === "restarting" && (
        <div className="space-y-1.5">
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            {t("economy.connectPlanRestarting")}
          </p>
          {flow.tail && (
            <pre className="max-h-24 overflow-auto rounded bg-muted px-2 py-1 font-mono text-[10.5px] text-muted-foreground">
              {flow.tail}
            </pre>
          )}
        </div>
      )}
      {flow.step === "exchanging" && (
        <div className="space-y-1.5">
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            {t("economy.connectPlanExchanging")}
          </p>
          {flow.tail && (
            <pre className="max-h-24 overflow-auto rounded bg-muted px-2 py-1 font-mono text-[10.5px] text-muted-foreground">
              {flow.tail}
            </pre>
          )}
        </div>
      )}
      {flow.step === "failed" && (
        <div className="space-y-1.5">
          <p className="text-xs">{t("economy.connectPlanFailed")}</p>
          <pre className="overflow-x-auto rounded bg-muted px-2 py-1 font-mono text-[10.5px] text-muted-foreground">
            {flow.tail}
          </pre>
          <Button size="sm" variant="outline" onClick={begin}>
            {t("economy.connectPlanRetry")}
          </Button>
        </div>
      )}
    </div>
  );
}
