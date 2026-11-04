use std::path::Path;

use crate::Error;
use crate::git::git;
use crate::landing::pinned;

const REFSPECS: [&str; 2] = ["+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"];

pub fn update_mirror(mirror: &Path, url: &str) -> Result<(), Error> {
    if mirror.exists() {
        let fetch = ["fetch", "-q", "--prune", "--no-write-fetch-head", "--", url];
        pinned(mirror, &fetch).args(REFSPECS).run()?;
        if !git(mirror, ["rev-parse", "-q", "--verify", "HEAD"]).succeeds() {
            follow_head(mirror, url)?;
        }
        return Ok(());
    }
    let parent = mirror.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(parent).map_err(|err| Error::io("creating the mirrors dir", err))?;
    let mut fresh = mirror.as_os_str().to_owned();
    fresh.push(".new");
    let fresh = Path::new(&fresh);
    let _ = std::fs::remove_dir_all(fresh);
    let cloned = pinned(parent, &["clone", "-q", "--bare", "--", url])
        .arg(fresh)
        .run()
        .and_then(|_| {
            std::fs::rename(fresh, mirror).map_err(|err| Error::io("placing the mirror", err))
        });
    if cloned.is_err() {
        let _ = std::fs::remove_dir_all(fresh);
    }
    cloned
}

fn follow_head(mirror: &Path, url: &str) -> Result<(), Error> {
    let listed = pinned(mirror, &["ls-remote", "--symref", "--", url, "HEAD"]).run()?;
    let head = listed.lines().find_map(|line| {
        let (target, name) = line.strip_prefix("ref: ")?.split_once('\t')?;
        (name == "HEAD" && target.starts_with("refs/heads/")).then_some(target)
    });
    match head {
        Some(head) => git(mirror, ["symbolic-ref", "HEAD", head]).run().map(drop),
        None => Ok(()),
    }
}
