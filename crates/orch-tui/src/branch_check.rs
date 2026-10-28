use std::path::PathBuf;
use std::time::{Duration, Instant};

use orch_protocol::{BranchLookup, LookupBranch, LookupDepth, OpenPr, Request};

pub(crate) const ORIGIN_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LookupKey {
    pub repo: PathBuf,
    pub branch: String,
    pub base: Option<String>,
}

impl LookupKey {
    pub(crate) fn request(&self, depth: LookupDepth) -> Request {
        Request::LookupBranch(LookupBranch {
            repo: self.repo.clone(),
            branch: self.branch.clone(),
            base: self.base.clone(),
            depth,
        })
    }
}

pub(crate) enum BranchStatus<'a> {
    Derived,
    Looking,
    Unknown(&'a str),
    Found(&'a BranchLookup),
}

#[derive(Default)]
pub(crate) struct BranchCheck {
    key: Option<LookupKey>,
    changed_at: Option<Instant>,
    local: Option<BranchLookup>,
    origin: Option<Result<BranchLookup, String>>,
    asked_origin: bool,
    waiting: bool,
}

impl BranchCheck {
    pub(crate) fn follow(
        &mut self,
        key: Option<LookupKey>,
        left_field: bool,
        now: Instant,
    ) -> Vec<(LookupKey, LookupDepth)> {
        let mut asks = Vec::new();
        if key != self.key {
            *self = Self {
                changed_at: Some(now),
                key,
                ..Self::default()
            };
            asks.extend(self.key.clone().map(|key| (key, LookupDepth::Local)));
        }
        let quiet = self
            .changed_at
            .is_some_and(|changed| now >= changed + ORIGIN_DELAY);
        if let Some(key) = &self.key
            && !self.asked_origin
            && (quiet || left_field || self.waiting)
        {
            self.asked_origin = true;
            asks.push((key.clone(), LookupDepth::Origin));
        }
        asks
    }

    pub(crate) fn origin_due(&self) -> bool {
        self.key.is_some() && !self.asked_origin
    }

    pub(crate) fn receive(
        &mut self,
        key: &LookupKey,
        depth: LookupDepth,
        lookup: Result<BranchLookup, String>,
    ) {
        if self.key.as_ref() != Some(key) {
            return;
        }
        match depth {
            LookupDepth::Origin => self.origin = Some(lookup),
            LookupDepth::Local => self.local = lookup.ok(),
        }
    }

    pub(crate) fn status(&self) -> BranchStatus<'_> {
        if self.key.is_none() {
            return BranchStatus::Derived;
        }
        match (&self.origin, &self.local) {
            (Some(Ok(lookup)), _) => BranchStatus::Found(lookup),
            (_, Some(refused @ BranchLookup::Refused { .. })) => BranchStatus::Found(refused),
            (Some(Err(reason)), _) => BranchStatus::Unknown(reason),
            _ => BranchStatus::Looking,
        }
    }

    pub(crate) fn refused(&self) -> bool {
        matches!(
            self.status(),
            BranchStatus::Found(BranchLookup::Refused { .. })
        )
    }

    pub(crate) fn looking(&self) -> bool {
        matches!(self.status(), BranchStatus::Looking)
    }

    pub(crate) fn open_pr(&self) -> Option<&OpenPr> {
        match self.status() {
            BranchStatus::Found(BranchLookup::Existing { pr, .. }) => pr.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn waiting(&self) -> bool {
        self.waiting
    }

    pub(crate) fn wait(&mut self) {
        self.waiting = true;
    }

    pub(crate) fn stop_waiting(&mut self) -> bool {
        std::mem::take(&mut self.waiting)
    }

    pub(crate) fn wait_over(&mut self) -> bool {
        self.waiting && !self.looking() && self.stop_waiting()
    }
}
