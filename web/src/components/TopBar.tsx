import { useEffect, useState } from "react";
import {
  Moon,
  Sun,
  Plus,
  UserPlus,
  ShieldCheck,
  ShieldAlert,
  Wifi,
  WifiOff,
  BrainCircuit,
  Plug,
  LogOut,
  ChevronDown,
  UserRoundPlus,
  Trash2,
} from "lucide-react";
import type { Company, LanguageCode, View } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Segmented } from "./ui/controls";
import { Inbox } from "./Inbox";
import { LanguageMenu } from "./LanguageMenu";
import { ConnectionsDialog } from "./ConnectionsDialog";
import { useT } from "../lib/i18n";
import { cn } from "../lib/utils";

export function TopBar({
  companies,
  companyId,
  onSelectCompany,
  onNewCompany,
  onHire,
  onNewTask,
  canCreateTask,
  view,
  onViewChange,
  showViews,
  onApprovalDecided,
  inboxSignal,
  onOpenMeeting,
  connected,
  tick,
  language,
  onChangeLanguage,
  theme,
  onToggleTheme,
  onLogout,
  onInvite,
  onDeleteCompany,
}: {
  companies: Company[];
  companyId: string | null;
  onSelectCompany: (id: string) => void;
  onNewCompany: () => void;
  onHire: () => void;
  onNewTask: () => void;
  canCreateTask: boolean;
  view: View;
  onViewChange: (v: View) => void;
  showViews: boolean;
  onApprovalDecided: (approvalId?: string) => void;
  inboxSignal: number;
  onOpenMeeting: (meetingId: string) => void;
  connected: boolean;
  tick: number;
  language: LanguageCode;
  onChangeLanguage: (code: LanguageCode) => void;
  theme: string;
  onToggleTheme: () => void;
  /** Absent while the instance is unclaimed: no session, nothing to end. */
  onLogout?: () => void;
  /** The invite surface (M25): present only for the owner. */
  onInvite?: () => void;
  /** Absent while no company is selected: nothing to delete (ADR-0034). */
  onDeleteCompany?: () => void;
}) {
  const t = useT();
  return (
    <header className="flex items-center gap-3 border-b border-border px-6 py-3">
      <div className="flex items-center gap-2">
        <Logo />
        <span className="text-base font-semibold tracking-tight">Overmind</span>
      </div>

      <div className="mx-1 h-5 w-px bg-border" />

      <div className="relative shrink">
        <select
          value={companyId ?? ""}
          onChange={(e) =>
            e.target.value === "__new" ? onNewCompany() : onSelectCompany(e.target.value)
          }
          className={cn(
            "h-9 max-w-40 cursor-pointer appearance-none rounded-full border border-input bg-background pl-3.5 pr-8 text-sm",
            "transition-all duration-200 focus-visible:outline-none focus-visible:border-primary",
            "focus-visible:shadow-[0_0_0_4px_var(--ring-soft,rgba(124,92,255,0.15))]",
          )}
        >
        {companies.length === 0 && <option value="">{t("nav.noCompany")}</option>}
        {companies.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name}
          </option>
        ))}
          <option value="__new">{t("nav.newCompany")}</option>
        </select>
        <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
      </div>

      {showViews && (
        <div className="ml-2">
          <Segmented<View>
            value={view}
            onChange={onViewChange}
            options={[
              { value: "chat", label: t("nav.chat") },
              { value: "board", label: t("nav.board") },
              { value: "meetings", label: t("nav.meetings") },
              { value: "org", label: t("nav.org") },
              { value: "memory", label: t("nav.memoryView") },
            ]}
          />
        </div>
      )}

      <div className="ml-auto flex items-center gap-2">
        {companyId && (
          <Inbox
            companyId={companyId}
            tick={tick}
            onDecided={onApprovalDecided}
            openSignal={inboxSignal}
            onOpenMeeting={onOpenMeeting}
          />
        )}
        <MemoryIndicator companyId={companyId} />
        <AuditIndicator tick={tick} />
        <ConnectionDot connected={connected} />
        {companyId && (
          <>
            <Button variant="outline" size="sm" onClick={onHire}>
              <UserPlus className="h-4 w-4" />
              {t("nav.hire")}
            </Button>
            <Button variant="primary" size="sm" onClick={onNewTask} disabled={!canCreateTask}>
              <Plus className="h-4 w-4" />
              {t("nav.newTask")}
            </Button>
          </>
        )}
        {companyId && <Connections companyId={companyId} />}
        {companyId && onDeleteCompany && (
          <Button
            variant="ghost"
            size="icon"
            onClick={onDeleteCompany}
            aria-label={t("nav.deleteCompany")}
            title={t("nav.deleteCompany")}
          >
            <Trash2 className="h-4.5 w-4.5" />
          </Button>
        )}
        {companyId && <LanguageMenu language={language} onChange={onChangeLanguage} />}
        <Button
          variant="ghost"
          size="icon"
          onClick={onToggleTheme}
          aria-label={t("nav.toggleTheme")}
        >
          {theme === "dark" ? <Sun className="h-4.5 w-4.5" /> : <Moon className="h-4.5 w-4.5" />}
        </Button>
        {onInvite && (
          <Button variant="ghost" size="icon" onClick={onInvite} aria-label={t("door.inviteMint")}>
            <UserRoundPlus className="h-4.5 w-4.5" />
          </Button>
        )}
        {onLogout && (
          <Button variant="ghost" size="icon" onClick={onLogout} aria-label={t("door.logout")}>
            <LogOut className="h-4.5 w-4.5" />
          </Button>
        )}
      </div>
    </header>
  );
}

/** The Overmind mark: a memory-trace (轍) — three ruts converging into the
 * present node. Kept identical to the marketing site's assets/brand/mark.svg. */
function Logo() {
  return (
    <svg viewBox="0 0 220 220" className="h-7 w-7" fill="none" role="img" aria-label="Overmind">
      <defs>
        <linearGradient
          id="om-track"
          x1="40"
          y1="110"
          x2="154"
          y2="110"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stopColor="#6d6386" stopOpacity="0.3" />
          <stop offset="0.55" stopColor="#7c5cff" stopOpacity="0.75" />
          <stop offset="1" stopColor="#9d7bff" stopOpacity="1" />
        </linearGradient>
        <radialGradient id="om-present" cx="0.5" cy="0.5" r="0.5">
          <stop offset="0" stopColor="#f0c07a" stopOpacity="0.95" />
          <stop offset="0.35" stopColor="#9d7bff" stopOpacity="0.9" />
          <stop offset="1" stopColor="#7c5cff" stopOpacity="0" />
        </radialGradient>
      </defs>
      <g stroke="url(#om-track)" strokeWidth="12" strokeLinecap="round">
        <line x1="46" y1="56" x2="152" y2="110" />
        <line x1="40" y1="110" x2="152" y2="110" />
        <line x1="46" y1="164" x2="152" y2="110" />
      </g>
      <circle cx="152" cy="110" r="44" fill="url(#om-present)" />
      <circle cx="152" cy="110" r="23" fill="#9d7bff" />
      <circle cx="152" cy="110" r="8.5" fill="#f0c07a" />
    </svg>
  );
}

function ConnectionDot({ connected }: { connected: boolean }) {
  const t = useT();
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-xs",
        connected ? "text-status-done" : "text-muted-foreground",
      )}
      title={connected ? t("nav.liveConnected") : t("nav.reconnecting")}
    >
      {connected ? <Wifi className="h-3.5 w-3.5" /> : <WifiOff className="h-3.5 w-3.5" />}
    </span>
  );
}

/** Shows whether organizational memory (Wadachi/MCP) is connected — and, with a
 *  company selected, whether *that company's* brain is on and where it lives
 *  (ADR-0024). A managed brain is a directory, and naming it is what makes it
 *  yours to open, inspect or back up. */
function MemoryIndicator({ companyId }: { companyId: string | null }) {
  const t = useT();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [dir, setDir] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    const load = companyId
      ? api.brainStatus(companyId).then((r) => ({
          on: r.provider_configured && r.enabled,
          dir: r.enabled ? r.brain_dir : null,
        }))
      : api.memoryStatus().then((r) => ({ on: r.enabled, dir: null }));
    load
      .then((r) => {
        if (cancelled) return;
        setEnabled(r.on);
        setDir(r.dir);
      })
      .catch(() => {
        if (!cancelled) setEnabled(null);
      });
    return () => {
      cancelled = true;
    };
  }, [companyId]);
  if (enabled === null) return null;
  const on = dir ? `${t("nav.memoryOn")} — ${dir}` : t("nav.memoryOn");
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-xs",
        enabled ? "text-primary" : "text-muted-foreground/50",
      )}
      title={enabled ? on : t("nav.memoryOff")}
    >
      <BrainCircuit className="h-3.5 w-3.5" />
      {t("nav.memory")}
    </span>
  );
}

/** The way in to this company's integration credentials (M9, ADR-0028).
 *
 * An icon, not a button with a word on it: most people never connect anything,
 * and the bar beside "New task" is not where a rarely-used control earns its
 * space. */
function Connections({ companyId }: { companyId: string }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        onClick={() => setOpen(true)}
        aria-label={t("connections.title")}
        title={t("connections.title")}
      >
        <Plug className="h-4.5 w-4.5" />
      </Button>
      <ConnectionsDialog open={open} onOpenChange={setOpen} companyId={companyId} />
    </>
  );
}

/** Periodically verifies the audit hash chain and shows a trust badge. */
function AuditIndicator({ tick }: { tick: number }) {
  const t = useT();
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
      title={valid ? t("nav.auditOk") : t("nav.auditBroken")}
    >
      {valid ? <ShieldCheck className="h-3.5 w-3.5" /> : <ShieldAlert className="h-3.5 w-3.5" />}
      {t("nav.audit")}
    </span>
  );
}
