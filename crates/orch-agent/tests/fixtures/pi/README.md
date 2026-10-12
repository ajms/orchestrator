pi payloads for the adapter tests. Findings: `docs/research/pi-fixtures.md`.

**Recorded live** from pi 1.1.0 (#99), then scrubbed. pi itself is real here: its agent loop, event bus, session writer, JSON/RPC/print modes, built-in tools, and the user's `subagent` and `question` extensions. **The model is scripted.** The local llama.cpp server wasn't reachable from the recording sandbox, so pi talked to `recording/fake_openai.py`, an OpenAI-compatible stub that picks a canned reply from a `[[scenario:<name>]]` tag in the first user message. Any assistant text, tool calls and `usage` numbers come from that stub, not from a real model. Everything pi wraps around them is pi's own output.

How it was recorded (`recording/`):

- `PI_CODING_AGENT_DIR` pointed at a throwaway agent dir with `settings.json` and `models.json` (provider `fake`, model `fake-model`). Its `extensions/` held symlinks to the user's `subagent` and `question` extensions and their helper modules, and its `agents/` held a copy of the user's agent definitions. `~/.pi` was not modified.
- `recorder.ts`, loaded with `-e`, appends every extension event as `{seq, ts, event, ctx, payload}` JSONL. `ctx` is `{mode, hasUI, cwd, sessionId, sessionFile, sessionName}` at that moment. `context` and `before_provider_request` payloads are omitted.
- `run.sh` runs one scenario in a fresh git repo (`mkrepo.sh`), with `--mode json`, `-p` or, through `rpc_driver.py`, `--mode rpc`. The RPC driver answers `extension_ui_request` dialogs from a scripted list and sends `abort` after an idle period.

Layout, one file per scenario:

- `extension_events/<scenario>.jsonl`: the recorder's stream. This is what orch's own extension will see.
- `json_mode/<scenario>.jsonl`: `pi --mode json` stdout (session header, then events).
- `rpc/<scenario>.stdout.jsonl` and `.stdin.jsonl`: `pi --mode rpc` traffic, including `extension_ui_request` and `extension_ui_response`.
- `print_mode/`: `pi -p` stdout and stderr. `print` exited 0 and `print-error` exited 1.
- `sessions/<scenario>.jsonl`: the session file pi wrote, at `<agentDir>/sessions/--<cwd with / as ->--/<ISO time>_<session id>.jsonl`.
- `lifecycle/*.json`, `subagent/*.json` and `question/*.json`: single payloads lifted from the streams above, for unit tests.

Scenarios:

| Scenario | Mode | What it covers |
|---|---|---|
| `basic` | json | Thinking and text reply, `agent_settled`, assistant `usage` |
| `tools` | json | `bash` tool: `tool_call`, `tool_execution_start/update/end`, `tool_result` |
| `error` | json | Provider HTTP 400: assistant `stopReason: "error"`, outcome `error` |
| `named` | json | `pi.setSessionName()` gives `session_info_changed` and a `session_info` entry |
| `cliname` | json | `--name` writes a `session_info` entry but emits no event |
| `print`, `print-error` | `-p` | Headless text output and its exit codes |
| `subagent-parallel` | json | Two `subagent` calls in one turn (explorer + planner), interleaved updates |
| `subagent-worktree` | json | `builder` with `worktree: true`, `worktreeCleanup` |
| `subagent-refused` | json | Frontmatter `refuse:` short-circuit, `stopReason: "refused"` |
| `question-single`, `question-other`, `question-cancel`, `question-text` | rpc | `question` select, "Other…" then input, cancel, free text |
| `question-multi` | rpc | `multiSelect: true`: no dialog in RPC, hangs until `abort` |
| `abort-bash` | rpc | `abort` during `sleep 30`: `agent_settled.aborted: true` |
| `subagent-question` | rpc | A child's `question` bridged to the parent's dialog |

Scrubbed: repo paths are `/home/dev/repo` and the agent dir is `/home/dev/.pi/agent`. Session-dir names become `--home-dev-repo--`, other home paths `/home/dev`, and temp paths `/tmp`. Strings over 400 characters are replaced by `<elided N chars>`; this mostly hits system-prompt sections and the user's agent prompts. In `extension_events/`, `message_update` drops its cumulative `message` and `assistantMessageEvent.partial`, as JSON mode does on the wire. `turn_end` and `agent_before_settle` replace `context` (a `BoundaryContextPreview`) with its key list. Session ids are fixed by `--session-id`.
