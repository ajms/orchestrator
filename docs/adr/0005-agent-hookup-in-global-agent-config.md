# An Agent hookup in the Agent's global config, for Agents that cannot take it per Session

For Claude Code, the Orchestrator injects its hooks, statusline and Preset rules per launch through `--settings`, and never touches `~/.claude`. Antigravity (`agy`) has no settings flag and no config-dir variable, so for Agents like it the Orchestrator installs an **Agent hookup** into the Agent's own global configuration: one named `orch` hook entry (`~/.gemini/config/hooks.json`) and a statusline tap (`~/.gemini/antigravity-cli/settings.json`). An explicit `orch agent install <agent>` writes it after showing the diff and asking. The hook works out its Session from the `ORCH_SESSION` variable the Holder already exports, and does nothing without it. The tap runs the user's previous statusline command as a passthrough. Per-Session autonomy travels through launch flags and the hook's decision, because global settings can't hold rules for one Session.

## Considered options

- **Redirecting `HOME` per Session.** This keeps auth, but it triggers agy's first-run onboarding in every Session, moves Conversations under the redirected directory, and leaks into git, gh and ssh.
- **Writing `.agents/hooks.json` into each Worktree** (git-excluded). This collides with a Repo's own committed hook file, and agy loads no workspace hooks until the Worktree is trusted, so orch would also have to write the user's `trustedWorkspaces`.

## Consequences

- Creating a Session for such an Agent is refused, with a hint, until its hookup is installed. `orch doctor` checks that it is still in place, and `orch agent uninstall <agent>` removes it and restores the previous statusline.
- A Session leaves nothing behind in Worktrees or global config, so Teardown has nothing extra to clean up.
- The hook fires for every run of the Agent on the machine. Payloads whose `--agent` doesn't match the Session's Agent are dropped. A nested run of the same Agent inside its own Session is an accepted edge case.
- orch never answers agy's workspace-trust screen, so each new Antigravity Session opens on it until the user confirms it.
