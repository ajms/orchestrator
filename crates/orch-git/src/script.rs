use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub command: String,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptOutcome {
    pub success: bool,
    pub output: String,
}

impl Script {
    pub fn run(&self, cwd: &Path) -> ScriptOutcome {
        let result = Command::new("sh")
            .arg("-c")
            .arg(format!("exec 2>&1\n{}", self.command))
            .current_dir(cwd)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .output();
        match result {
            Ok(output) => ScriptOutcome {
                success: output.status.success(),
                output: String::from_utf8_lossy(&output.stdout).into_owned(),
            },
            Err(err) => ScriptOutcome {
                success: false,
                output: err.to_string(),
            },
        }
    }
}
