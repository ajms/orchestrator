# Subagent rows show how a Subagent ended

Amends ADR 0004.

A Subagent row used to say only whether a Subagent was running or done, so a failed, refused or merge-conflicted Subagent looked the same as one that succeeded (#238). A Subagent now has a state, Working, Done or Errored, and `SubagentFinished` carries an outcome: whether it errored, its final text, why it failed, its model, its token use and cost, and the worktree it ran in with whether its work was merged back. The Daemon adds its duration. The sidebar marks an Errored Subagent with a red `✗` and keeps it out of the collapsed "n done" row, and the selected Subagent's view shows the outcome around its transcript.

Subagents stay read-only: the outcome is observed, never acted on, and nothing is sent to a Subagent.

## Considered options

- **Read the outcome from the Subagent transcript.** The transcript holds the child's messages, but not the exit code, the worktree merge or a failure before the child started, whose transcript is empty. Rejected.
- **Forward the outcome on the tool's end** (chosen). pi's `subagent` tool returns all of it in its final result, so orch's pi extension forwards the fields on `tool_execution_end`. Claude's `SubagentStop` gives only the final text.

## Consequences

- The outcome comes from inside the Sandbox, so it is untrusted: the extension keeps only known fields of the expected type, and orch strips control characters and cuts every string before it reaches the Daemon.
- Each field is optional. Claude Subagents are never Errored, as Claude doesn't report how a Subagent ended.
- There is no Needs input state for a Subagent: neither adapter can tell which Subagent asks. pi relays a child's question through the parent's UI without naming the call.
- Nested Subagents aren't shown: pi passes no parent id to a child, and a grandchild is visible only inside its parent's result (#238).
- `SubagentView` replaces `done` with `state` and the outcome fields, so the protocol version goes up.
