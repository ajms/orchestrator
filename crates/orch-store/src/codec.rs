use orch_agent::GuardKind;
use orch_agent::{mode_from_name, mode_name};
use orch_core::{AgentState, ChecksState, PermissionMode, Phase, PrState, ReviewDecision};
use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};

pub(crate) struct Stored<T>(pub(crate) T);

trait Named: Sized + Copy + 'static {
    const ALL: &'static [Self];
    fn name(self) -> &'static str;
}

impl Named for Phase {
    const ALL: &'static [Self] = &[
        Phase::SettingUp,
        Phase::SetupFailed,
        Phase::Active,
        Phase::PrOpen,
        Phase::Suspended,
        Phase::Landed,
        Phase::Discarded,
    ];
    fn name(self) -> &'static str {
        match self {
            Phase::SettingUp => "setting_up",
            Phase::SetupFailed => "setup_failed",
            Phase::Active => "active",
            Phase::PrOpen => "pr_open",
            Phase::Suspended => "suspended",
            Phase::Landed => "landed",
            Phase::Discarded => "discarded",
        }
    }
}

impl Named for ChecksState {
    const ALL: &'static [Self] = &[
        ChecksState::None,
        ChecksState::Pending,
        ChecksState::Passing,
        ChecksState::Failing,
    ];
    fn name(self) -> &'static str {
        match self {
            ChecksState::None => "none",
            ChecksState::Pending => "pending",
            ChecksState::Passing => "passing",
            ChecksState::Failing => "failing",
        }
    }
}

impl Named for ReviewDecision {
    const ALL: &'static [Self] = &[
        ReviewDecision::None,
        ReviewDecision::ReviewRequired,
        ReviewDecision::Approved,
        ReviewDecision::ChangesRequested,
    ];
    fn name(self) -> &'static str {
        match self {
            ReviewDecision::None => "none",
            ReviewDecision::ReviewRequired => "review_required",
            ReviewDecision::Approved => "approved",
            ReviewDecision::ChangesRequested => "changes_requested",
        }
    }
}

impl Named for PrState {
    const ALL: &'static [Self] = &[PrState::Open, PrState::Closed];
    fn name(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Closed => "closed",
        }
    }
}

impl Named for AgentState {
    const ALL: &'static [Self] = &[
        AgentState::Starting,
        AgentState::Working,
        AgentState::NeedsInput,
        AgentState::Idle,
        AgentState::Errored,
        AgentState::Exited,
        AgentState::Unknown,
    ];
    fn name(self) -> &'static str {
        match self {
            AgentState::Starting => "starting",
            AgentState::Working => "working",
            AgentState::NeedsInput => "needs_input",
            AgentState::Idle => "idle",
            AgentState::Errored => "errored",
            AgentState::Exited => "exited",
            AgentState::Unknown => "unknown",
        }
    }
}

impl Named for GuardKind {
    const ALL: &'static [Self] = &[
        GuardKind::BaseBranch,
        GuardKind::OtherRef,
        GuardKind::WorktreeManagement,
        GuardKind::WriteOutsideWorktree,
        GuardKind::ExternalTool,
    ];
    fn name(self) -> &'static str {
        match self {
            GuardKind::BaseBranch => "base_branch",
            GuardKind::OtherRef => "other_ref",
            GuardKind::WorktreeManagement => "worktree_management",
            GuardKind::WriteOutsideWorktree => "write_outside_worktree",
            GuardKind::ExternalTool => "external_tool",
        }
    }
}

impl<T: Named> ToSql for Stored<T> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.name()))
    }
}

impl<T: Named> FromSql for Stored<T> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        T::ALL
            .iter()
            .find(|candidate| candidate.name() == text)
            .map(|found| Stored(*found))
            .ok_or_else(|| FromSqlError::Other(format!("unknown stored value {text:?}").into()))
    }
}

impl ToSql for Stored<PermissionMode> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(mode_name(self.0)))
    }
}

impl FromSql for Stored<PermissionMode> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        mode_from_name(text)
            .map(Stored)
            .ok_or_else(|| FromSqlError::Other(format!("unknown permission mode {text:?}").into()))
    }
}
