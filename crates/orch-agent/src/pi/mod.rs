mod hooks;
mod profile;

use std::path::PathBuf;

use orch_core::{AgentEvent, ConversationId, PermissionMode};
use profile::{CREDENTIALS, Dirs, RESOURCES, extension, path_text};

use crate::{AgentAdapter, Argv, Capabilities, LaunchSpec, PayloadError, SandboxNeeds};

const USER_READ_ONLY: [&str; 12] = [
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
            titles: true,
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

    fn map_hook(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        hooks::map_hook(payload)
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
        profile::build(&user, &profile)
            .map_err(|err| format!("building pi's Agent profile {}: {err}", profile.display()))
    }

    fn env(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        let Some(Dirs { profile, .. }) = Dirs::resolve(lookup) else {
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
        }) = Dirs::resolve(lookup)
        else {
            return SandboxNeeds::default();
        };
        let installed = [".local/share/pi-node", ".local/bin"].map(|path| home.join(path));
        SandboxNeeds {
            writable: std::iter::once(profile.clone())
                .chain(CREDENTIALS.map(|name| user.join(name)))
                .collect(),
            read_only: USER_READ_ONLY
                .map(|name| user.join(name))
                .into_iter()
                .chain(installed)
                .collect(),
            carve_outs: RESOURCES.map(|name| profile.join(name)).into(),
            env: self.env(lookup),
            env_prefixes: Vec::new(),
        }
    }
}
