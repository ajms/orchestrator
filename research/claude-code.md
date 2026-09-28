# Claude Code integration surfaces

Research for [#3](https://github.com/ajms/orchestrator/issues/3), part of map [#1](https://github.com/ajms/orchestrator/issues/1).

**Question:** What surfaces does Claude Code offer a wrapper program on Linux, and what can each one observe and control?

Checked against Claude Code **v2.1.283** (installed locally, `claude --help`) and the official docs at code.claude.com on 2026-09-28. Features change often and many carry "requires vX" notes, so pin a minimum CLI version.

## TL;DR

| Surface | Observe | Control | Answer permission prompts | Multi-turn | Survives wrapper exit |
|---|---|---|---|---|---|
| Interactive TUI under a PTY | Screen bytes only. Structured data comes from hooks, statusline and the transcript | Keystrokes | Yes, by a human typing into the PTY (or a `PermissionRequest` hook) | Yes | Only if the PTY lives outside the TUI process (own daemon, tmux, or Claude's supervisor) |
| `claude -p` with stream-json in and out, plus the control protocol | Every message, tool call, partial token, hook event and result, with usage and cost | Prompts, interrupt, set permission mode, set model, apply settings | **Yes**, via `control_request` / `control_response` (the SDK's `canUseTool`), or `--permission-prompt-tool` (MCP), or `PermissionRequest` hooks | **Yes**, one long-lived process | No. The process belongs to whoever holds its stdio. Resume with `--resume <id>` |
| `claude -p` single shot (`json`) | Final result, usage, cost, `session_id` | Flags only | Only through hooks / `--permission-prompt-tool` / defer+resume | Via `--resume` per turn | Transcript persists and can be resumed |
| Agent SDK (Python/TS) | Same as stream-json (it *is* stream-json) | Same | Yes (`canUseTool`) | Yes | No |
| Background sessions (`claude --bg`, `claude agents`) | `claude agents --json`: state, `waitingFor`, pid, sessionId | Start, stop, respawn, rm, `attach` (interactive only) | Only by a human after `claude attach` | Yes, by attaching | **Yes**, hosted by Claude's own supervisor daemon |
| Hooks (any mode) | Lifecycle events with `session_id`, `transcript_path`, `cwd`, `permission_mode` | Allow/deny/ask/defer tools, block Stop, inject context | Yes (`PermissionRequest` decision; `PreToolUse` allow/deny) | n/a | n/a |
| Statusline command (interactive only) | Cost, context-window tokens, rate limits, model, worktree | None | No | n/a | n/a |
| Transcript JSONL | Full history, per-message usage | None (read-only, internal format) | No | n/a | Persists on disk (30-day default retention) |

In short, the headless stream-json control protocol is the only surface that is structured *and* lets a program answer permission prompts and hold a multi-turn conversation. Its catch is that the wrapper has to own the process, so persistence needs the Orchestrator's own daemon or a `--resume`. The PTY route and Claude's own background sessions keep the native TUI and survive closing, but give the Orchestrator status only (through hooks or `claude agents --json`), not structured control.

## 1. Interactive TUI under a PTY

- Running `claude` (no `-p`) in a PTY gives the full native UI: permission dialogs, `AskUserQuestion` cards, `Shift+Tab` mode cycling, slash commands. The wrapper sees only terminal output. There is **no documented machine-readable channel on stdout** in this mode. Observability comes from side channels:
  - **Hooks**, configured per Session without touching user files via `--settings <file-or-json>` (`claude --help`; [CLI reference](https://code.claude.com/docs/en/cli-reference)). They report state changes, permission requests, Stop, and more (see §5).
  - **Statusline** script on stdin: cost, tokens, and more (see §7).
  - **Transcript** JSONL at `transcript_path` (see §7).
- `--session-id <uuid>` lets the wrapper choose the session id up front, so it can match hook events to Sessions without parsing anything (`claude --help`).
- `-n/--name` sets a display name. `--permission-mode` sets the starting mode (§6).
- Built-in desktop notifications reach only Ghostty, Kitty and iTerm2 by default. Otherwise use `preferredNotifChannel: "terminal_bell"` or a Notification hook ([terminal config](https://code.claude.com/docs/en/terminal-config#get-a-terminal-bell-or-notification)).
- `--tmux` runs Claude inside a tmux session with PTY support (requires `--worktree`, per `claude --help`). This is one way to make a PTY outlive the wrapper ([CLI reference](https://code.claude.com/docs/en/cli-reference)).
- Answering permission prompts works by forwarding keystrokes. Notification timing is tuned for humans: `permission_prompt` fires only after about 6 s without typing, and `idle_prompt` about 60 s after the reply. For an immediate signal, use the `PermissionRequest` hook ([hooks](https://code.claude.com/docs/en/hooks#notification)).

## 2. Headless: `claude -p` with stream-json

Source: [Run Claude Code programmatically](https://code.claude.com/docs/en/headless), [CLI reference](https://code.claude.com/docs/en/cli-reference), `claude --help`.

- `--output-format text|json|stream-json`. `json` returns one object with `result`, `session_id`, `usage`, `total_cost_usd` and a per-model breakdown. `stream-json` writes NDJSON events and ends with a `result` message.
- `--input-format stream-json` makes stdin a real-time stream of `{"type":"user","message":{"role":"user","content":...},"parent_tool_use_id":null}` messages ([streaming input](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode)). This gives **multi-turn in a single long-lived process**: messages queue, and each turn emits its own `result` ([cost tracking](https://code.claude.com/docs/en/agent-sdk/cost-tracking#track-costs-in-streaming-input-mode)).
- Useful extra flags:
  - `--include-partial-messages` streams token deltas as `stream_event`.
  - `--include-hook-events` puts hook lifecycle events into the stream.
  - `--replay-user-messages` acks stdin messages.
  - `--forward-subagent-text` includes subagent text.
  - `--verbose`.
- The event stream carries:
  - `system/init`: model, tools, MCP servers, plugins, and a `capabilities` array for feature detection.
  - `system/api_retry`.
  - `assistant` and `user` messages, with `parent_tool_use_id` for subagents.
  - `permission_denied` system messages.
  - `result`: `total_cost_usd`, `usage`, `modelUsage`, `permission_denials`, `session_id`, `stop_reason`.
- **Control protocol.** The same stdio stream carries `control_request` / `control_response` envelopes, each with a `request_id`. Requests exist for:
  - `initialize`: can register hooks. The response includes `pending_permission_requests` so a reconnecting client can re-answer them.
  - `interrupt`: with optional `cancel_queued`.
  - permission requests to the host (`can_use_tool`).
  - `setPermissionMode`, `setModel`, `applyFlagSettings`, and `getContextUsage`.

  It is documented only indirectly, through the TypeScript SDK reference ([`SDKControlInitializeResponse`, `SDKControlInterruptResponse`, `CanUseTool.requestId`](https://code.claude.com/docs/en/agent-sdk/typescript)). No standalone wire spec was found, so a Rust client has to mirror the SDK's behaviour.
- **Permission prompts in `-p`:**
  - `--permission-prompts host|none` (v2.1.259+). With `host`, prompts go to the SDK host (`canUseTool`) or to `--permission-prompt-tool <mcp tool>`. With `none`, anything that would prompt is auto-denied, unless a `PermissionRequest` hook allows it ([headless](https://code.claude.com/docs/en/headless#turn-off-permission-prompts-in-unattended-runs)).
  - With no host, prompts are denied.
  - `AskUserQuestion` is offered only when a permission host exists.
- **Defer and resume** (no live process needed). A `PreToolUse` hook returning `permissionDecision: "defer"` makes `-p` exit with `stop_reason: "tool_deferred"` and a `deferred_tool_use`. The wrapper collects the answer, then runs `claude -p --resume <id>` again, and the hook returns `allow` with the `updatedInput`. This lets a Session wait on a human without a live process ([hooks: defer](https://code.claude.com/docs/en/hooks#defer-a-tool-call-for-later)).
- Budget and limits: `--max-budget-usd`, `--max-turns`, `--fallback-model`.
- Signals: SIGINT ends the turn cleanly. SIGTERM exits with code 143, leaves the turn unfinished, and leaves pending permission prompts unanswered. Background Bash is killed about 5 s after the result.
- `--bare` skips user and project hooks, plugins, MCP, and CLAUDE.md, so it is reproducible. But it **requires `ANTHROPIC_API_KEY`**, because OAuth/subscription login is never read. That likely rules `--bare` out for a subscription user.
- Caveats: `-p` shows no workspace-trust dialog. Project hooks and `.mcp.json` run without asking. `-p` sessions are hidden from the `--resume` picker and from `--continue`, but can be resumed by id ([sessions](https://code.claude.com/docs/en/sessions#resume-a-session)).

## 3. Agent SDK, and reaching it from Rust

- The official SDKs exist **only for Python and TypeScript**. The SDK "is a library that runs the Claude Code binary": it spawns the CLI and speaks stream-json plus the control protocol over stdio. TS exposes `spawnClaudeCodeProcess`, Python a custom `Transport` ([overview](https://code.claude.com/docs/en/agent-sdk/overview), [TS ref](https://code.claude.com/docs/en/agent-sdk/typescript), [Python ref](https://code.claude.com/docs/en/agent-sdk/python)).
- The docs' own advice for other languages: "run the CLI as a subprocess with the `-p` flag" ([overview](https://code.claude.com/docs/en/agent-sdk/overview)).
- SDK features the Orchestrator would want:
  - `canUseTool(toolName, input, {suggestions, requestId, toolUseID, agentID, ...})`, which returns allow (with optional `updatedInput` / `updatedPermissions`) or deny.
  - Answering `AskUserQuestion` through `canUseTool`, via `updatedInput.answers`.
  - `interrupt()`, `setPermissionMode()`, `setModel()` (streaming input mode only).
  - The `resume` / `forkSession` options.

  Sources: [user input](https://code.claude.com/docs/en/agent-sdk/user-input), [TS `Query` object](https://code.claude.com/docs/en/agent-sdk/typescript#query-object).
- **Rust has no official SDK.** Several community crates implement the same subprocess and control protocol: `claude-agent-sdk`, `cc-sdk` (claims parity with Python SDK v0.1.33), `claude-code-agent-sdk`, `claude-agents-sdk`, [rust-agent-sdk](https://github.com/JoaoHenriqueBarbosa/rust-agent-sdk), and [borg-ml/claude-agents](https://github.com/borg-ml/claude-agents). They are unofficial and may trail the CLI. Options: use one, or implement the narrow protocol subset directly with tokio plus serde (NDJSON over child stdio).
- **Auth / terms flag.** The SDK overview says third-party developers may not "offer claude.ai login or rate limits for their products" built on the SDK without approval. For a personal tool driving the user's own logged-in CLI this probably does not apply, but it matters if the Orchestrator is ever distributed. It is an open question for the owner, not settled here ([overview](https://code.claude.com/docs/en/agent-sdk/overview)).

## 4. Session ids, `--resume` and `--continue`

Source: [sessions](https://code.claude.com/docs/en/sessions), `claude --help`.

- `--session-id <uuid>` pre-assigns the id. `--resume <id|name|transcript-path>` resumes a session. `--continue` resumes the most recent one in the cwd (but skips `-p`/SDK sessions unless `-p` is also given). `--fork-session` resumes under a new id.
- `--resume <id>` works from any directory (v2.1.223+).
- **Resuming the same session in two processes interleaves both into one transcript.** The Orchestrator must make sure only one process owns a Session.
- A resumed session restores history, model, agent and (from a terminal) permission mode. It does **not** restore `--mcp-config`, `--settings`, `--plugin-dir`, `--add-dir`, or `--fallback-model`, so the wrapper must pass them again. `-p --resume` starts in the default `-p` permission mode unless the mode is passed again.
- A tool call that was cut off is marked "cut off" on resume. `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1` makes Claude continue the interrupted turn.
- Transcript files live at `~/.claude/projects/<cwd-with-non-alnum-as-dash>/<session-id>.jsonl`. `CLAUDE_CONFIG_DIR` plus `CLAUDE_CODE_PROJECT_DIR_NAME` let a host relocate them. Retention is `cleanupPeriodDays` (30 by default).
- In both `json` and stream-json results, `session_id` is the resume handle.

## 5. Hooks as a status and event channel

Source: [hooks reference](https://code.claude.com/docs/en/hooks).

- **Transport.** A `command` hook gets JSON on stdin. An **`http`** hook gets a POST with a JSON body, and its response body is the decision. `async: true` runs a hook in the background, but it then cannot block or decide anything. Hooks can be injected per Session with `--settings` (JSON string or file), so the Orchestrator never has to edit user settings.
- **Common input:**
  - `session_id` and `prompt_id`
  - `transcript_path` and `cwd`
  - `permission_mode`
  - `hook_event_name`
  - `effort`
  - `agent_id` / `agent_type` (inside subagents)
- **Events relevant to status:**
  - `SessionStart` (`startup` / `resume` / `clear` / `compact` / `fork`) and `SessionEnd`
  - `UserPromptSubmit`
  - `PreToolUse` / `PostToolUse` / `PostToolUseFailure`
  - `PermissionRequest` (fires immediately when a permission decision is needed) and `PermissionDenied` (auto-mode denials)
  - `Notification` with `notification_type` of `permission_prompt`, `idle_prompt`, `elicitation_*`, `agent_needs_input`, `agent_completed`, `auth_success`, or `quota_auto_resume_*`
  - `Stop` (turn finished, includes `last_assistant_message`) and `StopFailure` (API error)
  - `SubagentStart` / `SubagentStop`
  - `PreCompact` / `PostCompact`
  - `WorktreeCreate` / `WorktreeRemove`
  - `Elicitation`
- **Control:**
  - `PermissionRequest` can return `decision: allow|deny`. So **hooks can answer permission prompts in any mode, including the interactive TUI**. An `http` hook can block while the Orchestrator asks the user.
  - `PreToolUse` can return `permissionDecision: allow|deny|ask|defer` and can rewrite `updatedInput`.
  - `Stop` can force the turn to continue (exit 2).
- **Caveat.** `Notification` `permission_prompt` / `idle_prompt` are delayed and suppressed while the user types (about 6 s and 60 s). For a "Session needs input" indicator, derive state from `PermissionRequest` → (`PostToolUse` | `PermissionDenied`) and `Stop` / `UserPromptSubmit` instead.
- A suggested state machine for the status overview:
  - `UserPromptSubmit` → working
  - `PermissionRequest` → needs-input
  - `PostToolUse` → working
  - `Stop` → idle / awaiting prompt
  - `StopFailure` → error
  - `SessionEnd` → exited

## 6. Permission modes and flags (for autonomy presets)

Source: [permission modes](https://code.claude.com/docs/en/permission-modes), [CLI reference](https://code.claude.com/docs/en/cli-reference).

| Mode (`--permission-mode`) | Runs without asking |
|---|---|
| `default` (alias `manual`) | Reads only |
| `acceptEdits` | Reads, file edits, common filesystem commands (`mkdir`, `touch`, `mv`, `cp`) |
| `plan` | Reads (plus classifier-approved commands when auto is available) |
| `auto` | Everything, with a background classifier. Since v2.1.283 it is the built-in default for interactive terminal sessions |
| `dontAsk` | Reads plus pre-approved tools. Everything else is denied without a prompt |
| `bypassPermissions` (`--dangerously-skip-permissions`) | Everything (deny rules still apply) |

- Layer rules on top with `--allowedTools` / `--disallowedTools` (e.g. `Bash(git diff *)`), `permissions.allow|ask|deny` via `--settings`, and `--tools` to restrict which tools exist at all. `--permission-prompts none` turns any remaining prompt into a denial in `-p`.
- Some actions are never auto-approved in any mode, including bypass: explicit `ask` rules, `AskUserQuestion`, `rm` on critical paths, and a few others.
- `-p` starts in `default` unless a mode is given.
- The mode can change at runtime: `Shift+Tab` in the TUI, or `setPermissionMode` over the control protocol.
- Hooks see the current mode in `permission_mode`.
- Candidate presets, which are for the map to decide: **Supervised** = `default`, **Edits** = `acceptEdits`, **Auto** = `auto`, **Locked** = `dontAsk` plus an allowlist, **YOLO** = `bypassPermissions` (sensible only because each Session is on its own Worktree, and it is still not a sandbox).

## 7. Where token and cost usage can be read

| Source | What | Notes |
|---|---|---|
| `result` message (`-p` json / stream-json / SDK) | `total_cost_usd`, `usage` (input/output/cache tokens), `modelUsage` per model (`costUSD`, `inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`) | Client-side estimate at list price. In streaming input mode `total_cost_usd` is a running total for the process (read the latest, don't sum). Since v2.1.277 it also includes spend restored on resume. `usage` excludes subagents, `modelUsage` includes them ([cost tracking](https://code.claude.com/docs/en/agent-sdk/cost-tracking)) |
| `assistant` messages | `message.usage` per API step | Dedupe by `message.id`. `output_tokens` there is a placeholder, so take output from `result` |
| Statusline stdin (interactive) | `cost.total_cost_usd`, `cost.total_duration_ms`, lines added/removed, `context_window.{total_input_tokens,total_output_tokens,used_percentage,current_usage.*}`, `rate_limits.five_hour/seven_day.used_percentage`, `session_id`, `transcript_path`, `worktree.*` | Runs on each assistant message, compaction, and mode change (300 ms debounce), plus an optional `refreshInterval`. A statusline script can **push these values to the Orchestrator** (write a file or hit a socket) ([statusline](https://code.claude.com/docs/en/statusline#available-data)) |
| Transcript JSONL | Per-message usage | Format is "internal … changes between versions". Fallback only ([sessions](https://code.claude.com/docs/en/sessions#where-transcripts-are-stored)) |
| OpenTelemetry | Metrics `claude_code.cost.usage`, `claude_code.token.usage`, `claude_code.session.count`, and more | `CLAUDE_CODE_ENABLE_TELEMETRY=1` plus `OTEL_METRICS_EXPORTER=otlp\|prometheus\|console`. Works in every mode. The Orchestrator could run a small OTLP receiver ([monitoring](https://code.claude.com/docs/en/monitoring-usage)) |

For subscription users, dollar cost is notional. The more useful quota signal is the statusline `rate_limits.*` fields.

## 8. Claude Code's own background sessions (agent view)

Source: [agent view](https://code.claude.com/docs/en/agent-view), `claude agents --help`, `claude daemon --help`.

This is directly relevant to "Sessions survive the TUI closing".

- `claude --bg "<prompt>"` (or `/bg`, `←`) runs a session under a **supervisor process**. Each session is its own `claude` process, and it keeps running after the terminal closes. Sessions survive sleep and auto-updates, but not shutdown (recover with `claude respawn`). There is one supervisor per `CLAUDE_CONFIG_DIR`.
- Idle sessions that stay unattached for about an hour have their process stopped, and they resume on the next interaction.
- Background sessions **auto-isolate into a git worktree** at `.claude/worktrees/<id>/`, unless already inside a linked worktree. `worktree.bgIsolation: "none"` turns this off.
- **Status API:** `claude agents --json [--all] [--cwd]` returns entries with `id`, `sessionId`, `kind`, `state` (`working|blocked|done|failed|stopped`), `status` (`busy|waiting|idle`), `waitingFor` (`permission prompt`, `input needed`, `sandbox request`, …), `pid`, `cwd`, `startedAt`, and `name`. The docs call it "the supported way to read session state from outside Claude Code". The `~/.claude/jobs/*` files are not stable.
- **Control:** `attach`, `logs`, `stop`, `respawn`, `rm`, `daemon status|stop`. There is **no documented command to send a prompt or permission answer to a running background session** other than attaching interactively (or peek-and-reply inside `claude agents`).
- The feature is in research preview.
- Implication: the Orchestrator could delegate persistence entirely to Claude's supervisor, embed `claude attach <id>` in a PTY pane for live interaction, and poll `claude agents --json` for the overview. The trade-offs: worktree layout and lifecycle belong to Claude, not the Orchestrator, and the design depends on a preview feature.

## 9. Other surfaces (noted, out of MVP scope)

- `--remote-control` lets a session also be controlled from claude.ai or the mobile app ([remote control](https://code.claude.com/docs/en/remote-control)). It goes through the cloud, so it is not a local wrapper API.
- `-w/--worktree [name]` creates a git worktree for a session. That overlaps with the Orchestrator's own Worktree management.

## Implications for the map

1. **Integration choice.** There are three candidates:
   - (a) PTY-hosted interactive `claude`, kept alive by an Orchestrator daemon, observed through `--settings`-injected http/command hooks and a statusline push.
   - (b) An Orchestrator-rendered UI over `-p` stream-json plus the control protocol (full structured control, with the Orchestrator owning the process and resuming on restart).
   - (c) Delegating to `claude --bg` and the supervisor.

   Of these, (a) gives the best native fidelity for "live view + interaction" at the lowest protocol cost, and hooks already give status, notifications, and even remote permission answers.
2. Pre-assign `--session-id` per Session. It links hook events, transcripts and resumes without any parsing.
3. Build "needs input" from `PermissionRequest` / `Stop`, not from the delayed `Notification` types.
4. Autonomy presets map cleanly to `--permission-mode` plus allow/deny rules, and can change at runtime.
5. Cost: take it from statusline (interactive) or `result` (headless). OTel is a mode-independent alternative.

## Sources

- https://code.claude.com/docs/en/headless
- https://code.claude.com/docs/en/cli-reference
- https://code.claude.com/docs/en/hooks
- https://code.claude.com/docs/en/permission-modes
- https://code.claude.com/docs/en/statusline
- https://code.claude.com/docs/en/sessions
- https://code.claude.com/docs/en/agent-view
- https://code.claude.com/docs/en/agent-sdk/overview
- https://code.claude.com/docs/en/agent-sdk/user-input
- https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode
- https://code.claude.com/docs/en/agent-sdk/cost-tracking
- https://code.claude.com/docs/en/agent-sdk/typescript
- https://code.claude.com/docs/en/agent-sdk/python
- https://code.claude.com/docs/en/monitoring-usage
- https://code.claude.com/docs/en/terminal-config
- https://code.claude.com/docs/en/remote-control
- Local: `claude --help`, `claude agents --help`, `claude daemon --help` (v2.1.283)
- Community Rust crates (unofficial): https://crates.io/crates/claude-agent-sdk, https://crates.io/crates/cc-sdk, https://github.com/JoaoHenriqueBarbosa/rust-agent-sdk, https://github.com/borg-ml/claude-agents
