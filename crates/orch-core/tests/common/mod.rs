#![allow(dead_code)]

use std::time::Instant;

use orch_core::{AgentEvent, Effect, Observation, PhaseEvent, SessionStatus};

pub const OPEN_PR: PhaseEvent = PhaseEvent::PrOpened { number: 42 };

pub trait Feed {
    fn feed(&mut self, observation: Observation) -> Vec<Effect>;
    fn feed_event(&mut self, event: AgentEvent) -> Vec<Effect> {
        self.feed(Observation::Agent(event))
    }
}

impl Feed for SessionStatus {
    fn feed(&mut self, observation: Observation) -> Vec<Effect> {
        self.observe(observation, Instant::now())
    }
}

pub fn session_through(events: &[PhaseEvent]) -> SessionStatus {
    with_phase(SessionStatus::new(), events)
}

pub fn with_phase(mut status: SessionStatus, events: &[PhaseEvent]) -> SessionStatus {
    for event in events {
        status.transition(*event).unwrap();
    }
    status
}

pub fn active_session() -> SessionStatus {
    session_through(&[PhaseEvent::SetupSucceeded])
}

pub fn starting_session() -> SessionStatus {
    let mut status = active_session();
    status.feed(Observation::Spawned);
    status
}

pub fn idle_session() -> SessionStatus {
    let mut status = starting_session();
    status.feed_event(AgentEvent::SessionStarted);
    status
}

pub fn working_session() -> SessionStatus {
    let mut status = idle_session();
    status.feed_event(AgentEvent::PromptSubmitted);
    status
}

pub fn needs_input_session() -> SessionStatus {
    let mut status = idle_session();
    status.feed_event(AgentEvent::PermissionRequested);
    status
}

pub fn exited_session(code: i32) -> SessionStatus {
    let mut status = idle_session();
    status.feed(Observation::Exited { code: Some(code) });
    status
}
