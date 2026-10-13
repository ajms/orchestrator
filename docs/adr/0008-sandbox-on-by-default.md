# Every Preset runs its Agent in the Sandbox unless it opts out

A Preset has a `sandbox` setting that defaults to true for every Preset, built-in or user-defined. A sandboxed Agent runs inside bubblewrap. It can write only to its Session's allowed paths, can't see secrets, and keeps the network. A Preset opts out with `sandbox = false`, and in a committed `.orchestrator.toml` that opt-out loosens the Preset, so it needs Trust. In a sandboxed Session, WriteOutsideWorktree stops asking because the Sandbox enforces writes. The git Guards stay, because the git common dir must be writable. Existing Sessions pick up the Sandbox at their next spawn. Drafts and the Setup and Teardown scripts are not sandboxed.

## Considered options

- **Off by default, on for `auto` only.** Only the Preset that never asks would be walled in, but any Agent left in `edits` or `inherit` can still write anywhere a Guard misses.
- **A per-Session toggle outside the Preset.** That adds a second autonomy axis, while the Preset is already the one place autonomy is chosen, and `:preset` already relaunches the Agent.
- **Cutting the network too.** That needs an allowlisting proxy, so it is deferred.

## Consequences

- An upgrade sandboxes every Session at its next spawn. Hosts where bubblewrap can't run must fail closed (never fall back to unsandboxed) and tell the user to opt out.
- Only unsandboxed Sessions carry a marker, because sandboxed is the normal case.
- A write the Sandbox blocks shows only as the Agent's own error. orch doesn't prompt on it.
