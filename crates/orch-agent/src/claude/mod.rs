mod hooks;
mod profile;
mod settings;
mod statusline;
mod subagents;
mod transcript;

use std::path::{Path, PathBuf};

use hooks::HookEvent;
use orch_core::{AgentEvent, ConversationId, PermissionMode};
use profile::{CONFIG, CREDENTIALS, WRITABLE_DIRS};
use serde_json::{Map, Value, json};
use subagents::ClaudeSubagentTranscripts;
use transcript::TranscriptTitles;

use crate::profile::{Dirs, path_text};
use crate::shell::quote;
use crate::{
    AgentAdapter, Argv, Capabilities, DraftRun, DraftSpec, GuardAnswer, LaunchSpec, PayloadError,
    ProfileRender, Projection, SandboxNeeds, SubagentTranscripts, TitleWatch, WorktreeRequest,
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

/// Appended to Claude's system prompt so it keeps the Session's Plan (ADR 0021).
pub const PLAN_GUIDANCE: &str = "# Under orch: the Plan

You run inside an orch Session. Keep this Session's Plan in orch with `orch plan` (the user sees it in orch, read-only):
- `orch plan add \"<title>\" [--after <id>] [--blocked-by <id>]... [--link <issue URL or #123>]` prints the new Task's id.
- `orch plan status <id> pending|in-progress|done|dropped`, `orch plan block <id> --by <id>...` (`unblock` undoes it), `orch plan link <id> <ref>`, `orch plan attach <id> <subagent-id>`.
- `orch plan show [--json]` lists the Tasks and which are ready (every blocker Done or Dropped).
- When a Subagent serves a Task, put `[task:<id>]` in its description; orch attaches it to that Task.
- For multi-step work, write the Plan first and keep statuses current as you go.
";

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
            .chain(["--append-system-prompt", PLAN_GUIDANCE])
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
        match profile::dirs(lookup) {
            Some(Dirs { profile, .. }) => vec![profile.join("projects"), profile.join("plans")],
            None => Vec::new(),
        }
    }

    fn agent_profile(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<PathBuf>, String> {
        match profile::dirs(lookup) {
            Some(Dirs { profile, .. }) => Ok(Some(profile)),
            None => Err(NO_HOME.into()),
        }
    }

    fn prepare(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
        let Some(sources) = profile::sources(lookup) else {
            return Err(NO_HOME.into());
        };
        profile::build(&sources).map_err(|err| {
            format!(
                "building claude's Agent profile {}: {err}",
                sources.profile.display()
            )
        })
    }

    fn render_profile(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
        render: &ProfileRender<'_>,
    ) -> Result<(), String> {
        let Some(sources) = profile::sources(lookup) else {
            return Err(NO_HOME.into());
        };
        let profile = &sources.profile;
        let adopted = render.conversation.map_or(Ok(()), |conversation| {
            profile::adopt_conversation(&sources.user, profile, render.worktree, conversation)
        });
        adopted
            .and_then(|()| profile::render_config(&sources, render.worktree))
            .map_err(|err| {
                format!(
                    "rendering claude's Agent profile {}: {err}",
                    profile.display()
                )
            })
    }

    fn env(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        let mut env = vec![("DISABLE_AUTOUPDATER".into(), "1".into())];
        if let Some(Dirs { profile, .. }) = profile::dirs(lookup) {
            env.push(("CLAUDE_CONFIG_DIR".into(), path_text(&profile)));
        }
        env
    }

    fn sandbox_needs(&self, lookup: &dyn Fn(&str) -> Option<String>) -> SandboxNeeds {
        let Some(Dirs {
            home,
            user,
            profile,
        }) = profile::dirs(lookup)
        else {
            return SandboxNeeds::default();
        };
        let from_user = |name: &str, writable| Projection {
            source: user.join(name),
            target: profile.join(name),
            writable,
        };
        let config = Projection {
            source: profile.join(CONFIG),
            target: profile.join(CONFIG),
            writable: false,
        };
        let hidden = (user.is_dir() && !user.starts_with(&home)).then(|| user.clone());
        SandboxNeeds {
            scratch: vec![profile.clone()],
            writable: WRITABLE_DIRS.map(|name| profile.join(name)).into(),
            read_only: INSTALLED.map(|path| home.join(path)).into(),
            projected: CONFIG_CARVE_OUTS
                .map(|name| from_user(name, false))
                .into_iter()
                .chain(CREDENTIALS.map(|name| from_user(name, true)))
                .chain([config])
                .collect(),
            hidden: hidden.into_iter().collect(),
            env: self.env(lookup),
            env_prefixes: vec!["ANTHROPIC_".into(), "CLAUDE_".into()],
            ..SandboxNeeds::default()
        }
    }

    fn draft(
        &self,
        spec: &DraftSpec,
        lookup: &dyn Fn(&str) -> Option<String>,
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
            env: self.env(lookup),
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
