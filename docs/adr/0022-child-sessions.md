# Child Sessions

Stage D of #237, settled in #242. Amends the Subagent term ("not a Session", which formerly listed "Child session" as a word to avoid).

A Subagent is lightweight and lives inside its Session: it can't ask for input, has no pane and isn't Reviewed or Landed on its own. Long or interactive work needs more. A Session can now start a **Child Session**: a full Session (pane, Phase, Needs input, notifications, Review) whose Base branch is its **Parent Session**'s Branch. A Plan Task may point at a Subagent or a Child Session.

## Decisions

- **Starting**: the parent's Agent starts one with `orch session spawn` (prompt, optional Task, optional Agent/Preset), or the user does from the TUI on the selected Session. `spawn` returns the child's id at once.
- **Finishing**: the child's Agent ends with `orch session done "<summary>"`. Idle alone doesn't count, because the Agent may just be waiting. The child is then **Done**: it stays open, and the parent is handed its summary.
- **Landing is the parent's decision.** The parent's Agent Lands a Done child with `orch session land <child>`, which squashes it onto the parent's Branch. It may first look at the child's diff, send the child more work, or Discard it with `orch session discard <child>`. The user can still Land or Discard a child from the TUI, as with any Session. If the squash conflicts or the parent's Worktree has uncommitted changes in the same files, `land` fails with the reason and the conflicting files, the child stays open marked **Land failed**, and the parent Agent can fix it and retry.
- **Waiting**: `orch session wait <ids>` blocks until the children are Done or failed and returns each summary; `orch session status` checks without blocking.
- **Sandbox**: each child has its own Worktree. Neither the child nor the parent can write to the other's Worktree.
- **Agent and Preset**: inherited from the parent unless the spawner picks others. A child never gets a looser Preset than its parent without the user's approval.
- **Parent ending**: Discarding a parent Discards all its open children, after one confirmation that lists them. Landing a parent is refused while any child is open.
- **Depth**: children may start children, up to two levels deep by default.
- **TUI**: children nest, collapsibly, under their parent in the sidebar. Each child keeps its own Needs input, Unseen marks and notifications. A collapsed parent shows the most urgent mark in its subtree. Muting a parent mutes its children.

## Considered options

- **Approval per spawn.** Safe but makes the Agent wait on the user for routine fan-out. Rejected; the Preset ceiling covers the risk.
- **The user Lands every child after Review.** Keeps a human in the loop, but blocks the parent on the user for every child. Rejected; the user can still step in.
- **Auto-Land on `done`.** Fast, but the child's own claim decides what enters the parent's Branch, and the parent can't time it around its own work in progress. Rejected for the parent deciding, since it knows its Plan and the state of its Worktree.
- **Landing on Idle.** An Idle Agent may be waiting for input. Rejected for an explicit `done`.
- **Orphan children, or a prompt per child, when the parent ends.** Orphans have no Base branch to land into; per-child prompts are tedious. Rejected.
- **Unlimited depth.** Risks runaway fan-out. Rejected for a default limit of two.

## Consequences

- `orch session spawn/done/wait/status/land/discard` reach the Daemon from inside the Sandbox, so their input is untrusted. A Session can act only on itself and its own descendants.
- The Session record gains a parent id. Reconciliation, Discarding and Landing have to walk the tree.
- A Done child that the parent never Lands stays open. It counts as open for the parent's Landing, so it can't be lost silently.
- Landing writes to the parent's Branch while the parent's Worktree may hold uncommitted work. The squash must leave that work intact or fail as Land failed.
- Port blocks, Holders and notifications scale with the tree. The depth limit bounds this.
