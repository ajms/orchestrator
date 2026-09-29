#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    SettingUp,
    SetupFailed,
    Active,
    PrOpen,
    Suspended,
    Landed,
    Discarded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhaseEvent {
    SetupSucceeded,
    SetupFailed,
    SetupRetried,
    SetupSkipped,
    PrOpened { number: u64 },
    Landed,
    PrMerged,
    PrAbandoned,
    Suspended,
    Resumed,
    Discarded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: Phase,
    pub event: PhaseEvent,
}

impl Phase {
    pub fn is_live(self) -> bool {
        matches!(self, Phase::Active | Phase::PrOpen)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Phase::Landed | Phase::Discarded)
    }

    pub(crate) fn after(self, event: PhaseEvent, has_pr: bool) -> Result<Phase, InvalidTransition> {
        use Phase as P;
        use PhaseEvent as E;
        let next = match (self, event) {
            (P::SettingUp, E::SetupSucceeded) => P::Active,
            (P::SettingUp, E::SetupFailed) => P::SetupFailed,
            (P::SetupFailed, E::SetupRetried) => P::SettingUp,
            (P::SetupFailed, E::SetupSkipped) => P::Active,
            (P::Active, E::PrOpened { .. }) => P::PrOpen,
            (P::Suspended, E::PrOpened { .. }) if !has_pr => P::Suspended,
            (P::Active, E::Landed) => P::Landed,
            (P::Suspended, E::Landed) if !has_pr => P::Landed,
            (P::PrOpen, E::PrMerged) => P::Landed,
            (P::PrOpen, E::PrAbandoned) => P::Active,
            (P::Suspended, E::PrMerged) if has_pr => P::Landed,
            (P::Active | P::PrOpen, E::Suspended) => P::Suspended,
            (P::Suspended, E::Resumed) if has_pr => P::PrOpen,
            (P::Suspended, E::Resumed) => P::Active,
            (from, E::Discarded) if !from.is_terminal() => P::Discarded,
            (from, event) => return Err(InvalidTransition { from, event }),
        };
        Ok(next)
    }
}
