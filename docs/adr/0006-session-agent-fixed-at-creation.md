# A Session's Agent is chosen at creation and never changes

Each Session stores the name of its Agent (`sessions.agent`), taken from the Repo's `[agent] name` when the Session is created (default `claude`). There is no Agent picker in the `:new` form: its summary only names the Agent that will be used. A later config change never switches an existing Session to another Agent: its Conversation, transcript, hooks and resume command all belong to one Agent, and none of them carry over to another. The binary and arguments are still resolved from the current `[agents.<name>]` config at every launch, resume and Draft, so a moved or upgraded binary just works. An unknown Agent, or one whose binary is missing, fails visibly (Errored, with a message). orch never falls back to Claude silently.

Sessions stored before the column existed have `NULL` there, which means `claude`, so they need no backfill.

## Considered options

- **Resolving the Agent from Repo config every time it is needed**, as orch did while Claude was the only Agent. Changing the Repo default would then resume existing Sessions under a different Agent that has none of their history.
- **Switching a live Session between Agents.** This is out of scope, since no Agent can resume another's Conversation.
- **An Agent picker in `:new`.** Left out: the Repo default is enough, and the summary shows which Agent a new Session gets.

## Consequences

- `orch hook`, `orch tap` and `orch worktree-hook` take `--agent <name>`, defaulting to `claude`, so they pick the adapter without a lookup on the synchronous Guard path. Claude's command lines leave it out; other Agents' adapters pass it. The Daemon drops payloads from the wrong Agent.
- A committed `[agent] name` needs no Trust, because it only picks a compiled-in adapter. A committed `[agents.<name>]` binary or args still does.
- Older builds refuse the newer schema, as with any migration.
