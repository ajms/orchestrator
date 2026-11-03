# pi's mcp.json is rendered into the Agent profile

0015 gives an Agent extra variables such as `ASANA_MCP_CLIENT_SECRET`, meant for `"clientSecret": "${ASANA_MCP_CLIENT_SECRET}"` in `~/.pi/agent/mcp.json`. pi reads `mcp.json` with plain `JSON.parse` and never expands `${VAR}`, so the symlink 0014 put in the profile handed pi the literal placeholder.

At every launch and resume, after the Agent's env is resolved and before the Holder spawns, orch now writes the profile's `mcp.json` itself: the user's file parsed as JSON, with `${NAME}` (`NAME` = `[A-Za-z_][A-Za-z0-9_]*`) in every string value replaced from the Agent's env (the adapter's variables plus 0015's). An unknown variable stays literal, so pi fails as before instead of the Session erroring; the Daemon's own environment is never consulted. Without a user `mcp.json` the profile has none; an invalid one fails the launch naming its path.

## Consequences

- The secret now sits in plain text in `agent-profiles/pi/mcp.json`, mode 0600 inside the 0700 profile directory. This is the price of pi having no interpolation; drop the rendering once pi gains it.
- The profile is shared by every pi Session, so the last launch wins. Sessions whose env commands print different values race for the file; pi reads it at start-up, so a running Session is unaffected.
