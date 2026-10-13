use orch_core::UsageSample;
use orch_protocol::FromDaemon;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RateLimits {
    five_hour: Option<f64>,
    seven_day: Option<f64>,
}

impl RateLimits {
    pub(crate) fn is_known(self) -> bool {
        self != Self::default()
    }

    pub(crate) fn after(self, sample: &UsageSample) -> Self {
        Self {
            five_hour: sample
                .five_hour
                .map(|limit| limit.used_percent)
                .or(self.five_hour),
            seven_day: sample
                .seven_day
                .map(|limit| limit.used_percent)
                .or(self.seven_day),
        }
    }
}

impl From<RateLimits> for FromDaemon {
    fn from(limits: RateLimits) -> Self {
        FromDaemon::RateLimits {
            five_hour: limits.five_hour,
            seven_day: limits.seven_day,
        }
    }
}
