use std::process::Command;

use orch_git::Script;

#[test]
fn a_script_ignores_a_git_dir_inherited_from_the_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (worktree, decoy) = (base.join("worktree"), base.join("decoy"));
    for dir in [&worktree, &decoy] {
        let status = Command::new("git")
            .args(["init", "-q"])
            .arg(dir)
            .env_remove("GIT_DIR")
            .status()
            .unwrap();
        assert!(status.success());
    }
    // SAFETY: this binary holds a single test, so nothing else reads the environment concurrently.
    unsafe { std::env::set_var("GIT_DIR", decoy.join(".git")) };

    let outcome = Script {
        command: "git rev-parse --absolute-git-dir".into(),
        env: vec![],
    }
    .run(&worktree);

    assert!(outcome.success, "{}", outcome.output);
    assert_eq!(
        outcome.output,
        format!("{}\n", worktree.join(".git").display())
    );
}
