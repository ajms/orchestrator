# Sandboxed Agents reach GitHub through the Broker

A sandboxed Agent gets no GitHub credentials. When it runs `git fetch`, `git push` or `gh`, shims inside the Sandbox forward the call over the Session's Holder socket to the Daemon. The Daemon runs it with the user's own credentials, outside the Sandbox, and only within the Session's limits:
- **fetch:** any ref on origin, into the object store the Worktree shares with the main checkout; the Agent's own git then updates the refs;
- **push:** this Session's Branch only, to origin under the same name, with `--force-with-lease` allowed and hooks and ssh pinned;
- **pull request:** create it for the Branch and edit its title, body and draft state, with the base fixed to the Base branch;
- **issues:** create, comment on, edit, close and reopen any issue in this Repo, assigning only the user, and link sub-issues and blocking issues through exactly two `api` POST endpoints;
- **reads:** `view`/`list` reads and GET-only `api` calls under this Repo's GitHub remote.

Fetches never ask. Pushes, PR changes and issue changes follow the Preset: plan and ask prompt, and the others go through. Everything else fails with a clear message. This reuses the push and PR code Landing already runs in the Daemon.

## Considered options

- **A private ssh-agent with the account key, limited to github.com.** The key can't be read from inside, but it can still push to every repo the user can write to, and an admin bypasses the main ruleset. Claude's own Bash sandbox also blocks plain ssh, and the user's ssh aliases and known_hosts are hidden.
- **A deploy key plus a fine-grained `GH_TOKEN` per Repo.** These are limited to one Repo, but the user creates both by hand for each Repo, and the token sits in plain text in the Agent's environment while the network is open.

## Consequences

- No credential can leak from a Sandbox, and the Agent can't touch other repos or refs.
- orch now runs git on a Worktree that the Agent can write. The Broker must therefore pin `core.hooksPath`, `core.sshCommand` and `--no-verify`, and never trust the Worktree's config.
- The shims and the allowlist are orch's to keep up to date as Claude's habits with `gh` change.
