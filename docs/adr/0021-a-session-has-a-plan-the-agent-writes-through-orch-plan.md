# A Session has a Plan the Agent writes through `orch plan`

Stage C of #237, settled in #241.

orch saw what a Session spawned (Subagents) but not what it meant to do. A Session now has a Plan: ordered Tasks, each with a status (Pending, In progress, Done, Dropped), the Tasks it is blocked by, optional Subagents attached to it and an optional issue link. The Daemon stores it with the Session, so it outlives Conversations and Agent restarts.

## Decisions

- **Only the Agent writes the Plan**, through an `orch plan` CLI run in the Sandbox (e.g. add, set status, block, attach, link). The user views it read-only.
- **Tasks stand alone.** A Task needs no Subagent. When the Agent spawns a Subagent it may name the Task it serves; the Task then shows that Subagent's state. Unattached Subagents show as today.
- **Dependencies, no nesting.** Tasks have "blocked by" edges, so the view can show which Tasks are ready; there are no sub-Tasks.
- **TUI**: a compact progress marker on the Session's sidebar row (e.g. `3/7`) and a full Plan view for the selected Session.
- **Issues are links only.** A Task may carry a link to an issue in any tracker (a URL or a reference such as `#241`); orch doesn't sync with GitHub or any other tracker.
- **Lifetime**: one Plan per Session, readable after Landing or Discarding until the Session record is removed.

## Considered options

- **A user-editable Plan, or one the user writes for the Agent.** Needs a way to tell the Agent about edits and to resolve conflicting writes. Rejected for now; can be added later.
- **A native tool per Agent** (a pi extension tool, a Claude Code equivalent). Nicer for the Agent but two implementations to keep in step. Rejected for the CLI, which works for every Agent.
- **Mirroring the Agent's own todo list** (Claude Code's TodoWrite, pi's todo). No effort for the Agent, but holds no dependencies or links and differs per Agent. Rejected.
- **Every Subagent creates a Task.** Turns the Plan into a log of spawned work rather than intent. Rejected.
- **Sync with GitHub sub-issues** (import from a wayfinder map, or two-way). Binds the Plan to one tracker. Rejected.

## Consequences

- "Task" is no longer an _Avoid_ term for Session; Plan and Task join CONTEXT.md.
- `orch plan` reaches the Daemon from inside the Sandbox, so its input is untrusted and scoped to the caller's own Session.
- Subagent spawns need a way to name a Task (e.g. a field the orch extension/hook forwards), per Agent adapter.
