# Orchestrator

A terminal UI for running and supervising many coding-agent sessions in parallel across several git repositories, each isolated on its own worktree.

## Language

**Orchestrator**:
The long-lived system that owns all Sessions and the TUI through which the user supervises them.
_Avoid_: Manager, squad, dashboard (the dashboard is one view of it)

**Repo**:
A git repository the user has registered with the Orchestrator so Sessions can be started in it.
_Avoid_: Project, workspace

**Session**:
One Agent conversation working on one Worktree on one Branch of a Repo, from creation until it is Landed or Discarded. Restarting or resuming the Agent keeps the same Session.
_Avoid_: Task, job, instance, run

**Agent**:
A coding-agent program (e.g. Claude Code) that the Orchestrator drives inside a Session.
_Avoid_: Bot, assistant, model

**Worktree**:
The git worktree dedicated to a single Session, checked out on that Session's Branch.
_Avoid_: Checkout, sandbox

**Base branch**:
The branch a Session's Branch was created from and is Landed back into.
_Avoid_: Parent, target, trunk

**Landing**:
Finishing a Session by getting its changes out of the Worktree: either a local merge into the Base branch or pushing the Branch and opening a pull request.
_Avoid_: Shipping, completing, finishing

**Discarding**:
Ending a Session by throwing its Worktree and Branch away without Landing.
_Avoid_: Cancelling, deleting, aborting
