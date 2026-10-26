# The Sandbox's environment is an allowlist

A sandboxed Agent starts with a cleared environment (`bwrap --clearenv`). Only the following come back:
- a short list of harmless host variables: `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `LANG`, `LANGUAGE`, `LC_*`, `TZ`, `COLORTERM` and the XDG base and runtime dirs;
- the variables the Agent's adapter names as its own (for Claude, `ANTHROPIC_*` and `CLAUDE_*`);
- the variables orch sets for the Session: `TMPDIR`, `TERM`, the `ORCH_*` variables and the Broker's `PATH` and git config.

Tokens in the Daemon's environment (`GH_TOKEN`, `AWS_*`, `GOOGLE_APPLICATION_CREDENTIALS`, other tools' API keys) never reach the Agent, and neither do `SSH_AUTH_SOCK`, the display or the D-Bus session. Before this, the Sandbox unset only those last four, and everything else in the shell the Daemon was started from leaked in. This follows the same reasoning as 0009: whatever the Agent needs is granted deliberately.

## Considered options

- **Unset a list of known secrets.** As with `$HOME`, every new tool's variable would leak until someone listed it.
- **Keep everything except names that look secret (`*TOKEN*`, `*KEY*`).** That is guesswork in both directions.

## Consequences

- The Agent keeps its own login (`ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`) and config (`CLAUDE_CONFIG_DIR`). That exposes nothing new, because its credentials in `~/.claude` are already inside.
- A tool that relies on a variable outside the list sees it unset in the Sandbox. The user can set it in the Repo's own config files, or run the tool from a Shell (0012).
- The holder passes its own variables to `bwrap` as `--setenv`, after the clear. The Sandbox now needs bubblewrap 0.5.0 or later.
