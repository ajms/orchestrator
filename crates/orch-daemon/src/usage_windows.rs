use orch_core::UsageSample;
use orch_protocol::{AgentUsageWindows, FromDaemon, UsageWindowView};

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct UsageWindows {
    agents: Vec<AgentUsageWindows>,
}

impl UsageWindows {
    pub(crate) fn is_known(&self) -> bool {
        !self.agents.is_empty()
    }

    pub(crate) fn note(&mut self, agent: &str, sample: &UsageSample) -> bool {
        let mut changed = false;
        for window in &sample.windows {
            let latest = UsageWindowView::from(window);
            let windows = self.windows(agent);
            match windows.iter_mut().find(|known| known.name == latest.name) {
                Some(known) if *known == latest => {}
                Some(known) => {
                    *known = latest;
                    changed = true;
                }
                None => {
                    windows.push(latest);
                    changed = true;
                }
            }
        }
        changed
    }

    fn windows(&mut self, agent: &str) -> &mut Vec<UsageWindowView> {
        let index = match self.agents.iter().position(|known| known.agent == agent) {
            Some(index) => index,
            None => {
                self.agents.push(AgentUsageWindows {
                    agent: agent.into(),
                    windows: Vec::new(),
                });
                self.agents.len() - 1
            }
        };
        &mut self.agents[index].windows
    }

    pub(crate) fn message(&self) -> FromDaemon {
        FromDaemon::UsageWindows {
            agents: self.agents.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use orch_agent::ClaudeCode;
    use orch_core::UsageWindow;

    use super::*;

    fn sample(windows: &[(&str, &str, f64)]) -> UsageSample {
        UsageSample {
            windows: windows
                .iter()
                .map(|(name, label, used)| UsageWindow {
                    name: (*name).into(),
                    label: (*label).into(),
                    used_percent: *used,
                    resets_at_unix: None,
                })
                .collect(),
            ..UsageSample::default()
        }
    }

    fn agent(name: &str, windows: &[(&str, &str, f64)]) -> AgentUsageWindows {
        AgentUsageWindows {
            agent: name.into(),
            windows: sample(windows).windows.iter().map(Into::into).collect(),
        }
    }

    #[test]
    fn each_agent_keeps_the_latest_value_of_its_own_windows() {
        let mut windows = UsageWindows::default();
        windows.note(ClaudeCode::NAME, &sample(&[("five_hour", "5h", 42.0)]));
        windows.note(
            "antigravity",
            &sample(&[("gemini-weekly", "gemini-wk", 7.0)]),
        );
        windows.note(
            ClaudeCode::NAME,
            &sample(&[("five_hour", "5h", 44.0), ("seven_day", "7d", 18.0)]),
        );

        assert_eq!(
            windows.message(),
            FromDaemon::UsageWindows {
                agents: vec![
                    agent(
                        ClaudeCode::NAME,
                        &[("five_hour", "5h", 44.0), ("seven_day", "7d", 18.0)]
                    ),
                    agent("antigravity", &[("gemini-weekly", "gemini-wk", 7.0)]),
                ]
            }
        );
    }

    #[test]
    fn a_sample_changes_the_windows_only_when_a_window_moves() {
        let mut windows = UsageWindows::default();
        let claude = ClaudeCode::NAME;
        assert!(!windows.note(claude, &sample(&[])));
        assert!(windows.note(claude, &sample(&[("five_hour", "5h", 42.0)])));
        assert!(!windows.note(claude, &sample(&[("five_hour", "5h", 42.0)])));
        assert!(windows.note("antigravity", &sample(&[("five_hour", "5h", 42.0)])));
    }
}
