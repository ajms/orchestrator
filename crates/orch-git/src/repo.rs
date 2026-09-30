use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::git::{Git, git, head_ref};
use crate::{Error, GitVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

fn require_supported_git() -> Result<(), Error> {
    static CHECK: OnceLock<Result<(), Error>> = OnceLock::new();
    CHECK
        .get_or_init(|| GitVersion::installed()?.require_minimum())
        .clone()
}

impl Repo {
    pub fn open(path: &Path) -> Result<Repo, Error> {
        require_supported_git()?;
        let common = git(
            path,
            ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .run()?;
        let common = PathBuf::from(common);
        let root = match common.file_name() {
            Some(name) if name == ".git" => common.parent().unwrap_or(&common).to_path_buf(),
            _ => git(path, ["rev-parse", "--show-toplevel"]).run()?.into(),
        };
        let root = root
            .canonicalize()
            .map_err(|error| Error::io(format!("resolve {}", root.display()), error))?;
        Ok(Repo { root })
    }

    pub fn exists(path: &Path) -> bool {
        path.is_dir() && git(path, ["rev-parse", "--git-dir"]).succeeds()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn default_base(&self, configured: Option<&str>) -> Result<String, Error> {
        if let Some(base) = configured {
            return Ok(base.to_string());
        }
        if let Some(branch) = self.origin_default_branch() {
            return Ok(branch);
        }
        self.git(["symbolic-ref", "--short", "HEAD"]).run()
    }

    pub fn head_branch(&self) -> Result<Option<String>, Error> {
        let command = self.git(["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let description = command.describe();
        let output = command.output()?;
        match output.status.code() {
            Some(0) => Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            )),
            Some(1) => Ok(None),
            _ => Err(Error::Git {
                command: description,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
        }
    }

    pub fn branch_exists(&self, name: &str) -> bool {
        self.git(["show-ref", "--verify", "--quiet", &head_ref(name)])
            .succeeds()
    }

    fn origin_default_branch(&self) -> Option<String> {
        let _ = self
            .git(["remote", "set-head", "origin", "--auto"])
            .succeeds();
        let remote_head = self
            .git(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .run()
            .ok()?;
        let branch = remote_head.strip_prefix("origin/")?.to_string();
        let local_ready = self.branch_exists(&branch)
            || self
                .git(["branch", "--no-track", &branch])
                .arg(format!("refs/remotes/origin/{branch}"))
                .succeeds();
        local_ready.then_some(branch)
    }

    pub(crate) fn git<I, S>(&self, args: I) -> Git<'_>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        git(&self.root, args)
    }
}
