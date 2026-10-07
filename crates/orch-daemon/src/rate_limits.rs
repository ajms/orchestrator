use orch_core::UsageSample;
use orch_protocol::{AgentRateLimits, FromDaemon, RateLimitView};

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RateLimits {
    agents: Vec<AgentRateLimits>,
}

impl RateLimits {
    pub(crate) fn is_known(&self) -> bool {
        !self.agents.is_empty()
    }

    pub(crate) fn note(&mut self, agent: &str, sample: &UsageSample) -> bool {
        let before = self.clone();
        for limit in &sample.limits {
            let windows = self.windows(agent);
            let latest = RateLimitView {
                name: limit.name.clone(),
                label: limit.label.clone(),
                used_percent: limit.used_percent,
            };
            match windows.iter_mut().find(|window| window.name == limit.name) {
                Some(window) => *window = latest,
                None => windows.push(latest),
            }
        }
        *self != before
    }

    fn windows(&mut self, agent: &str) -> &mut Vec<RateLimitView> {
        let index = match self.agents.iter().position(|known| known.agent == agent) {
            Some(index) => index,
            None => {
                self.agents.push(AgentRateLimits {
                    agent: agent.into(),
                    limits: Vec::new(),
                });
                self.agents.len() - 1
            }
        };
        &mut self.agents[index].limits
    }

    pub(crate) fn message(&self) -> FromDaemon {
        FromDaemon::RateLimits {
            agents: self.agents.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use orch_core::RateLimit;
    use orch_protocol::{AgentRateLimits, RateLimitView};

    use super::*;

    fn sample(windows: &[(&str, &str, f64)]) -> UsageSample {
        UsageSample {
            limits: windows
                .iter()
                .map(|(name, label, used)| RateLimit {
                    name: (*name).into(),
                    label: (*label).into(),
                    used_percent: *used,
                    resets_at_unix: None,
                })
                .collect(),
            ..UsageSample::default()
        }
    }

    fn agent(name: &str, windows: &[(&str, &str, f64)]) -> AgentRateLimits {
        AgentRateLimits {
            agent: name.into(),
            limits: windows
                .iter()
                .map(|(name, label, used)| RateLimitView {
                    name: (*name).into(),
                    label: (*label).into(),
                    used_percent: *used,
                })
                .collect(),
        }
    }

    #[test]
    fn each_agent_keeps_the_latest_value_of_its_own_windows() {
        let mut limits = RateLimits::default();
        limits.note("claude", &sample(&[("five_hour", "5h", 42.0)]));
        limits.note(
            "antigravity",
            &sample(&[("gemini-weekly", "gemini-wk", 7.0)]),
        );
        limits.note(
            "claude",
            &sample(&[("five_hour", "5h", 44.0), ("seven_day", "7d", 18.0)]),
        );

        assert_eq!(
            limits.message(),
            FromDaemon::RateLimits {
                agents: vec![
                    agent(
                        "claude",
                        &[("five_hour", "5h", 44.0), ("seven_day", "7d", 18.0)]
                    ),
                    agent("antigravity", &[("gemini-weekly", "gemini-wk", 7.0)]),
                ]
            }
        );
    }

    #[test]
    fn a_sample_changes_the_limits_only_when_a_window_moves() {
        let mut limits = RateLimits::default();
        assert!(!limits.note("claude", &sample(&[])));
        assert!(limits.note("claude", &sample(&[("five_hour", "5h", 42.0)])));
        assert!(!limits.note("claude", &sample(&[("five_hour", "5h", 42.0)])));
        assert!(limits.note("antigravity", &sample(&[("five_hour", "5h", 42.0)])));
    }
}
