mod hooks;
mod settings;
mod statusline;
mod transcript;

use std::path::PathBuf;

use hooks::HookEvent;
use orch_core::{AgentEvent, ConversationId, PermissionMode};
use serde_json::{Map, Value, json};
use transcript::TranscriptTitles;

use crate::shell::quote;
use crate::{AgentAdapter, Argv, Capabilities, GuardAnswer, LaunchSpec, PayloadError, TitleWatch};

const GUARD_HOOK: HookEvent = HookEvent::PreToolUse;
const GUARD_WAIT_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCode {
    pub program: String,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        Self {
            program: "claude".into(),
        }
    }
}

impl ClaudeCode {
    fn settings(&self, spec: &LaunchSpec) -> Value {
        let orch_command = |subcommand: &str| {
            format!(
                "{} {subcommand} --session {}",
                quote(&spec.orch_program),
                quote(spec.session.as_str())
            )
        };
        let hook_command = orch_command("hook");
        let hooks: Map<String, Value> = HookEvent::OBSERVED
            .into_iter()
            .map(|event| {
                let mut hook = json!({ "type": "command", "command": hook_command });
                if event == GUARD_HOOK {
                    hook["timeout"] = json!(GUARD_WAIT_SECS);
                }
                (event.name(), json!([{ "matcher": "*", "hooks": [hook] }]))
            })
            .collect();
        let mut settings = json!({
            "hooks": hooks,
            "statusLine": { "type": "command", "command": orch_command("tap") },
        });
        let preset = &spec.preset;
        if !preset.allow.is_empty() || !preset.deny.is_empty() {
            settings["permissions"] = json!({ "allow": preset.allow, "deny": preset.deny });
        }
        settings
    }

    fn interactive(
        &self,
        conversation: [&str; 2],
        spec: &LaunchSpec,
        mode: Option<PermissionMode>,
    ) -> Argv {
        let settings = self.settings(spec).to_string();
        let args = conversation
            .into_iter()
            .chain(["--settings", &settings])
            .map(String::from)
            .chain(mode_args(mode))
            .collect();
        Argv {
            program: self.program.clone(),
            args,
        }
    }
}

impl AgentAdapter for ClaudeCode {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hooks: true,
            resume: true,
            usage: true,
            modes: true,
            guards: true,
            subagents: true,
            titles: true,
        }
    }

    fn launch(&self, spec: &LaunchSpec) -> Argv {
        let mut argv = self.interactive(
            ["--session-id", spec.session.as_str()],
            spec,
            spec.preset.mode,
        );
        if let Some(prompt) = &spec.prompt {
            argv.args.extend(["--".into(), prompt.clone()]);
        }
        argv
    }

    fn resume(
        &self,
        spec: &LaunchSpec,
        conversation: &ConversationId,
        observed_mode: Option<PermissionMode>,
    ) -> Option<Argv> {
        let mode = spec.preset.mode.and(observed_mode.or(spec.preset.mode));
        Some(self.interactive(["--resume", conversation.as_str()], spec, mode))
    }

    fn map_hook(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        hooks::map_hook(payload)
    }

    fn title_watch(&self) -> Option<Box<dyn TitleWatch>> {
        Some(Box::new(TranscriptTitles::default()))
    }

    fn map_tap(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        statusline::map_tap(payload)
    }

    fn is_guard_payload(&self, payload: &str) -> bool {
        hooks::event_name(payload) == Some(GUARD_HOOK)
    }

    fn guard_answer(&self, answer: &GuardAnswer) -> Option<String> {
        let (decision, reason) = match answer {
            GuardAnswer::Proceed => return None,
            GuardAnswer::Ask => ("ask", None),
            GuardAnswer::Deny { reason } => ("deny", Some(reason)),
        };
        let mut output = json!({
            "hookSpecificOutput": {
                "hookEventName": GUARD_HOOK.name(),
                "permissionDecision": decision,
            }
        });
        if let Some(reason) = reason {
            output["hookSpecificOutput"]["permissionDecisionReason"] = json!(reason);
        }
        Some(output.to_string())
    }

    fn agent_dirs(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        settings::config_dir(lookup)
            .map(|dir| vec![dir.join("projects"), dir.join("plans")])
            .unwrap_or_default()
    }

    fn draft(&self, conversation: &ConversationId) -> Option<Argv> {
        Some(Argv {
            program: self.program.clone(),
            args: vec![
                "-p".into(),
                "--resume".into(),
                conversation.as_str().into(),
                "--fork-session".into(),
            ],
        })
    }
}

fn mode_args(mode: Option<PermissionMode>) -> Vec<String> {
    mode.map(|mode| vec!["--permission-mode".into(), mode_name(mode).into()])
        .unwrap_or_default()
}

pub fn mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Plan => "plan",
        PermissionMode::Auto => "auto",
        PermissionMode::DontAsk => "dontAsk",
        PermissionMode::BypassPermissions => "bypassPermissions",
    }
}

pub fn mode_from_name(name: &str) -> Option<PermissionMode> {
    match name {
        "default" | "manual" => Some(PermissionMode::Default),
        "acceptEdits" => Some(PermissionMode::AcceptEdits),
        "plan" => Some(PermissionMode::Plan),
        "auto" => Some(PermissionMode::Auto),
        "dontAsk" => Some(PermissionMode::DontAsk),
        "bypassPermissions" => Some(PermissionMode::BypassPermissions),
        _ => None,
    }
}
