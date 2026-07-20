import { useEffect, useState } from "react";
import { Moon, Sun, Plus, UserPlus, ShieldCheck, ShieldAlert, Wifi, WifiOff } from "lucide-react";
import type { Company } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";

export function TopBar({
  companies,
  companyId,
  onSelectCompany,
  onNewCompany,
  onHire,
  onNewTask,
  canCreateTask,
  connected,
  tick,
  theme,
  onToggleTheme,
}: {
  companies: Company[];
  companyId: string | null;
  onSelectCompany: (id: string) => void;
  onNewCompany: () => void;
  onHire: () => void;
  onNewTask: () => void;
  canCreateTask: boolean;
  connected: boolean;
  tick: number;
  theme: string;
  onToggleTheme: () => void;
}) {
  return (
    <header className="flex items-center gap-3 border-b border-border px-6 py-3">
      <div className="flex items-center gap-2">
        <Logo />
        <span className="text-base font-semibold tracking-tight">Overmind</span>
      </div>

      <div className="mx-1 h-5 w-px bg-border" />

      <select
        value={companyId ?? ""}
        onChange={(e) => (e.target.value === "__new" ? onNewCompany() : onSelectCompany(e.target.value))}
        className="h-9 rounded-md border border-input bg-background px-2.5 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {companies.length === 0 && <option value="">No company</option>}
        {companies.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name}
          </option>
        ))}
        <option value="__new">+ New company…</option>
      </select>

      <div className="ml-auto flex items-center gap-2">
        <AuditIndicator tick={tick} />
        <ConnectionDot connected={connected} />
        {companyId && (
          <>
            <Button variant="outline" size="sm" onClick={onHire}>
              <UserPlus className="h-4 w-4" />
              Hire
            </Button>
            <Button variant="primary" size="sm" onClick={onNewTask} disabled={!canCreateTask}>
              <Plus className="h-4 w-4" />
              New task
            </Button>
          </>
        )}
        <Button variant="ghost" size="icon" onClick={onToggleTheme} aria-label="Toggle theme">
          {theme === "dark" ? <Sun className="h-4.5 w-4.5" /> : <Moon className="h-4.5 w-4.5" />}
        </Button>
      </div>
    </header>
  );
}

function Logo() {
  return (
    <span className="flex h-7 w-7 items-center justify-center rounded-md bg-primary text-primary-foreground">
      <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth={2.2}>
        <circle cx="12" cy="12" r="3" />
        <path d="M12 2v4M12 18v4M2 12h4M18 12h4M5 5l2.5 2.5M16.5 16.5L19 19M19 5l-2.5 2.5M7.5 16.5L5 19" />
      </svg>
    </span>
  );
}

function ConnectionDot({ connected }: { connected: boolean }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-xs",
        connected ? "text-status-done" : "text-muted-foreground",
      )}
      title={connected ? "Live updates connected" : "Reconnecting…"}
    >
      {connected ? <Wifi className="h-3.5 w-3.5" /> : <WifiOff className="h-3.5 w-3.5" />}
    </span>
  );
}

/** Periodically verifies the audit hash chain and shows a trust badge. */
function AuditIndicator({ tick }: { tick: number }) {
  const [valid, setValid] = useState<boolean | null>(null);
  useEffect(() => {
    api
      .auditVerify()
      .then((r) => setValid(r.valid))
      .catch(() => setValid(null));
  }, [tick]);
  if (valid === null) return null;
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-xs",
        valid ? "text-status-done" : "text-destructive",
      )}
      title={valid ? "Audit chain verified" : "Audit chain BROKEN"}
    >
      {valid ? <ShieldCheck className="h-3.5 w-3.5" /> : <ShieldAlert className="h-3.5 w-3.5" />}
      audit
    </span>
  );
}
