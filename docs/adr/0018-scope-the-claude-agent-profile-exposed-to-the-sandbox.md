# Claude runs from an Agent profile the Sandbox projects from the user's setup

Claude Sessions no longer run on the user's real `~/.claude`. orch builds an Agent profile in `$XDG_STATE_HOME/orchestrator/agent-profiles/claude` (falling back to `~/.local/state`) and points `CLAUDE_CONFIG_DIR` at it on every launch, sandboxed or not, the same way pi runs from one (ADR 0014). A Session can switch between sandboxed and unsandboxed Presets, so the Conversations have to stay in one place.

This refines ADR 0009: the allowlist no longer binds `~/.claude` and `~/.claude.json` into the Sandbox. The Agent no longer sees every past Session's Conversations, the user's history and statsig, or the project-scoped MCP servers and foreign-project state in `~/.claude.json`.

_Contradicts ADR 0014's rule that the profile holds its read-only resources itself, because Claude's profile can't: its carve-outs must sit at the root of `CLAUDE_CONFIG_DIR`, the root must stay writable for Claude's other state, and bwrap can't mount onto a link in it. So the Sandbox never shows the profile's root at all._

## On the host

`prepare` rebuilds the profile before every launch, resume and restart. What's on disk is what a Claude run on the host reads, such as an unsandboxed Session or a Draft:

- `settings.json`, `settings.local.json`, `hooks/`, `skills/`, `agents/`, `commands/`, `rules/`, `plugins/`, `CLAUDE.md` and `.credentials.json` are links to the user's, when those exist.
- `projects/`, `todos/`, `shell-snapshots/` and `plans/` are created and kept, so resume finds the Session's Conversations.
- `.claude.json` is rendered with only what orch owns: `hasCompletedOnboarding`, and `projects["<cwd>"].hasTrustDialogAccepted` for each Worktree. Without that trust Claude silently ignores `permissions.allow`. The user-scope `mcpServers` are copied in from the user's `.claude.json` (`$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`), so servers such as Mattermost still reach the Agent. A missing or malformed user file carries none. Anything else in the file is dropped at every render, and the Daemon doesn't follow a link there.

## In the Sandbox

The profile's path is a tmpfs, the same allowlist ADR 0009 uses for `$HOME`:

- The state dirs are bound back writable, so Conversations persist.
- The carve-outs are read-only binds from the user's own paths. A missing one is skipped, so orch writes nothing under `~/.claude`.
- `.credentials.json` is a writable bind of the user's file. A rename onto it fails with EBUSY and Claude falls back to writing in place, as it does for `~/.claude.json` under ADR 0009, so a refreshed token reaches the user's file.
- `.claude.json` is a read-only bind of the rendered file.
- Everything else Claude writes at the root lands on the tmpfs and vanishes when the Agent stops.

Nothing the Agent does reaches the profile's root on disk, so another Session can't pick up what it planted and the Daemon reads nothing the Agent could have written there. Only the state dirs are shared, and the Daemon writes into them without following a link. When `CLAUDE_CONFIG_DIR` already points outside `$HOME` (`/srv/claude`), `sandbox_needs` hides that dir too, because the root read-only bind would otherwise expose it.

The user's carve-outs stay a hard wall for `[sandbox]` config, at their real paths.

## Conversations from before the profile

They live in the real `~/.claude/projects`. On resume, a Conversation that's missing from the profile is copied in, along with its Subagent transcripts. Only that Conversation moves, so other projects' history stays out of the Sandbox. A Conversation already in the profile is never overwritten.

## Considered options

- **Bind the real `~/.claude` writable with settings carved out (ADR 0009's layout).** It exposes all of the above.
- **Keep the links in a writable profile and mount their targets read-only.** The Agent could replace a link that other Sessions read, the Daemon would have to read links the Agent can write, and orch would create placeholders under `~/.claude`.
- **A read-only profile with named writable dirs.** Claude writes many paths at the root, and the list would have to follow Claude.
- **One profile per Session.** It needs a Session id in the adapter API, and ADR 0014 rejected it for pi.
- **Copy all of `~/.claude/projects` once.** It puts every past Conversation back in the Sandbox.
- **Copy the credentials.** A copy goes stale on the next token refresh.

## Consequences

- An Agent can create a missing carve-out on its tmpfs. It's visible only to its own Session and is gone when the Agent stops.
- Claude's up-arrow history and other root state last only as long as the Agent in the Sandbox.
- Anything in a user-scope MCP server's config, secrets included, is readable by the Agent in the Sandbox.
- A user-scope server changed on the host reaches a Session at its next launch, resume or restart. `claude mcp add` run inside a Session doesn't stick.
- Claude's saves to `.claude.json` fail silently in the Sandbox (ADR 0009 records this for EROFS). Not confirmed against a live Session.
- **Open risk:** Claude 2.1.319's credential writer appears to refuse to write through a symlink, going by its code; this hasn't been run. A Claude on the host would then read the token through the profile's link but fail to save a refreshed one. This applies to unsandboxed Sessions and Drafts, not to the Sandbox. If it's confirmed, those runs need a file that isn't a link.
- A Claude run outside orch is unaffected.
