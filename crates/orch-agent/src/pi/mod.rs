mod draft;
mod hooks;
mod profile;
mod subagents;
mod usage;

use std::path::PathBuf;

pub use draft::session_dir_name as pi_session_dir_name;
use orch_core::{AgentEvent, ConversationId, PermissionMode};
use profile::{CREDENTIALS, RESOURCES, extension};
use subagents::PiSubagentTranscripts;

use crate::profile::{Dirs, path_text};
use crate::{
    AgentAdapter, Argv, Capabilities, DraftRun, DraftSpec, LaunchSpec, PayloadError, ProfileRender,
    SandboxNeeds, SubagentTranscripts,
};

const USER_READ_ONLY: [&str; 13] = [
    "install",
    "bin",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "agents",
    "SYSTEM.md",
    "AGENTS.md",
    "keybindings.json",
    "mcp.json",
    "settings.json",
    "sandbox.json",
];

const NO_HOME: &str = "pi's Agent profile needs HOME to be set";

pub const PI: &str = "pi";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pi {
    pub program: String,
}

impl Default for Pi {
    fn default() -> Self {
        Self {
            program: "pi".into(),
        }
    }
}

impl Pi {
    fn argv_for(&self, conversation: &str, spec: &LaunchSpec) -> Argv {
        let mut args: Vec<String> = ["-na", "--session-id", conversation]
            .map(String::from)
            .into();
        if let Some(profile) = &spec.agent_profile {
            args.extend([
                "-e".into(),
                path_text(&extension(profile)),
                "--orch-bin".into(),
                spec.orch_program.clone(),
                "--orch-session".into(),
                spec.session.as_str().into(),
            ]);
        }
        Argv {
            program: self.program.clone(),
            args,
        }
    }
}

impl AgentAdapter for Pi {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            hooks: true,
            resume: true,
            subagents: true,
            titles: true,
            transcripts: true,
            usage: true,
            ..Capabilities::default()
        }
    }

    fn launch(&self, spec: &LaunchSpec) -> Argv {
        let mut argv = self.argv_for(spec.session.as_str(), spec);
        if let Some(prompt) = &spec.prompt {
            argv.args.extend(["--".into(), prompt.clone()]);
        }
        argv
    }

    fn launch_conversation(&self, spec: &LaunchSpec) -> Option<ConversationId> {
        Some(ConversationId(spec.session.as_str().into()))
    }

    fn resume(
        &self,
        spec: &LaunchSpec,
        conversation: &ConversationId,
        _mode: Option<PermissionMode>,
    ) -> Option<Argv> {
        Some(self.argv_for(conversation.as_str(), spec))
    }

    fn needs_draft_scratch(&self) -> bool {
        true
    }

    fn draft(
        &self,
        spec: &DraftSpec,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<DraftRun>, String> {
        let Some(Dirs { profile, .. }) = profile::dirs(lookup) else {
            return Ok(None);
        };
        let fork = draft::copy_conversation(&profile, spec)
            .map_err(|err| format!("copying pi's Conversation for the Draft: {err}"))?;
        Ok(fork.map(|fork| DraftRun {
            argv: draft::argv(&self.program, &fork),
            env: self.env(lookup),
            pipe_base_diff: true,
        }))
    }

    fn map_hook(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        hooks::map_hook(payload)
    }

    fn map_tap(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        usage::map_tap(payload)
    }

    fn subagent_transcripts(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Box<dyn SubagentTranscripts>> {
        let Dirs { profile, .. } = profile::dirs(lookup)?;
        Some(Box::new(PiSubagentTranscripts::new(profile)))
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
        let Some(Dirs { user, profile, .. }) = profile::dirs(lookup) else {
            return Err(NO_HOME.into());
        };
        profile::build(&user, &profile)
            .map_err(|err| format!("building pi's Agent profile {}: {err}", profile.display()))
    }

    fn render_profile(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
        render: &ProfileRender<'_>,
    ) -> Result<(), String> {
        let Some(Dirs { user, profile, .. }) = profile::dirs(lookup) else {
            return Err(NO_HOME.into());
        };
        profile::render_mcp(&user, &profile, render.env)
            .map_err(|err| format!("rendering pi's mcp.json in {}: {err}", profile.display()))
    }

    fn env(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        let Some(Dirs { profile, .. }) = profile::dirs(lookup) else {
            return Vec::new();
        };
        vec![
            ("PI_CODING_AGENT_DIR".into(), path_text(&profile)),
            ("PI_OFFLINE".into(), "1".into()),
            ("PI_INDEX_ROOT".into(), path_text(&profile.join("index"))),
        ]
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
        let installed = [".local/share/pi-node", ".local/bin"].map(|path| home.join(path));
        let linked_skills: Vec<PathBuf> = profile::skills(&user)
            .links
            .into_iter()
            .map(|(_, target)| target)
            .filter(|target| !target.starts_with(&user))
            .collect();
        SandboxNeeds {
            writable: std::iter::once(profile.clone())
                .chain(CREDENTIALS.map(|name| user.join(name)))
                .collect(),
            read_only: USER_READ_ONLY
                .map(|name| user.join(name))
                .into_iter()
                .chain(installed)
                .chain(linked_skills)
                .collect(),
            carve_outs: RESOURCES.map(|name| profile.join(name)).into(),
            env: self.env(lookup),
            env_prefixes: Vec::new(),
            ..SandboxNeeds::default()
        }
    }
}
