Recorded with pi 1.1.0.

`hooks/` holds what orch's pi extension (`crates/orch-agent/src/pi/extension/`) sends to `orch hook --agent pi`, as real pi emitted it. Re-record after a pi update, or after changing the extension's payloads:

```sh
node crates/orch-agent/tests/fixtures/pi/record.mjs
```

`record.mjs` runs `pi --mode rpc` (`PI="node /path/to/cli.js"` picks another pi) in a throwaway `HOME` and Agent profile, so it never reads or writes `~/.pi`. It loads the shipped extension with `--orch-bin` pointed at `capture.sh`, and `scripted.ts`, which adds:

- the `orch-scripted` model, pi's own faux provider answering by prompt (`reply`, `tool`, `question`, `slow`, `fail <errorMessage>`), so no credentials or network are needed;
- a `question` tool that asks through `ctx.ui.select`, like the user's own;
- `/orch-dialog`, a dialog opened while idle, and `/orch-unname`, which clears the session name.

It drives startup, a prompt, a tool call, a question, an idle dialog, an abort, five provider errors with auto-retry off, a rename, a cleared name, `/new` and a resume of the named Conversation. The error texts are scripted to look like Anthropic's.

Starting has no payload: the Daemon shows it from the launch until the first `session_start`.
