# Agent adapters

An **Agent adapter** teaches the Orchestrator to launch, resume and observe one kind of Agent. Adapters are compiled in: the trait lives in `crates/orch-agent/src/agent.rs`, and there are two implementations: Claude Code (`crates/orch-agent/src/claude/`) and pi (`crates/orch-agent/src/pi/`).

The Agent's own TUI always runs in the Holder's PTY (ADR 0001). The adapter never renders anything. It only builds command lines and translates the Agent's side-channel output (hooks, statusline, and files the Agent writes such as its transcript) into the normalized event vocabulary. The Daemon's status machine depends on nothing else.

## The trait

```rust
pub trait AgentAdapter {
    fn capabilities(&self) -> Capabilities;
    fn launch(&self, spec: &LaunchSpec) -> Argv;
    fn launch_conversation(&self, spec: &LaunchSpec) -> Option<ConversationId>;
    fn resume(&self, spec: &LaunchSpec, conversation: &ConversationId, mode: Option<PermissionMode>) -> Option<Argv>;
    fn restart(&self, spec: &LaunchSpec, conversation: Option<&ConversationId>, mode: Option<PermissionMode>) -> Argv;
    fn draft(&self, conversation: &ConversationId) -> Option<Argv>;
    fn map_hook(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError>;
    fn map_tap(&self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError>;
    fn title_watch(&self) -> Option<Box<dyn TitleWatch>>;
    fn subagent_transcripts(&self) -> Option<Box<dyn SubagentTranscripts>>;
    fn is_guard_payload(&self, payload: &str) -> bool;
    fn guard_answer(&self, answer: &GuardAnswer) -> Option<String>;
    fn worktree_request(&self, payload: &str) -> Option<WorktreeRequest>;
    fn user_statusline_command(&self, cwd: &Path, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String>;
    fn agent_dirs(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf>;
    fn prepare(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Result<(), String>;
    fn env(&self, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)>;
    fn sandbox_needs(&self, lookup: &dyn Fn(&str) -> Option<String>) -> SandboxNeeds;
}
```

Only `capabilities` and `launch` are required. Every other method has a default that matches "capability absent".

| Method | Supplies |
|---|---|
| `launch` | The argv that starts a fresh Agent in the Worktree. `LaunchSpec` carries the Session id, the absolute `orch` binary (for hook and tap commands), the effective Preset, an optional initial prompt and the Session's Worktree. Claude gets `--session-id`, `--settings` JSON (hooks → `orch hook --session <id>`, `WorktreeCreate`/`WorktreeRemove` → `orch worktree-hook --worktree <worktree>`, `statusLine` → `orch tap --session <id>`, the Preset's allow/deny), and `--permission-mode` unless the Preset is `inherit`. |
| `launch_conversation` | The Conversation a fresh launch opens, when the Agent lets orch name it. The Daemon records it as soon as the Holder runs, so an Agent without hooks still resumes the same Conversation. The initial prompt is kept, unlike after a Conversation reported by a hook, so the default Draft of such an Agent still starts from it. Claude names none, because its SessionStart hook reports it. |
| `resume` | The argv that continues the latest Conversation in the last observed mode. `None` means the Agent can't resume, so `restart` falls back to `launch`. |
| `draft` | A side-channel one-shot that drafts a commit message or PR title/body without adding turns to the live Conversation. Claude uses `-p --resume <conv> --fork-session`. |
| `map_hook` | Parses one hook payload, which `orch hook` forwards through the Holder, into normalized events. |
| `map_tap` | Parses one statusline payload, which `orch tap` forwards, into `UsageSample`s. |
| `title_watch` | A stateful `TitleWatch` that reports the Session title. The Daemon keeps one per Holder connection in its own task: it hands the watch every hook and statusline payload (`follow`, which only notes where to look) and calls `poll` right after each payload and about once a second, off the Daemon's state lock, so a title shows without waiting for the next hook. `poll` returns `TitleChanged` only when the title differs from the last one it reported. Claude follows `transcript_path`, reads the transcript incrementally and reports the last `custom-title` entry (`/rename`); the `session_title` field of SessionStart/UserPromptSubmit hooks only fills in while the transcript has no `custom-title` entry, and is forgotten when the transcript changes (a new Conversation). |
| `subagent_transcripts` | A stateful `SubagentTranscripts` per Session that finds each Subagent's transcript file. The Daemon hands it every hook payload (`follow`) and asks it for a Subagent's file (`locate`) and for a `TranscriptReader` per subscribing Client. `read` returns the entries added since the last read, and `reset` when it started over because the file changed or shrank. Claude derives `<transcript dir>/<conversation>/subagents/agent-<id>.jsonl` from the `transcript_path` of the Subagent's first hook (so it survives a new Conversation) and switches to `agent_transcript_path` once `SubagentStop` names it. It drops thinking, and a tool call's key argument is the first of `command`, `file_path`, `notebook_path`, `pattern`, `url`, `query`, `skill`, `description`, `prompt` in its input. |
| `is_guard_payload` / `guard_answer` | Marks the hook payloads that block until the Daemon answers a Guard, and renders the answer in the Agent's hook-output format. |
| `worktree_request` | Recognises the hook payloads in which the Agent asks for a Subagent worktree to be created (`Create { name }`) or removed (`Remove { path }`). `orch worktree-hook` answers them itself, outside the event vocabulary: it makes the worktree under `.orchestrator/subagents/<slug>/` and prints its path, so Guards count it as part of the Session. Claude sends `name` in `WorktreeCreate` and `worktree_path` in `WorktreeRemove`. |
| `user_statusline_command` | The user's own statusline command for the Worktree, which `orch tap` runs on the payload and prints; without one it prints the model and context gauge. Claude reads it from its settings files. |
| `agent_dirs` | Directories the Agent keeps its own state in (Claude: `projects` and `plans` under its config dir). Guards let the Agent write there, as they do in the Session's Subagent worktrees. |
| `prepare` | Runs before every launch, resume and restart, before the Sandbox is set up. An error leaves the Session Errored with it. Claude needs nothing; pi rebuilds its Agent profile. |
| `env` | Variables every launch of the Agent sets, sandboxed or not. The Daemon passes them to the Holder. |
| `sandbox_needs` | What the Agent needs from the Sandbox, as `SandboxNeeds { writable, read_only, carve_outs, env }`. The Agent-agnostic `sandbox_args` (`crates/orch-agent/src/sandbox.rs`) turns these and the Session's paths into bwrap args. `writable` paths are bound back read-write, `read_only` paths read-only, and `carve_outs` are read-only again inside a writable path. Missing paths are skipped, except carve-outs, which `orch` creates empty first so the Agent can't create them. These paths beat the user's `[sandbox]` config: a configured `readable`, `writable` or `hidden` entry at or inside one of them is ignored, and they come back inside a hidden directory. Claude: `~/.claude` (or `$CLAUDE_CONFIG_DIR`) and `~/.claude.json` writable; `settings.json`, `settings.local.json`, `hooks/`, `skills/`, `agents/`, `commands/`, `rules/`, `plugins/` and `CLAUDE.md` in it carved out; `~/.local/share/claude` and `~/.local/bin` read-only; `DISABLE_AUTOUPDATER=1`. The default needs nothing, so such an Agent sees only the shipped allowlist. |

## Choosing a Session's Agent

A Session's Agent is stored on the Session when it is created (`sessions.agent`) and never changes (ADR 0006). A Session stored before that column existed has `NULL` there, which means `claude`.

- **Config.** `[agent] name = "<agent>"` picks a Repo's default Agent (`[defaults.agent] name` the global default, `claude` if unset). A committed `name` needs no Trust, because it only picks a compiled-in adapter. Each Agent's `binary` and `args` live in its own table, `[agents.<name>]` (e.g. `[agents.claude]`), and a committed `binary` or `args` needs Trust. `RepoConfig::default_agent` reads the name and `RepoConfig::agent(name)` one Agent's binary and args.
- **Resolving.** Launch, resume, Draft and the launch checks resolve the adapter with `adapter_named` (`crates/orch-agent/src/registry.rs`) from the **Session's** Agent, and the binary and args from `[agents.<that name>]`, read again each time. Changing the Repo's default only affects new Sessions. The Holder is started with `--agent` and reports it in its Hello, so a Session rebuilt from an orphaned Holder keeps its Agent (a Holder too old to report one counts as `claude`). An unknown Agent, such as one written by a newer build, leaves the Session Errored with the reason; a missing binary does too. orch never falls back to Claude.
- **Routing.** `orch hook`, `orch tap` and `orch worktree-hook` take `--agent <name>` (default `claude`) and use that Agent's adapter. Claude's command lines leave it out; another adapter puts `--agent <name>` into the commands it builds. The Holder passes the name on with each payload, and the Daemon drops a hook or statusline payload whose Agent differs from the Session's (a dropped Guard is answered with ask), as it does every payload of a Session whose Agent it doesn't know.

## pi

pi (`@earendil-works/pi-coding-agent` 1.1) is registered as `pi`. Its capabilities are `hooks`, `resume` and `titles`, so its Sessions show Agent states and Session titles, but have no Guards, Preset modes, usage or Subagents.

- **Launch.** `pi -na --session-id <Session id> -e <profile>/extensions/orch/extension/index.ts --orch-bin <orch> --orch-session <Session id> [-- <prompt>]`. `-na` ignores a Repo's committed `.pi/`. `--session-id` creates the Conversation on its first message and opens it again later. `launch_conversation` names the Session id, so the Daemon records it before pi starts; pi's own `session_start` then reports the same Conversation, which changes nothing.
- **Resume.** The same, with `--session-id <latest Conversation>` and without the prompt. pi reports each `/new`, resume and fork as `ConversationChanged`, so this is the Conversation pi was last in.
- **Env.** `PI_CODING_AGENT_DIR=<profile>`, `PI_OFFLINE=1` and `PI_INDEX_ROOT=<profile>/index`.
- **Agent profile** (ADR 0014). `prepare` rebuilds `$XDG_STATE_HOME/orchestrator/agent-profiles/pi` from `~/.pi/agent` before every launch, and never writes to `~/.pi`:
  - `settings.json` is the user's without `sessionDir`. It lists `~/.pi/agent/{extensions,skills,prompts,themes}` by absolute path, with the user's relative entries resolved against `~/.pi/agent`, and excludes `git-worktree-isolation`, `sandbox`, `notify` and `caffeinate` by their exact `index.ts`.
  - `SYSTEM.md`, `AGENTS.md`, `keybindings.json`, `agents/`, `auth.json` and `models-store.json` link to the user's, if the user has them.
  - `extensions/orch/extension/` holds orch's own extension (below). pi only discovers `extensions/*.ts` and `extensions/*/index.ts`, so it loads just through `-e`, never in a Subagent child or `conversation-name`'s `pi -p`.
  - Only these generated entries and the four resource dirs are rebuilt. Everything pi writes itself (`sessions/`, `index/`, `trust.json`, logs, …) is left alone. Each entry is staged and renamed into place, and an entry in the way is renamed aside and removed. No write follows a link the Agent may have planted, and a profile that is a link is refused.
- **`sandbox_needs`**:
  - `writable`: the profile, `~/.pi/agent/auth.json` and `~/.pi/agent/models-store.json`.
  - `read_only`: `~/.pi/agent/{install,bin,extensions,skills,prompts,themes,agents,SYSTEM.md,AGENTS.md,keybindings.json,settings.json,sandbox.json}`, `~/.local/share/pi-node` and `~/.local/bin` (the `pi` launcher). The user's own `sessions/` stay hidden.
  - `carve_outs`: the profile's `extensions/`, `skills/`, `prompts/` and `themes/`.
  - `env`: the three variables above. No host `PI_*` or provider-key variables pass through.

### orch's pi extension

orch's pi extension (`crates/orch-agent/src/pi/extension/`, TypeScript) is a thin forwarder; all mapping is in `map_hook`. It is embedded with `include_str!` and uses only node built-ins and type-only pi imports, so it needs no `node_modules` at runtime.

- It registers `--orch-bin` (default `orch`) and `--orch-session`, and forwards nothing without a Session or before pi's first `session_start`.
- On each coarse event it spawns `orch hook --agent pi --session <id>` with a compact JSON payload on stdin: `event` plus the fields orch needs. It uses `node:child_process.spawn`, because `pi.exec` ignores stdin and a sync exec freezes the TUI. Spawns are chained, so payloads arrive in order, and no handler waits for one; each is killed after 10 seconds. `message_update` is never forwarded.
- pi rebuilds the extension runtime on `/new`, resume and fork, and `session_start` can fire twice, so every `session_start` reopens the state: the chain (kept if the flags are unchanged) and the last assistant outcome.
- `message_end` of an assistant message is kept, not forwarded, and `agent_settled` carries its `stopReason` and `errorMessage`. `ui_prompt_end` carries `idle`, whether pi was at its prompt.

`map_hook` (`crates/orch-agent/src/pi/hooks.rs`):

| Payload | Events | Agent state |
|---|---|---|
| `session_start` | `ConversationChanged { sessionId }` unless `reason` is `reload`, `SessionStarted`, and `TitleChanged` if it has a non-empty `name` | Idle |
| `input`, `agent_start` | `PromptSubmitted` | Working |
| `tool_call` | `ToolStarted`, or `QuestionAsked` for the `question` tool | Working, or Needs input |
| `tool_execution_end` | `ToolFinished` | Working |
| `ui_prompt_start` | `QuestionAsked` | Needs input |
| `ui_prompt_end` | `PromptSubmitted`, or `SessionStarted` when `idle` (a dialog a command opened at pi's prompt) | Working, or Idle |
| `agent_settled` | `TurnEnded`, including `aborted: true` (Escape). With `aborted: false` and `stopReason: "error"`, `Failed { kind }` instead, matched from `errorMessage`: 429 or rate limit → RateLimited, 401, 403 or auth → Authentication, overloaded or another 5xx → Server, context or length → MaxOutputTokens, otherwise `Other(errorMessage)` | Idle, or Errored |
| `session_info_changed` | `TitleChanged { name }`; an empty name clears it | — |

Starting lasts from the launch until the first `session_start`. `agent_end` isn't used, because it can fire several times per prompt. pi retries some errors silently, so a retried 429 stays Working.

**Titles.** pi's session name is the Session title: `/name`, `pi.setSessionName()` (such as the user's `conversation-name` extension) and `--name`, which only `session_start` reports. There is no `title_watch`. A title is kept across a new Conversation until it is renamed. Renames from pi's `/resume` selector emit nothing and are missed.

**Testing.** `crates/orch-agent/tests/fixtures/pi/hooks/` holds payloads recorded from real pi by `record.mjs` next to them; its README names the pi version and how to re-record. `pi_hooks.rs` maps them. The extension's `package.json` pins `@earendil-works/pi-coding-agent` for its types: `npx tsc --noEmit` and `npm test` (`node --test` against a fake ExtensionAPI) run in CI's `pi-extension` job. `ORCH_REAL_PI=1 cargo test -p orch --test daemon real_pi` launches the `pi` on PATH through the Daemon; it is skipped without either, and isn't run in CI.

## Capabilities and graceful degrade

`Capabilities { hooks, resume, usage, modes, guards, subagents, titles, transcripts }` declares what the Agent can do. Sessions of an Agent that lacks a capability still work, with less information:

| Missing | Behaviour |
|---|---|
| `hooks` | The Session uses the unobserved status machine, so its Agent state is **Unknown** ("activity unknown"). Exited and Errored still come from the process exit. There are no Guards, because Guards arrive as hooks. |
| `resume` | `resume` returns `None`, so resume, reboot recovery and `:preset` start a fresh Conversation. |
| `usage` | The adapter emits no `UsageSample`, so there is no context gauge, token totals or rate-limit badge. |
| `modes` | The Daemon launches and resumes the Agent with the `inherit` Preset whatever was chosen (`Capabilities::effective_preset`, applied in `agent_command`), and no `ModeChanged` is reported. |
| `guards` | The Daemon lets every blocking hook proceed without evaluating it (`Capabilities::guards_available`, which needs both hooks and guards, gates the Guard path). |
| `subagents` | No `SubagentStarted`/`SubagentFinished`, so no nested Subagent rows. |
| `titles` | `title_watch` returns `None` and no `TitleChanged` is reported, so the Session keeps showing its slug. |
| `transcripts` | `subagent_transcripts` returns `None`, so subscribing to a Subagent is refused and there is no Subagent transcript (ADR 0004). |

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

A Subagent transcript is a list of `orch_core::TranscriptEntry`, which the Daemon streams to subscribed Clients as is:

- `Prompt { text }`, what the Subagent was asked
- `Text { text }`, what it said
- `ToolCall { id, tool, argument }`, a tool it called and that call's key argument, if any
- `ToolResult { id, text, error }`, the full result of the call with the same `id`, and whether it failed

The mapping from these events to Agent states (Starting, Working, Needs input, Idle, Errored, Exited) is fixed in `orch-core` and is the same for every Agent.

## Checklist: adding an Agent (e.g. Codex CLI)

1. **Research the Agent.** Find out how it starts with a given id and initial prompt, how it resumes, which modes it has, whether it offers hooks or a statusline-like side channel, and whether it supports a non-interactive one-shot for drafting.
2. **Record fixtures.** Capture real hook and side-channel payloads into `crates/orch-agent/tests/fixtures/<agent>/`, and note the Agent version you recorded them with.
3. **Implement `AgentAdapter`** in `crates/orch-agent/src/<agent>/`:
   - declare `Capabilities` honestly, since a missing capability is fine and degrades as described above;
   - write `launch`, and `resume` / `draft` if the Agent supports them;
   - write `map_hook` / `map_tap` into the vocabulary above, and map anything that doesn't fit to nothing rather than inventing events;
   - if the Agent lets the user name a Conversation, implement `title_watch`; keep file reads there, so `map_hook` / `map_tap` stay pure;
   - if the Agent writes its Subagents' transcripts to files, implement `subagent_transcripts` into the entries above;
   - for Guards, implement `is_guard_payload` and `guard_answer` for the Agent's blocking-hook protocol, and `agent_dirs` for the Agent's own state;
   - if the Agent can isolate Subagents in worktrees through a hook, route that hook to `orch worktree-hook` and implement `worktree_request`.
   - implement `sandbox_needs` with the paths the Agent writes and reads in `$HOME`, the config inside them it must not change, and any env it needs in the Sandbox.
4. **Never write to the user's own Agent config.** Inject everything per launch through flags or env, as the Claude adapter does with `--settings`, or build an Agent profile in `prepare` that only reads the user's config, as pi does.
5. **Test the mapping as pure functions** (Seam 3): fixture → events, and launch/resume argv for each Preset and capability combination. Test a `TitleWatch` through `follow` / `poll` and `SubagentTranscripts` through `follow` / `locate` / `read` against temporary transcript files. See `crates/orch-agent/tests/claude_*.rs`.
6. **Register the adapter** in `adapter_named` (`crates/orch-agent/src/registry.rs`) under its config name. That one lookup serves the Daemon and `orch hook`, `orch tap` and `orch worktree-hook`, so `[defaults.agent] name = "<agent>"` (or a Repo's `[agent] name`) selects it for new Sessions and `[agents.<agent>]` sets its binary. Have its hook and statusline commands pass `--agent <agent>`.
7. **Run it end to end** with the scriptable fake Agent (`orch fake-agent`) through the Daemon tests, if the new adapter changes launch or event routing.
8. **Update this document** and `CONTEXT.md` if the Agent brings a concept the glossary lacks.
