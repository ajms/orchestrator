# How orch's pi extension talks to orch

Research for #98 (map #96). Baseline: pi 1.1.0. Sources are the installed package at
`~/.pi/agent/install/releases/1.1.0/node_modules/@earendil-works/` (abbreviated `pca/` for
`pi-coding-agent/` and `tui/` for `pi-tui/`), the user's extensions in `~/.pi/agent/extensions/`, and
orch at this commit. Claims marked **verified** were checked by running pi 1.1.0 with throwaway
extensions and pi's built-in faux model provider. Those runs used a temporary `PI_CODING_AGENT_DIR`,
so no real model, credentials or `~/.pi` state was involved.

## Answer in brief

- **Blocking works and does not stall the TUI, provided the handler waits asynchronously.** pi awaits
  each `tool_call` handler with no timeout. An async child process (`execFile`/`spawn`) leaves the
  event loop free. `spawnSync` freezes the whole process, TUI included. Spawning `orch hook` asynchronously reuses
  `crates/orch/src/hook.rs` and the Holder's Guard path unchanged.
- **Ask can use `ctx.ui.confirm`.** It runs in the pane, fires `ui_prompt_start`/`ui_prompt_end`, and
  makes pi report OSC 7501 `blocked` with `kind=permission`. In print or JSON mode there is no UI and
  `confirm` returns `false`, so Ask fails closed.
- **Shipping:** embed the `.ts` in the `orch` binary (`include_str!`), write it to a
  version-keyed path under the XDG data dir, and pass `-e <path>` per launch. pi compiles TypeScript
  itself (jiti), so nothing needs building.
- **`-e` and discovered extensions:** CLI extensions load first, so orch's `tool_call` handler runs
  before the user's, and its block short-circuits theirs. A duplicate **tool** or **flag** name
  between any two extensions aborts startup (exit 1). Duplicate commands are only renamed. orch's
  extension should register no tools and at most `orch-`-prefixed flags.
- **OSC 7501** gives `working`/`blocked`/`done`/`error`/`idle`, but only in interactive mode, only as
  a coarse state, and only if orch's Holder learns to parse it. The extension sees every event
  that OSC 7501 is derived from, plus tools, prompts and the session id. Use the extension as the
  channel. OSC 7501 is optional and redundant.

## 1. Blocking a `tool_call` on orch

**How pi runs the handler.** `ExtensionRunner.emitToolCall` awaits every `tool_call` handler in
extension order. It returns on the first result with `block: true` and has no timeout
(`pca/dist/core/extensions/runner.js:957-971`). `AgentSession._beforeToolCall` awaits that call
before the tool runs (`pca/dist/core/agent-session.js:327-346`). A handler that throws blocks the tool
"as a fail-safe" (`pca/docs/extensions.md:260`), and the thrown message becomes the model-facing
result (verified). `ToolCallEventResult` is `{ block?, reason?, terminate? }`, and arguments are
changed by mutating `event.input` (`pca/dist/core/extensions/types.d.ts:1065-1074`).

**Verified** with a handler that waits 2 s and then returns
`{ block: true, reason: "orch Guard denied: probe" }`, plus a 250 ms `setInterval` ticker as an
event-loop probe:

| Wait implementation | Tool ran? | Model saw | Ticker during the 2 s wait |
|---|---|---|---|
| `await` on `execFile("sleep", ["2"])` | no | `orch Guard denied: probe` | kept ticking (8 ticks) |
| `spawnSync("sleep", ["2"])` | no | `orch Guard denied: probe` | no ticks at all: event loop frozen |
| `throw new Error("orch unreachable")` | no | `orch unreachable` | n/a |

So a synchronous `orch hook` call would freeze rendering, input and Escape for as long as a Guard
waits. orch's Guard wait is up to `GUARD_WAIT_SECS` = 7 days (`crates/orch-agent/src/guard/mod.rs:16`).
The extension must spawn `orch hook` asynchronously. pi has no hook timeout of its own, so unlike
Claude's `timeout` field (`crates/orch-agent/src/claude/mod.rs:54`) nothing has to be configured to
allow the long wait.

**Transport.** There are two options:

1. **Spawn `orch hook --agent pi --event <name>` asynchronously** with the JSON payload on stdin and read
   the answer from stdout. `hook.rs` already does the rest. It tags the event (`tag_hook_event`), uses
   `is_guard_payload` to choose between a blocking `request_guard` over the Holder socket and a
   fire-and-forget `report` (`crates/orch/src/hook.rs:26-60`, `crates/orch-holder/src/report.rs`).
   Then it prints `guard_answer`. With no `--session`, it falls back to `ORCH_SESSION`, which the Daemon
   sets in every Holder's environment (`crates/orch-daemon/src/lifecycle.rs:97-109`,
   `docs/agent-adapters.md`, `hookup` row). The pi adapter's `guard_answer` can pick any output format,
   for example `{"decision":"proceed"|"allow"|"ask"|"deny","reason":…}`.
2. **Speak the Holder socket directly** from TypeScript. The frame is a 4-byte big-endian length plus
   serde JSON of `ToHolder::Guard`/`Hook` (`crates/orch-holder/src/frame.rs:9-15`). This saves a
   process per event but duplicates the protocol, the socket location rules (`ORCH_HOLDER_SOCKET`,
   runtime dir) and the failure fallbacks in TypeScript.

Recommendation: option 1. It keeps one implementation of the Holder protocol (Rust), costs one
short-lived process per lifecycle event, and makes `orch fake-agent` replays work exactly as for the
other adapters. Report only lifecycle events (`session_start`, `before_agent_start`/`agent_start`,
`tool_call`, `tool_execution_end`, `ui_prompt_start`/`_end`, `agent_settled`, `session_shutdown`), never
`message_update` or `provider_stream_event`.

Points for the design:

- **Escape while a Guard waits.** `emitToolCall` does not race the abort signal. A handler that
  ignores `ctx.signal` keeps the tool pending until orch answers. Pass `ctx.signal` to `spawn`, and
  treat an abort as a block.
- **Parallel calls.** Tool calls from one assistant message can run in parallel
  (`pca/docs/extensions.md:123`), so several Guards can be pending at once. The Holder's `GuardTable`
  already keys pending Guards by id (`crates/orch-holder/src/guards.rs`).
- **Nested calls.** `ctx.executeTool()` calls, for example from codemode scripts, go through `tool_call`
  too, with `parentToolCallId` set (`pca/docs/extensions.md:148`). They are guarded with no extra
  work.
- **No orch.** If `ORCH_SESSION` is unset or `orch` can't be spawned, decide explicitly. `hook.rs`
  answers Proceed without a session and Ask without a socket, and a thrown error blocks.

## 2. Ask through `ctx.ui.confirm`

pi has no permission prompts of its own (`pca/docs/security.md:3`). An Ask therefore has to be raised by
the extension. `ctx.ui.confirm(title, message, { timeout?, signal? })` renders a Yes/No selector in the
editor slot of the TUI (`pca/dist/modes/interactive/interactive-mode.js:2108-2158`). It runs in the pane,
so the user answers in orch's PTY.

`ExtensionRunner.wrapUIPromptContext` wraps `select`, `confirm`, `input`, `editor` and `custom`.
The outermost prompt emits `ui_prompt_start` and its end emits `ui_prompt_end`, both through
`queueMicrotask`, so they are notifications that arrive slightly after the dialog opens
(`pca/dist/core/extensions/runner.js:354-397`). The confirm dialog also reports OSC 7501
`blocked` with `kind: "permission"` (`interactive-mode.js:2152-2158`).

**Verified** in a PTY (`script`, `PI_PROGRAM_STATUS=1`) with a `tool_call` handler that calls
`ctx.ui.confirm("orch Guard", …, { timeout: 1500 })`:

```
188  tool_call mode=tui hasUI=true
189  ui_prompt_start {"reason":"ui_prompt","kind":"confirm","title":"orch Guard"}
2189 ui_prompt_end   {"reason":"ui_prompt","kind":"confirm","title":"orch Guard"}
2189 confirm -> false            (timed out)
2197 agent_settled {"aborted":false}
OSC 7501: idle → working → blocked kind=permission msg="orch Guard" → working → done → clear
```

Consequences:

- The extension can map `ui_prompt_start` to `PermissionRequested` (or `QuestionAsked` for a
  `select`/`input` from the user's `question` tool), and `ui_prompt_end` to `PermissionCleared`. This
  covers dialogs from the user's own extensions as well, such as their `sandbox` and `question`.
- In print or JSON mode (the headless Draft) `ctx.hasUI` is false and `confirm` resolves `false`
  (`noOpUIContext`, `runner.js:132-140`). An Ask then blocks, which is the right default.
- When orch's own Guard prompt is the one asking (a Guard hit), the user answers in orch, not in pi.
  `ctx.ui.confirm` is only for `GuardAnswer::Ask`.

## 3. Shipping the extension from the Rust binary

What pi accepts:

- `-e`/`--extension <path>` takes a `.ts`/`.js` file or a directory with `index.ts`. TypeScript is
  loaded through jiti with no build step (`pca/docs/extensions.md:32-38,46`). **Verified** with files in
  `$TMPDIR`.
- Imports of `@earendil-works/pi-coding-agent`, `@earendil-works/pi-ai`, `typebox` and the other
  `@earendil-works/*` packages resolve to pi's bundled copies (`pca/dist/core/extensions/virtual-modules.js`).
  `node:` builtins work as usual. A single self-contained file with no `package.json` is enough.
- Packages (`pi install`) write to the user's pi settings. That breaks the adapter rule "per launch,
  never writing to the user's own Agent config" (`docs/agent-adapters.md`, checklist step 4).

Proposal:

1. `crates/orch-agent/src/pi/extension.ts`, embedded with `include_str!`.
2. At launch, write it to `$XDG_DATA_HOME/orchestrator/pi/orch-<version or content hash>.ts`, unless
   that file already exists with the same bytes. Write to a temporary file and rename, so concurrent
   launches can't see a half-written file. Keying by version or hash means a Session that is still running keeps the file it
   loaded, and every `orch` build loads its own copy. This is the sync guarantee. Old files can be
   pruned, for example by `orch doctor`.
3. The launch argv adds `-e <that path>`. Session and Worktree come from `ORCH_SESSION`/`ORCH_WORKTREE`
   in the environment. `LaunchSpec.orch_program`, the absolute `orch`, can't travel through the
   environment, because `Argv` has no env. Pass it as an `orch-`-prefixed flag the extension
   registers (`pi.registerFlag("orch-bin", …)`), or bake it into the written file. Note that a bad
   path would then be cached under the version key.
4. Add a version handshake. The extension sends its embedded version in its `session_start` report, so
   the adapter can flag a mismatch. That can only happen if someone points `-e` at a stale file.

Resume (`--session`/`--continue`) needs the same `-e`, because extensions are per process.

## 4. `-e` next to the user's discovered extensions

**Load order.** `mergePaths(cliEnabledExtensions, enabledExtensions)` puts `-e` paths first and drops
duplicate canonical paths (`pca/dist/core/resource-loader.js:405,782-794`). Handlers run in load
order (`pca/docs/extensions.md:97`). **Verified:** with orch's handler passing, a discovered
extension's `tool_call` handler ran after it. With orch's handler blocking, the discovered handler
never ran. So the user's `sandbox` extension
(`~/.pi/agent/extensions/sandbox/index.ts:56`) still sees, and can block, everything orch lets
through, but it never sees what orch denies.

**Conflicts.** `detectExtensionConflicts` reports a second extension that registers an existing
**tool** or **flag** name as `Failed to load extension …`, and `main.js` exits 1 on any such error
(`pca/dist/core/resource-loader.js:579-585,970-1003`, `pca/dist/main.js:645-652,745-753`).
**Verified** for both:

```
Error: Failed to load extension ".../tool-b.ts": Tool "dup_tool" conflicts with .../tool-a.ts
Error: Failed to load extension ".../flag-b.ts": Flag "--no-worktree-isolation" conflicts with .../flag-a.ts
exit=1
```

The check only compares extensions with each other. Overriding a built-in tool name such as `bash`
is allowed (verified), which is how the user's `read`, `edit` and `bash-output` extensions work.
Commands never abort: duplicates get `:<n>` invocation names (`runner.js:566-581`). Shortcuts
conflicts are warnings (`runner.js:464-472`).

The user's extensions register the tools `skill`, `read`, `web_search`, `question`,
`clean-up-isolated-branch`, `isolated_new_branch` (from `git-worktree-isolation`), `web_browse`, `edit`, `search`, `code_tree`, `subagent` and `bash`,
and the flag `--no-worktree-isolation` (`grep registerTool|registerFlag` under `~/.pi/agent/extensions`).
orch's extension should register **no tools** and only `orch-`-prefixed flags.

**Side finding for the adapter.** Passing `--no-worktree-isolation` when no loaded extension
registers it fails startup with `Error: Unknown option: --no-worktree-isolation` (exit 1, verified).
The flag belongs to the user's `git-worktree-isolation` extension, so orch can only force it when that
extension is present. Otherwise orch's extension must not register a flag of that name either,
because that clashes when the user's extension *is* present.

## 5. OSC 7501 program status

`ProgramStatusReporter` derives the state from `agent_start`, `message_end`, `compaction_*` and
`agent_settled`, plus `setBlocked` for dialogs and login
(`pca/dist/modes/interactive/program-status-reporter.js`). It reports `working` (with the session name),
`blocked` (`kind=permission` for confirm, `question` for other dialogs, `msg` the dialog title), `done`,
`error` (the first line of the error) and `idle` (`pca/docs/terminal-setup.md:221-233`). The wire format
is `ESC ] 7501 ; state=<s>:app=pi[:kind=<k>][:msg=<base64>] ESC \`, and `state=clear` on exit
(`tui/dist/program-status.js`). Reports are sent only after the terminal answers the `ESC]7501;?` query,
or always with `PI_PROGRAM_STATUS=1` (`tui/dist/terminal.js:197-211`). Only interactive mode creates a
reporter.

What it would take: orch's Holder parses the PTY with `vt100` 0.16, whose `Callbacks::unhandled_osc`
receives unknown OSCs (`crates/orch-holder/src/screen.rs`, vt100 `src/callbacks.rs:66`). Today it only
handles OSC 52 and focus reporting. orch would also have to set `PI_PROGRAM_STATUS=1` (the Holder
doesn't answer the query) and add a Holder-to-Daemon path for a non-hook signal.

What it gives: Starting/Idle (`idle`), Working, Needs input (`blocked`), Idle after a turn (`done`) and
Failed (`error`). It doesn't give tools, prompts, Subagents, the conversation id, usage, or the
difference between a Guard and a question beyond `kind`.

Verdict: OSC 7501 is not free. It needs new Holder plumbing, and everything it reports comes from
extension events that orch's extension receives anyway (`agent_start`, `agent_settled` with `aborted`,
`message_end` with `stopReason: "error"`, `ui_prompt_*`). Map those through `orch hook` and leave OSC 7501
unused, or keep it as a later cross-check.

## Open questions for later tickets

- Subagents run as child `pi` processes (`~/.pi/agent/extensions/subagent/`). Whether they receive orch's
  `-e`, and so its Guards, depends on how the user's `subagent` extension builds their argv. That
  belongs to the subagent ticket.
- The exact payload schema for `--agent pi` hooks, and the event-to-`AgentEvent` mapping, are adapter
  design and not settled here.
