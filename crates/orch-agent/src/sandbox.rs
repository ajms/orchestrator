use std::path::{Path, PathBuf};

const HOME_READ_ONLY: [&str; 16] = [
    ".gitconfig",
    ".config/git",
    ".cargo",
    ".rustup",
    ".local/share/uv",
    ".nvm",
    ".pyenv",
    ".rbenv",
    ".volta",
    ".bun",
    ".deno",
    ".local/share/mise",
    ".local/share/fnm",
    ".asdf",
    ".sdkman",
    "go",
];
const UNSET: [&str; 4] = [
    "SSH_AUTH_SOCK",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
];
const SANDBOX_TMP: &str = "/tmp";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxNeeds {
    pub writable: Vec<PathBuf>,
    pub read_only: Vec<PathBuf>,
    pub carve_outs: Vec<PathBuf>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxLayout {
    pub home: PathBuf,
    pub tmp: PathBuf,
    pub runtime_dir: Option<PathBuf>,
    pub holder_socket: PathBuf,
    pub worktree: PathBuf,
    pub subagent_worktrees: PathBuf,
    pub main_checkout: PathBuf,
    pub git_common_dir: PathBuf,
    pub read_only: Vec<PathBuf>,
    pub hidden: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Tmpfs,
    Bind,
    BindTry,
    ReadOnly,
    ReadOnlyTry,
}

#[derive(Debug, Clone)]
struct Mount {
    kind: Kind,
    source: PathBuf,
    target: PathBuf,
}

impl Mount {
    fn same(kind: Kind, path: &Path) -> Self {
        Self {
            kind,
            source: path.into(),
            target: path.into(),
        }
    }

    fn args(&self) -> Vec<String> {
        let flag = match self.kind {
            Kind::Tmpfs => "--tmpfs",
            Kind::Bind => "--bind",
            Kind::BindTry => "--bind-try",
            Kind::ReadOnly => "--ro-bind",
            Kind::ReadOnlyTry => "--ro-bind-try",
        };
        let mut args = vec![flag.to_string()];
        if self.kind != Kind::Tmpfs {
            args.push(text(&self.source));
        }
        args.push(text(&self.target));
        args
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    Dir,
    File(&'static str),
}

pub fn sandbox_carve_outs(
    layout: &SandboxLayout,
    needs: &SandboxNeeds,
) -> Vec<(PathBuf, Placeholder)> {
    let placeholder = |path: &Path| match path.extension().and_then(|ext| ext.to_str()) {
        None => Placeholder::Dir,
        Some("json") => Placeholder::File("{}"),
        Some(_) => Placeholder::File(""),
    };
    let agents = needs
        .carve_outs
        .iter()
        .map(|path| (path.clone(), placeholder(path)));
    let git = [
        ("hooks", Placeholder::Dir),
        ("config", Placeholder::File("")),
    ]
    .map(|(name, placeholder)| (layout.git_common_dir.join(name), placeholder));
    agents.chain(git).collect()
}

pub fn sandbox_policy(layout: &SandboxLayout, needs: &SandboxNeeds) -> Vec<String> {
    let mut mounts = vec![Mount {
        kind: Kind::Bind,
        source: layout.tmp.clone(),
        target: SANDBOX_TMP.into(),
    }];
    mounts.push(Mount::same(Kind::Tmpfs, &layout.home));
    mounts.extend(
        layout
            .runtime_dir
            .iter()
            .map(|dir| Mount::same(Kind::Tmpfs, dir)),
    );

    let home_entries = HOME_READ_ONLY.iter().map(|entry| layout.home.join(entry));
    let read_only = home_entries
        .chain(needs.read_only.iter().cloned())
        .chain(layout.read_only.iter().cloned());
    mounts.extend(read_only.map(|path| Mount::same(Kind::ReadOnlyTry, &path)));
    mounts.push(Mount::same(Kind::ReadOnly, &layout.main_checkout));

    mounts.extend(
        needs
            .writable
            .iter()
            .map(|path| Mount::same(Kind::BindTry, path)),
    );
    for path in [
        &layout.worktree,
        &layout.subagent_worktrees,
        &layout.git_common_dir,
        &layout.holder_socket,
    ] {
        mounts.push(Mount::same(Kind::Bind, path));
    }

    let carve_outs = sandbox_carve_outs(layout, needs);
    mounts.extend(
        carve_outs
            .iter()
            .map(|(path, _)| Mount::same(Kind::ReadOnly, path)),
    );

    let mut args: Vec<String> = ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]
        .map(String::from)
        .into();
    args.extend(mounts.iter().flat_map(Mount::args));
    for hidden in &layout.hidden {
        args.extend(Mount::same(Kind::Tmpfs, hidden).args());
        let inside = mounts
            .iter()
            .filter(|mount| mount.target.starts_with(hidden) && mount.target != *hidden);
        args.extend(inside.flat_map(Mount::args));
    }

    for var in UNSET {
        args.extend(["--unsetenv".into(), var.into()]);
    }
    let env = std::iter::once(("TMPDIR", SANDBOX_TMP)).chain(
        needs
            .env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    for (key, value) in env {
        args.extend(["--setenv".into(), key.into(), value.into()]);
    }
    args.extend(["--chdir".into(), text(&layout.worktree)]);
    args
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
