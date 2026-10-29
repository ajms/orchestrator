# pi 1.1 extension events for Agent state and usage

Research for #193 (map #191, pi Agent adapter).

**Answer.** An orch-shipped pi extension can drive every Agent state from pi's extension events. The events are `session_start` (Starting/Idle, Conversation id), `input`/`agent_start` (Working), `ui_prompt_start`/`ui_prompt_end` plus `tool_call` for the `question` tool (Needs input), and `agent_settled` (Idle; read the run's last assistant `stopReason` to tell Errored from aborted). Usage comes from `ctx.model`, `ctx.getContextUsage()` and per-message `usage` (tokens and cost). Subscription quota is not exposed. Handlers are awaited in order, so forwarding must use async child processes or a socket, never a synchronous exec.

## Sources

Primary: the installed release, `PI=~/.pi/agent/install/releases/1.1.0/node_modules/@earendil-works`.

- `PI/pi-coding-agent/dist/core/extensions/types.d.ts`: event, context and API declarations (cited as `types.d.ts:<line>`)
- `PI/pi-coding-agent/dist/core/agent-session.js`: emit sites (`agent-session.js:<line>`)
- `PI/pi-coding-agent/dist/core/extensions/runner.js`: dispatch and the UI-prompt wrapper (`runner.js:<line>`)
- `PI/pi-coding-agent/dist/core/agent-session-runtime.js`: session replacement (`/new`, resume, fork)
- `PI/pi-coding-agent/docs/extensions.md`, `docs/json.md`, `docs/cli.md`
- `PI/pi-ai/dist/types.d.ts`: `Usage`, `StopReason`, `AssistantMessage`, `Model`
- Worked examples in `~/.pi/agent/extensions/`: `notify`, `statusline`, `question`, `conversation-name`, `subagent`

## 1. Lifecycle events: when they fire and what they carry

All handlers get `(event, ctx: ExtensionContext)`. `ctx` exposes `cwd`, `mode` (`"tui"|"rpc"|"json"|"print"`), `hasUI`, `sessionManager` (read-only), `model`, `isIdle()`, `signal`, `hasPendingMessages()`, `getContextUsage()` and more (`types.d.ts:213-249`).

| Event | Payload (types.d.ts) | When it fires |
|---|---|---|
| `session_start` | `reason: "startup"\|"reload"\|"new"\|"resume"\|"fork"`, `previousSessionFile?: string` (562-568) | Once per extension runtime, after load (`agent-session.js:2618`). `/new`, resume and fork tear down the runtime and build a new one, which then emits `session_start` with that reason (`agent-session-runtime.js:141,165,211,229,246,291`). `/reload` uses `reason: "reload"` (`agent-session.js:2966`). |
| `session_shutdown` | `reason: "quit"\|"reload"\|"new"\|"resume"\|"fork"`, `targetSessionFile?` (624-629) | Before the runtime is torn down. |
| `session_before_switch` / `session_before_fork` | `reason`, `targetSessionFile?` / `entryId`, `position` (576-587) | Before `/new`, resume or fork. These can be cancelled. |
| `session_info_changed` | `name: string \| undefined` (570-575) | After `pi.setSessionName()` or `/name` (`agent-session.js:3179-3183`). |
| `input` | `text`, `images?`, `source: "interactive"\|"rpc"\|"extension"`, `streamingBehavior?: "steer"\|"followUp"` (886-897) | When the user submits, before agent processing (`agent-session.js:1538`). It does not fire for extension `/commands`, which are tried first (`agent-session.js:1527`). `streamingBehavior` is set when the input is queued while the agent is running. A handler can return `handled`, which means no run starts. |
| `before_agent_start` | `prompt`, `images?`, `systemPrompt`, `systemPromptOptions` (708-719) | After the prompt is accepted, before the loop (`agent-session.js:1591`). |
| `agent_start` | none (721-723) | At the start of each low-level agent run (`agent-session.js:880-882`). |
| `turn_start` | `turnIndex: number`, `timestamp: number` (799-803) | At the start of each assistant response. |
| `message_start` / `message_update` / `message_end` | `message: AgentMessage`. `message_update` also carries `assistantMessageEvent` (813-827) | For user, assistant and toolResult messages. `message_update` fires for every streamed delta. |
| `tool_call` | `toolCallId`, `toolName`, `input` (mutable), `parentToolCallId?` (899-946) | Before a tool runs. It can return `{ block: true, reason }`, which makes it the Guard equivalent. A handler that throws blocks the tool. |
| `tool_execution_start` / `_update` / `_end` | `toolCallId`, `toolName`, `args`. `_update` adds `partialResult`. `_end` adds `result`, `isError: boolean`, `durationMs?` (829-859) | Around `execute()`. |
| `tool_result` | `toolCallId`, `toolName`, `input`, `content`, `isError`, `details`, `usage?` (955+) | After a tool runs. Handlers can rewrite the result. |
| `turn_end` | `turnIndex`, `message`, `toolResults`, `messageEntryId`, `toolResultEntryIds`, `outcome: "completed"\|"aborted"\|"error"` and other boundary state (804-811, 760-769) | After each assistant response and its tools. `outcome` comes from `message.stopReason` (`agent-session.js:489-490`). It can request `continue: true`. |
| `agent_end` | `messages: AgentMessage[]` (725-728) | At the end of one low-level run (`agent-session.js:885`). **This is not the end of the prompt**: retries, overflow compaction, queued steering or follow-ups, and `agent_before_settle` continuations start new runs (`agent-session.js:1389-1412`). JSON mode adds `willRetry`, which the extension type lacks. |
| `agent_before_settle` | boundary state with `outcome` (773-775) | Emitted only when some handler is registered (`agent-session.js:1453-1458`). It can request one continuation. |
| `agent_settled` | `aborted: boolean` (777-781) | **Once per prompt**, after all automatic work has ended (`agent-session.js:684-691`, called from the `finally` of `_runAgentPrompt`, line 1412). This is the real "back at the prompt" signal (`docs/json.md:48`, `docs/cli-integration.md:50`). |
| `ui_prompt_start` / `ui_prompt_end` | `reason: "ui_prompt"`, `kind: "select"\|"confirm"\|"input"\|"editor"\|"custom"`, `title?` (783-797) | When an extension's blocking `ctx.ui` dialog opens or closes (section 2). |
| `model_select` | `model`, `previousModel`, `source: "set"\|"cycle"\|"restore"` (861-866) | When the model changes (`agent-session.js:1934`). |
| `thinking_level_select` | `level`, `previousLevel` (868-872) | When the thinking level changes. |
| `session_before_compact` / `session_compact` / `session_compact_failed` | `reason: "manual"\|"threshold"\|"overflow"`, `willRetry`, plus `errorMessage?` and `aborted` on failure (588-622) | Around compaction. |
| `after_provider_response` | `status: number`, `headers: Record<string,string>` (694-698) | After each HTTP provider response (section 3). |

Sequence for one prompt: `input` → `before_agent_start` → `agent_start` → (`turn_start` → `message_*` → `tool_call` → `tool_execution_*` → `tool_result` → `turn_end`)× → `agent_end` → [more `agent_start`…`agent_end` for retries or follow-ups] → `agent_settled`.

Retry notices (`auto_retry_start` / `auto_retry_end`, `agent-session.js:3018,3041`) and `queue_update` go only to session listeners (JSON and RPC mode). **Extensions do not receive them.** An extension sees a retry only as another `agent_start`.

## 2. Waiting on the user, errors and aborts

**Idle at the prompt.** `agent_settled`, plus `session_start` at startup or after `/new` and resume, before any prompt. `ctx.isIdle()` is `!agentRunActive && !compacting` (`agent-session.js:1053-1055`). No event fires while the user is typing.

**Extension dialogs.** `ExtensionRunner.wrapUIPromptContext` wraps `ctx.ui.select/confirm/input/editor/custom` (`runner.js:359-368`). `withUIPrompt` emits `ui_prompt_start` on the outermost open and `ui_prompt_end` on close, counting depth so nested dialogs produce a single pair (`runner.js:369-395`). Both events are queued with `queueMicrotask` (`runner.js:396-399`). This covers any permission-style gate an extension builds with `ctx.ui.confirm` in `tool_call`.

**The user's `question` tool** (`~/.pi/agent/extensions/question/index.ts`) is a registered tool named `question`:

- Single choice or free text goes through `ui.select` / `ui.input` (lines 151-167), so `ui_prompt_start`/`_end` fire.
- **Multi-select does not**: it uses `ui.onTerminalInput` plus `ui.setWorkingMessage` (lines 75-145), which are not wrapped.
- Reliable signal: `tool_call` with `toolName === "question"` means Needs input. The matching `tool_execution_end` means the question was answered, which takes the Session back to Working. This mirrors Claude's `PreToolUse` with `AskUserQuestion`. The `notify` extension uses exactly this (`notify/index.ts:265-271`).
- In a subagent child (`PI_SUBAGENT_CHILD=1`) the question is bridged to the parent over stderr and fd 3 (`_question_protocol/protocol.ts`), and the parent's `subagent` tool opens the dialog. The parent sees no `question` tool call, only its own `ui.select`/`ui.input`, which emit `ui_prompt_*`. A child bridge that uses multi-select would be invisible.

**Built-in dialogs.** pi has no tool-permission prompts: it "does not ask for approval before every tool call" (`docs/security.md:3`). Built-in selectors such as `/model`, `/resume`, `/tree` and `/login` are user-initiated while idle and do not emit `ui_prompt_*`. The startup project-trust prompt runs before most extensions bind (`docs/extensions.md`, `project_trust`).

**Errors and aborts.**

- `AssistantMessage.stopReason: "pending"|"stop"|"length"|"toolUse"|"error"|"aborted"|"deferred"`, with `errorMessage?: string` and `rawStopReason?` (`pi-ai/dist/types.d.ts:313, 374-405`).
- `turn_end.outcome` and `agent_before_settle.outcome` reduce this to `completed|aborted|error`.
- Escape gives `agent_settled { aborted: true }`.
- A non-retryable failure ends with `agent_settled { aborted: false }`, and the last assistant message has `stopReason: "error"`. Retryable errors (`isRetryableAssistantError`, `agent-session.js:2978-2983`) are retried silently first.
- Recommended pattern: remember the last `turn_end.outcome` and the assistant `errorMessage` from `message_end`, then decide at `agent_settled`.
- The error is only a free-text `errorMessage`. There is no structured code like Claude's `StopFailure.error`. `notify/index.ts:309-345` classifies by substring (`429`, `rate limit`, `overloaded`, …).
- A failed compaction gives `session_compact_failed { errorMessage?, aborted, willRetry }`.

## 3. Model, context, tokens, cost, quota

- **Model**: `ctx.model` gives `id`, `name`, `provider`, `api`, `contextWindow`, `maxTokens`, `cost` (`pi-ai/dist/types.d.ts:917-946`). Changes arrive as `model_select`. Each assistant message also records `provider`, `model` and `responseModel?`.
- **Context window use**: `ctx.getContextUsage()` returns `{ tokens: number|null, contextWindow: number, percent: number|null }` (`types.d.ts:197-203`). The implementation is at `agent-session.js:3438-3470`. `tokens` and `percent` are `null` after a compaction, until the next response. The result is `undefined` without a model or with `contextWindow <= 0`.
- **Per-message usage**: `AssistantMessage.usage: Usage` is `{ input, output, cacheRead, cacheWrite, cacheWrite1h?, reasoning?, totalTokens, cost: { input, output, cacheRead, cacheWrite, total } }` (`pi-ai/dist/types.d.ts:291-312`). It is available on `message_end` and `turn_end.message`. `cost` is computed by pi from the model's list rates (`pi-ai/dist/models.js:658`), so on subscription providers it is a notional dollar value. `tool_result.usage?` carries nested model calls.
- **Session totals**: there is no extension API for them. `AgentSession.getSessionStats()` (`agent-session.js:~3390-3436`) is not on `ctx`. Sum `usage` over assistant (and toolResult) messages in `ctx.sessionManager.getEntries()`, or keep a running sum from `message_end`. The `setFooter` docs say the same (`types.d.ts:105-110`).
- **Subscription quota is not visible.** No field carries it.
  - The `statusline` extension gets it out of band. For Codex it sends its own probe request to `chatgpt.com/backend-api/codex/responses` and parses the `x-codex-*` headers (`statusline/codexQuota.ts:27-60`, README "How It Works"). For Claude it spawns `claude -p /usage` (`codexQuota.ts:227-260`).
  - `after_provider_response` hands extensions the raw response `headers` on HTTP/SSE paths: Anthropic (`pi-ai/dist/api/anthropic-messages.js:443`) and Codex SSE (`openai-codex-responses.js:283`). It does **not** fire on the Codex WebSocket transport (`openai-codex-responses.js:178-200`, no `onResponse`).
  - So an orch extension could parse provider rate-limit headers when present. Unverified: which Anthropic OAuth headers carry 5h/7d utilisation.

## 4. Running subprocesses: synchronous vs async

- `ExtensionRunner.emit` awaits each handler in turn (`runner.js:813-825`). An async handler that awaits a subprocess delays that event and everything after it:
  - streaming for `message_update` and `provider_stream_event` (`docs/extensions.md`, "Handlers are awaited in stream order");
  - tool execution for `tool_call`;
  - the UI going idle for `agent_settled`.
- A synchronous `execFileSync`/`spawnSync` blocks Node's event loop, which freezes the whole TUI (rendering and input) until it returns. Do not use it.
- `pi.exec(cmd, args, { signal, timeout, cwd })` is async (`types.d.ts:1256`, `dist/core/exec.js` uses `spawn`), but it opens **stdin as `"ignore"`**. It cannot pipe a payload to `orch hook`/`orch tap`, which read stdin (`crates/orch/src/hook.rs:9-11`, `tap.rs:15-17`). Use `node:child_process.spawn` directly, as `notify`, `statusline` and `subagent` do, or pass the payload another way.
- **Recommendation**:
  - Fire-and-forget (`spawn`, do not await) for status events.
  - Await only where orch must answer: a Guard in `tool_call`. The tool waits and the TUI stays responsive.
  - One process per event loses ordering between rapid events and costs a fork per event. Forward only coarse events (never `message_update`), or keep one long-lived channel opened at `session_start` (a persistent `orch` child reading JSONL on stdin, or a socket) and close it in `session_shutdown`.
  - `docs/extensions.md` says not to start processes in the factory. Start them from `session_start`.

## 5. Session name and Conversation changes

**Name.**

- `pi.setSessionName(name)` and `/name` append a `session_info` entry (`session-manager.d.ts:93-96`) and emit `session_info_changed { name }` (`agent-session.js:3179-3183`, `interactive-mode.js:5579`). `pi.getSessionName()` reads it back.
- `--name` on the CLI writes the entry before startup without an event (`main.js:567-574`). Read `getSessionName()` at `session_start`.
- Renaming from the `/resume` selector writes to that file via a fresh `SessionManager` and emits nothing (`interactive-mode.js:4766-4772`).
- The user's `conversation-name` extension sets the name asynchronously after each user `message_end` through `pi.setSessionName` (`conversation-name/index.ts:56-140`). The name arrives seconds after the prompt, as a `session_info_changed`. This maps to `TitleChanged` without a file watcher.

**Conversation.**

- `ctx.sessionManager.getSessionId()` and `getSessionFile()` identify the Conversation (`session-manager.d.ts:178`).
- `/new`, resume, fork and `/reload` each give `session_shutdown { reason, targetSessionFile? }` on the old runtime, then `session_start { reason, previousSessionFile? }` on a **fresh extension runtime**. Read the new id there and emit `ConversationChanged`. `reason: "reload"` keeps the same Conversation.
- In-memory extension state does not survive. Captured `ctx`/`pi` objects go stale and throw (`runner.js` `invalidate`), so a long-lived forwarding channel must be reopened in each `session_start`.
- `/tree` navigation stays in the same file (`session_tree`).
- The CLI can pin the id: `--session-id <id>` for a new session, and `--session <path|id>` or `-c` to resume (`docs/cli.md:86-107`). Extensions load with `-e/--extension <path>` (`docs/cli.md:194`).

## Poor fits with orch's Agent states

- **`agent_end` is not Idle.** Use `agent_settled`. Claude's `Stop` fires once per prompt; pi's `agent_end` can fire several times.
- **Errors carry no structured kind.** `FailureKind` needs substring matching of `errorMessage`, or `Other(msg)`. Silent auto-retries are invisible: a 429 that pi retries shows as Working throughout. That is arguably right, but orch cannot show "retrying".
- **Abort vs error.** `agent_settled.aborted` separates them. Escape should probably map to `TurnEnded` (Idle), not `Failed`.
- **Needs input covers two signals.** Use the `question` tool through `tool_call`/`tool_execution_end`, and generic extension dialogs through `ui_prompt_start`/`_end`. Multi-select questions emit only the first. A dialog opened during `tool_call` by another extension (a permission gate) also emits `ui_prompt_*`. pi has no `PermissionRequested`/`PermissionDenied` of its own.
- **No permission modes.** pi has no plan or accept-edits modes, so there is no `ModeChanged`. Thinking level (`thinking_level_select`) is the nearest knob and doesn't fit.
- **Compaction.** A manual `/compact` runs without `agent_start`. Auto-compaction runs inside a prompt. Neither needs its own state, but a manual compact won't show as Working unless orch maps `session_before_compact`.
- **Subagents.** The user's `subagent` tool runs child `pi --mode json -p --no-session` processes (`subagent/index.ts:545, 745`, env `PI_SUBAGENT_CHILD=1`). The parent's extension sees only the `subagent` tool's `tool_call`/`tool_execution_*` (with `partialResult` updates) and no per-child lifecycle. Children inherit env, so an orch extension enabled via env must skip `PI_SUBAGENT_CHILD=1` (as `notify` does, `notify/index.ts:250`) or check `ctx.mode !== "tui"`. A `-e` flag is not inherited.
- **Usage totals and rate limits** are assembled by the extension: totals from session entries, rate limits only from provider headers where `after_provider_response` fires. Codex over WebSocket yields none.
- **orch side.** `orch hook` and `orch tap` hard-code `ClaudeCode` (`crates/orch/src/hook.rs:13`, `tap.rs:24`) and must learn to select the pi adapter.
