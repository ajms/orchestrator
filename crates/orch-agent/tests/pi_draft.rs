use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, Argv, DraftRun, DraftSpec, Pi};
use orch_core::ConversationId;
use tempfile::TempDir;

const CONVERSATION: &str = "5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90";
const WORKTREE: &str = "/repos/my:app/.orchestrator/worktrees/fix";
const LIVE: &str = "{\"type\":\"session\",\"id\":\"live\"}\n{\"type\":\"message\"}\n";

struct Setup {
    state: TempDir,
    scratch: TempDir,
}

impl Setup {
    fn new() -> Self {
        Self {
            state: tempfile::tempdir().unwrap(),
            scratch: tempfile::tempdir().unwrap(),
        }
    }

    fn profile(&self) -> PathBuf {
        self.state.path().join("orchestrator/agent-profiles/pi")
    }

    fn sessions(&self) -> PathBuf {
        self.profile()
            .join("sessions/--repos-my-app-.orchestrator-worktrees-fix--")
    }

    fn seed(&self, name: &str, content: &str) -> PathBuf {
        let path = self.sessions().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn live(&self) -> PathBuf {
        self.seed(
            &format!("2026-10-30T09-00-00-000Z_{CONVERSATION}.jsonl"),
            LIVE,
        )
    }

    fn draft(&self) -> Option<DraftRun> {
        let state = self.state.path().to_string_lossy().into_owned();
        let spec = DraftSpec {
            conversation: ConversationId(CONVERSATION.into()),
            worktree: WORKTREE.into(),
            scratch: self.scratch.path().into(),
        };
        Pi::default()
            .draft(&spec, &|key| match key {
                "HOME" => Some("/home/dev".into()),
                "XDG_STATE_HOME" => Some(state.clone()),
                _ => None,
            })
            .unwrap()
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn a_draft_forks_a_copy_of_the_conversation_with_read_only_tools_and_the_diff_piped() {
    let setup = Setup::new();
    let live = setup.live();
    let scratch = setup.scratch.path();

    let draft = setup.draft().expect("pi drafts");

    let fork = scratch.join("conversation.jsonl");
    assert_eq!(
        draft.argv,
        Argv {
            program: "pi".into(),
            args: [
                "-p",
                "-ne",
                "-ns",
                "-np",
                "--tools",
                "read,grep,find,ls",
                "--fork",
                &text(&fork),
                "--session-dir",
                &text(&scratch.join("sessions")),
            ]
            .map(String::from)
            .into(),
        }
    );
    assert!(draft.diff);
    assert!(
        draft
            .env
            .contains(&("PI_CODING_AGENT_DIR".into(), text(&setup.profile())))
    );
    assert_eq!(std::fs::read_to_string(&fork).unwrap(), LIVE);
    assert_eq!(std::fs::read_to_string(&live).unwrap(), LIVE);
}

#[test]
fn a_draft_forks_the_conversations_newest_file_and_no_other_conversation() {
    let setup = Setup::new();
    setup.seed(
        &format!("2026-10-29T09-00-00-000Z_{CONVERSATION}.jsonl"),
        "old\n",
    );
    setup.live();
    setup.seed("2026-10-31T09-00-00-000Z_other.jsonl", "other\n");

    setup.draft().expect("pi drafts");

    let fork = setup.scratch.path().join("conversation.jsonl");
    assert_eq!(std::fs::read_to_string(fork).unwrap(), LIVE);
}

#[test]
fn without_the_conversations_file_there_is_no_pi_draft() {
    let setup = Setup::new();
    setup.seed("2026-10-31T09-00-00-000Z_other.jsonl", "other\n");

    assert_eq!(setup.draft(), None);
}

#[test]
fn a_linked_conversation_file_is_never_forked() {
    let setup = Setup::new();
    let elsewhere = setup.state.path().join("secret.jsonl");
    std::fs::write(&elsewhere, "secret\n").unwrap();
    std::fs::create_dir_all(setup.sessions()).unwrap();
    let name = format!("2026-10-30T09-00-00-000Z_{CONVERSATION}.jsonl");

    std::os::unix::fs::symlink(&elsewhere, setup.sessions().join(&name)).unwrap();
    assert_eq!(setup.draft(), None);

    std::fs::remove_file(setup.sessions().join(&name)).unwrap();
    std::fs::hard_link(&elsewhere, setup.sessions().join(&name)).unwrap();
    assert_eq!(setup.draft(), None);
    assert!(!setup.scratch.path().join("conversation.jsonl").exists());
}

#[test]
fn a_sessions_dir_linked_out_of_the_profile_is_never_read() {
    let setup = Setup::new();
    let outside = setup.state.path().join("outside");
    let name = format!("2026-10-30T09-00-00-000Z_{CONVERSATION}.jsonl");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join(name), "secret\n").unwrap();
    std::fs::create_dir_all(setup.sessions().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside, setup.sessions()).unwrap();

    assert_eq!(setup.draft(), None);
}
