import { useCallback, useEffect, useState } from "react";
import { UserPlus } from "lucide-react";
import { api, ApiError, type Member } from "../lib/api";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Input } from "./ui/primitives";
import { useT } from "../lib/i18n";

/**
 * Who is inside this company, and the one verb membership has today:
 * bringing in a colleague who already has an account (M25, ADR-0033). Any
 * member can; the list is founder-first because that is the order people
 * came in, and the instance owner is marked because the administrator is
 * worth knowing -- not because there is a per-company role, there is none.
 */
export function MembersDialog({
  open,
  onOpenChange,
  companyId,
  me,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  companyId: string;
  /** The signed-in name, to say "you" beside it. */
  me: string | null;
}) {
  const t = useT();
  const [members, setMembers] = useState<Member[]>([]);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.listMembers(companyId).then(setMembers).catch(() => setMembers([]));
  }, [companyId]);

  useEffect(() => {
    if (open) refresh();
  }, [open, refresh]);

  const add = async () => {
    const who = name.trim();
    if (!who || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.addMember(companyId, who);
      setName("");
      refresh();
    } catch (e) {
      setError(e instanceof ApiError && e.status === 404 ? t("door.membersUnknown") : String(e));
    } finally {
      setBusy(false);
    }
  };

  const since = (iso: string) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        onOpenChange(o);
        if (!o) {
          setName("");
          setError(null);
        }
      }}
      title={t("door.members")}
    >
      <div className="space-y-4 p-4">
        <ul className="divide-y divide-border/60">
          {members.map((m) => (
            <li key={m.id} className="flex items-center justify-between py-2 text-sm">
              <span className="flex items-center gap-2">
                <span className="font-medium">{m.name}</span>
                {m.role === "owner" && (
                  <span className="rounded-full bg-primary/10 px-2 py-0.5 text-xs text-primary">
                    {t("door.membersOwner")}
                  </span>
                )}
                {me && m.name === me && (
                  <span className="text-xs text-muted-foreground">{t("door.membersYou")}</span>
                )}
              </span>
              <span className="text-xs text-muted-foreground">
                {t("door.membersSince", { date: since(m.added_at) })}
              </span>
            </li>
          ))}
        </ul>
        <div className="flex items-center gap-2">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("door.membersAddPlaceholder")}
            aria-label={t("door.membersAddPlaceholder")}
            onKeyDown={(e) => {
              if (e.key === "Enter") void add();
            }}
          />
          <Button onClick={add} disabled={!name.trim() || busy}>
            <UserPlus className="h-4 w-4" />
            {t("door.membersAdd")}
          </Button>
        </div>
        {error && <p className="text-sm text-destructive">{error}</p>}
        <p className="text-xs text-muted-foreground">{t("door.membersHint")}</p>
      </div>
    </Dialog>
  );
}
