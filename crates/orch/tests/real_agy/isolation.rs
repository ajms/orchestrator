use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const PASSED: [&str; 4] = ["PATH", "TERM", "LANG", "DBUS_SESSION_BUS_ADDRESS"];

pub struct Isolation {
    pub home: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
}

impl Isolation {
    pub fn under(root: &Path) -> Self {
        Self {
            home: root.join("home"),
            config: root.join("config"),
            state: root.join("state"),
        }
    }

    fn redirected(&self) -> Vec<(&'static str, OsString)> {
        vec![
            ("HOME", self.home.clone().into()),
            ("XDG_CONFIG_HOME", self.config.clone().into()),
            ("XDG_STATE_HOME", self.state.clone().into()),
            ("XDG_DATA_HOME", self.home.join(".local/share").into()),
            ("XDG_CACHE_HOME", self.home.join(".cache").into()),
        ]
    }

    pub fn vars(&self) -> Vec<(OsString, OsString)> {
        let passed = std::env::vars_os().filter(|(key, _)| {
            let key = key.to_string_lossy();
            PASSED.contains(&key.as_ref()) || key.starts_with("LC_")
        });
        let redirected = self
            .redirected()
            .into_iter()
            .map(|(key, value)| (key.into(), value));
        passed.chain(redirected).collect()
    }

    pub fn apply<'c>(&self, command: &'c mut Command) -> &'c mut Command {
        let set: Vec<(OsString, OsString)> = command
            .get_envs()
            .filter_map(|(key, value)| Some((key.to_owned(), value?.to_owned())))
            .collect();
        command.env_clear();
        let passed = self.vars();
        let (redirected, passed): (Vec<_>, Vec<_>) = passed
            .into_iter()
            .partition(|(key, _)| self.redirected().iter().any(|(name, _)| key == name));
        command.envs(passed).envs(set).envs(redirected)
    }

    pub fn shell_line(&self, program: &Path) -> String {
        let quote =
            |value: &OsString| format!("'{}'", value.to_string_lossy().replace('\'', r"'\''"));
        let vars: Vec<String> = self
            .vars()
            .iter()
            .map(|(key, value)| format!("{}={}", key.to_string_lossy(), quote(value)))
            .collect();
        format!("env -i {} {}", vars.join(" "), program.display())
    }
}

fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub fn copy_dir(from: &Path, to: &Path) {
    copy_tree(from, to, from, to);
}

fn copy_tree(from: &Path, to: &Path, root: &Path, copy: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, root, copy);
        } else if kind.is_symlink() {
            let link = std::fs::read_link(entry.path()).unwrap();
            let points_at = normalized(&from.join(&link));
            let inside = points_at.strip_prefix(root).unwrap_or_else(|_| {
                panic!(
                    "{} links outside the template to {}",
                    entry.path().display(),
                    link.display()
                )
            });
            let link = match link.is_absolute() {
                true => copy.join(inside),
                false => link,
            };
            std::os::unix::fs::symlink(link, target).unwrap();
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copied_template_keeps_its_links_inside_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let template = dir.path().join("template");
        std::fs::create_dir_all(template.join("home/bin")).unwrap();
        std::fs::write(template.join("home/bin/tool"), "tool").unwrap();
        std::os::unix::fs::symlink(
            template.join("home/bin/tool"),
            template.join("home/absolute"),
        )
        .unwrap();
        std::os::unix::fs::symlink("bin/tool", template.join("home/relative")).unwrap();
        let copy = dir.path().join("copy");

        copy_dir(&template, &copy);

        assert_eq!(
            std::fs::read_link(copy.join("home/absolute")).unwrap(),
            copy.join("home/bin/tool")
        );
        assert_eq!(
            std::fs::read_to_string(copy.join("home/relative")).unwrap(),
            "tool"
        );
    }

    #[test]
    #[should_panic(expected = "links outside the template")]
    fn a_template_link_out_of_the_template_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let template = dir.path().join("template");
        std::fs::create_dir_all(&template).unwrap();
        std::os::unix::fs::symlink("../elsewhere", template.join("escape")).unwrap();

        copy_dir(&template, &dir.path().join("copy"));
    }

    #[test]
    fn an_isolated_command_sees_only_the_redirected_dirs_and_the_allowlist() {
        let isolation = Isolation::under(Path::new("/iso"));
        let mut command = Command::new("true");
        command.env("ORCH_RUNTIME_DIR", "/run/orch");

        isolation.apply(&mut command);

        let envs: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into(),
                    value.map(|value| value.to_string_lossy().into()),
                )
            })
            .collect();
        let value = |name: &str| {
            envs.iter()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| value.clone())
        };
        assert_eq!(value("HOME").as_deref(), Some("/iso/home"));
        assert_eq!(value("XDG_CACHE_HOME").as_deref(), Some("/iso/home/.cache"));
        assert_eq!(value("ORCH_RUNTIME_DIR").as_deref(), Some("/run/orch"));
        assert!(
            envs.iter().all(|(key, _)| {
                PASSED.contains(&key.as_str())
                    || key.starts_with("LC_")
                    || isolation.redirected().iter().any(|(name, _)| name == key)
                    || key == "ORCH_RUNTIME_DIR"
            }),
            "{envs:?}"
        );
    }
}
