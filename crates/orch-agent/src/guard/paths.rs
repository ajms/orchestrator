use std::path::{Component, Path, PathBuf};

const HARMLESS_TARGETS: [&str; 4] = ["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty"];

pub(crate) fn resolve(cwd: &Path, raw: &str) -> PathBuf {
    let expanded = match raw.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => std::env::home_dir()
            .map(|home| home.join(rest.trim_start_matches('/')))
            .unwrap_or_else(|| PathBuf::from(raw)),
        _ => PathBuf::from(raw),
    };
    resolve_symlinks(&normalize(&cwd.join(expanded)))
}

pub(crate) fn is_harmless(path: &Path) -> bool {
    HARMLESS_TARGETS
        .iter()
        .any(|target| path == Path::new(target))
}

pub(crate) fn is_temp(path: &Path) -> bool {
    [PathBuf::from("/tmp"), std::env::temp_dir()]
        .iter()
        .map(|root| resolve(Path::new("/"), &root.to_string_lossy()))
        .any(|root| path.starts_with(root))
}

fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normal = resolve_symlinks(&normal);
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    normal
}

fn resolve_symlinks(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|ancestor| {
            let real = ancestor.canonicalize().ok()?;
            let rest = path.strip_prefix(ancestor).ok()?;
            Some(if rest.as_os_str().is_empty() {
                real
            } else {
                real.join(rest)
            })
        })
        .unwrap_or_else(|| path.to_path_buf())
}
