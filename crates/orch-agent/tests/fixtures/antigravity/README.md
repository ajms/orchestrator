Antigravity (`agy`) payloads for the adapter tests.

**Recorded live** from agy 1.3.1 by the opt-in real-agy suite (`ORCH_REAL_AGY=1 ORCH_REAL_AGY_RECORD=1`, see `crates/orch/tests/real_agy/README.md`), then scrubbed:

- `hooks/pre_invocation.json`, `hooks/post_invocation.json`, `hooks/stop_fully_idle.json`, `hooks/post_tool_use.json`;
- `hooks/pre_tool_use_invoke_subagent.json`, `hooks/subagent_pre_invocation.json`, `hooks/subagent_post_tool_use.json` and `hooks/subagent_stop.json` (a Subagent with its own `conversationId`, `8d2f61b7-…`);
- `transcripts/subagent_full.jsonl`, that Subagent's `transcript_full.jsonl`;
- `statusline/trust_screen.json` and `statusline/tool_confirmation_accept_edits.json`.

Live findings these record, against the agy 1.2.17 probe: `PostToolUse` carries `toolCall` (and an `error` field, which `PreToolUse` lacks), so a tool's end names its tool; early statusline lines carry `"model": null`; the trust screen shows only as `agent_state: initializing` with an empty `conversation_id` and no `tool_confirmation_pending`.

**Built by hand**, from the probe on `research/agy-probe` and agy's hook reference and `exa.hooks_pb` descriptors embedded in its binary, because the live suite doesn't produce them:

- `hooks/pre_tool_use_run_command.json` and the other `pre_tool_use_*.json` tool calls, apart from `invoke_subagent`. Guesses: the `ask_question` arguments, `send_command_input`'s `Input`, `open_browser_url`'s `Url`, the MCP tool name `mcp_chrome_devtools_take_memory_snapshot` (agy names MCP tools `mcp_<server>_<tool>`, with `-` turned into `_`), and the path fields of the file-writing tools other than `write_to_file`'s `TargetFile`.
- `hooks/stop_error.json`, `hooks/stop_max_steps_exceeded.json` (guessed `MAX_STEPS_EXCEEDED` spelling), `hooks/stop_waiting_on_subagent.json`, `hooks/subagent_pre_tool_use_run_command.json` and `hooks/subagent_stop_error.json`.
- `hooks/pre_tool_use_invoke_two_subagents.json` and `transcripts/subagent_every_step_kind.jsonl`: two Subagents and every transcript step kind (thinking, a system notice, an error result, a second prompt), which one live Subagent doesn't cover.
- `statusline/idle.json`, `statusline/working_plan.json` and `statusline/quota_exhausted.json`. Guesses: an RFC 3339 `reset_time`, and that a missing `remaining_fraction` means 100% used (from agy's `omitempty` JSON tag).

`hooks/` holds raw hook stdin. agy names no event in the payload; orch's hookup passes it as `orch hook --event <name>`, so the tests tag each fixture with its event.

Scrubbed: paths use `/home/dev`, the email is `dev@example.com`, and conversation ids are fixed.
