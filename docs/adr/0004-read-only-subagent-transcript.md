# A read-only Subagent transcript beside the embedded Agent

Selecting a Subagent shows a read-only, live Subagent transcript in the pane: its prompt, assistant text and tool calls with their results. This is the one place the Orchestrator renders an Agent's conversation itself, a deliberate exception to ADR 0001. The Agent's own TUI shows a Subagent only as a collapsed summary, so there is no native view to embed. The exception is kept narrow: the view never takes input, never replaces the Session's live pane, and the Session's Conversation stays in the Agent's own TUI.

The Agent adapter finds and follows the Subagent's transcript file and maps it to Agent-neutral transcript entries, and the Daemon streams them to the Clients that subscribe. Clients never parse an Agent's own format.

## Consequences

- The transcript format is the Agent's private file layout, not a stable API, so an Agent update can break or blank the view without affecting the Session.
- An Agent without the `transcripts` capability has no Subagent transcript; its Subagent rows still show.
- Sending input to a Subagent stays out of scope.
