Antigravity (`agy`) payloads for the adapter tests, shaped after agy 1.2.17.

These are **not live captures**. They were built from:

- the hands-on probe on `research/agy-probe` (`docs/research/agy-probe.md`), which recorded the hook and statusline fields of agy 1.2.17;
- agy's own hook reference and the `exa.hooks_pb` descriptors embedded in the agy binary (field names of `HookArgsCommon`, `PreToolHookArgs`, `PostToolHookArgs`, `Pre/PostInvocationHookArgs` and `StopHookArgs`, and the statusline's JSON tags).

Recording live was not possible where they were written: the only agy installed was 1.3.0, and running it needs a throwaway `HOME` with onboarding, the trust screen and network auth. Replace them with real captures from the opt-in real-agy suite when one exists.

- `hooks/` holds raw hook stdin. agy names no event in the payload; orch's hookup passes it as `orch hook --event <name>`, so the tests tag each fixture with its event. Guesses: the `ask_question` arguments, the `MAX_STEPS_EXCEEDED` spelling of the termination reason, `send_command_input`'s `Input` argument, the `Url` argument of `open_browser_url`, and the MCP tool name `mcp_chrome_devtools_take_memory_snapshot` (agy 1.3.0's binary names MCP tools `mcp_<server>_<tool>`, with `-` turned into `_`). `write_to_file`'s `TargetFile` and `run_command`'s `CommandLine`/`Cwd` were seen in the probe; the other file-writing tools are assumed to name their file the same way.
- `hooks/subagent_*.json` come from a Subagent (its own `conversationId`, `8d2f61b7-…`). `hooks/pre_tool_use_invoke_subagent.json` is the parent's `invoke_subagent` call, with the `Subagents[]` shape of a real agy 1.3.0 transcript.
- `transcripts/subagent_full.jsonl` is a Subagent's `transcript_full.jsonl`, modelled on the step shape of real agy 1.3.0 transcripts (the parent's task as a `[Message] … sender=<parent>` system message, `PLANNER_RESPONSE`/`GENERIC` pairs, a `sender=system` notice); its content is made up.
- `statusline/` holds raw statusline stdin. `trust_screen.json` assumes, as the probe reported, that the trust screen shows `agent_state: initializing` with `tool_confirmation_pending: true`. `quota_exhausted.json` drops one pool's `remaining_fraction` and another's `reset_time`. Guesses: an RFC 3339 `reset_time`, and that a missing `remaining_fraction` means 100% used (from agy 1.3.0's `omitempty` JSON tag; the 1.2.17 probe never saw an exhausted pool).

Scrubbed: paths use `/home/dev`, the email is `dev@example.com`, and the conversation id is made up.
