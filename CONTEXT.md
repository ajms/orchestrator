# Orchestrator

A terminal UI for running and supervising many coding-agent sessions in parallel across several git repositories, each isolated on its own worktree.

## Language

**Orchestrator**:
The long-lived system that owns all Sessions and the TUI through which the user supervises them.
_Avoid_: Manager, squad, dashboard (the dashboard is one view of it)

**Daemon**:
The background process that is the Orchestrator's single source of truth for Repos and Sessions, and outlives any TUI.
_Avoid_: Server, backend, supervisor

**Client**:
A TUI (or CLI command) connected to the Daemon; many can be connected at once.
_Avoid_: Frontend, viewer

**Holder**:
The process that keeps one Session's Agent and its screen alive, independent of the Daemon.
_Avoid_: Shim, wrapper, runner

**Suspended**:
The Phase of a Session whose Agent is not running (e.g. after a reboot) but can be resumed with its conversation intact.
_Avoid_: Paused, stopped, dead

**Phase**:
Where a Session is in its lifecycle: Setting up, Setup failed, Active, PR open, Suspended, Landed or Discarded.
_Avoid_: Stage, lifecycle state

**Agent state**:
What a live Session's Agent is doing right now: Starting, Working, Needs input, Idle, Errored or Exited.
_Avoid_: Activity, run state

**Needs input**:
The Agent state in which the Agent is blocked on the user: a permission prompt or a question it asked.
_Avoid_: Waiting, blocked, paused

**Unseen**:
A mark on a Session that finished a turn, needs input or failed while no Client was looking at it.
_Avoid_: Unread, new, dirty

**Stalled**:
A soft mark on a Working Session that has shown no activity for a configured time.
_Avoid_: Hung, frozen, stuck

**Repo**:
A git repository known to the Orchestrator, identified by the location of its main checkout; it becomes known the first time a Session is started in it.
_Avoid_: Project, workspace

**Repo config**:
The settings that apply to one Repo: those committed in the Repo itself, layered under the user's personal overrides for it.
_Avoid_: Project settings, repo file

**Trust**:
The user's approval to run the scripts a Repo config brings from the Repo itself; it lapses when those scripts change.
_Avoid_: Allow-list, permission

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
Finishing a Session by getting its changes out of the Worktree: either a squash of the Worktree's changes onto the Base branch or pushing the Branch and opening a pull request.
_Avoid_: Shipping, completing, finishing

**Setup script**:
A per-Repo command run in a new Worktree before its Agent starts.
_Avoid_: Init hook, bootstrap

**Teardown script**:
A per-Repo command run in a Worktree just before it is removed.
_Avoid_: Cleanup hook, archive script

**Insert mode**:
The input mode in which keystrokes go to the focused Session's Agent.
_Avoid_: Terminal mode, passthrough

**Normal mode**:
The input mode in which keystrokes go to the Orchestrator itself.
_Avoid_: Command mode, orchestrator mode

**Port block**:
A contiguous range of ports reserved for one live Session so parallel Sessions don't collide.
_Avoid_: Port range, port offset

**Review**:
Inspecting a Session's changes against its Base branch, usually right before Landing.
_Avoid_: Diff view, inspection

**Discarding**:
Ending a Session by throwing its Worktree and Branch away without Landing.
_Avoid_: Cancelling, deleting, aborting
