# Real-agy suite

Opt-in tests that drive a real, signed-in `agy` through orch's Daemon and Holder. Every test is `#[ignore]`. Without `ORCH_REAL_AGY=1`, even `--ignored` runs them as no-ops and prints one `real-agy suite skipped` line, so CI never starts agy. The unit tests next to them (scrubbing, fixture selection, isolation, the OSC 11 timing, the onboarding heuristics) run everywhere.

## Run

```sh
ORCH_REAL_AGY=1 cargo test -p orch --test real_agy -- --ignored
```

The tests take a shared lock, so they run one at a time without `--test-threads=1`. `ORCH_REAL_AGY=1 cargo test -- --ignored` also runs them, along with the workspace's other ignored tests (the freedesktop notification tests need a session bus).

Prerequisites:

- `agy` on `PATH`, signed in. The spec targets agy 1.2.17; the suite was written while 1.3.0 was installed and has not yet run against either.
- Network access and some quota: seven tests run model turns.
- Auth lives in the keyring, so the session bus (`DBUS_SESSION_BUS_ADDRESS`) must be reachable.

Isolation: agy, `orch hold` and the Daemon run with a cleared environment. They get only `PATH`, `TERM`, `LANG`, `LC_*` and `DBUS_SESSION_BUS_ADDRESS`, plus `HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME` inside the test's temp dir (`isolation.rs`), so your `~/.gemini` is never touched. `XDG_RUNTIME_DIR` is left out. If the keyring needs it on your machine, add it to `PASSED` in `isolation.rs`.

## Template HOME

The first run builds a template in `target/tmp/real-agy-template/`, and every test copies it:

1. It starts agy in orch's Holder with the template as `HOME` and drives the onboarding: Enter on the theme picker, then Down until the highlighted option reads as off ("No", "Don't", "off", …), then Enter. It stops at the trust screen.
2. It requires `settings.json` to exist and parse, with no sharing or telemetry key set to `true`, and with at least one known key (`SHARING_KEYS` in `template.rs`: `telemetryEnabled`, `dataSharingEnabled`, …) set to `false`. This check also runs after onboarding by hand. agy's real key name is unconfirmed, so the first run may ask you to add it.
3. It sets a capture command as the user's own statusline. It prints nothing.
4. It runs `orch agent install antigravity --yes`, so the Agent hookup is orch's own.

The template is rebuilt when `agy --version` or the `orch` binary path changes; delete the directory to rebuild it by hand. If the onboarding driver gets stuck, the failure prints the screen and an exact `env -i …` command for onboarding the template yourself. Run `touch target/tmp/real-agy-template/onboarded` afterwards and start the suite again. Symlinks inside the template are re-pointed at each copy, and a link that leaves the template fails the copy.

Each test also adds a named hook, `orch-real-agy-capture`, to its own hooks file. It saves the stdin of `PostToolUse`, `PreInvocation`, `PostInvocation` and `Stop` under `captures/<Session id>/` and answers `{}`. It adds `PreToolUse` only when re-recording (see below).

## Tests

| Test | Checks |
|---|---|
| `each_fresh_worktree_needs_input_on_agys_trust_screen` | Two Sessions in one Repo each show Needs input on agy's trust screen. Enter answers it, and the prompt runs to Idle. |
| `the_initial_prompt_reaches_orch_through_the_hookup_until_a_fully_idle_stop` | agy runs with `-i <prompt>`. Its `PreToolUse` reaches orch through the installed hookup as a WriteOutsideWorktree Guard prompt, and the denial holds. The Session goes Idle, and a `Stop` with `fullyIdle: true` from the Session's Conversation was captured. Idle alone wouldn't prove the Stop: an idle statusline also leads to Idle. |
| `a_permission_prompt_needs_input_until_the_user_answers_it` | agy's own `run_command` prompt shows as Needs input. Enter approves it, the command runs, and the Session goes Idle. |
| `resume_reopens_the_conversation_in_the_mode_cycled_with_shift_tab` | Shift+Tab moves agy from accept-edits to plan. After the Holder dies, Resume runs `agy --conversation <id> --mode plan` without `-i`, in the same Conversation. |
| `a_subagent_becomes_a_subagent_row_with_its_transcript` | One `invoke_subagent` gives exactly one Subagent row. Its id is the Conversation the root's `invoke_subagent` created, it finishes, and its transcript holds the task. The Session is never Idle while the row runs, and the root's `fullyIdle` Stop is captured. |
| `agys_print_mode_reads_its_prompt_from_stdin` | The adapter's Draft argv (`agy -p`, no argument) answers a prompt piped on stdin, the way the Daemon pipes it. |
| `a_draft_comes_from_a_fresh_headless_agy_over_the_base_diff` | The Draft argv has no `--conversation`. A Squash Draft for a Worktree with an untracked `greeting.txt` mentions it. The Session's Conversation id and its `transcript_full.jsonl` are unchanged afterwards. |
| `agys_osc_11_query_does_not_stall_its_start_in_the_holder` | Starts agy twice in a bare Holder, behind a one-second `sh` wrapper so the test is subscribed before agy writes anything. The first run leaves OSC 11 unanswered; the second answers it with a light background. It records whether agy sent the query, the silence after it (fails at ≥ 1 s) and whether the trust screen's colours differ between the runs. The finding is printed and saved to `target/tmp/real-agy-osc11.txt` for the PR. |

The answers the suite types are guesses until a run confirms them: Enter on the trust screen and on the permission prompt (`TRUST_ANSWER`, `PERMISSION_ANSWER` in `agy.rs`).

## Re-recording fixtures

```sh
ORCH_REAL_AGY=1 ORCH_REAL_AGY_RECORD=1 cargo test -p orch --test real_agy -- --ignored
```

While recording, the capture hook also saves `PreToolUse`, and the statusline command saves every statusline payload.

- **The capture hook's `PreToolUse` answer.** It answers with agy's neutral `{"decision":"ask"}`. agy's hook reference (embedded in the binary) says named hooks for one event are "merged and executed sequentially", without saying how their decisions combine. The probe saw a reply without a decision deny the call, so the capture hook can't stay silent. If agy let this `ask` override orch's `deny`, the Guard test would fail while recording.
- **Which test writes which fixture.** Each fixture is written by one test only, so the result doesn't depend on test order:
  - the trust test (first Session): `statusline/trust_screen.json`, `statusline/idle.json`;
  - the hookup test: `hooks/pre_invocation.json`, `post_invocation.json`, `pre_tool_use_run_command.json`, `stop_fully_idle.json`;
  - the permission test: `hooks/post_tool_use.json`, `statusline/tool_confirmation_accept_edits.json`;
  - the Subagent test: `hooks/pre_tool_use_invoke_subagent.json`, `stop_waiting_on_subagent.json`, every `subagent_*` hook, and `transcripts/subagent_full.jsonl`.
- **Which Conversations count.** Only captures from the Session's own Conversation, or from a Subagent the root's `invoke_subagent` created (read from the root's transcript), are used.
- **How a fixture is chosen and scrubbed.** The earliest matching capture is used. The Session's Conversation becomes `3c1e9a40-…` and the Subagent `8d2f61b7-…`. Worktree, Repo, test HOME and temp-root paths (plain and canonical), and your real HOME, become `/home/dev/…`. Every email, `%40` included, becomes `dev@example.com`.
- **Leak check.** A fixture is not written if it still contains your HOME, `$USER`, the hostname, `/tmp/` or another email.

Fixtures no test produces stay as they are: errors, `ask_question`, `write_to_file`, MCP and browser tools, exhausted quota, `working_plan`, and the Subagent's tool calls (its task uses no tools). Then run `cargo test -p orch-agent`, fix the assertions that relied on made-up content, review the diff, and note the agy version in the fixtures README.
