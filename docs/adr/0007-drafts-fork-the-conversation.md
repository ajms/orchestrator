# Drafts fork the Conversation when the Agent can

Landing drafts a commit message or PR text with a headless one-shot of the Session's Agent. The Daemon runs it in the Worktree, outside the Sandbox, and it must never add turns to the Session's live Conversation. So a Draft forks the Conversation when the Agent can: the fork sees the whole history and the live Conversation is only read.

- **Claude** forks with `claude -p --resume <conversation> --fork-session`. It can run `git diff` itself, so it gets only the Draft instruction on stdin.
- **pi** forks with `pi -p -ne -ns -np --tools read,grep,find,ls --fork <copy> --session-dir <scratch>/sessions`, in its Agent profile (ADR 0014). pi rejects `--fork` with `--no-session`, so the fork is written to a scratch dir that the Daemon creates for each Draft and removes afterwards, whether the Draft succeeds, fails or times out. The Daemon copies the latest Conversation's session file into that dir and forks the copy. The live file is opened once, without following a link out of the profile, because the Sandbox can write the profile. The model is the one the Conversation last used. `-ne -ns -np` loads no extensions, skills or prompt templates, so the fork sends no hooks and isn't titled.
- pi runs a tool without asking, and outside the Sandbox `bash`, `edit` or `write` could do anything, so the Draft gets only read-only tools. These can't run `git`, so the Daemon puts `git status` and the diff against the Base branch (uncommitted and untracked files included) after the instruction on stdin. The adapter says so with `DraftRun::pipe_base_diff`.
- An Agent that can't fork gets no Draft from it. Landing then offers the default Draft: the Session's first prompt, or its slug.

## Considered options

- **Resuming the live Conversation headlessly.** This pollutes the Session's history and races the live TUI.
- **A fresh headless run with only the Base diff.** It works for any Agent, but the Draft loses the Conversation's intent. orch did this for Antigravity, which had no fork, until Antigravity was removed.
- **pi with all its tools in the Sandbox.** The Draft could run `git diff` itself, but it would need a second Sandbox set up per Draft for a one-shot that only reads.
- **Passing pi the live session file.** The file's path is checked before pi opens it, and the Agent could swap it in between. A copy is read once.

## Consequences

- A pi Draft's `read` can read any file the user can, because it runs outside the Sandbox. Its model only has the Session's own Conversation and the diff as context.
- A pi Draft without its Conversation's session file, or with one that is a link, falls back to the default Draft.
- The Draft subprocess has no `ORCH_SESSION`, so nothing that keys on it, such as the user's pi extensions, acts on it.
