# The Sandbox shows $HOME read-only, minus secrets

Inside the Sandbox `$HOME` is a tmpfs, and each real top-level entry of `$HOME` is bound back read-only, except a hidden list of secrets. These are writable: the Worktree, the git common dir, a per-Session `/tmp`, the Agent's own config tree (built into its adapter) and the user's `writable` extras. `.git/hooks` and `.git/config` are bound read-only and can never be made writable.

The tmpfs is there because Claude saves `~/.claude.json` with a lock dir and a temp file next to it in `$HOME`. On a read-only `$HOME` those saves are silently lost. With a tmpfs `$HOME`, the rename fails with EBUSY, and Claude's in-place fallback keeps the save.

## Considered options

- **Hide all of `$HOME` and bind back an allowlist.** This is safer, but every toolchain dir (`~/.cargo`, `~/.rustup`, `~/.local/bin`, `~/.nvm`) would need listing before a build works.
- **Bind `$HOME` read-only as a whole.** Then Claude loses its config saves without any error.
- **Leave `.git/hooks` and `.git/config` writable with the rest of the git common dir.** A hook, `core.hooksPath` or `core.fsmonitor` written there runs outside the Sandbox on the user's next git command.

## Consequences

- The hidden list (secrets, `~/.ssh`, `~/.config/gh`, orch's state dir, `$XDG_RUNTIME_DIR`) means sandboxed Agents can't push over SSH or use `gh` unless the user opts out of the shipped list.
- New top-level entries created in `$HOME` after spawn are not visible inside.
- A sandboxed Agent can't install git hooks or set local git config.
- The Claude config lock lives on the Sandbox's private tmpfs, so it doesn't exclude Claude processes outside it.
