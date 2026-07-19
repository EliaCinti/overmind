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

/// Structured agent characterization (ADR-0005). Compiled into both the
/// agent's prompt context and its server-enforced configuration — one
/// source of truth for both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentTraits {
    pub focus_areas: Vec<String>,
    pub permissions: Vec<String>,
    pub autonomy: Autonomy,
    pub review_strictness: ReviewStrictness,
    pub monthly_budget_cents: i64,
    pub model: String,
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
        self
    }
}

/// Audit event kinds. Centralized so the catalog of what gets audited is
/// visible in one place.
pub mod event_kind {
    pub const COMPANY_CREATED: &str = "company.created";
    pub const AGENT_HIRED: &str = "agent.hired";
    pub const PROJECT_CREATED: &str = "project.created";
    pub const GOAL_CREATED: &str = "goal.created";
    pub const TASK_CREATED: &str = "task.created";
    pub const TASK_TRANSITIONED: &str = "task.transitioned";
    pub const WORKSPACE_CREATED: &str = "workspace.created";
    pub const SESSION_STARTED: &str = "session.started";
    pub const SESSION_FINISHED: &str = "session.finished";
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
