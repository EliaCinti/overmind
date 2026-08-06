use serde::{Deserialize, Serialize};

/// Lifecycle of a task. Status set follows Paperclip's canon
/// (docs/PAPERCLIP-ALIGNMENT.md). Transitions are validated server-side;
/// every accepted transition appends an audit event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Blocked,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Todo => "todo",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::InReview => "in_review",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Done => "done",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "backlog" => Some(TaskStatus::Backlog),
            "todo" => Some(TaskStatus::Todo),
            "in_progress" => Some(TaskStatus::InProgress),
            "in_review" => Some(TaskStatus::InReview),
            "blocked" => Some(TaskStatus::Blocked),
            "done" => Some(TaskStatus::Done),
            "cancelled" => Some(TaskStatus::Cancelled),
            _ => None,
        }
    }

    /// The complete transition table. `Done` and `Cancelled` are terminal.
    /// `InReview -> InProgress` is the "review rejected, back to work" path;
    /// `Blocked` is reachable from any active status and resumes to
    /// `Todo` or `InProgress`.
    pub fn can_transition(self, to: Self) -> bool {
        use TaskStatus::*;
        matches!(
            (self, to),
            (Backlog, Todo)
                | (Backlog, Cancelled)
                | (Todo, InProgress)
                | (Todo, Blocked)
                | (Todo, Cancelled)
                | (InProgress, InReview)
                | (InProgress, Blocked)
                | (InProgress, Cancelled)
                | (InReview, InProgress)
                | (InReview, Done)
                | (InReview, Cancelled)
                | (Blocked, Todo)
                | (Blocked, InProgress)
                | (Blocked, Cancelled)
        )
    }
}

/// Task priority (Paperclip canon: default `medium`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

impl TaskPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskPriority::Low => "low",
            TaskPriority::Medium => "medium",
            TaskPriority::High => "high",
            TaskPriority::Urgent => "urgent",
        }
    }
}

/// How much an agent may do on its own (ADR-0005: enforced, not suggested).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    ProposeOnly,
    ActWithApproval,
    ActWithinBudget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStrictness {
    Lenient,
    Standard,
    Strict,
}

/// What a run produces (ADR-0017). `code` = git worktree + diff (ADR-0008);
/// `knowledge` = no git, the agent produces artifacts (documents, tables,
/// research, decisions). `Code` is the default so existing tasks are unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    #[default]
    Code,
    Knowledge,
}

impl ExecutionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionKind::Code => "code",
            ExecutionKind::Knowledge => "knowledge",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "code" => Some(ExecutionKind::Code),
            "knowledge" => Some(ExecutionKind::Knowledge),
            _ => None,
        }
    }
}

/// Structured agent characterization (ADR-0005). Compiled into both the
/// agent's prompt context and its server-enforced configuration — one
/// source of truth for both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentTraits {
    pub focus_areas: Vec<String>,
    /// What the agent is allowed to do. Two families:
    ///
    /// - **`task:code` / `task:knowledge`** — *enforced*. The runner refuses to
    ///   check an agent out onto a task whose execution kind it does not hold
    ///   (see [`perm::for_execution_kind`]). A researcher cannot be put on a
    ///   code task by accident, or by an agent that assigned it one.
    /// - everything else (`repo:read`, `pr:approve`, …) — *declared*, compiled
    ///   into the prompt so the agent knows its remit, but not policed: we
    ///   shell out to an external CLI and cannot stop it. Real enforcement of
    ///   those needs sandboxing (M10) — pretending otherwise would be the
    ///   "security by prayer" ADR-0005 rejects.
    pub permissions: Vec<String>,
    pub autonomy: Autonomy,
    pub review_strictness: ReviewStrictness,
    pub monthly_budget_cents: i64,
    /// Which model runs this agent. Validated against [`crate::model`] wherever
    /// traits enter the system — an id the catalog does not name is refused at
    /// the boundary, not stored and handed to a prompt later.
    pub model: String,
    /// Whether the agent is characterized to work with visual material —
    /// images, screenshots, diagrams, video stills (ADR-0021).
    ///
    /// Enforced at checkout: a task carrying image inputs may only be handed to
    /// an agent that declares this. That is the same *kind* of rule as
    /// `task:code` — not a claim about what the spawned CLI can do, but a
    /// refusal to hand an agent work it was never characterized for.
    ///
    /// It is also checked against the model at hire time, where it is vacuous
    /// today: every model in the catalog can read images. Written anyway,
    /// because the catalog is where that fact lives and a model without vision
    /// is a plausible next entry.
    ///
    /// `#[serde(default)]` so agents hired before ADR-0021 read as `false`
    /// rather than failing to deserialize.
    #[serde(default)]
    pub multimodal: bool,
}

/// Partial override applied on top of an archetype's defaults at hire time
/// (UX Level 2 "tune": every field optional, absent means "keep default").
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TraitsPatch {
    pub focus_areas: Option<Vec<String>>,
    pub permissions: Option<Vec<String>>,
    pub autonomy: Option<Autonomy>,
    pub review_strictness: Option<ReviewStrictness>,
    pub monthly_budget_cents: Option<i64>,
    pub model: Option<String>,
    pub multimodal: Option<bool>,
}

/// What a domain contributes on top of a function's defaults (ADR-0021).
///
/// Additive by construction: a domain may add focus areas, add *declared*
/// capabilities, say the field is visual by nature, and describe itself to the
/// agent in one line. It cannot remove anything, and it cannot grant
/// `task:code` / `task:knowledge` — which kind of work an agent may be checked
/// out onto is a property of the function, not of the subject matter.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DomainPatch {
    #[serde(default)]
    pub focus_areas: Vec<String>,
    /// Declared capabilities only. Any `task:*` entry here is dropped when the
    /// patch is applied — see [`AgentTraits::with_domain`].
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Whether work in this field is visual by nature (A/V, design, physical
    /// spaces). Seeds `multimodal`; the user can still turn it off at hire.
    #[serde(default)]
    pub multimodal: bool,
    /// One line telling the agent what field it works in, compiled into its
    /// prompt alongside the persona (ADR-0005).
    #[serde(default)]
    pub context: String,
}

impl AgentTraits {
    pub fn apply(mut self, patch: TraitsPatch) -> Self {
        if let Some(v) = patch.focus_areas {
            self.focus_areas = v;
        }
        if let Some(v) = patch.permissions {
            self.permissions = v;
        }
        if let Some(v) = patch.autonomy {
            self.autonomy = v;
        }
        if let Some(v) = patch.review_strictness {
            self.review_strictness = v;
        }
        if let Some(v) = patch.monthly_budget_cents {
            self.monthly_budget_cents = v;
        }
        if let Some(v) = patch.model {
            self.model = v;
        }
        if let Some(v) = patch.multimodal {
            self.multimodal = v;
        }
        self
    }

    /// Fold a domain's contribution in, between the function's defaults and the
    /// user's own tuning (ADR-0021). Additive: focus areas and declared
    /// capabilities are merged without duplicates, `multimodal` can only be
    /// turned *on* here, and `task:*` is ignored — a field of work does not
    /// decide what kind of task an agent may be checked out onto.
    pub fn with_domain(mut self, patch: &DomainPatch) -> Self {
        for area in &patch.focus_areas {
            if !self.focus_areas.contains(area) {
                self.focus_areas.push(area.clone());
            }
        }
        for grant in &patch.permissions {
            if grant == perm::TASK_CODE || grant == perm::TASK_KNOWLEDGE {
                continue;
            }
            if !self.permissions.contains(grant) {
                self.permissions.push(grant.clone());
            }
        }
        self.multimodal |= patch.multimodal;
        self
    }
}

/// The permissions the server actually enforces (M14).
pub mod perm {
    use super::ExecutionKind;

    /// May be checked out onto `code` tasks: works in a git worktree, produces a diff.
    pub const TASK_CODE: &str = "task:code";
    /// May be checked out onto `knowledge` tasks: research, documents, decisions.
    pub const TASK_KNOWLEDGE: &str = "task:knowledge";

    /// Which permission a task of this kind requires of whoever works it.
    pub fn for_execution_kind(kind: ExecutionKind) -> &'static str {
        match kind {
            ExecutionKind::Code => TASK_CODE,
            ExecutionKind::Knowledge => TASK_KNOWLEDGE,
        }
    }
}

/// Audit event kinds. Centralized so the catalog of what gets audited is
/// visible in one place.
pub mod event_kind {
    pub const COMPANY_CREATED: &str = "company.created";
    pub const AGENT_HIRED: &str = "agent.hired";
    pub const AGENT_REASSIGNED: &str = "agent.reassigned";
    pub const PROJECT_CREATED: &str = "project.created";
    pub const GOAL_CREATED: &str = "goal.created";
    pub const TASK_CREATED: &str = "task.created";
    pub const TASK_TRANSITIONED: &str = "task.transitioned";
    pub const WORKSPACE_CREATED: &str = "workspace.created";
    pub const SESSION_STARTED: &str = "session.started";
    pub const SESSION_FINISHED: &str = "session.finished";
    pub const SESSION_RESUMED: &str = "session.resumed";
    pub const TASK_RELEASED: &str = "task.released";
    pub const ARTIFACT_CREATED: &str = "artifact.created";
    pub const CONVERSATION_CREATED: &str = "conversation.created";
    pub const MESSAGE_POSTED: &str = "message.posted";
    pub const ATTACHMENT_ADDED: &str = "attachment.added";
    pub const MEETING_REQUESTED: &str = "meeting.requested";
    pub const MEETING_CONVENED: &str = "meeting.convened";
    pub const MEETING_DECIDED: &str = "meeting.decided";
    pub const MEETING_DECLINED: &str = "meeting.declined";
    pub const MEETING_DROPPED: &str = "meeting.dropped";
    pub const MEETING_PAUSED: &str = "meeting.paused";
    pub const ORG_PROPOSED: &str = "org.proposed";
    pub const ORG_ACCEPTED: &str = "org.accepted";
    pub const ORG_REJECTED: &str = "org.rejected";
    pub const MEETING_FAILED: &str = "meeting.failed";
    pub const WAKEUP_REQUESTED: &str = "agent.wakeup_requested";
    pub const WAKEUP_PROCESSED: &str = "agent.wakeup_processed";
    pub const BUDGET_BLOCKED: &str = "budget.blocked";
    pub const APPROVAL_REQUESTED: &str = "approval.requested";
    pub const APPROVAL_DECIDED: &str = "approval.decided";
    pub const AGENT_PAUSED: &str = "agent.paused";
    pub const AGENT_RESUMED: &str = "agent.resumed";
    pub const AGENT_TERMINATED: &str = "agent.terminated";
    pub const CONFIG_REVISED: &str = "agent.config_revised";
    pub const CONFIG_ROLLED_BACK: &str = "agent.config_rolled_back";
}

#[cfg(test)]
mod tests {
    use super::TaskStatus::*;

    #[test]
    fn transition_table() {
        let valid = [
            (Backlog, Todo),
            (Backlog, Cancelled),
            (Todo, InProgress),
            (Todo, Blocked),
            (Todo, Cancelled),
            (InProgress, InReview),
            (InProgress, Blocked),
            (InProgress, Cancelled),
            (InReview, InProgress),
            (InReview, Done),
            (InReview, Cancelled),
            (Blocked, Todo),
            (Blocked, InProgress),
            (Blocked, Cancelled),
        ];
        for (from, to) in valid {
            assert!(
                from.can_transition(to),
                "{from:?} -> {to:?} should be valid"
            );
        }
        let invalid = [
            (Backlog, InProgress),
            (Backlog, Done),
            (Todo, Done),
            (Todo, InReview),
            (InProgress, Done),
            (Blocked, InReview),
            (Blocked, Done),
            (Done, InProgress),
            (Done, Backlog),
            (Cancelled, Todo),
            (InReview, InReview),
        ];
        for (from, to) in invalid {
            assert!(
                !from.can_transition(to),
                "{from:?} -> {to:?} should be invalid"
            );
        }
    }
}
