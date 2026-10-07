mod hookup;

use std::path::Path;

use serde_json::{Value, json};

use crate::hookup::AgentHookup;
use crate::{AgentAdapter, Argv, Capabilities, GuardAnswer, LaunchSpec};
use hookup::AntigravityHookup;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Antigravity {
    pub program: String,
}

impl Default for Antigravity {
    fn default() -> Self {
        Self {
            program: "agy".into(),
        }
    }
}

impl Antigravity {
    pub const NAME: &str = "antigravity";
}

impl AgentAdapter for Antigravity {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn launch(&self, _spec: &LaunchSpec) -> Argv {
        Argv {
            program: self.program.clone(),
            args: Vec::new(),
        }
    }

    fn is_guard_payload(&self, payload: &str) -> bool {
        serde_json::from_str::<Value>(payload)
            .is_ok_and(|payload| payload.get("toolCall").is_some())
    }

    fn guard_answer(&self, answer: &GuardAnswer) -> Option<String> {
        let answer = match answer {
            GuardAnswer::Proceed => json!({ "decision": "ask" }),
            GuardAnswer::Ask => json!({ "decision": "force_ask" }),
            GuardAnswer::Deny { reason } => json!({ "decision": "deny", "reason": reason }),
        };
        Some(answer.to_string())
    }

    fn hookup(&self) -> Option<Box<dyn AgentHookup>> {
        Some(Box::new(AntigravityHookup))
    }

    fn user_statusline_command(
        &self,
        _cwd: &Path,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        hookup::saved_status_line_command(lookup)
    }
}
