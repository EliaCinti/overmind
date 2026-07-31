import { useState } from "react";
import { motion } from "motion/react";
import { Building2, FolderGit2, ArrowRight } from "lucide-react";
import type { Company } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";
import { connectRepo } from "../lib/repo";

/**
 * First run. Name a company — that is the only required step; the company is
 * usable the moment it exists. Then the offer to connect a git repo, which is
 * **optional**: it is what `code` tasks need (ADR-0008), and a company doing
 * research, documents or decisions (ADR-0016/0017) never needs one. Skipping
 * costs nothing — the repo can be connected later, from where you reach for it.
 */
export function Onboarding({ onDone }: { onDone: (companyId: string) => void }) {
  const [created, setCreated] = useState<Company | null>(null);

  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <motion.div
        key={created ? "repo" : "company"}
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25, ease: [0.16, 1, 0.3, 1] }}
        className="w-full max-w-md"
      >
        {!created ? (
          <CompanyStep onCreated={setCreated} />
        ) : (
          <RepoStep company={created} onDone={() => onDone(created.id)} />
        )}
      </motion.div>
    </div>
  );
}

function StepShell({
  icon,
  step,
  title,
  subtitle,
  children,
}: {
  icon: React.ReactNode;
  step: string;
  title: string;
  subtitle: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-border bg-card p-7 shadow-soft">
      <div className="mb-5 flex flex-col items-center text-center">
        <span className="mb-3 flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
          {icon}
        </span>
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {step}
        </span>
        <h1 className="mt-1 text-xl font-semibold">{title}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>
      </div>
      {children}
    </div>
  );
}

function CompanyStep({ onCreated }: { onCreated: (c: Company) => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    try {
      onCreated(await api.createCompany(name.trim()));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed");
      setBusy(false);
    }
  };

  return (
    <StepShell
      icon={<Building2 className="h-6 w-6" />}
      step="Step 1 of 2"
      title="Name your company"
      subtitle="An organization of AI agents that work for you."
    >
      <div className="flex flex-col gap-4">
        <Field label="Company name">
          <Input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Acme Labs"
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </Field>
        {error && <p className="text-sm text-destructive">{error}</p>}
        <Button variant="primary" onClick={submit} disabled={busy || !name.trim()}>
          {busy ? "Creating…" : "Continue"}
          <ArrowRight className="h-4 w-4" />
        </Button>
      </div>
    </StepShell>
  );
}

function RepoStep({ company, onDone }: { company: Company; onDone: () => void }) {
  const [cwd, setCwd] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!cwd.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await connectRepo(company.id, cwd.trim());
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed");
      setBusy(false);
    }
  };

  return (
    <StepShell
      icon={<FolderGit2 className="h-6 w-6" />}
      step="Step 2 of 2 · optional"
      title="Connect a git repo"
      subtitle="Only for agents that write code — each run gets its own isolated worktree. Research, documents and decisions need no repo."
    >
      <div className="flex flex-col gap-4">
        <Field label="Repository path" hint="An absolute path to a git repository on this machine.">
          <Input
            autoFocus
            value={cwd}
            onChange={(e) => setCwd(e.target.value)}
            placeholder="/Users/you/code/my-project"
            className="mono"
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </Field>
        {error && <p className="text-sm text-destructive">{error}</p>}
        <Button variant="primary" onClick={submit} disabled={busy || !cwd.trim()}>
          {busy ? "Setting up…" : "Finish setup"}
          <ArrowRight className="h-4 w-4" />
        </Button>
        <button
          onClick={onDone}
          disabled={busy}
          className="-mt-1 self-center rounded-md px-2 py-1 text-sm text-muted-foreground transition hover:bg-muted hover:text-foreground disabled:opacity-50"
        >
          Skip — no code work yet
        </button>
      </div>
    </StepShell>
  );
}
