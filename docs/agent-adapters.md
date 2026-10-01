# Agent adapters

An **Agent adapter** teaches the Orchestrator to launch, resume and observe one kind of Agent. Adapters are compiled in: the trait lives in `crates/orch-agent/src/agent.rs`, and Claude Code (`crates/orch-agent/src/claude/`) is the only implementation today.

The Agent's own TUI always runs in the Holder's PTY (ADR 0001). The adapter never renders anything. It only builds command lines and translates the Agent's side-channel output (hooks, statusline, and files the Agent writes such as its transcript) into the normalized event vocabulary. The Daemon's status machine depends on nothing else.

## The trait

```rust
pub trait AgentAdapter {
    fn capabilities(&self) -> Capabilities;
    fn launch(&self, spec: &LaunchSpec) -> Argv;
    fn resume(&self, spec: &LaunchSpec, conversation: &ConversationId, mode: Option<PermissionMode>) -> Option<Argv>;
    fn restart(&self, spec: &LaunchSpec, conversation: Option<&ConversationId>, mode: Option<PermissionMode>) -> Argv;
    fn draft(&self, conversation: &ConversationId) -> Option<Argv>;
    fn map_hook(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError>;
    fn map_tap(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError>;
    fn title_watch(&self) -> Option<Box<dyn TitleWatch>>;
    fn is_guard_payload(&self, payload: &str) -> bool;
    fn guard_answer(&self, answer: &GuardAnswer) -> Option<String>;
}
```

Only `capabilities` and `launch` are required. Every other method has a default that matches "capability absent".

| Method | Supplies |
|---|---|
| `launch` | The argv that starts a fresh Agent in the Worktree. `LaunchSpec` carries the Session id, the absolute `orch` binary (for hook and tap commands), the effective Preset and an optional initial prompt. Claude gets `--session-id`, `--settings` JSON (hooks → `orch hook --session <id>`, `statusLine` → `orch tap --session <id>`, the Preset's allow/deny), and `--permission-mode` unless the Preset is `inherit`. |
| `resume` | The argv that continues the latest Conversation in the last observed mode. `None` means the Agent can't resume, so `restart` falls back to `launch`. |
| `draft` | A side-channel one-shot that drafts a commit message or PR title/body without adding turns to the live Conversation. Claude uses `-p --resume <conv> --fork-session`. |
| `map_hook` | Parses one hook payload, which `orch hook` forwards through the Holder, into normalized events. |
| `map_tap` | Parses one statusline payload, which `orch tap` forwards, into `UsageSample`s. |
| `title_watch` | A stateful `TitleWatch` that reports the Session title. The Daemon keeps one per Holder connection in its own task: it hands the watch every hook and statusline payload (`follow`, which only notes where to look) and calls `poll` right after each payload and about once a second, off the Daemon's state lock, so a title shows without waiting for the next hook. `poll` returns `TitleChanged` only when the title differs from the last one it reported. Claude follows `transcript_path`, reads the transcript incrementally and reports the last `custom-title` entry (`/rename`); the `session_title` field of SessionStart/UserPromptSubmit hooks only fills in while the transcript has no `custom-title` entry, and is forgotten when the transcript changes (a new Conversation). |
| `is_guard_payload` / `guard_answer` | Marks the hook payloads that block until the Daemon answers a Guard, and renders the answer in the Agent's hook-output format. |

## Capabilities and graceful degrade

`Capabilities { hooks, resume, usage, modes, guards, subagents, titles }` declares what the Agent can do. Sessions of an Agent that lacks a capability still work, with less information:

| Missing | Behaviour |
|---|---|
| `hooks` | The Session uses the unobserved status machine, so its Agent state is **Unknown** ("activity unknown"). Exited and Errored still come from the process exit. There are no Guards, because Guards arrive as hooks. |
| `resume` | `resume` returns `None`, so resume, reboot recovery and `:preset` start a fresh Conversation. |
| `usage` | The adapter emits no `UsageSample`, so there is no context gauge, token totals or rate-limit badge. |
| `modes` | The Daemon launches and resumes the Agent with the `inherit` Preset whatever was chosen (`Capabilities::effective_preset`, applied in `agent_command`), and no `ModeChanged` is reported. |
| `guards` | The Daemon lets every blocking hook proceed without evaluating it (`Capabilities::guards_available`, which needs both hooks and guards, gates the Guard path). |
| `subagents` | No `SubagentStarted`/`SubagentFinished`, so no nested Subagent rows. |
| `titles` | `title_watch` returns `None` and no `TitleChanged` is reported, so the Session keeps showing its slug. |

## Normalized event vocabulary

The events come from `orch_core::AgentEvent`:

- `SessionStarted`, `PromptSubmitted`, `TurnEnded`
- `ToolStarted { tool, subagent }`, `ToolFinished { tool, subagent }`
- `PermissionRequested`, `PermissionDenied`, `QuestionAsked`
- `Failed { kind }`
- `ModeChanged { mode }`, `ConversationChanged { id }`
- `TitleChanged { title }`, the Session title the user gave through the Agent; an empty title clears it and brings the slug back
- `UsageSample { … }`
- `SubagentStarted { id, agent_type, description }`, `SubagentFinished { id }`
- `GuardCheck { tool, input_json, cwd }`, which the Daemon answers with allow, deny or ask

The mapping from these events to Agent states (Starting, Working, Needs input, Idle, Errored, Exited) is fixed in `orch-core` and is the same for every Agent.

## Checklist: adding an Agent (e.g. Codex CLI)

1. **Research the Agent.** Find out how it starts with a given id and initial prompt, how it resumes, which modes it has, whether it offers hooks or a statusline-like side channel, and whether it supports a non-interactive one-shot for drafting.
2. **Record fixtures.** Capture real hook and side-channel payloads into `crates/orch-agent/tests/fixtures/<agent>/`, and note the Agent version you recorded them with.
3. **Implement `AgentAdapter`** in `crates/orch-agent/src/<agent>/`:
   - declare `Capabilities` honestly, since a missing capability is fine and degrades as described above;
   - write `launch`, and `resume` / `draft` if the Agent supports them;
   - write `map_hook` / `map_tap` into the vocabulary above, and map anything that doesn't fit to nothing rather than inventing events;
   - if the Agent lets the user name a Conversation, implement `title_watch`; keep file reads there, so `map_hook` / `map_tap` stay pure;
   - for Guards, implement `is_guard_payload` and `guard_answer` for the Agent's blocking-hook protocol.
4. **Never write to the user's own Agent config.** Inject everything per launch through flags or env, as the Claude adapter does with `--settings`.
5. **Test the mapping as pure functions** (Seam 3): fixture → events, and launch/resume argv for each Preset and capability combination. Test a `TitleWatch` through `follow` / `poll` against temporary transcript files. See `crates/orch-agent/tests/claude_*.rs`.
6. **Register the adapter** in `crates/orch-daemon/src/agents.rs` (`adapter_for`) under its config name, so that `[defaults.agent] name = "<agent>"` (or a Repo's `[agent]`) selects it.
7. **Run it end to end** with the scriptable fake Agent (`orch fake-agent`) through the Daemon tests, if the new adapter changes launch or event routing.
8. **Update this document** and `CONTEXT.md` if the Agent brings a concept the glossary lacks.
