# Patches for the user's own extensions

orch never changes the user's Agent setup (ADR 0005). These patches change it to work with orch; apply them yourself.

## `pi-subagent-orch-worktrees.patch`

Makes the pi `subagent` extension create and remove `worktree: true` Subagent worktrees through `orch worktree-hook` when pi runs in an orch Session ([ADR 0020](../adr/0020-pi-subagent-worktrees-go-through-orch.md)). Outside orch it keeps its `/tmp` worktrees.

Paths in the patch are relative to the extension's directory:

```sh
cd ~/.pi/agent/extensions/subagent
git apply --check /path/to/orch/docs/patches/pi-subagent-orch-worktrees.patch
git apply /path/to/orch/docs/patches/pi-subagent-orch-worktrees.patch
```

The patched extension:

- takes the orch path when `ORCH_BIN` and `ORCH_WORKTREE` are set and the repo's real path is the Session's Worktree;
- creates the worktree with `$ORCH_BIN worktree-hook --agent pi --worktree $ORCH_WORKTREE` and `{"event":"worktree_create","name":"<agent>-<stamp>"}` on stdin, uses the printed path, and reads the branch and base from it;
- still merges or applies the Subagent's work, then removes the worktree with one `{"event":"worktree_remove","path":…}` hook call. A failed hook is reported in the cleanup message, not thrown;
- reports unmerged commits as kept on orch's branch until the orch Session is removed;
- adds `conflictFiles` (the files a failed merge-back conflicted on) and `discarded: true` (a failed attempt discarded without merging) to `worktreeCleanup`, so orch can tell a conflict and an abandoned run from other outcomes;
- on exit or a signal, removes orch's worktrees through the hook and never deletes them itself.

If `git apply --check` fails, the extension has changed since the patch was made; apply the hunks by hand.
