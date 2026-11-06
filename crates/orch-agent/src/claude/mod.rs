mod hooks;
mod profile;
mod settings;
mod statusline;
mod subagents;
mod transcript;

use std::path::{Path, PathBuf};

use hooks::HookEvent;
use orch_core::{AgentEvent, ConversationId, PermissionMode};
use profile::{CREDENTIALS, Dirs, path_text};
use serde_json::{Map, Value, json};
use subagents::ClaudeSubagentTranscripts;
use transcript::TranscriptTitles;

use crate::shell::quote;
use crate::{
    AgentAdapter, Argv, Capabilities, DraftRun, DraftSpec, GuardAnswer, LaunchSpec, PayloadError,
    SandboxNeeds, SubagentTranscripts, TitleWatch, WorktreeRequest,
};

const GUARD_HOOK: HookEvent = HookEvent::PreToolUse;
const GUARD_WAIT_SECS: u64 = 7 * 24 * 60 * 60;
const CONFIG_CARVE_OUTS: [&str; 9] = [
    "settings.json",
    "settings.local.json",
    "hooks",
    "skills",
    "agents",
    "commands",
    "rules",
    "plugins",
    "CLAUDE.md",
];
const INSTALLED: [&str; 2] = [".local/share/claude", ".local/bin"];
const NO_HOME: &str = "claude's Agent profile needs HOME to be set";

pub const CLAUDE: &str = "claude";

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
        let mut hooks: Map<String, Value> = HookEvent::OBSERVED
            .into_iter()
            .map(|event| {
                let mut hook = json!({ "type": "command", "command": hook_command });
                if event == GUARD_HOOK {
                    hook["timeout"] = json!(GUARD_WAIT_SECS);
                }
                (event.name(), json!([{ "matcher": "*", "hooks": [hook] }]))
            })
            .collect();
        if let Some(worktree) = &spec.worktree {
            let command = format!(
                "{} worktree-hook --worktree {}",
                quote(&spec.orch_program),
                quote(&worktree.to_string_lossy())
            );
            let hook = json!([{ "hooks": [{ "type": "command", "command": command }] }]);
            for event in [HookEvent::WorktreeCreate, HookEvent::WorktreeRemove] {
                hooks.insert(event.name(), hook.clone());
            }
        }
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
            transcripts: true,
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

    fn subagent_transcripts(
        &self,
        _lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Box<dyn SubagentTranscripts>> {
        Some(Box::new(ClaudeSubagentTranscripts::default()))
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

    fn worktree_request(&self, payload: &str) -> Option<WorktreeRequest> {
        hooks::worktree_request(payload)
    }

    fn user_statusline_command(
        &self,
        cwd: &Path,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        settings::user_statusline_command(cwd, lookup)
    }

    fn agent_dirs(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        match Dirs::resolve(lookup) {
            Some(Dirs { profile, .. }) => vec![profile.join("projects"), profile.join("plans")],
            None => Vec::new(),
        }
    }

    fn agent_profile(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<PathBuf>, String> {
        match Dirs::resolve(lookup) {
            Some(Dirs { profile, .. }) => Ok(Some(profile)),
            None => Err(NO_HOME.into()),
        }
    }

    fn prepare(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
        let Some(Dirs { user, profile, .. }) = Dirs::resolve(lookup) else {
            return Err(NO_HOME.into());
        };
        profile::build(&user, &profile).map_err(|err| {
            format!(
                "building claude's Agent profile {}: {err}",
                profile.display()
            )
        })
    }

    fn render_profile(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
        env: &[(String, String)],
    ) -> Result<(), String> {
        let Some(Dirs { profile, .. }) = Dirs::resolve(lookup) else {
            return Err(NO_HOME.into());
        };
        let cwd = env
            .iter()
            .find(|(key, _)| key == "ORCH_WORKTREE")
            .map(|(_, value)| PathBuf::from(value));
        profile::render_config(&profile, cwd.as_deref()).map_err(|err| {
            format!(
                "rendering claude's .claude.json in {}: {err}",
                profile.display()
            )
        })
    }

    fn env(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        let mut env = vec![("DISABLE_AUTOUPDATER".into(), "1".into())];
        if let Some(Dirs { profile, .. }) = Dirs::resolve(lookup) {
            env.push(("CLAUDE_CONFIG_DIR".into(), path_text(&profile)));
        }
        env
    }

    fn sandbox_needs(&self, lookup: &dyn Fn(&str) -> Option<String>) -> SandboxNeeds {
        let Some(Dirs {
            home,
            user,
            profile,
        }) = Dirs::resolve(lookup)
        else {
            return SandboxNeeds::default();
        };
        SandboxNeeds {
            writable: std::iter::once(profile.clone())
                .chain(CREDENTIALS.map(|name| user.join(name)))
                .collect(),
            read_only: INSTALLED.map(|path| home.join(path)).into(),
            carve_outs: CONFIG_CARVE_OUTS.map(|name| profile.join(name)).into(),
            env: self.env(lookup),
            env_prefixes: vec!["ANTHROPIC_".into(), "CLAUDE_".into()],
        }
    }

    fn draft(
        &self,
        spec: &DraftSpec,
        _lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<DraftRun>, String> {
        Ok(Some(DraftRun {
            argv: Argv {
                program: self.program.clone(),
                args: vec![
                    "-p".into(),
                    "--resume".into(),
                    spec.conversation.as_str().into(),
                    "--fork-session".into(),
                ],
            },
            env: Vec::new(),
            pipe_base_diff: false,
        }))
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
