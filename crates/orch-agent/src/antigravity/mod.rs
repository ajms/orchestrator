mod hooks;
mod hookup;
mod statusline;
mod usage;

use std::path::Path;

use orch_core::{AgentEvent, ConversationId, PermissionMode};
use serde_json::json;

use crate::hookup::AgentHookup;
use crate::{
    AgentAdapter, Argv, Capabilities, Draft, DraftInput, GuardAnswer, LaunchSpec, PayloadError,
};
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

    fn argv(&self, args: Vec<String>) -> Argv {
        Argv {
            program: self.program.clone(),
            args,
        }
    }
}

const GUARD_EVENT: &str = "PreToolUse";
const NAMED_MODES: [PermissionMode; 2] = [PermissionMode::AcceptEdits, PermissionMode::Plan];

fn mode_name(mode: PermissionMode) -> Option<&'static str> {
    match mode {
        PermissionMode::AcceptEdits => Some("accept-edits"),
        PermissionMode::Plan => Some("plan"),
        _ => None,
    }
}

fn mode_from_name(name: Option<&str>) -> Option<PermissionMode> {
    match name {
        None | Some("") => Some(PermissionMode::Default),
        Some(name) => NAMED_MODES
            .into_iter()
            .find(|mode| mode_name(*mode) == Some(name)),
    }
}

fn mode_args(mode: Option<PermissionMode>) -> Vec<String> {
    mode.and_then(mode_name)
        .map(|name| vec!["--mode".into(), name.into()])
        .unwrap_or_default()
}

impl AgentAdapter for Antigravity {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hooks: true,
            resume: true,
            usage: true,
            modes: true,
            ..Capabilities::default()
        }
    }

    fn modes(&self) -> &'static [PermissionMode] {
        &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
        ]
    }

    fn launch(&self, spec: &LaunchSpec) -> Argv {
        let mut args = mode_args(spec.preset.mode);
        if let Some(prompt) = &spec.prompt {
            args.extend(["-i".into(), prompt.clone()]);
        }
        self.argv(args)
    }

    fn resume(
        &self,
        spec: &LaunchSpec,
        conversation: &ConversationId,
        observed_mode: Option<PermissionMode>,
    ) -> Option<Argv> {
        let mut args = vec!["--conversation".into(), conversation.as_str().into()];
        args.extend(mode_args(spec.resume_mode(observed_mode)));
        Some(self.argv(args))
    }

    fn draft(&self, _conversation: Option<&ConversationId>) -> Option<Draft> {
        Some(Draft {
            argv: self.argv(vec!["-p".into()]),
            input: DraftInput::InstructionAndBaseDiff,
        })
    }

    fn map_hook(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        hooks::map_hook(payload)
    }

    fn map_tap(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        statusline::map_tap(payload)
    }

    fn is_guard_payload(&self, payload: &str) -> bool {
        hooks::is_guard_payload(payload)
    }

    fn guard_answer(&self, answer: &GuardAnswer) -> Option<String> {
        let answer = match answer {
            GuardAnswer::Proceed => json!({ "decision": "ask" }),
            GuardAnswer::Ask => json!({ "decision": "force_ask" }),
            GuardAnswer::Deny { reason } => json!({ "decision": "deny", "reason": reason }),
        };
        Some(answer.to_string())
    }

    fn fallback_hook_reply(&self) -> Option<String> {
        Some(json!({ "decision": "ask" }).to_string())
    }

    fn fallback_statusline(&self, _payload: &str) -> String {
        String::new()
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
