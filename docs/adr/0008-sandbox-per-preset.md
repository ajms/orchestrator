# An OS Sandbox per Preset, taking over from the WriteOutsideWorktree Guard

A Preset can run its Agent in the Sandbox (`sandbox = true`; built-in `auto` has it on). The Sandbox is bubblewrap, Linux only, wrapped around the Agent's argv on the Holder's PTY, so it covers everything the Agent runs, including Subagents that orch can't see. It limits file writes. The network stays open, because Agents call their model API from their own process, and a usable "off" needs an allowlisting proxy.

In a sandboxed Session the WriteOutsideWorktree Guard no longer asks. The Sandbox enforces writes, and its writable extras are deliberate. The git Guards (BaseBranch, OtherRef, WorktreeManagement) and ExternalTool stay, because the git common dir must be writable and the Sandbox can't tell one ref from another.

## Considered options

- **Guards only.** Guards see only the tool calls an Agent reports, and they can be argued around: shell parsing, or children that bypass the hooks, such as pi's `subagent` extension.
- **Sandbox as a Session setting separate from the Preset.** That would be a second autonomy knob beside the Preset, which already names "how free is this Agent".
- **Keeping every Guard.** A write outside the Worktree would ask first and then fail anyway, or succeed only in a writable extra the user chose on purpose.

## Consequences

- The Sandbox is fixed for the life of the Agent's process. A `:preset` switch that crosses the line takes effect at the next launch, resume or restart.
- An action blocked by the Sandbox shows only as the Agent's own tool error. orch shows that a Session is sandboxed, but it doesn't detect hits.
- Existing `auto` Sessions become sandboxed at their next spawn.
- Drafts aren't sandboxed: they run with no tools.
