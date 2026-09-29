use std::path::PathBuf;

use orch_agent::Presets;

const DEFAULT_REVIEW_COMMAND: &str = r#"git -p diff "$ORCH_MERGE_BASE" "$ORCH_REVIEW_TREE""#;

#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub repos: Vec<PathBuf>,
    pub cwd_repo: Option<PathBuf>,
    pub presets: Presets,
    pub branch_prefix: String,
    pub review_command: String,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            repos: Vec::new(),
            cwd_repo: None,
            presets: Presets::default(),
            branch_prefix: orch_git::DEFAULT_BRANCH_PREFIX.into(),
            review_command: DEFAULT_REVIEW_COMMAND.into(),
        }
    }
}
