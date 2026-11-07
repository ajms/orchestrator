# Sandboxed Agents read other repos through the Broker

This amends 0010, which kept a sandboxed Agent to its own Repo. A sandboxed Agent can now read other repos, but only those of the GitHub owners the user lists in the global config under `[broker] readable_owners`. It can never change them. The list is empty by default, so nothing beyond the Session's own Repo is reachable until the user adds owners.

- **gh:** the read-only calls the Broker already passes for this Repo (`view`/`list` reads, `pr diff`/`checks`, `run`, `repo view` and GET-only `gh api`) also pass with `--repo`, a URL or a `repos/<owner>/<repo>` endpoint of a readable owner's repo. PR and issue changes and the `api` POSTs stay limited to this Repo.
- **git:** inside the Sandbox, `https://github.com/<owner>/`, `git@github.com:<owner>/` and `ssh://git@github.com/<owner>/` of each readable owner are rewritten to the Broker's remote helper, as written in the config and in lower case. Both `insteadOf` and `pushInsteadOf` are set, so a push can't slip around the helper by a `pushInsteadOf` in the user's git config. Origin gets a `pushInsteadOf` of its own, so listing the Repo's own owner still pushes its Branch. For `git clone`, `fetch` or `pull`, the Daemon first brings a bare mirror of that repo up to date with the user's credentials: it fetches heads and tags with `--prune`, with hooks and ssh pinned. The mirror is reached the way origin is, with only the owner and name swapped. Each readable owner's directory of mirrors is bound read-only into the Sandbox, and the helper copies objects from the mirror into the Agent's own clone. A push to such a repo is always refused.

The Daemon checks the owner on every request. When an Agent starts, and on every brokered read, the Daemon removes the mirrors of owners no longer listed. A dropped owner's repos then disappear from every Sandbox, including Sandboxes already running. The rewrites and binds are set up when the Agent starts. Reads never ask, whatever the Preset.

## Considered options

- **Any repo the user's credentials can read.** This needs no config, but every private repo the user can see becomes readable by anything running in the Sandbox, with the network open.
- **Read-only gh only.** This is a smaller change, but the contents API is a poor way to read a codebase.
- **The Daemon fetches straight into the Agent's clone.** The clone lives wherever the Agent put it, often in the Sandbox's private `/tmp`, which the Daemon can't see.
- **One generic `https://github.com/` rewrite.** When two `insteadOf` rules match a URL with the same length, git uses whichever was defined first. A rule like that in the user's own git config would beat the Sandbox's rewrite. The per-owner prefixes are longer, so they win.
- **A mirror per Session.** That keeps each Session from seeing another Session's mirrors, but it costs disk and a full clone per Session. Every mirror belongs to a readable owner anyway.

## Consequences

- Mirrors live next to the Sessions' directories, under `mirrors/<owner>/<repo>.git` in orch's state. They are removed only when their owner is dropped.
- A sandboxed Session sees every mirror any Session has made for a listed owner, but none of them can be changed from inside.
- After a fetch, a mirror whose `HEAD` no longer resolves takes origin's `HEAD` again. A renamed default branch is followed, but a changed default whose old branch still exists is not.
- A fetch from a mirror is a full fetch: shallow clones (`--depth`) aren't supported.
- With an ssh origin whose key needs a touch, every brokered read of another repo asks for a touch, just like a brokered fetch of origin.
