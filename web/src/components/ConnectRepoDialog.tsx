import { useState } from "react";
import { connectRepo } from "../lib/repo";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";
import { Field, Input } from "./ui/primitives";

/**
 * Connect a git repo to a company — the one thing `code` tasks need and
 * `knowledge` tasks don't (ADR-0017). Offered when you actually reach for it,
 * not as a toll gate at first run.
 *
 * Creates the project + primary workspace + the default goal that code tasks
 * attach to; the same three calls the first-run step makes.
 */
export function ConnectRepoDialog({
  open,
  onOpenChange,
  companyId,
  onConnected,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  companyId: string;
  onConnected: () => void;
}) {
  const [cwd, setCwd] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!cwd.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await connectRepo(companyId, cwd.trim());
      onConnected();
      onOpenChange(false);
      setCwd("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to connect the repository");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title="Connect a git repo"
      description="Agents work here — each code run gets its own isolated worktree."
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
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={busy || !cwd.trim()}>
            {busy ? "Connecting…" : "Connect"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
