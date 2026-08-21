import { useEffect, useRef, useState } from "react";
import { ArrowUpRight, KeyRound, Loader2 } from "lucide-react";
import type { Economy } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * The founder is told what is missing where they are looking (M22) — and can
 * fix it from here (M23): the subscription sign-in is the server driving the
 * CLI's own OAuth flow, so nobody has to know a terminal command exists.
 *
 * Shown only for `unknown_kind === "not_signed_in"`: a custom adapter's
 * unknown is deliberate, and an unreadable probe is not an invitation to sign
 * in.
 */
export function SignInNotice({
  economy,
  onSignedIn,
}: {
  economy: Economy | null;
  onSignedIn: () => void;
}) {
  const t = useT();
  const [flow, setFlow] = useState<
    | { step: "idle" }
    | { step: "starting" }
    | { step: "url"; url: string }
    | { step: "exchanging" }
    | { step: "done" }
    | { step: "failed"; tail: string }
  >({ step: "idle" });
  const [code, setCode] = useState("");
  const polling = useRef<number | null>(null);

  const active =
    flow.step === "starting" || flow.step === "url" || flow.step === "exchanging";

  // While a flow runs, follow the server's view of it.
  useEffect(() => {
    if (!active) return;
    polling.current = window.setInterval(async () => {
      try {
        const s = await api.claudeAuthStatus();
        if (s.state === "url_ready") setFlow({ step: "url", url: s.url });
        else if (s.state === "exchanging") setFlow({ step: "exchanging" });
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

  if (
    !economy ||
    economy.kind !== "unknown" ||
    economy.unknown_kind !== "not_signed_in"
  ) {
    return null;
  }

  const begin = async () => {
    setFlow({ step: "starting" });
    try {
      await api.claudeAuthStart();
    } catch (e) {
      setFlow({ step: "failed", tail: e instanceof Error ? e.message : String(e) });
    }
  };

  return (
    <div className="mx-auto w-full max-w-2xl px-4 pt-4">
      <div className="rounded-lg border border-border bg-card p-4 shadow-soft">
        <div className="flex items-start gap-3">
          <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <KeyRound className="h-4 w-4" />
          </span>
          <div className="min-w-0 flex-1 space-y-2.5">
            <div>
              <p className="text-sm font-medium">{t("economy.notSignedIn")}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {t("economy.notSignedInBody")}
              </p>
            </div>

            {/* The guided way: the subscription, from the product. */}
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
            {flow.step === "exchanging" && (
              <p className="flex items-center gap-2 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("economy.connectPlanExchanging")}
              </p>
            )}
            {flow.step === "done" && (
              <p className="text-xs font-medium text-primary">
                {t("economy.connectPlanDone")}
              </p>
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

            {/* The manual ways stay, quieter, below the guided one. */}
            <details className="pt-1">
              <summary className="cursor-pointer text-[11px] text-muted-foreground">
                {t("economy.notSignedInKey")}
              </summary>
              <code className="mt-1 block overflow-x-auto rounded bg-muted px-2 py-1 font-mono text-[11px]">
                export ANTHROPIC_API_KEY=sk-ant-…
              </code>
              <p className="mt-1 text-[11px] text-muted-foreground">
                {t("economy.notSignedInPlanHost")}
              </p>
            </details>
          </div>
        </div>
      </div>
    </div>
  );
}
