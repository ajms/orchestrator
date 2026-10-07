# Drafts come from a fresh headless run when the Agent can't fork a Conversation

Landing drafts a commit message or PR text with a headless one-shot of the Session's Agent, and this must never add turns to the Session's live Conversation. Claude Code forks the Conversation (`-p --resume <id> --fork-session`), so its Draft sees the whole history. Antigravity has no fork: `agy -p --conversation <id>` appends to the live Conversation. Its adapter therefore drafts with a fresh `agy -p` in the Worktree, and the Daemon pipes in the Draft instruction plus the diff against the Base branch (uncommitted and untracked files included). The adapter declares that its `draft` needs the diff as input.

## Considered options

- **Resuming the live Conversation headlessly.** This pollutes the Session's history and races the live TUI.
- **No Drafts for Antigravity.** Landing would work, but without the drafted text that Claude Sessions get.

## Consequences

- An Antigravity Draft is based on the diff alone, not on the Conversation's intent, so its text may be less informed than Claude's.
- The draft subprocess has no `ORCH_SESSION`, so the global Agent hookup (ADR 0005) stays inert for it.
