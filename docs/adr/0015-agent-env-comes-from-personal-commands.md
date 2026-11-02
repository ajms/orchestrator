# An Agent's extra environment comes from personal commands

0013 keeps a Sandbox's environment to an allowlist, so a tool inside it that needs a secret, such as an MCP server's client secret for pi, has no way in: an `export` in `setup` dies with the setup shell, and the Daemon's own environment is cleared. An Agent can now name extra variables, each with a command that prints its value:

```toml
[defaults.agents.pi.env]
ASANA_MCP_CLIENT_SECRET = { command = "secret-tool lookup service asana-mcp" }
```

The Daemon runs each command with `sh -c` in the Worktree, outside the Sandbox, at every launch and resume, and adds its stdout without trailing newlines to that Agent's environment, sandboxed or not. A failing command leaves the Session Errored, naming the variable but not the output. The table is per Agent, so a Claude Session never sees pi's variables. It layers per variable like the rest of `[agents.<name>]`, with the personal override winning.

`agents.<name>.env` is not allowed in `.orchestrator.toml`: a committed command would run outside the Sandbox with the user's keyring, which no Trust prompt should grant.

## Considered options

- **Static `KEY = "value"`.** The secret would sit in plain text in the config.
- **An env prefix per adapter, like Claude's `ANTHROPIC_`.** The Daemon runs under systemd with only `PATH`, so the user would also have to push the secret into the systemd user manager.
- **Variables printed by `setup`.** Setup runs once per Worktree, so a resume would need the secret written to disk.

## Consequences

- The values travel the existing path: `orch hold --env KEY=VALUE`, then bwrap's `--setenv`. They are therefore in both processes' command lines, readable from `/proc` by other processes outside the Sandbox, though not by the sandboxed Agent, which has its own PID namespace. Passing them through a pipe and bwrap's `--args FD` would close this.
- A command that waits, e.g. for the keyring to unlock, holds the launch for up to 60 seconds.
