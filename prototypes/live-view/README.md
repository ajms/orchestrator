# PROTOTYPE — live agent view (throwaway)

Answers [Live agent view: embed native TUI or build our own](https://github.com/ajms/orchestrator/issues/7).
Not production code. Lives only on the `prototype/live-view` branch.

```sh
cd prototypes/live-view
cargo run --release -- embed    # real Claude Code TUI in a PTY pane + hook-derived status
cargo run --release -- stream   # own UI over `claude -p` stream-json + control protocol
```

Both variants create a throwaway git repo under `$TMPDIR/orch-proto-*` and start `claude` there.
Try the same task in both, e.g. "fix the bug in fizz.py and run it".

## embed
- Left: fake Session list with status derived **only** from hooks injected via `--settings`, plus a raw hook event log.
- Right: Claude Code's own TUI (vt100 emulator + tui-term widget). All keys go to Claude.
- `F10` quit · `F7`/`F8` scroll back/forward.

## stream
- Message log rendered by us from stream-json; status from the stream; cost/tokens from `result`.
- Type a prompt + `Enter`. On a permission request: `y` allow, `n` deny.
- `Esc` interrupt · `F10` quit.
