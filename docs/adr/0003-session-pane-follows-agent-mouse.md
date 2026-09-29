# The Session pane follows the Agent's mouse request

When the Agent has requested mouse tracking (Claude Code's fullscreen renderer does), the Session pane forwards every mouse event to it, re-encoded relative to the pane in the Agent's requested protocol, and the Agent does its own selection, copy, scrolling and link handling. Only when the Agent has not requested the mouse does the Orchestrator do its own Pane selection, with the same semantics as Claude Code's (copy on release, double-click word, triple-click line, wheel scrolls scrollback). This is the model tmux and zellij use. We rejected always owning selection ourselves (disabling the Agent's mouse), because it loses Claude's transcript scrolling, Ctrl+click links and list clicks, and "feels exactly like Claude Code" was the requirement. We also rejected forcing the fullscreen renderer, because it overrides the user's own Claude configuration and leaves Agents without mouse support unhandled.

## Consequences

- The mouse is independent of Insert and Normal mode and never switches between them, with one exception: clicking a Session in the Sidebar while in Insert mode keeps Insert on the newly shown Session, or drops to Normal if it has no live Agent.
- A gesture that starts in the pane stays with the pane until release, with coordinates clamped to its edge.
- Shift-drag remains the terminal's native escape hatch and selects across the whole window; the Orchestrator does not intercept it.
- We own a mouse encoder (vt100 and crossterm have none) and must keep the Agent's copies reaching the clipboard, including OSC 52, which the emulator drops today.
- Focus-in/out is sent to the Agent when its Session is shown in a focused terminal.
