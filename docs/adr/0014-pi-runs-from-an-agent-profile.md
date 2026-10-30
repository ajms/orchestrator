# pi runs from an Agent profile built from the user's own setup

pi reads all of its user-level state from one agent dir, `~/.pi/agent` unless `PI_CODING_AGENT_DIR` moves it. orch's pi Sessions run from an **Agent profile** instead: one shared agent dir in orch's state dir (`$XDG_STATE_HOME/orchestrator/agent-profiles/pi`), which orch rebuilds from `~/.pi/agent` before every launch. orch only reads `~/.pi`, and never writes to it.

- `settings.json` is the user's, without `sessionDir`. It lists `~/.pi/agent/{extensions,skills,prompts,themes}` by absolute path, so the user's resources load without being copied. The user's own relative entries are resolved against `~/.pi/agent`.
- Four extensions are left out with exact `-<abs>/extensions/<name>/index.ts` entries, because pi ignores directory and glob exclusions there:
  - `git-worktree-isolation` and `sandbox`, because orch owns the Worktree and the Sandbox;
  - `notify`, because orch owns notifications;
  - `caffeinate`, which is macOS-only.
- `SYSTEM.md`, `AGENTS.md`, `keybindings.json` and `agents/` are links to the user's.
- `auth.json` and `models-store.json` are links to the user's, so there is no second login.
- `sessions/` and the `search` extension's `index/` are kept across launches. Everything else is rebuilt, so whatever an Agent changed (e.g. `settings.json`, `trust.json`, a planted extension) is reset. Files and links are staged and renamed into place, so a pi Session that is already running isn't disturbed.

pi is launched with `-na` (a Repo's committed `.pi/` is ignored) and `PI_OFFLINE=1`.

## Considered options

- **Run pi on `~/.pi/agent` itself.** orch's Conversations would mix with the user's own `pi -r` list, and the user's worktree, sandbox and notify extensions would fight orch's. Hiding them would mean writing to the user's config.
- **A profile per Session.** It costs a rebuild per Session for no gain, since nothing in the profile is per Session. `--session-id` already keeps Conversations apart.
- **Build the profile once.** The user's changes, e.g. a new extension from chezmoi, would never reach it. Rebuilding at every launch costs a few file writes.
- **Copy the credentials, or pass provider keys through the env.** A copy goes stale on the next OAuth refresh, and the keys live in `auth.json` anyway.

## Consequences

- pi locks `auth.json` next to the profile's link, not next to the real file. The user's own pi and orch's pi can therefore refresh an Anthropic OAuth token at the same time, and the loser has to log in again. This is accepted.
- The Sandbox (0009, 0013) binds the profile and the two credential files writable, and `~/.pi/agent`'s installation and resources read-only. The user's own `sessions/` stay hidden. The profile's `extensions/`, `skills/`, `prompts/` and `themes/` are read-only, so an Agent can't plant a resource that loads in later Sessions.
- The Agent can read the user's pi credentials, as Claude can read its own in `~/.claude`.
- Extensions that hard-code `~/.pi/agent` (the user's `sandbox` hook, `statusline`) see the user's files, not the profile. Their stand-down under `ORCH_SESSION` is dotfiles work.
