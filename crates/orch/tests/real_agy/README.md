# Real-agy suite

Opt-in tests that drive a real, authenticated `agy` through orch's Daemon and Holder. Every test is `#[ignore]`, and without `ORCH_REAL_AGY=1` even `--ignored` runs them as no-ops, so CI never starts agy. The two `fixtures::tests` run everywhere: they check the scrubbing and fixture selection used by re-recording.

## Run

```sh
ORCH_REAL_AGY=1 cargo test -p orch --test real_agy -- --ignored --test-threads=1
```

`ORCH_REAL_AGY=1 cargo test -- --ignored` also runs it, along with the workspace's other ignored tests (the freedesktop notification tests need a session bus).

Prerequisites:

- `agy` on `PATH`, signed in. The spec targets agy 1.2.17, and the suite was written while 1.3.0 was installed. Auth lives in the keyring, not in `~/.gemini`, so the session bus (`DBUS_SESSION_BUS_ADDRESS`) has to be reachable.
- Network access and some quota: seven tests run model turns.
- The suite never touches your `~/.gemini`. Each test runs with `HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME` inside its own temp dir.

## Template HOME

The first run builds a template in `target/tmp/real-agy-template/` and every test copies it:

1. It starts agy in orch's Holder with the template as `HOME` and walks the onboarding: Enter on the theme picker, then the option that turns data sharing **off**, and stops at the trust screen. It refuses a template whose `settings.json` has a sharing or telemetry key set to `true`.
2. It sets a capture command as the user's own statusline (it prints nothing and only writes files while re-recording).
3. It runs `orch agent install antigravity --yes`, so the Agent hookup is orch's own.

The template is rebuilt when `agy --version` or the `orch` binary path changes. Delete the directory to rebuild it by hand. If the onboarding screens defeat the driver, the failure prints the screen and the exact command to onboard the template yourself; `touch target/tmp/real-agy-template/onboarded` afterwards and run again.

## Tests

| Test | Checks |
|---|---|
| `each_fresh_worktree_needs_input_on_agys_trust_screen` | Two Sessions in one Repo each show Needs input on agy's trust screen; Enter answers it and the prompt runs to Idle. |
| `the_initial_prompt_reaches_orch_through_the_hookup_until_a_fully_idle_stop` | agy runs with `-i <prompt>`; its `PreToolUse` reaches orch through the installed hookup as a WriteOutsideWorktree Guard prompt; the denial holds; Idle follows, which after a turn only a `fullyIdle` Stop gives. |
| `a_permission_prompt_needs_input_until_the_user_answers_it` | agy's own `run_command` prompt shows as Needs input; Enter approves it, the command runs and the Session goes Idle. |
| `resume_reopens_the_conversation_in_the_mode_cycled_with_shift_tab` | Shift+Tab moves agy from accept-edits to plan; after the Holder dies, Resume runs `agy --conversation <id> --mode plan` without `-i`, in the same Conversation. |
| `a_subagent_becomes_a_subagent_row_with_its_transcript` | One `invoke_subagent` becomes one Subagent row that finishes, and its transcript starts with the task. |
| `agys_print_mode_reads_its_prompt_from_stdin` | The adapter's Draft argv (`agy -p`, no argument) answers a prompt piped on stdin, as the Daemon pipes it. |
| `a_draft_comes_from_a_fresh_headless_agy_over_the_base_diff` | A Squash Draft for a Worktree with an untracked `greeting.txt` mentions it. |
| `agys_osc_11_query_does_not_stall_its_start_in_the_holder` | agy in a bare Holder reaches its trust screen, and its OSC 11 background query is not followed by a second of silence. It prints the timings as `OSC 11 finding: …` (see it with `--nocapture`); a stall would mean the Holder has to answer OSC 11. |

The answers it types are guesses until a run confirms them: Enter on the trust screen and on the permission prompt (`TRUST_ANSWER`, `PERMISSION_ANSWER` in `agy.rs`).

## Re-recording fixtures

```sh
ORCH_REAL_AGY=1 ORCH_REAL_AGY_RECORD=1 cargo test -p orch --test real_agy -- --ignored --test-threads=1
```

Recording adds a second named hook, `orch-real-agy-capture`, to each test's hooks file. It saves every hook's stdin (and answers `PreToolUse` with agy's neutral `{"decision":"ask"}`), while the template's statusline command saves every statusline payload. After each test the captures are scrubbed and written over `crates/orch-agent/tests/fixtures/antigravity/`: the first capture that fits each fixture wins, the Session's Conversation becomes `3c1e9a40-…` and its first Subagent `8d2f61b7-…`, the Worktree, Repo and HOME paths become `/home/dev/…`, and every email becomes `dev@example.com`. The Subagent's `transcript_full.jsonl` becomes `transcripts/subagent_full.jsonl`. Fixtures no run produces (errors, `ask_question`, MCP and browser tools, exhausted quota) stay as they are.

Then run `cargo test -p orch-agent`, fix the assertions that relied on made-up content, review the diff for anything private, and note the agy version in the fixtures README.
