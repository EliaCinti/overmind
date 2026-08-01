// Typed client for the Overmind API (served under /api). One function per
// endpoint; every request body/response is typed so the UI can't drift from
// the server contract.

export type Autonomy = "propose_only" | "act_with_approval" | "act_within_budget";
export type ReviewStrictness = "lenient" | "standard" | "strict";

export type TaskStatus =
  "backlog" | "todo" | "in_progress" | "in_review" | "blocked" | "done" | "cancelled";

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

export type LanguageCode = "en" | "it";

export interface Company {
  id: string;
  name: string;
  /** The language the company works in — UI *and* what the agents write (M16). */
  language: LanguageCode;
  created_at: string;
}

export interface Agent {
  id: string;
  name: string;
  archetype: string;
  traits: AgentTraits;
  custom_brief: string | null;
  title: string | null;
  reports_to: string | null;
  requires_approval: boolean;
  status: string;
}

export interface Approval {
  id: string;
  type: string;
  status: "pending" | "approved" | "rejected";
  summary: string;
  decision_note: string | null;
  created_at: string;
  decided_at: string | null;
}

export interface AgentBudget {
  agent_id: string;
  name: string;
  budget_cents: number;
  spent_cents: number;
  reserved_cents: number;
}

/** How the company reaches you (ADR-0020). Actionable when `approval_id` is set. */
export interface Notification {
  id: string;
  kind: string;
  /** Server-composed English. The fallback when `params` is absent (M16). */
  title: string;
  body: string;
  /** The values the sentence is made of, so we can word it in any language. */
  params: Record<string, string | number | null> | null;
  agent_id: string | null;
  subject_type: string | null;
  subject_id: string | null;
  approval_id: string | null;
  read_at: string | null;
  created_at: string;
}

/** The main surfaces of the app. */
export type View = "chat" | "board" | "meetings" | "org";

export type MeetingStatus = "requested" | "open" | "decided" | "declined" | "failed";

/** A meeting an agent asked for (or you convened). */
export interface Meeting {
  id: string;
  topic: string;
  reason: string;
  convener_agent_id: string | null;
  convener_name: string | null;
  turn_cap: number;
  status: MeetingStatus;
  decision: string | null;
  approval_id: string | null;
  created_at: string;
  decided_at: string | null;
}

export interface MeetingTurn {
  id: string;
  agent_id: string;
  agent_name: string;
  ordinal: number;
  content: string;
  created_at: string;
}

export interface MeetingDetail {
  meeting: Meeting & { company_id: string };
  participants: { id: string; name: string; title: string | null }[];
  turns: MeetingTurn[];
}

export type OrgProposalStatus = "proposed" | "accepted" | "rejected";

/** One hire the CEO suggests. `reports_to` is another member's *name*. */
export interface OrgProposalMember {
  id: string;
  position: number;
  name: string;
  archetype: string;
  title: string | null;
  reports_to: string | null;
  brief: string | null;
  rationale: string | null;
  excluded: boolean;
  hired_agent_id: string | null;
}

/** A team the CEO drew up (M15). Nobody is hired until you accept. */
export interface OrgProposal {
  id: string;
  summary: string;
  proposed_by_name: string | null;
  status: OrgProposalStatus;
  decline_note: string | null;
  approval_id: string | null;
  created_at: string;
  decided_at: string | null;
  members: OrgProposalMember[];
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

export type ExecutionKind = "code" | "knowledge";

export interface Task {
  id: string;
  goal_id: string | null;
  title: string;
  status: TaskStatus;
  priority: TaskPriority;
  assignee_agent_id: string | null;
  execution_kind: ExecutionKind;
  updated_at: string;
}

/** A knowledge run's deliverable (ADR-0017): a document, table, or research note. */
export interface Artifact {
  id: string;
  session_id: string;
  kind: string;
  title: string;
  mime: string;
  content: string | null;
  file_path: string | null;
  created_at: string;
}

/** A turn in the CEO conversation (M12 / ADR-0018). */
export type MessageRole = "user" | "ceo" | "system";

/** A file/image attached to a message. */
export interface Attachment {
  id: string;
  filename: string;
  mime: string;
  size_bytes: number;
}

export interface Message {
  id: string;
  role: MessageRole;
  content: string;
  created_at: string;
  attachments?: Attachment[];
}

export interface Conversation {
  id: string;
  agent_id: string;
  title: string;
  created_at: string;
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
  title?: string | null;
  reports_to?: string | null;
}

export const api = {
  listCompanies: () => req<{ companies: Company[] }>("GET", "/companies").then((r) => r.companies),
  createCompany: (name: string) => req<Company>("POST", "/companies", { name }),
  setCompanyLanguage: (companyId: string, language: LanguageCode) =>
    req<{ id: string; language: LanguageCode }>("POST", `/companies/${companyId}/language`, {
      language,
    }),

  listArchetypes: () =>
    req<{ archetypes: Archetype[] }>("GET", "/archetypes").then((r) => r.archetypes),

  listAgents: (companyId: string) =>
    req<{ agents: Agent[] }>("GET", `/companies/${companyId}/agents`).then((r) => r.agents),
  hireAgent: (companyId: string, body: HireAgentBody) =>
    req<Agent>("POST", `/companies/${companyId}/agents`, body),
  reassignAgent: (agentId: string, body: { reports_to?: string | null; title?: string }) =>
    req<{ id: string }>("POST", `/agents/${agentId}/reassign`, body),
  pauseAgent: (agentId: string) => req<unknown>("POST", `/agents/${agentId}/pause`),
  resumeAgent: (agentId: string) => req<unknown>("POST", `/agents/${agentId}/resume`),
  terminateAgent: (agentId: string) => req<unknown>("POST", `/agents/${agentId}/terminate`),
  setApprovalGate: (agentId: string, requires_approval: boolean) =>
    req<unknown>("POST", `/agents/${agentId}/approval-gate`, { requires_approval }),

  listApprovals: (companyId: string) =>
    req<{ approvals: Approval[] }>("GET", `/companies/${companyId}/approvals`).then(
      (r) => r.approvals,
    ),
  decideApproval: (approvalId: string, decision: "approve" | "reject", note?: string) =>
    req<{ id: string; status: string }>("POST", `/approvals/${approvalId}/decision`, {
      decision,
      note,
    }),

  listNotifications: (companyId: string) =>
    req<{ notifications: Notification[]; unread: number }>(
      "GET",
      `/companies/${companyId}/notifications`,
    ),
  readNotification: (id: string) => req<unknown>("POST", `/notifications/${id}/read`),
  readAllNotifications: (companyId: string) =>
    req<{ read: number }>("POST", `/companies/${companyId}/notifications/read`),

  listOrgProposals: (companyId: string) =>
    req<{ proposals: OrgProposal[] }>("GET", `/companies/${companyId}/org-proposals`).then(
      (r) => r.proposals,
    ),
  setProposalMemberExcluded: (proposalId: string, memberId: string, excluded: boolean) =>
    req<{ id: string; excluded: boolean }>(
      "POST",
      `/org-proposals/${proposalId}/members/${memberId}`,
      { excluded },
    ),

  listMeetings: (companyId: string) =>
    req<{ meetings: Meeting[] }>("GET", `/companies/${companyId}/meetings`).then((r) => r.meetings),
  getMeeting: (meetingId: string) => req<MeetingDetail>("GET", `/meetings/${meetingId}`),

  budgetSummary: (companyId: string) =>
    req<{ budgets: AgentBudget[]; window_start: string }>(
      "GET",
      `/companies/${companyId}/budget`,
    ).then((r) => r.budgets),

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
    body: {
      title: string;
      description?: string;
      goal_id?: string;
      priority?: TaskPriority;
      execution_kind?: ExecutionKind;
    },
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
    req<{ sessions: TaskSessionRef[] }>("GET", `/tasks/${taskId}/sessions`).then((r) => r.sessions),
  listTaskArtifacts: (taskId: string) =>
    req<{ artifacts: Artifact[] }>("GET", `/tasks/${taskId}/artifacts`).then((r) => r.artifacts),

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

  memoryStatus: () => req<{ enabled: boolean }>("GET", "/memory/status"),

  // Conversation with an agent — the CEO is the org leader (ADR-0019).
  getConversation: (companyId: string, agentId: string) =>
    req<{ conversation: Conversation | null; messages: Message[] }>(
      "GET",
      `/companies/${companyId}/agents/${agentId}/conversation`,
    ),
  postMessage: (companyId: string, agentId: string, content: string, attachmentIds?: string[]) =>
    req<{ conversation_id: string }>(
      "POST",
      `/companies/${companyId}/agents/${agentId}/conversation/messages`,
      { content, attachment_ids: attachmentIds ?? [] },
    ),
  /** Upload a file/image to an agent's thread; returns its attachment metadata. */
  uploadAttachment: (companyId: string, agentId: string, file: File): Promise<Attachment> => {
    const form = new FormData();
    form.append("file", file);
    return fetch(`/api/companies/${companyId}/agents/${agentId}/conversation/attachments`, {
      method: "POST",
      body: form,
    }).then(async (res) => {
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
      return (await res.json()) as Attachment;
    });
  },
  /** URL that serves an attachment's bytes (for <img> / download links). */
  attachmentUrl: (companyId: string, attachmentId: string) =>
    `/api/companies/${companyId}/conversation/attachments/${attachmentId}`,
};
