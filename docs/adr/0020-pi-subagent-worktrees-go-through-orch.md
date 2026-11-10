# pi Subagent worktrees go through orch

Amends ADR 0019.

The user's pi `subagent` extension made each `worktree: true` Subagent a worktree under `/tmp` on a `subagent/*` branch, merged it back and removed it. orch saw only the outcome ADR 0019 forwards: the path and branch of a worktree that no longer existed, so there was nothing to review per Subagent and Guards didn't count the worktree as part of the Session (#240). pi Subagent worktrees are now Subagent worktrees. When pi runs in an orch Session, the extension asks `orch worktree-hook --agent pi` to create and remove them, as Claude's `WorktreeCreate` and `WorktreeRemove` hooks do. orch records each worktree's base when it creates it and its tip when it is removed, so `d` / `D` on the Subagent's row opens a Review of that Subagent's work alone.

Merging back stays with the Agent: the extension still merges or applies the Subagent's work into the Session's Worktree before it asks orch to remove the worktree.

## Considered options

- **orch's own worktree tool for pi.** orch's pi extension could offer a `subagent` tool of its own. It would duplicate the user's tool, with its agents, model fallback and merge logic, and drift from it. Rejected.
- **Rewrite the `/tmp` paths afterwards.** orch could move or re-point the extension's worktrees once it sees them. They live in the Session's `/tmp` inside the Sandbox, are gone by `tool_execution_end`, and orch can't reach them from the Daemon. Rejected.
- **Record base and tip in the database through the Holder socket.** `orch worktree-hook` could report them to the Daemon. That adds IPC on the hook's path, and at create time the hook doesn't know which Subagent the worktree belongs to. Rejected.
- **Record base and tip as git refs** (chosen). `orch worktree-hook` writes `refs/orch/subagents/<slug>/<name>/base` on create and `.../head` on remove, a snapshot that includes uncommitted and untracked files. The Subagent row finds them by the `worktreePath` its outcome already carries.

## Consequences

- The extension takes the orch path only when `ORCH_BIN` and `ORCH_WORKTREE` are set and its repo is the Session's Worktree; orch's pi extension sets `ORCH_BIN` at `session_start` when it has a Session. Elsewhere it keeps its `/tmp` worktrees.
- The user's extension is theirs and isn't changed by orch: `docs/patches/pi-subagent-orch-worktrees.patch` is the change to apply to it.
- A branch with unmerged commits is kept as orch's `worktree-<name>` until the Session is removed, rather than renamed to `subagent-preserved/*`.
- The refs are written from inside the Sandbox's reach (the git common dir is writable), so orch validates them as untrusted before a Review reads them. They are dropped with the Session.
- Claude Subagent worktrees get the same refs, so their rows open the same Review.
