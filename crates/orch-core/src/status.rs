use std::time::{Duration, Instant};

use crate::flags::{Attention, ChecksState, Effect, Flags, PrState, PrStatus, ReviewDecision};
use crate::gate::{DiscardPlan, GateRefusal, agent_settled};
use crate::{
    AgentEvent, AgentState, ConversationId, InvalidTransition, Observation, PermissionMode, Phase,
    PhaseEvent, Subagent, SubagentId, UsageSample,
};

type PrCondition = fn(&PrStatus) -> bool;

#[derive(Debug, Clone)]
pub struct SessionStatus {
    phase: Phase,
    agent_state: Option<AgentState>,
    agent_process_alive: bool,
    turn_reported: bool,
    permission_pending: bool,
    observed: bool,
    flags: Flags,
    watched: bool,
    last_activity: Option<Instant>,
    permission_mode: Option<PermissionMode>,
    conversation: Option<ConversationId>,
    usage: Option<UsageSample>,
    subagents: Vec<Subagent>,
    raised: Vec<Attention>,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStatus {
    pub fn new() -> Self {
        Self {
            phase: Phase::SettingUp,
            agent_state: None,
            agent_process_alive: false,
            turn_reported: false,
            permission_pending: false,
            observed: true,
            flags: Flags::default(),
            watched: false,
            last_activity: None,
            permission_mode: None,
            conversation: None,
            usage: None,
            subagents: Vec::new(),
            raised: Vec::new(),
        }
    }

    pub fn unobserved() -> Self {
        Self {
            observed: false,
            ..Self::new()
        }
    }

    pub fn restore(&mut self, phase: Phase, flags: Flags) {
        self.phase = phase;
        self.flags = flags;
    }

    pub fn restore_agent(&mut self, state: AgentState, process_alive: bool) {
        self.agent_state = Some(state);
        self.agent_process_alive = process_alive;
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn agent_state(&self) -> Option<AgentState> {
        self.agent_state
    }

    pub fn flags(&self) -> &Flags {
        &self.flags
    }

    pub fn permission_mode(&self) -> Option<PermissionMode> {
        self.permission_mode
    }

    pub fn conversation(&self) -> Option<&ConversationId> {
        self.conversation.as_ref()
    }

    pub fn usage(&self) -> Option<&UsageSample> {
        self.usage.as_ref()
    }

    pub fn subagents(&self) -> &[Subagent] {
        &self.subagents
    }

    pub fn take_attention(&mut self) -> Vec<Attention> {
        std::mem::take(&mut self.raised)
    }

    pub fn transition(&mut self, event: PhaseEvent) -> Result<(), InvalidTransition> {
        let pr_closed = self
            .flags
            .pr
            .as_ref()
            .is_some_and(|pr| pr.state == PrState::Closed);
        if event == PhaseEvent::PrAbandoned && !pr_closed {
            return Err(InvalidTransition {
                from: self.phase,
                event,
            });
        }
        self.phase = self.phase.after(event, self.has_pr())?;
        match event {
            PhaseEvent::PrOpened { number } => self.flags.pr = Some(PrStatus::opened(number)),
            PhaseEvent::PrAbandoned => self.flags.pr = None,
            _ => {}
        }
        if !self.phase.is_live() {
            self.agent_state = None;
            self.agent_process_alive = false;
            self.flags.stalled = false;
        }
        match event {
            PhaseEvent::SetupFailed => self.raise(Attention::SetupFailed),
            PhaseEvent::PrMerged => self.raise(Attention::PrMerged),
            _ => {}
        }
        Ok(())
    }

    pub fn observe(&mut self, observation: Observation, at: Instant) -> Vec<Effect> {
        if !self.phase.is_live() {
            return Vec::new();
        }
        match observation {
            Observation::Spawned => self.agent_process_alive = true,
            Observation::Exited { .. } => self.agent_process_alive = false,
            _ if !self.agent_process_alive => return Vec::new(),
            _ => {}
        }
        if !matches!(observation, Observation::Agent(AgentEvent::UsageSample(_))) {
            self.last_activity = Some(at);
            self.flags.stalled = false;
        }

        if let Observation::Agent(event) = &observation {
            self.record(event);
        }

        let before = self.agent_state;
        let current = before.unwrap_or(AgentState::Starting);
        let after = match self.ignores(&observation) {
            true => current,
            false => current.after(&observation, self.observed),
        };
        self.agent_state = Some(after);
        self.track_side_channel(&observation, after);

        let turn_ended = observation == Observation::Agent(AgentEvent::TurnEnded);
        let attention = match after {
            _ if before == Some(after) => None,
            AgentState::NeedsInput => Some(Attention::NeedsInput),
            AgentState::Errored => Some(Attention::Errored),
            AgentState::Idle if turn_ended => Some(Attention::TurnEnded),
            _ => None,
        };
        if let Some(attention) = attention {
            self.raise(attention);
        }
        match turn_ended && self.flags.needs_rebase {
            true => vec![Effect::RecheckRebase],
            false => Vec::new(),
        }
    }

    pub fn tick(&mut self, now: Instant, stalled_after: Duration) {
        let quiet_for = self
            .last_activity
            .map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
        if self.agent_state == Some(AgentState::Working) && quiet_for >= stalled_after {
            self.flags.stalled = true;
        }
    }

    pub fn is_watched(&self) -> bool {
        self.watched
    }

    pub fn set_watched(&mut self, watched: bool) {
        self.watched = watched;
        if watched {
            self.flags.unseen = false;
        }
    }

    pub fn flag_needs_rebase(&mut self) {
        self.flags.needs_rebase = true;
    }

    pub fn rebase_checked(&mut self, contains_base_tip: bool) {
        if contains_base_tip {
            self.flags.needs_rebase = false;
        }
    }

    pub fn update_pr(&mut self, pr: PrStatus) {
        let previous = self.flags.pr.replace(pr.clone());
        let news_worth_attention: [(Attention, PrCondition); 3] = [
            (Attention::ChecksFailing, |pr| {
                pr.checks == ChecksState::Failing
            }),
            (Attention::ChangesRequested, |pr| {
                pr.review == ReviewDecision::ChangesRequested
            }),
            (Attention::PrClosed, |pr| pr.state == PrState::Closed),
        ];
        for (attention, holds) in news_worth_attention {
            if holds(&pr) && !previous.as_ref().is_some_and(holds) {
                self.raise(attention);
            }
        }
    }

    pub fn set_recovered(&mut self, recovered: bool) {
        self.flags.recovered = recovered;
    }

    pub fn set_worktree_missing(&mut self, missing: bool) {
        self.flags.worktree_missing = missing;
    }

    pub fn set_base_missing(&mut self, missing: bool) {
        self.flags.base_missing = missing;
    }

    pub fn forget_permission_mode(&mut self) {
        self.permission_mode = None;
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.flags.muted = muted;
    }

    pub fn check_landing(&self) -> Result<(), GateRefusal> {
        self.check_phase_allows(PhaseEvent::Landed)?;
        self.agent_settled()
    }

    pub fn check_preset_change(&self) -> Result<(), GateRefusal> {
        match self.phase {
            Phase::Active | Phase::PrOpen | Phase::Suspended => self.agent_settled(),
            phase => Err(GateRefusal::WrongPhase { phase }),
        }
    }

    pub fn check_discard(&self) -> Result<DiscardPlan, GateRefusal> {
        self.check_phase_allows(PhaseEvent::Discarded)?;
        Ok(DiscardPlan {
            stop_agent: self.agent_process_alive,
            stop_setup: self.phase == Phase::SettingUp,
        })
    }

    fn has_pr(&self) -> bool {
        self.flags.pr.is_some()
    }

    fn check_phase_allows(&self, event: PhaseEvent) -> Result<(), GateRefusal> {
        self.phase
            .after(event, self.has_pr())
            .map(|_| ())
            .map_err(|refused| GateRefusal::WrongPhase {
                phase: refused.from,
            })
    }

    fn agent_settled(&self) -> Result<(), GateRefusal> {
        if self.phase == Phase::Suspended {
            return Ok(());
        }
        agent_settled(self.agent_state)
    }

    fn ignores(&self, observation: &Observation) -> bool {
        match observation {
            Observation::Agent(AgentEvent::Ready) => self.turn_reported,
            Observation::Agent(AgentEvent::PermissionCleared) => !self.permission_pending,
            _ => false,
        }
    }

    fn track_side_channel(&mut self, observation: &Observation, after: AgentState) {
        use AgentEvent as E;
        match observation {
            Observation::Spawned => self.turn_reported = false,
            Observation::Agent(
                E::PromptSubmitted
                | E::ToolStarted { .. }
                | E::ToolFinished { .. }
                | E::QuestionAsked
                | E::TurnEnded
                | E::Failed { .. },
            ) => self.turn_reported = true,
            _ => {}
        }
        self.permission_pending = match observation {
            Observation::Agent(E::PermissionRequested) => true,
            Observation::Agent(E::QuestionAsked) | Observation::GuardPrompted => false,
            _ => self.permission_pending && after == AgentState::NeedsInput,
        };
    }

    fn record(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ModeChanged { mode } => self.permission_mode = Some(*mode),
            AgentEvent::ConversationChanged { id } => self.conversation = Some(id.clone()),
            AgentEvent::UsageSample(sample) => self.usage = Some(sample.clone()),
            AgentEvent::SubagentStarted {
                id,
                agent_type,
                description,
            } => match self.subagent_mut(id) {
                Some(resumed) => resumed.done = false,
                None => self.subagents.push(Subagent {
                    id: id.clone(),
                    agent_type: agent_type.clone(),
                    description: description.clone(),
                    tool_count: 0,
                    done: false,
                }),
            },
            AgentEvent::ToolStarted {
                subagent: Some(id), ..
            } => {
                if let Some(subagent) = self.subagent_mut(id) {
                    subagent.tool_count += 1;
                }
            }
            AgentEvent::SubagentFinished { id } => {
                if let Some(subagent) = self.subagent_mut(id) {
                    subagent.done = true;
                }
            }
            _ => {}
        }
    }

    fn subagent_mut(&mut self, id: &SubagentId) -> Option<&mut Subagent> {
        self.subagents
            .iter_mut()
            .find(|subagent| &subagent.id == id)
    }

    fn raise(&mut self, attention: Attention) {
        if attention.marks_unseen() && !self.watched {
            self.flags.unseen = true;
        }
        self.raised.push(attention);
    }
}
