use orch_git::Script;

use crate::common::{Fixture, git, write};

fn script(command: &str) -> Script {
    Script {
        command: command.into(),
        env: vec![("ORCH_SESSION".into(), "s-1".into())],
    }
}

#[test]
fn a_script_runs_in_the_given_directory_with_its_env_and_captures_output() {
    let fixture = Fixture::new();

    let outcome = script("pwd; echo \"$ORCH_SESSION\"; echo oops >&2").run(&fixture.root);

    assert!(outcome.success);
    assert_eq!(
        outcome.output,
        format!("{}\ns-1\noops\n", fixture.root.display())
    );
}

#[test]
fn a_failing_script_reports_failure() {
    let fixture = Fixture::new();
    assert!(!script("exit 3").run(&fixture.root).success);
}

#[test]
fn removal_runs_teardown_then_removes_worktree_and_branch() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    write(&worktree.path, "dirty.txt", "uncommitted\n");
    let marker = fixture.outside().join("teardown-ran");
    let teardown = script(&format!(
        "test -f dirty.txt && echo \"$ORCH_SESSION\" > {}",
        marker.display()
    ));
    let repo = fixture.repo();

    let outcome = repo
        .remove_session_worktree(&worktree, Some(&teardown))
        .unwrap();

    assert!(outcome.unwrap().success);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "s-1\n");
    assert!(!worktree.path.exists());
    assert!(!repo.branch_exists("orch/work"));
    assert!(!git(&fixture.root, &["worktree", "list"]).contains("work"));
}

#[test]
fn a_locked_worktree_is_removed_too() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    git(
        &fixture.root,
        &["worktree", "lock", worktree.path.to_str().unwrap()],
    );

    fixture
        .repo()
        .remove_session_worktree(&worktree, None)
        .unwrap();

    assert!(!worktree.path.exists());
}

#[test]
fn a_failing_teardown_is_reported_but_removal_still_happens() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();

    let outcome = repo
        .remove_session_worktree(&worktree, Some(&script("echo broken; exit 1")))
        .unwrap()
        .unwrap();

    assert!(!outcome.success);
    assert_eq!(outcome.output, "broken\n");
    assert!(!worktree.path.exists());
    assert!(!repo.branch_exists("orch/work"));
}

#[test]
fn removal_of_a_vanished_worktree_skips_teardown_and_forgets_it() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    std::fs::remove_dir_all(&worktree.path).unwrap();
    let repo = fixture.repo();

    let outcome = repo
        .remove_session_worktree(&worktree, Some(&script("true")))
        .unwrap();

    assert_eq!(outcome, None);
    assert!(!repo.branch_exists("orch/work"));
    assert!(!git(&fixture.root, &["worktree", "list"]).contains("work"));
}

#[test]
fn removal_of_an_unregistered_worktree_directory_deletes_it_and_the_branch() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    std::fs::remove_file(worktree.path.join(".git")).unwrap();
    git(&fixture.root, &["worktree", "prune"]);
    let repo = fixture.repo();

    repo.remove_session_worktree(&worktree, None).unwrap();

    assert!(!worktree.path.exists());
    assert!(!repo.branch_exists("orch/work"));
}

#[test]
fn removal_tolerates_an_already_deleted_branch() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    git(&worktree.path, &["switch", "-q", "--detach"]);
    git(&fixture.root, &["branch", "-D", "orch/work"]);

    fixture
        .repo()
        .remove_session_worktree(&worktree, None)
        .unwrap();

    assert!(!worktree.path.exists());
}
