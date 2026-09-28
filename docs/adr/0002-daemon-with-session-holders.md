# A daemon plus one holder process per Session, not tmux

Sessions must outlive the TUI, and embedded Agents (ADR 0001) need a process that owns their PTY. Each Agent therefore runs under its own small Holder process. The Holder owns the PTY, a terminal emulator with scrollback, and a local socket, and it buffers hook events. A single Daemon owns everything else: SQLite state (it is the only writer), hook routing, PR polling, Port blocks, and relaying screens to any number of TUI Clients over a Unix socket. Because of the Holders, the daemon can crash or be upgraded without killing Agents; only a reboot suspends them, and they come back through `claude --resume`.

## Considered options

- **tmux.** It would give persistence for free. We would still need a background process for PR polling and state, plus a control-mode parser, and we would give up control over rendering and hook routing.
- **Claude Code's `claude --bg` supervisor.** It only takes input through `claude attach` (research preview), it only runs Claude, and it creates its own worktrees, which conflicts with our Session lifecycle.
- **Daemon without Holders.** This is simpler, but every daemon crash or upgrade would interrupt running Agents.
