mod profile;

use std::path::PathBuf;

use orch_core::{ConversationId, PermissionMode};
use profile::{RESOURCES, home, profile_dir, user_dir};

use crate::{AgentAdapter, Argv, Capabilities, LaunchSpec, SandboxNeeds};

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
const CREDENTIALS: [&str; 2] = ["auth.json", "models-store.json"];

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
    fn open(&self, conversation: &str) -> Argv {
        Argv {
            program: self.program.clone(),
            args: ["-na", "--session-id", conversation]
                .map(String::from)
                .into(),
        }
    }
}

impl AgentAdapter for Pi {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            resume: true,
            ..Capabilities::default()
        }
    }

    fn launch(&self, spec: &LaunchSpec) -> Argv {
        let mut argv = self.open(spec.session.as_str());
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
        _spec: &LaunchSpec,
        conversation: &ConversationId,
        _mode: Option<PermissionMode>,
    ) -> Option<Argv> {
        Some(self.open(conversation.as_str()))
    }

    fn env(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        let Some(profile) = profile_dir(lookup) else {
            return Vec::new();
        };
        let text = |path: PathBuf| path.to_string_lossy().into_owned();
        vec![
            ("PI_CODING_AGENT_DIR".into(), text(profile.clone())),
            ("PI_OFFLINE".into(), "1".into()),
            ("PI_INDEX_ROOT".into(), text(profile.join("index"))),
        ]
    }

    fn sandbox_needs(&self, lookup: &dyn Fn(&str) -> Option<String>) -> SandboxNeeds {
        let (Some(home), Some(user), Some(profile)) =
            (home(lookup), user_dir(lookup), profile_dir(lookup))
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
