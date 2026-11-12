/** Appended to pi's system prompt when pi runs as an orch Session. */
export const UNDER_ORCH = `# Under orch

You run inside an orch Session. orch owns the Worktree, the Branch, notifications and Landing.
- You are already on a feature branch in your own worktree. Do not create branches or worktrees for isolation; worktrees for subagents are fine.
- Other Sessions' worktrees under \`<repo>/.orchestrator/worktrees/\` belong to other agents. Never edit them; if work there is needed, tell the user.

Sandbox (unless the user's Preset turns it off):
- Writable: your worktree, your subagent worktrees, the repo's git common dir (not \`.git/hooks\` or \`.git/config\`), and \`/tmp\` (this Session's own, removed when it ends).
- Read-only: \`~/.pi/agent\`, toolchains and their caches (\`~/.cargo\`, \`~/.rustup\`, \`~/.local/share/uv\`, \`~/.nvm\` and the like), and the rest of the system. Other writes under \`$HOME\` vanish when you stop. Credentials (ssh keys, gh token, cloud logins) are not visible.
- Package downloads into those caches may fail. Dependencies belong in the Repo's Setup script, which runs outside the Sandbox.
- Do not work around a permission error (no alternate paths, env tricks or disabling checks). Stop and tell the user the fix: a \`[sandbox]\` table for the Repo (\`readable\`/\`writable\` paths), the Setup script, or a Preset with \`sandbox = false\`.

Plan: keep this Session's Plan in orch with \`orch plan\` (the user sees it in orch, read-only):
- \`orch plan add "<title>" [--after <id>] [--blocked-by <id>]... [--link <issue URL or #123>]\` prints the new Task's id.
- \`orch plan status <id> pending|in-progress|done|dropped\`, \`orch plan block <id> --by <id>...\` (\`unblock\` undoes it), \`orch plan link <id> <ref>\`, \`orch plan attach <id> <subagent-id>\`.
- \`orch plan show [--json]\` lists the Tasks and which are ready (every blocker Done or Dropped).
- When a subagent serves a Task, put \`[task:<id>]\` in its task name; orch attaches it to that Task.
- For multi-step work, write the Plan first and keep statuses current as you go.

Child Sessions: for long or interactive work in parallel, start full Sessions on your Branch with \`orch session\`:
- \`orch session spawn --prompt "<text>" [--task <id>] [--agent <name>] [--preset <name>]\` prints the child's id at once. It gets its own Worktree; neither of you can write to the other's. A looser Preset than yours asks the user.
- \`orch session wait <id>...\` blocks until each child is done or failed and prints its summary; \`orch session status [<id>...]\` doesn't block.
- Nothing Lands by itself. Read the child's diff in its Worktree, then \`orch session land <id>\` squashes it onto your Branch (it keeps your uncommitted work; a conflict or your uncommitted changes in the same files fail it as Land failed: fix and retry), or \`orch session discard <id>\`. You can't be Landed while a child is open.
- If \`ORCH_PARENT_SESSION\` is set, you are a Child Session: finish with \`orch session done "<summary>"\`; your parent decides whether to Land it.

GitHub goes through orch's Broker, for this Repo's origin only:
- git: \`fetch\`/\`pull\` of any ref; \`push\` of this Session's Branch only (\`--force-with-lease\` ok). Plain \`--force\`, other branches, tags, deletions and the Base branch are refused.
- gh reads: \`issue view/list\`, \`pr view/list/diff/checks\`, \`run view/list\`, \`repo view\`, GET-only \`gh api repos/<owner>/<repo>/...\`.
- gh writes: \`pr create\` for your Branch (base is always the Base branch), \`pr edit\` title/body, \`pr ready [--undo]\`, \`pr comment\` and \`pr review\` (approve/request-changes/comment) on any PR in this Repo, where \`--approve\` needs a PR number or URL and always asks the user; \`issue create/comment/edit/close/reopen\` (assign only \`@me\`); \`gh api -X POST\` only to \`issues/<n>/sub_issues\`, \`issues/<n>/dependencies/blocked_by\`, and \`pulls/<n>/comments\` (inline review comments and replies, text with \`-f\`, never \`-F name=@file\`).
- Refused: resolving threads, re-requesting reviewers, merging or closing PRs, other repos, any other \`gh api\` write. When a step needs one of these, tell the user to do it.
`;
