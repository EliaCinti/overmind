// Typed client for the Overmind API (served under /api). One function per
// endpoint; every request body/response is typed so the UI can't drift from
// the server contract.

export type Autonomy = "propose_only" | "act_with_approval" | "act_within_budget";
export type ReviewStrictness = "lenient" | "standard" | "strict";

export type TaskStatus =
  | "backlog"
  | "todo"
  | "in_progress"
  | "in_review"
  | "blocked"
  | "done"
  | "cancelled";

export type TaskPriority = "low" | "medium" | "high" | "urgent";

export interface AgentTraits {
  focus_areas: string[];
  permissions: string[];
  autonomy: Autonomy;
  review_strictness: ReviewStrictness;
  monthly_budget_cents: number;
  model: string;
}

export interface Archetype {
  id: string;
  slug: string;
  name: string;
  description: string;
  default_traits: AgentTraits;
}

export interface Company {
  id: string;
  name: string;
  created_at: string;
}

export interface Agent {
  id: string;
  name: string;
  archetype: string;
  traits: AgentTraits;
  custom_brief: string | null;
  status: string;
}

export interface Project {
  id: string;
  title: string;
  created_at: string;
}

export interface Workspace {
  id: string;
  name: string;
  cwd: string;
  default_ref: string | null;
  is_primary: boolean;
}

export interface ProjectDetail {
  id: string;
  title: string;
  created_at: string;
  goals: { id: string; title: string }[];
  workspaces: { id: string; name: string; cwd: string; is_primary: boolean }[];
}

export interface Task {
  id: string;
  goal_id: string | null;
  title: string;
  status: TaskStatus;
  priority: TaskPriority;
  assignee_agent_id: string | null;
  updated_at: string;
}

export interface Session {
  id: string;
  task_id: string;
  agent_id: string;
  status: string;
  branch: string;
  workspace_path: string;
  base_sha: string | null;
  output: string | null;
  exit_code: number | null;
  last_error: string | null;
  cost_cents: number;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
}

export interface TaskSessionRef {
  id: string;
  agent_id: string;
  status: string;
  exit_code: number | null;
  last_error: string | null;
  created_at: string;
}

export interface AuditEvent {
  seq: number;
  company_id: string | null;
  task_id: string | null;
  kind: string;
  payload: unknown;
  created_at: string;
  hash: string;
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body ? { "content-type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    let message = res.statusText;
    try {
      const data = await res.json();
      if (data?.error) message = data.error;
    } catch {
      // keep statusText
    }
    throw new ApiError(res.status, message);
  }
  const text = await res.text();
  return (text ? JSON.parse(text) : null) as T;
}

export interface HireAgentBody {
  name: string;
  archetype: string;
  traits?: Partial<AgentTraits>;
  custom_brief?: string | null;
}

export const api = {
  listCompanies: () => req<{ companies: Company[] }>("GET", "/companies").then((r) => r.companies),
  createCompany: (name: string) => req<Company>("POST", "/companies", { name }),

  listArchetypes: () =>
    req<{ archetypes: Archetype[] }>("GET", "/archetypes").then((r) => r.archetypes),

  listAgents: (companyId: string) =>
    req<{ agents: Agent[] }>("GET", `/companies/${companyId}/agents`).then((r) => r.agents),
  hireAgent: (companyId: string, body: HireAgentBody) =>
    req<Agent>("POST", `/companies/${companyId}/agents`, body),

  listProjects: (companyId: string) =>
    req<{ projects: ProjectDetail[] }>("GET", `/companies/${companyId}/projects`).then(
      (r) => r.projects,
    ),
  createProject: (companyId: string, title: string) =>
    req<Project>("POST", `/companies/${companyId}/projects`, { title }),
  createGoal: (projectId: string, title: string) =>
    req<{ id: string }>("POST", `/projects/${projectId}/goals`, { title }),
  createWorkspace: (projectId: string, name: string, cwd: string, default_ref?: string) =>
    req<Workspace>("POST", `/projects/${projectId}/workspaces`, { name, cwd, default_ref }),
  listWorkspaces: (projectId: string) =>
    req<{ workspaces: Workspace[] }>("GET", `/projects/${projectId}/workspaces`).then(
      (r) => r.workspaces,
    ),

  listTasks: (companyId: string) =>
    req<{ tasks: Task[] }>("GET", `/companies/${companyId}/tasks`).then((r) => r.tasks),
  createTask: (
    companyId: string,
    body: { title: string; description?: string; goal_id?: string; priority?: TaskPriority },
  ) => req<Task>("POST", `/companies/${companyId}/tasks`, body),
  transitionTask: (taskId: string, to: TaskStatus, agent_id?: string) =>
    req<{ id: string; status: TaskStatus }>("POST", `/tasks/${taskId}/transition`, {
      to,
      agent_id,
    }),
  startTask: (taskId: string, agentId: string) =>
    req<{ session_id: string; branch: string; workspace_path: string }>(
      "POST",
      `/tasks/${taskId}/start`,
      { agent_id: agentId },
    ),

  getSession: (id: string) => req<Session>("GET", `/sessions/${id}`),
  getSessionDiff: (id: string) =>
    fetch(`/api/sessions/${id}/diff`).then((r) => (r.ok ? r.text() : "")),
  listTaskSessions: (taskId: string) =>
    req<{ sessions: TaskSessionRef[] }>("GET", `/tasks/${taskId}/sessions`).then(
      (r) => r.sessions,
    ),

  requestWakeup: (agentId: string, reason?: string) =>
    req<{ id: string }>("POST", `/agents/${agentId}/wakeup`, { reason }),

  auditEvents: (companyId: string) =>
    req<{ events: AuditEvent[] }>("GET", `/audit/events?company_id=${companyId}`).then(
      (r) => r.events,
    ),
  auditVerify: () =>
    req<{ valid: boolean; events_checked: number; first_invalid_seq: number | null }>(
      "GET",
      "/audit/verify",
    ),
};
