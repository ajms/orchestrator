# agy 1.2.17 hands-on probe

Observed on 2026-10-05 against `agy` 1.2.17 (`~/.local/bin/agy`, a 210 MB Go binary). It ran in a tmux PTY (120×40) inside a git worktree (`/tmp/claude-1000/probe/wt`, branch `orch/probe`). Hooks were wired through the workspace file `.agents/hooks.json`, with every event pointing at [`agy-probe-hook.sh`](agy-probe-hook.sh). The raw PTY output was captured with `tmux pipe-pane`. Everything below was seen directly unless it is marked *docs*.

## Launch and resume

- `agy --help` flags relevant to orch:
  - `-i/--prompt-interactive <prompt>` starts the TUI with an initial prompt that is already submitted. **This works.**
  - `--conversation <id>` resumes by id, interactively, and **works combined with `-i` and `--mode`**. The resumed Conversation remembered earlier turns.
  - `-c/--continue` resumes the most recent Conversation.
  - `--mode accept-edits|plan` sets the execution mode at launch. With no flag the mode is default.
  - `--dangerously-skip-permissions` auto-approves everything. `--sandbox` restricts the terminal.
  - Also `--add-dir`, `--agent`, `--model`, `--effort`, `--project`, `--new-project`, `--log-file`.
  - Print mode: `-p`, `--output-format text|json|stream-json`, `--input-format stream-json`.
- The mode can be changed in-session with **Shift+Tab**, which cycles default → accept-edits → plan. It is shown on the input line and in the footer.
- On exit (`/exit`) the TUI prints `Resume with -c (or command below):` followed by `agy --conversation=<uuid>`.
- Headless `agy -p '…' --output-format json` returns `{"conversation_id", "status", "response", "duration_seconds", "num_turns", "usage": {input/output/thinking/cache_read/total tokens}}`.
- Conversation ids are UUIDs. They are exposed through:
  - the hook payload (`conversationId`),
  - the hook **environment** (`ANTIGRAVITY_CONVERSATION_ID`),
  - the statusline payload (`conversation_id`, `session_id`, with the same value),
  - the exit message and the headless json.

## Workspace trust (blocker for unattended start)

- The first launch in a directory not listed in `settings.json` `trustedWorkspaces` shows a full-screen **"Do you trust the contents of this project?"** prompt *before* the initial prompt runs.
- **Workspace hooks are not loaded until trust is given.** `cli.log` shows `loaded 0 named hooks` at start and `loaded 1 named hooks from 1 hooks.json file(s)` only after the prompt was accepted.
- Every new Worktree would hit this unless orch adds the Worktree path to `trustedWorkspaces` in the settings file that `agy` reads.

## Hooks

- The workspace `.agents/hooks.json` in a **git worktree** is honoured (once trusted). Hook commands run with **cwd = the `.agents/` directory**, not the workspace root. The environment is inherited plus `ANTIGRAVITY_CONVERSATION_ID`.
- The common payload has:
  - `conversationId`
  - `workspacePaths`
  - `transcriptPath` (`~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript_full.jsonl`)
  - `artifactDirectoryPath` (`…/brain/<id>`)
  - `modelName` (e.g. `gemini-3.8-flash-high`)
- Events seen in a turn with tools:
  `PreInvocation` → `PreToolUse` → `PostToolUse` → `PostInvocation` → `PreInvocation` → … → `PostInvocation` → `Stop`.
  - `PreInvocation`/`PostInvocation` wrap every model call (`invocationNum`, `initialNumSteps`).
  - **`Stop` fires once per turn**, not at exit: `{"executionNum", "terminationReason": "NO_TOOL_CALL", "error": "", "fullyIdle": true}`.
  - **No hook fires on session start, on prompt submit (only the following `PreInvocation`), on exit, or while waiting for the user.**
- **`PreToolUse` reply semantics:**
  - Replying `{}`, or a reply without a decision, **denies** the tool call. The transcript records `"tool call denied by pre-tool hook: "`.
  - Replying `{"decision":"ask"}` gives normal behaviour: agy's own policy then auto-allowed `write_to_file` in default mode and prompted for `run_command`.
  - Any orch hook must therefore always return an explicit decision.
- Tool names and args seen:
  - `write_to_file` `{TargetFile, CodeContent, Overwrite, Description}`
  - `run_command` `{CommandLine, Cwd, WaitMsBeforeAsync}`
  - `view_file` `{AbsolutePath}`
  - `invoke_subagent` `{Subagents:[{Prompt, Role, TypeName, Model, Workspace}]}`
  - `send_message` `{Recipient, Message}`
  - Every tool also carries `toolAction` and `toolSummary`.
  - *Docs* also list edit tools (not exercised here). The guard must enumerate them from the transcript or the tool list.
- `PostToolUse` for a prompted command fires only after the user answers. The gap from `PreToolUse` to `PostToolUse` was 59 s, made up of the approval wait plus execution time.

## Needs input and Agent state

- While a permission prompt was open there was: **no hook, no BEL, and no OSC notification** in the PTY output.
- **The statusline is the only state signal.** Its payload carries:
  - `agent_state`, observed values: `authenticating`, `initializing`, `working`, `tool_use`, `idle`;
  - **`tool_confirmation_pending: true`** while a permission prompt is open (`agent_state: "tool_use"`); the field is absent or null otherwise;
  - `cycle_mode` (e.g. `"plan"`) after a Shift+Tab mode change.
- The statusline command runs **only when the line re-renders** (state or mode changes), not on a timer.

## Statusline payload

The statusline is configured as `statusLine: {type:"command", command}` in `settings.json`. The observed payload (1.2.17) has these keys:

- `cwd`, `session_id`, `conversation_id`, `transcript_path`, `version`, `product: "antigravity"`
- `model {id, display_name, effort}`, `workspace {current_dir, project_dir}`
- `context_window {total_input_tokens, total_output_tokens, context_window_size, used_percentage, remaining_percentage, current_usage}`, `exceeds_200k_tokens`
- **`quota`**, keyed by pool. Each pool has `{remaining_fraction, reset_time, reset_in_seconds}`; the pools seen were `gemini-weekly` and `3p-weekly`, so these are weekly windows.
- `agent_state`, `tool_confirmation_pending`, `cycle_mode`
- `vcs {type}`, `sandbox {enabled}`, `plan_tier`, `email`, `terminal_width`

**No cost field.** The payload's `transcript_path` points at `~/.gemini/antigravity/brain/…`, which does not exist. The real location is `~/.gemini/antigravity-cli/brain/…`, as in the hook payloads.

## Config location and injection

- Settings live in `~/.gemini/antigravity-cli/settings.json` (`trustedWorkspaces`, `statusLine`, and permissions persisted from prompts). Global hooks are in `~/.gemini/config/hooks.json`.
- **No per-launch settings flag** and no config-dir env var. The binary's env strings show nothing like that; the `AGY_*`/`ANTIGRAVITY_*` vars are internal or auth related.
- **`HOME` redirect works.**
  - `HOME=<dir> agy` creates `<dir>/.gemini/antigravity-cli/` and keeps the user signed in (auth is not in `~/.gemini`, presumably the keyring).
  - But a fresh dir runs **first-run onboarding**: theme picker, then a data-sharing consent screen. All Conversations and brain data also land under the redirected `HOME`.
  - Child processes (git, gh, ssh) would inherit that `HOME`.
- Workspace `.agents/hooks.json` works per Worktree, subject to the trust caveat above.

## Embedding (PTY output)

- DECSET seen:
  - `?2026` synchronized output (heavily used),
  - `?2004` bracketed paste,
  - `?25` cursor,
  - `?1049` alt screen, used only for the trust prompt; the main TUI draws inline on the normal screen.
- **No mouse tracking requested** (no `?1000/1002/1003/1006`). There is no OSC 8, OSC 52 or bell; one OSC 11 background-colour query was seen.

## Subagents and transcripts

- Transcript files are `brain/<id>/.system_generated/logs/transcript.jsonl`, plus `transcript_full.jsonl`, which has typed args.
- Each line is a step:

  ```json
  {"step_index", "source": "USER_EXPLICIT|MODEL|SYSTEM",
   "type": "USER_INPUT|PLANNER_RESPONSE|GENERIC|SYSTEM_MESSAGE",
   "status": "DONE|ERROR", "created_at",
   "content", "thinking", "tool_calls": [{"name", "args"}],
   "input_tokens", "output_tokens", "error"}
  ```

  Tool results are separate `GENERIC` steps.
- `invoke_subagent` spawns a subagent with **its own conversation id**.
  - Its hooks fire in the same `.agents/hooks.json` with that id. **The payload has no parent id.**
  - The parent's tool-result step names the child: `Created the following subagents: {"conversationId": …, "logAbsoluteUri": "file:///…/brain/<child>/.system_generated/logs/transcript.jsonl", …}`.
  - The child reports back through `send_message` (`Recipient` = the parent id).
- `fullyIdle` is `false` on the parent's `Stop` while a subagent still runs. The subagent's own final `Stop` carries `fullyIdle: true`. So "Idle" means a `Stop` with `fullyIdle: true` from any conversation in the tree.
- After `--conversation` resume, the transcript records that all subagents and background tasks were stopped "due to server restart".
