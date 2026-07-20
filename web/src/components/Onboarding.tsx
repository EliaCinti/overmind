import { useState } from "react";
import { motion } from "motion/react";
import { Building2, FolderGit2, ArrowRight } from "lucide-react";
import type { Company } from "../lib/api";
import { api } from "../lib/api";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";

/**
 * First-run guidance. Two steps, shown one at a time:
 *  1. name a company, then
 *  2. point it at a git repository (creates the project + primary workspace +
 *     a default goal that tasks attach to).
 * `needsWorkspace` tells us which step the current company is on.
 */
export function Onboarding({
  company,
  needsWorkspace,
  onCompanyCreated,
  onReady,
}: {
  company: Company | null;
  needsWorkspace: boolean;
  onCompanyCreated: (id: string) => void;
  onReady: () => void;
}) {
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25, ease: [0.16, 1, 0.3, 1] }}
        className="w-full max-w-md"
      >
        {!company ? (
          <CompanyStep onCreated={onCompanyCreated} />
        ) : needsWorkspace ? (
          <WorkspaceStep company={company} onReady={onReady} />
        ) : null}
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

function CompanyStep({ onCreated }: { onCreated: (id: string) => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const c = await api.createCompany(name.trim());
      onCreated(c.id);
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

function WorkspaceStep({ company, onReady }: { company: Company; onReady: () => void }) {
  const [cwd, setCwd] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!cwd.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const project = await api.createProject(company.id, "Workspace");
      await api.createWorkspace(project.id, "main", cwd.trim());
      await api.createGoal(project.id, "Tasks");
      onReady();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed");
      setBusy(false);
    }
  };

  return (
    <StepShell
      icon={<FolderGit2 className="h-6 w-6" />}
      step="Step 2 of 2"
      title="Connect a git repo"
      subtitle="Agents work here — each run gets its own isolated worktree."
    >
      <div className="flex flex-col gap-4">
        <Field
          label="Repository path"
          hint="An absolute path to a git repository on this machine."
        >
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
      </div>
    </StepShell>
  );
}
