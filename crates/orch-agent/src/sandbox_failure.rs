#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxFailure {
    Missing,
    Failed(String),
}

const WAYS_OUT: &str = "set [defaults] sandbox = false in your global config, \
    or :preset to a Preset with sandbox = false";

const USERNS_OFF: [(&str, &str, &str); 3] = [
    ("kernel.unprivileged_userns_clone", "0", "set it to 1"),
    ("user.max_user_namespaces", "0", "raise it above 0"),
    (
        "kernel.apparmor_restrict_unprivileged_userns",
        "1",
        "add an AppArmor profile for bwrap (Ubuntu 24.04 and later)",
    ),
];

pub fn sandbox_refusal(
    failure: &SandboxFailure,
    sysctl: impl Fn(&str) -> Option<String>,
) -> String {
    let (cause, fix) = match failure {
        SandboxFailure::Missing => ("bwrap is not installed".to_string(), "install bubblewrap"),
        SandboxFailure::Failed(stderr) => {
            let set = |(key, value, _): &&(&str, &str, &str)| {
                sysctl(key).is_some_and(|actual| actual.trim() == *value)
            };
            match USERNS_OFF.iter().find(set) {
                Some((key, value, fix)) => (format!("{key} is {value}; {}", stderr.trim()), *fix),
                None => (stderr.trim().to_string(), "fix the host"),
            }
        }
    };
    format!("The Sandbox can't run: {cause}. To fix it, {fix}, or {WAYS_OUT}.")
}
