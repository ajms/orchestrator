Recorded with pi 1.1.0.

`hooks/` holds what orch's pi extension (`crates/orch-agent/src/pi/extension/`) sends to `orch hook --agent pi`, `taps/` what it sends to `orch tap --agent pi`, and `transcripts/` the Subagent transcript it writes to the Agent profile, as real pi emitted it. Re-record after a pi update, or after changing the extension's payloads:

```sh
(cd crates/orch-agent/src/pi/extension && npm ci)
node crates/orch-agent/tests/fixtures/pi/record.mjs
```

`record.mjs` runs `pi --mode rpc` from the extension's pinned dev dependency (`PI="node /path/to/cli.js"` picks another pi). Its `package.json` overrides pi's `@earendil-works/*` siblings to the same version, because pi's `^` ranges otherwise pull newer ones its CLI can't load. It runs in a throwaway `HOME` and Agent profile, so it never reads or writes `~/.pi`. It loads the shipped extension with `--orch-bin` pointed at `capture.sh`, and `scripted.ts`, which adds:

- the `orch-scripted` model (named `Scripted`, priced like a mid-size Anthropic model), pi's own faux provider answering by prompt (`reply`, `tool`, `subagent`, `question`, `slow`, `fail <errorMessage>`), so no credentials or network are needed;
- a `question` tool that asks through `ctx.ui.select`, like the user's own;
- `/orch-dialog`, a dialog opened while idle, and `/orch-unname`, which clears the session name.

It also loads `subagent.ts`, a stand-in for the user's `subagent` extension: a `subagent` tool (`agent`, `taskName`, `task`) that reports a child's message list in `details.messages` on each update, starts over once as a model-fallback retry would, and returns the full list.

It drives startup, a prompt, a tool call, a subagent call, a question, an idle dialog, an abort, five provider errors with auto-retry off, a rename, a cleared name, `/new` and a prompt in it, a clone (a fork at the leaf, whose copied entries keep their ids, so its tap's cost is 0) and a prompt in it, and a resume of the named Conversation. The error texts are scripted to look like Anthropic's.

Starting has no payload: the Daemon shows it from the launch until the first `session_start`.
