# A Session's Agent is chosen at creation and never changes

Each Session stores the name of its Agent (`sessions.agent`), chosen in the `:new` form and defaulting to the Repo's `agent =` setting. A later config change never switches an existing Session to another Agent: its Conversation, transcript, hooks and resume command all belong to one Agent, and none of them carry over to another. The binary and arguments are still resolved from the current `[agents.<name>]` config at every launch, resume and Draft, so a moved or upgraded binary just works. An unknown Agent, or one whose binary is missing, fails visibly (Errored, with a message). orch never falls back to Claude silently.

## Considered options

- **Resolving the Agent from Repo config every time it is needed**, as orch did while Claude was the only Agent. Changing the Repo default would then resume existing Sessions under a different Agent that has none of their history.
- **Switching a live Session between Agents.** This is out of scope, since no Agent can resume another's Conversation.

## Consequences

- Hook and statusline commands carry `--agent <name>`, so `orch hook` and `orch tap` pick the adapter without a lookup on the synchronous Guard path, and the Daemon drops payloads from the wrong Agent.
- A committed `agent =` needs no Trust, because it only picks a compiled-in adapter. A committed `[agents.<name>]` binary or args still does.
