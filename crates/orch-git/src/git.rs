use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::process::{Command, Output};

use crate::Error;

const ENV_REDIRECTING_GIT: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

pub(crate) fn head_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
}

impl GitVersion {
    pub const MINIMUM: GitVersion = GitVersion {
        major: 2,
        minor: 40,
    };

    pub fn parse(banner: &str) -> Option<GitVersion> {
        let version = banner.trim().strip_prefix("git version ")?;
        let mut parts = version.split('.');
        Some(GitVersion {
            major: parts.next()?.parse().ok()?,
            minor: parts.next()?.parse().ok()?,
        })
    }

    pub fn installed() -> Result<GitVersion, Error> {
        let banner = git(Path::new("."), ["--version"]).run()?;
        GitVersion::parse(&banner).ok_or_else(|| Error::Git {
            command: "git --version".into(),
            stderr: format!("unrecognised version banner: {banner}"),
        })
    }

    pub fn require_minimum(self) -> Result<(), Error> {
        if self < GitVersion::MINIMUM {
            return Err(Error::GitTooOld { found: self });
        }
        Ok(())
    }
}

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

pub(crate) struct Git<'a> {
    command: Command,
    args: Vec<String>,
    dir: &'a Path,
}

pub(crate) fn git<I, S>(dir: &Path, args: I) -> Git<'_>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    for key in ENV_REDIRECTING_GIT {
        command.env_remove(key);
    }
    Git {
        command,
        args: Vec::new(),
        dir,
    }
    .args(args)
}

impl Git<'_> {
    pub(crate) fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_string_lossy().into_owned());
        self.command.arg(arg);
        self
    }

    pub(crate) fn args<I, S>(self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        args.into_iter().fold(self, |git, arg| git.arg(arg))
    }

    pub(crate) fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    pub(crate) fn output(mut self) -> Result<Output, Error> {
        let command = self.describe();
        self.command
            .output()
            .map_err(|error| Error::io(command, error))
    }

    pub(crate) fn succeeds(self) -> bool {
        self.output().is_ok_and(|output| output.status.success())
    }

    pub(crate) fn run(self) -> Result<String, Error> {
        let command = self.describe();
        let output = self.output()?;
        if !output.status.success() {
            return Err(Error::Git {
                command,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    pub(crate) fn describe(&self) -> String {
        format!("git -C {} {}", self.dir.display(), self.args.join(" "))
    }
}
