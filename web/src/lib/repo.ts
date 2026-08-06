import { api } from "./api";

/**
 * Connect a git repo to a company: project + primary workspace + the default
 * goal that `code` tasks attach to. Optional by design — knowledge work needs
 * none of it (ADR-0017) — so this runs when you reach for code, not at setup.
 */
export async function connectRepo(companyId: string, cwd: string) {
  const project = await api.createProject(companyId, "Workspace");
  await api.createWorkspace(project.id, "main", cwd);
  await api.createGoal(project.id, "Tasks");
}
