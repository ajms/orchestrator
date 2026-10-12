# pi 1.1.0 fixtures: events, session files, subagent results

Answers #99, part of map #96. The fixtures are in `crates/orch-agent/tests/fixtures/pi/`, and their README says how they were recorded and what each file is.

## How the fixtures were recorded, and the caveat

They are real pi 1.1.0 output, but the model is scripted. The local llama.cpp server (`127.0.0.1:8080`, the user's default provider) wasn't reachable from the recording sandbox, which has its own network namespace. So pi ran against an OpenAI-compatible stub (`recording/fake_openai.py`), configured through a throwaway `PI_CODING_AGENT_DIR`. The stub only decides what the model "says". Everything else is pi's own output: events, session entries, tool execution, the subagent child processes, the RPC dialog protocol and exit codes. Treat assistant text and `usage` numbers as placeholders.

What wasn't recorded live:

- The TUI (interactive mode). `ui_prompt_*` was recorded in RPC mode, where `ctx.hasUI` is true and dialogs work.
- Compaction, auto-retry, `project_trust` (it only fires for project-local extensions), `model_select` and `thinking_level_select`.
- A real model's `usage`, including non-zero `cacheRead` and `cost`.

## Event lifecycle as observed

Sources: `extension_events/*.jsonl`, and `dist/core/extensions/types.d.ts` in the pi 1.1.0 install (lines 522–1000).

- A run gives: `session_start {reason:"startup"}` → `resources_discover` → `input {source:"interactive"}` → `before_agent_start` → `agent_start` → (`turn_start` … `turn_end`)+ → `agent_end` → `agent_before_settle` → `agent_settled` → `session_shutdown {reason:"quit"}` (`extension_events/basic.jsonl`). `input.source` is `"interactive"` even under `--mode json` and `-p`.
- Each extension event can read `ctx.sessionManager.getSessionId()`, `getSessionFile()` and `getSessionName()`, plus `ctx.mode` (`json`, `rpc` or `print`) and `ctx.hasUI` (true only in RPC and TUI). See the `ctx` field on every recorder line.
- **`tool_execution_start` fires before `tool_call`.** In every recording, the order is `tool_execution_start`, then `tool_call` (where an extension can block), then `tool_execution_update`*, `tool_result` and `tool_execution_end`. This ordering suggests a blocked call has already emitted `tool_execution_start`, but no block was recorded to confirm it.
- Extension `agent_end` carries `{type, messages}` only. The JSON-mode wire record adds `willRetry` (`lifecycle/agent_end_*.json` vs `json_mode/basic.jsonl`).
- **How each outcome looks:**

  | Outcome | Last assistant message | `agent_before_settle.outcome` | `agent_settled.aborted` | Fixture |
  |---|---|---|---|---|
  | Completed | `stopReason: "stop"` | `"completed"` | `false` | `basic` |
  | Provider error | `stopReason: "error"`, `errorMessage: "400: {…}"` | `"error"` | `false` | `error` |
  | Abort (RPC `abort`) | `stopReason: "error"`, `errorMessage: "This operation was aborted"`, empty content | **not emitted** | `true` | `abort-bash`, `question-multi` |

  An abort therefore looks like an error on the message. Only `agent_settled.aborted` (or a missing `agent_before_settle`) tells the two apart. The aborted bash call ends with `isError: true` and the text "Command aborted".
- `session_info_changed {name}` fires when an extension calls `pi.setSessionName()` (`lifecycle/session_info_changed.json`). The user's `conversation-name` extension names sessions that way (`extensions/conversation-name/index.ts`, `setSessionName` calls at lines 74 and 140). **`--name` fires no event.** It writes a `session_info` entry as the first entry after the header, before extensions bind (`sessions/cliname.jsonl`). A reader should get the name from the session file, or from `ctx.sessionManager.getSessionName()` at `session_start`.
- `agent_before_settle` and `turn_end` carry `{outcome, entries, continue, context}`. `context` is a large `BoundaryContextPreview`, which the fixtures elide.

## Session JSONL

Source: `sessions/*.jsonl`. Format reference: `docs/session-format.md` in the pi install.

- Path: `<agentDir>/sessions/--<cwd with "/" replaced by "-">--/<ISO timestamp with ":" and "." replaced by "-">Z_<sessionId>.jsonl`. `--session-id` sets the id, and on first use pi warns on stderr: "No project session found with id …; creating a new session with that id".
- Entries form a tree linked by `id` and `parentId`. Recorded types: `session` (header: `version: 3`, `id`, `timestamp`, `cwd`), `model_change`, `thinking_level_change`, `session_info {name}` and `message`. Message roles are `system` (with `sections` and `toolsAdded`), `user`, `assistant` and `toolResult`.
- Assistant `usage` is `{input, output, cacheRead, cacheWrite, reasoning, totalTokens, cost: {input, output, cacheRead, cacheWrite, total}}`, alongside `api`, `provider`, `model` and `stopReason` (`toolUse`, `stop` or `error`). `toolResult` entries carry `toolCallId`, `toolName`, `content`, `isError` and `durationMs`.
- The `subagent` tool's result entry stores its whole `details`, including the child's messages, so a parent session file can be large.

## Headless runs

- `--mode json` (`json_mode/*.jsonl`) writes the session header first, then the same event shapes. `message_update` is delta-only on the wire. **It exits 0 even when the provider fails.** The failure only shows as `stopReason: "error"` (`json_mode/error.jsonl`).
- `-p` (`print_mode/`) prints the final assistant text to stdout and exits 0. On a provider failure it prints `errorMessage` to stderr and **exits 1**.
- `--mode rpc` (`rpc/`) answers `{"type":"prompt"}` with `{"type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}` and then streams events. It has no session header.

## `subagent` extension

Source: `~/.pi/agent/extensions/subagent/index.ts`, with `SingleResult` at lines 199–228 and the child spawn at 697–746. Fixtures are in `subagent/`.

- Each call spawns `pi --mode json -p --no-session [--tools …] [--no-skills --skill …] --system-prompt <tmpfile> --model <m> "Task: <task>"`. The child gets env `PI_SUBAGENT_CHILD=1`, `PI_QUESTION_RESPONSE_FD=3` and `PI_SUBAGENT_SESSION_NAME=<agent><n>: <taskName>`. Child sessions are not persisted (`--no-session`), so orch only sees a child through the parent's `subagent` tool events.
- Progress comes as `tool_execution_update`, with `partialResult = {content: [{type:"text", text: <latest assistant text or "(running...)">}], details: SingleResult}`, emitted on every child `message_end`, `tool_execution_update` and `tool_execution_end`. `details.messages` is cumulative, so each update repeats the child transcript so far (`parallel_update_*.json`).
- Parallel calls in one assistant message run concurrently, and their updates interleave by `toolCallId` (`json_mode/subagent-parallel.jsonl`). Each call ends with its own `tool_execution_end`.
- Final `details` (`SingleResult`): `agent`, `sessionLabel` (e.g. `"explorer1: Find readme"`), `agentSource`, `task`, `exitCode`, `messages`, `stderr`, `usage {input, output, cacheRead, cacheWrite, cost, contextTokens, turns}`, `model`, `stopReason`, `modelAttempts [{model, exitCode, succeeded, stopReason}]`. Optional fields: `errorMessage`, `worktreePath`, `worktreeBranch`, `worktreeCleanup` and `partialResults`.
- **`worktree: true` (builder)**: `worktreePath` is under `$TMPDIR/pi-subagent-wt-*/<agent>` and the branch is `subagent/<agent>-<id>`. When the child leaves its change uncommitted, as the recorded builder did, `worktreeCleanup` is `{merged: true, skipped: false, message: "Propagated uncommitted worktree changes from subagent/… to parent (no commit)."}`. The change then lands **staged, without a commit**, in the parent's checkout (`git status` showed `A CHANGELOG.md`). The README's `Merge <agent>: <taskName>` commit only happens when the child commits.
- **Refusal**: `isError: true`, `stopReason: "refused"`, `exitCode: 1`, `turns: 0`, no `model`. The content text is `"Agent refused: [builder] refused: …"` (`refused_end.json`).
- A call with no per-call `model` first tries the agent's frontmatter models (the user's agents name `openai-codex/…`) and then falls back. The fixtures pass `model: "fake/fake-model"`, so `modelAttempts` holds one entry. The retry path wasn't recorded.

## `question` tool

Source: `~/.pi/agent/extensions/question/index.ts`, with the bridge protocol in `_question_protocol/protocol.ts`. Fixtures are in `question/` and `rpc/question-*`.

- Single select calls `ctx.ui.select(title, [...options, "Other (type your own)…"])`. This emits `ui_prompt_start` and `ui_prompt_end` with `{reason:"ui_prompt", kind:"select", title}`, and in RPC an `extension_ui_request {method:"select", title, options}`. Picking "Other…" opens a second dialog (`kind:"input"`). Free text without options is a single `input`.
- The result is always `isError: false` with `details` absent. Its text is the answer, or a refusal string such as `"User dismissed question. Proceed with a reasonable default."`. An abort gives `"aborted Proceed with a reasonable default."`. With no UI (`--mode json`, `-p`) the text is `"No interactive UI available. …"` (source line 302). With `PI_NON_INTERACTIVE=1` it is `"Non-interactive mode active — questions disabled. …"`. A wrapper has to match on text to tell an answer from a refusal.
- **Multi-select (`multiSelect: true`) isn't a dialog.** It uses `ui.onTerminalInput` and `ui.setWorkingMessage` (source lines 76–151), so it **never emits `ui_prompt_start`**, not even in the TUI (that part is from source, not recorded). In RPC both calls are no-ops (`docs/rpc-extension-ui.md`, "Limitations"), so the tool hangs until the run is aborted (`rpc/question-multi.*`). orch can't spot a pending multi-select from `ui_prompt_*`.
- **Subagent bridge**: a child's `question` writes `PI_QUESTION_REQUEST {id, questions}` on stderr. The parent's subagent extension opens the dialog itself through `dispatchAsk(ctx, …)` and answers on fd 3 with `PI_QUESTION_RESPONSE {id, answers|error}`. The parent therefore emits `ui_prompt_start` and `ui_prompt_end` in the middle of its own `subagent` tool execution (`extension_events/subagent-question.jsonl`).

## Leads for later tickets

- Use `agent_settled.aborted` to tell an abort from an error, not `stopReason`.
- Take the session name from the `session_info` entry or `getSessionName()`, as well as from `session_info_changed`.
- Treat `question` with `multiSelect` as invisible to `ui_prompt_*`, or have orch's extension watch `tool_execution_start` for `toolName: "question"`.
- A `worktree: true` builder that doesn't commit leaves staged changes in the Session's worktree.
- Re-record against a reachable model with `recording/run.sh` to get real `usage`, including cache and cost.
