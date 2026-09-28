# Embed the Agent's own TUI instead of rendering our own

The live view of a Session runs the Agent's native interactive TUI (Claude Code) in a PTY and renders it through a terminal emulator inside an Orchestrator pane. The Orchestrator observes the Agent through hooks it injects per Session, and leaves the conversation to that TUI. We rejected building our own UI over Claude Code's headless stream-json and control protocol. That route gives structured data and permission control, but every tool that took it has fallen behind Claude Code's own features (slash commands, prompts, settings, MCP). A hands-on prototype of both confirmed that the embedded view behaves correctly and that hook-derived status tracks reality.

## Consequences

- Keys go to the Agent by default, so the Orchestrator is modal like nvim's `:terminal`. In Insert mode keys go to the Agent, and `Ctrl-\ Ctrl-n` switches to Normal mode, which is for the Orchestrator. Esc always reaches the Agent.
- Status, attention and usage have to come from hooks and other side channels (e.g. the statusline), because stdout carries nothing structured.
- Terminal-emulation fidelity (keys, paste, resize, scrollback) is our responsibility.
