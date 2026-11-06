# Scope the Claude config dir exposed to the Sandbox

A sandboxed Claude Session no longer sees the user's real `~/.claude`. orch
synthesizes a minimal config dir under
`$XDG_STATE_HOME/orchestrator/agent-profiles/claude` (falling back to
`~/.local/state`) and points `CLAUDE_CONFIG_DIR` at it, the same way pi runs
from an Agent profile (ADR 0014). This refines ADR 0009: the allowlist no longer
binds the whole `~/.claude` and `~/.claude.json` back into the Sandbox.

The profile is rebuilt before every launch, resume and restart (`prepare`):
- `.credentials.json` is symlinked (not copied) from the real `~/.claude`, so
  Claude's token refresh keeps working. The link target is the only thing from
  the real config dir marked writable in the Sandbox.
- The config carve-outs (`settings.json`, `settings.local.json`, `hooks/`,
  `skills/`, `agents/`, `commands/`, `rules/`, `plugins/` and `CLAUDE.md`) are
  symlinked into the profile (always, even when the source is still missing). The
  Sandbox resolves each link to its real `~/.claude` target and mounts only that
  target read-only — binding over the profile symlink itself is impossible
  (bwrap refuses to mount onto a symlink destination), and a missing target gets
  an empty read-only placeholder. Claude therefore reads the real carve-out
  through the link but can't plant hooks that would run in the user's other
  Claude sessions, nor create a missing `skills/`/`hooks/` under `~/.claude`.
  Only these carve-outs are exposed; the rest of `~/.claude` stays hidden.
- `projects/`, `todos/` and `shell-snapshots/` are created empty and writable,
  and kept across rebuilds so resume still finds the Session's transcripts.
- A minimal `.claude.json` is rendered into the profile (Claude reads it from
  inside `$CLAUDE_CONFIG_DIR`, not from `$HOME`). It sets
  `hasCompletedOnboarding: true`, and at launch time, once the Session's worktree
  is known (`ORCH_WORKTREE`, passed to `render_profile`), it pre-trusts that cwd
  with `projects["<cwd>"].hasTrustDialogAccepted: true`. Without the trust seed
  Claude silently ignores `permissions.allow` from settings.

The host's `projects`, history, todos, shell-snapshots, statsig and the real
`~/.claude.json` (foreign-project history and MCP server configs) never enter the
profile, so they never enter the Sandbox.

## Considered options

- **Bind the real `~/.claude` writable with settings carved out (ADR 0009's
  Claude layout).** Simple, but the Agent sees every past Session's transcripts,
  the user's history, statsig and the MCP servers and foreign-project state in
  `~/.claude.json`.
- **Copy the config into the profile.** Credentials copied this way go stale on
  token refresh; linking preserves it.
- **Seed trust from a config toggle.** Unnecessary: the restricted profile is the
  only mode, with no toggle to maintain.

## Consequences

- The cwd to pre-trust is only known at launch, so `.claude.json` is finished in
  `render_profile`, which the Daemon now calls with the Session's `ORCH_WORKTREE`
  in its env.
- Parallel launches share the one profile, so the render merges `.claude.json`
  rather than overwriting it, keeping each Session's trusted project.
- A user who runs Claude outside orch is unaffected; their real `~/.claude` is
  only ever read through the links.
