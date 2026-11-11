# Child Sessions

Stage D of #237, settled in #242. Amends the Subagent term ("not a Session", which formerly listed "Child session" as a word to avoid).

A Subagent is lightweight and lives inside its Session: it can't ask for input, has no pane and isn't Reviewed or Landed on its own. Long or interactive work needs more. A Session can now start a **Child Session**: a full Session (pane, Phase, Needs input, notifications, Review) whose Base branch is its **Parent Session**'s Branch. A Plan Task may point at a Subagent or a Child Session.

## Decisions

- **Starting**: the parent's Agent starts one with `orch session spawn` (prompt, optional Task, optional Agent/Preset), or the user does from the TUI on the selected Session. `spawn` returns the child's id at once.
- **Finishing and Landing**: the child's Agent ends with `orch session done "<summary>"`. Idle alone doesn't count, because the Agent may just be waiting. orch then squashes the child onto the parent's Branch automatically and hands the summary to the parent. If the squash conflicts or the parent's Worktree has uncommitted changes in the same files, the child stays open marked **Land failed**: the user is notified and the parent Agent is told, and either may fix it.
- **Waiting**: `orch session wait <ids>` blocks until the children are Done or failed and returns each summary and Land outcome; `orch session status` checks without blocking.
- **Sandbox**: each child has its own Worktree. Neither the child nor the parent can write to the other's Worktree.
- **Agent and Preset**: inherited from the parent unless the spawner picks others. A child never gets a looser Preset than its parent without the user's approval.
- **Parent ending**: Discarding a parent Discards all its open children, after one confirmation that lists them. Landing a parent is refused while any child is open.
- **Depth**: children may start children, up to two levels deep by default.
- **TUI**: children nest, collapsibly, under their parent in the sidebar. Each child keeps its own Needs input, Unseen marks and notifications. A collapsed parent shows the most urgent mark in its subtree. Muting a parent mutes its children.

## Considered options

- **Approval per spawn.** Safe but makes the Agent wait on the user for routine fan-out. Rejected; the Preset ceiling covers the risk.
- **The user Lands every child after Review.** Keeps a human in the loop, but blocks the parent on the user for every child. Rejected for auto-Land on an explicit `done`, with Land failed as the safety valve. The child's Review stays available.
- **Landing on Idle.** An Idle Agent may be waiting for input. Rejected for an explicit `done`.
- **Orphan children, or a prompt per child, when the parent ends.** Orphans have no Base branch to land into; per-child prompts are tedious. Rejected.
- **Unlimited depth.** Risks runaway fan-out. Rejected for a default limit of two.

## Consequences

- `orch session spawn/done/wait/status` reach the Daemon from inside the Sandbox, so their input is untrusted. A Session can act only on itself and its own descendants.
- The Session record gains a parent id. Reconciliation, Discarding and Landing have to walk the tree.
- Auto-Landing writes to the parent's Branch while the parent's Agent may be working in its Worktree. The squash must leave the parent's uncommitted work intact or fail as Land failed.
- Port blocks, Holders and notifications scale with the tree. The depth limit bounds this.
