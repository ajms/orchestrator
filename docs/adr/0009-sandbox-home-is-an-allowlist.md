# The Sandbox's $HOME is an allowlist

Inside the Sandbox, `$HOME` is a private tmpfs. Only allowlisted entries are bound back from the real home, read-only unless marked writable. The rest of the system outside `$HOME` stays visible read-only. The shipped allowlist is minimal:
- Claude's own paths;
- git config;
- a set of common toolchains.

Credentials (ssh keys, the `gh` token, cloud and keyring files) are hidden simply because they aren't listed. Inside the allowlisted `~/.claude`, the files that configure Claude stay read-only (settings, hooks, skills, agents, commands, rules, plugins and `CLAUDE.md`). The rest of `~/.claude` and `~/.claude.json` are writable. That stops an Agent from planting hooks that would run unsandboxed in the user's other Claude sessions. A tmpfs `$HOME` is also the only layout in which Claude's `~/.claude.json` saves survive (its in-place EBUSY fallback).

## Considered options

- **Everything visible minus a list of hidden secrets.** This is easier to get working, but every secret a tool adds later leaks until someone lists it.
- **A read-only bind of the real `$HOME` with writable carve-outs.** Claude's `~/.claude.json` saves are then lost silently (EROFS on its lock and temp file).

## Consequences

- Tools that write caches in `$HOME` (`~/.cache`, `~/.npm`, `~/.cargo/registry`) fail until the user allows them through `[sandbox]` config.
- Stray writes elsewhere in `$HOME` land on the tmpfs and vanish when the Agent stops.
- The Agent can't push or use `gh` with the user's own credentials. Any access must be granted deliberately.
