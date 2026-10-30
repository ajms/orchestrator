use std::path::PathBuf;

pub(super) const RESOURCES: [&str; 4] = ["extensions", "skills", "prompts", "themes"];

pub(super) fn home(lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    lookup("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

pub(super) fn user_dir(lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    home(lookup).map(|home| home.join(".pi/agent"))
}

pub(super) fn profile_dir(lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let state = lookup("XDG_STATE_HOME")
        .filter(|state| !state.is_empty())
        .map(PathBuf::from)
        .or_else(|| home(lookup).map(|home| home.join(".local/state")))?;
    Some(state.join("orchestrator/agent-profiles/pi"))
}
