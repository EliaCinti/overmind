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
  /** Characterized to work with visual material (ADR-0021). */
  multimodal: boolean;
}

/** The *function* an agent performs (ADR-0021). */
export interface Archetype {
  id: string;
  slug: string;
  name: string;
  description: string;
  default_traits: AgentTraits;
}

/** The *field* it performs it in — the second axis (ADR-0021). */
export interface Domain {
  id: string;
  slug: string;
  name: string;
  description: string;
  traits_patch: {
    focus_areas: string[];
    permissions: string[];
    multimodal: boolean;
    context: string;
  };
}

/**
 * A model an agent may run on. The dialog used to offer three hand-written
 * strings that were not model identifiers at all; it now reads the server's
 * catalog, which is the only thing that knows what we actually ship.
 */
export interface Model {
  id: string;
  display_name: string;
  vision: boolean;
}

export type LanguageCode = "en" | "it";

/**
 * A credential this company issued to something outside Overmind — an editor
 * session, a script (ADR-0028). Never carries the secret: that is shown once,
 * when it is created, and never again.
 */
export interface CompanyToken {
  id: string;
  label: string;
  created_at: string;
  /** `null` until something uses it — which is how you tell a live connection
   *  from one you set up and forgot. */
  last_used_at: string | null;
  /** Withdrawn. The row stays so the audit events naming it still point at
   *  something. */
  revoked_at: string | null;
}

/** The one response that carries the secret. */
export interface IssuedToken {
  id: string;
  label: string;
  token: string;
  created_at: string;
}

export interface Company {
  id: string;
  name: string;
  /** The language the company works in — UI *and* what the agents write (M16). */
  language: LanguageCode;
  created_at: string;
  /** Whether this company's brain is switched on (ADR-0024). Off is the
   *  no-provider path: agents work, they just stop remembering. */
  brain_enabled: boolean;
}

export interface Agent {
  id: string;
  name: string;
  archetype: string;
  /** `null` for agents hired before ADR-0021. */
  domain: string | null;
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
export type View = "chat" | "board" | "meetings" | "org" | "memory";

/**
 * Why a memory page is empty (ADR-0025). Four situations that render
 * identically if you only check `items.length`, and that ask different things
 * of the reader: configure a provider, switch the brain back on, do some work,
 * or accept that this provider cannot be listed.
 */
export type MemoryState = "ok" | "no_provider" | "brain_off" | "not_browsable";

/** One row of what the organization remembers. Every field is optional
 *  because the provider contract is generic — we render what we recognize. */
export interface MemoryItem {
  id: string | null;
  title: string | null;
  content: string | null;
  category: string | null;
  project: string | null;
  created_at: string | null;
  /** The task or meeting that produced it, when Overmind recorded one. */
  subject: { type: "task" | "meeting"; id: string; title: string } | null;
}

export interface MemoryPage {
  state: MemoryState;
  items: MemoryItem[];
}

export type MeetingStatus = "requested" | "open" | "decided" | "declined" | "failed"
  /** Out of budget mid-deliberation; waiting to be resumed (ADR-0022). */
  | "paused";

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
  /** Why the room is waiting, when it is (ADR-0022). */
  paused_note?: string | null;
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
  domain: string | null;
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
  /** Path relative to the run's deliverable root — `research/sources.csv`. */
  title: string;
  mime: string;
  /** Inline text, when the artifact is text and small enough to preview. */
  content: string | null;
  /** Whether bytes can be fetched from `artifactUrl` (M17). */
  downloadable: boolean;
  size_bytes: number;
  relative_path: string | null;
  created_at: string;
}

/** A turn in the CEO conversation (M12 / ADR-0018). */
/**
 * `system` is Overmind's own voice; `escalation` is an agent's, routed to the
 * leader's thread. They are separate roles on purpose — an agent that can write
 * a `system` message can tell you the system said something it did not
 * (ADR-0023, M10 slice 4).
 */
export type MessageRole = "user" | "ceo" | "system" | "escalation";

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
  /** The agent's own words, unwrapped from the adapter's envelope. */
  said: string | null;
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
  domain?: string | null;
  traits?: Partial<AgentTraits>;
  custom_brief?: string | null;
  title?: string | null;
  reports_to?: string | null;
}

/**
 * How this Overmind pays for the work it does (ADR-0030).
 *
 * A property of the *server*, not of a company: two companies on one machine
 * cannot be paying different ways. `metered` is the one the interface acts on —
 * it is the difference between a cap that is a ceiling in real money and a cap
 * that is an equivalent nobody will ever be charged.
 */
/**
 * Where a subscription stands in the window governing it (ADR-0030).
 *
 * Learned from the adapter's own `rate_limit_event`, so it is as fresh as the
 * last run and absent until one has happened. Never present under an API key,
 * where plan windows do not apply at all.
 */
/** The window names a plan has, in the order they bite. Mirrors `economy::PLAN_WINDOWS`. */
export const PLAN_WINDOWS = ["five_hour", "seven_day"] as const;

export interface PlanWindow {
  /** `five_hour` or `seven_day` — which limit is doing the limiting. */
  window: string;
  /** Unix epoch seconds. */
  resets_at: number;
  health: "allowed" | "warning" | "exhausted";
}

export type Economy =
  | {
      kind: "key";
      metered: true;
      /** A claude.ai login exists and the key is winning — the state where
       *  somebody believes their plan is paying and it is not. */
      overrides_login: boolean;
    }
  | { kind: "subscription"; metered: false; plan: string | null }
  | {
      kind: "unknown";
      metered: false;
      reason: string;
      /** Why it is unknown, machine-readable (M22): only `not_signed_in`
       *  carries a remedy the interface should offer; `custom_adapter` is
       *  deliberate and stays quiet. */
      unknown_kind: "not_signed_in" | "custom_adapter" | "unreadable";
    };

export const api = {
  /** Server identity, and how it pays. */
  /** The door (M24): where it stands, and the three ways through it. */
  authState: () =>
    req<{ state: "unclaimed" | "locked" | "in"; name?: string }>("GET", "/auth"),
  authClaim: (name: string, password: string) =>
    req<{ state: "in"; name: string }>("POST", "/auth/claim", { name, password }),
  authLogin: (name: string, password: string) =>
    req<{ state: "in"; name: string }>("POST", "/auth/login", { name, password }),
  authLogout: () => req<{ state: "locked" }>("POST", "/auth/logout"),

  /** The subscription sign-in flow, orchestrated by the server (M23). */
  claudeAuthStart: () => req<void>("POST", "/claude-auth/start"),
  claudeAuthStatus: () =>
    req<
      | { state: "idle" }
      | { state: "starting"; tail?: string }
      | { state: "exchanging"; tail?: string }
      | { state: "url_ready"; url: string; tail?: string }
      | { state: "done"; economy: Economy }
      | { state: "failed"; tail: string }
    >("GET", "/claude-auth"),
  claudeAuthCode: (code: string) => req<void>("POST", "/claude-auth/code", { code }),

  health: () =>
    req<{
      status: string;
      version: string;
      economy: Economy;
      /** Keyed by window name (`five_hour`, `seven_day`); a window nobody has
       *  reported yet is simply absent. */
      plan_windows: Record<string, PlanWindow>;
    }>("GET", "/health"),

  listCompanies: () => req<{ companies: Company[] }>("GET", "/companies").then((r) => r.companies),
  createCompany: (name: string, language: LanguageCode) =>
    req<Company>("POST", "/companies", { name, language }),
  listTokens: (companyId: string) =>
    req<{ tokens: CompanyToken[] }>("GET", `/companies/${companyId}/tokens`).then((r) => r.tokens),
  createToken: (companyId: string, label: string) =>
    req<IssuedToken>("POST", `/companies/${companyId}/tokens`, { label }),
  revokeToken: (tokenId: string) =>
    req<{ id: string; revoked_at: string }>("POST", `/tokens/${tokenId}/revoke`),

  setCompanyLanguage: (companyId: string, language: LanguageCode) =>
    req<{ id: string; language: LanguageCode }>("POST", `/companies/${companyId}/language`, {
      language,
    }),

  listArchetypes: () =>
    req<{ archetypes: Archetype[] }>("GET", "/archetypes").then((r) => r.archetypes),
  listDomains: () => req<{ domains: Domain[] }>("GET", "/domains").then((r) => r.domains),
  listModels: () => req<{ models: Model[] }>("GET", "/models").then((r) => r.models),

  listAgents: (companyId: string) =>
    req<{ agents: Agent[] }>("GET", `/companies/${companyId}/agents`).then((r) => r.agents),
  hireAgent: (companyId: string, body: HireAgentBody) =>
    req<Agent>("POST", `/companies/${companyId}/agents`, body),
  /** Raise (or lower) an agent's monthly cap — what unblocks a paused room. */
  setAgentBudget: (agentId: string, monthlyBudgetCents: number) =>
    req<{ id: string; monthly_budget_cents: number }>("POST", `/agents/${agentId}/budget`, {
      monthly_budget_cents: monthlyBudgetCents,
    }),
  resumeMeeting: (companyId: string, meetingId: string) =>
    req<{ id: string }>("POST", `/companies/${companyId}/meetings/${meetingId}/resume`, {}),
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

  memoryStatus: () => req<{ enabled: boolean; managed: boolean }>("GET", "/memory/status"),

  /** This company's own brain: is there a provider, is it on, where does it
   *  live (ADR-0024). `brain_dir` is null unless brains are managed. */
  brainStatus: (companyId: string) =>
    req<{ provider_configured: boolean; managed: boolean; enabled: boolean; brain_dir: string | null }>(
      "GET",
      `/companies/${companyId}/brain`,
    ),

  setBrainEnabled: (companyId: string, enabled: boolean) =>
    req<{ id: string; enabled: boolean }>("POST", `/companies/${companyId}/brain`, { enabled }),

  /** Browse what the company remembers. A `query` searches by meaning
   *  (`recall`); without one the brain is enumerated (ADR-0025). */
  browseMemory: (companyId: string, kind: "memories" | "decisions", query?: string) =>
    req<MemoryPage>(
      "GET",
      `/companies/${companyId}/memory/${kind}` +
        (query ? `?q=${encodeURIComponent(query)}` : ""),
    ),

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
  uploadAttachment: (companyId: string, agentId: string, file: File): Promise<Attachment> =>
    postFile(`/api/companies/${companyId}/agents/${agentId}/conversation/attachments`, file),

  /** URL that serves an attachment's bytes (for <img> / download links). */
  attachmentUrl: (companyId: string, attachmentId: string) =>
    `/api/companies/${companyId}/conversation/attachments/${attachmentId}`,

  // --- Files on a task (M17): what an agent picking it up will be handed.
  uploadTaskAttachment: (taskId: string, file: File): Promise<Attachment> =>
    postFile(`/api/tasks/${taskId}/attachments`, file),
  listTaskAttachments: (taskId: string) =>
    req<{ attachments: Attachment[] }>("GET", `/tasks/${taskId}/attachments`).then(
      (r) => r.attachments,
    ),
  removeTaskAttachment: (taskId: string, attachmentId: string) =>
    req<unknown>("DELETE", `/tasks/${taskId}/attachments/${attachmentId}`),

  /** URL that serves an artifact's bytes — images render from it, everything
   *  else downloads from it (M17). */
  artifactUrl: (artifactId: string) => `/api/artifacts/${artifactId}/download`,
};

/** Multipart upload of a single file. `fetch` directly: `req` sends JSON. */
function postFile(url: string, file: File): Promise<Attachment> {
  const form = new FormData();
  form.append("file", file);
  return fetch(url, { method: "POST", body: form }).then(async (res) => {
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
}
