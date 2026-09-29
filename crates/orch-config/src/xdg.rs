use std::path::PathBuf;

pub fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

pub fn home(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    lookup("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

pub fn config_home(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    base_dir(lookup, "XDG_CONFIG_HOME", ".config")
}

pub fn runtime_dir(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    lookup("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

pub fn state_home(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    base_dir(lookup, "XDG_STATE_HOME", ".local/state")
}

fn base_dir(
    lookup: impl Fn(&str) -> Option<String>,
    var: &str,
    home_fallback: &str,
) -> Option<PathBuf> {
    lookup(var)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home(lookup).map(|home| home.join(home_fallback)))
}
