# User Shells run outside the Sandbox, from the TUI

The user sometimes needs to run something in a sandboxed Session's Worktree with their own credentials, such as a `gh` call the Broker refuses or a tool that writes caches in `$HOME`. `:sh` opens their `$SHELL` there, and `:!<command>` runs one command. The TUI client spawns either in a PTY of its own and draws it in the Session pane, in place of the Agent, with the sidebar still beside it. Each Session can have one Shell. It keeps running while the user looks at other Sessions. It ends when the user exits it, when the Session is suspended, Landed, Discarded or removed, or when the TUI quits.

Shells start only from the TUI's command line. They never go through the Daemon, the Holder or the Broker, so the protocol has no request a sandboxed Agent could send to start one.

## Considered options

- **Suspend the TUI and run the command fullscreen**, the way the external Review command and `$EDITOR` run. This is the simplest option, but it hides every other Session while the shell runs, and the shell can't stay open beside the Agent.
- **Let the Holder keep the Shell**, like the Agent's PTY, so it survives a TUI restart. The Holder socket is reachable from inside the Sandbox, and the Broker already listens on it. A Shell request there would be a way out of the Sandbox for the Agent.
- **Open a Shell through the Broker.** It would be an unlimited command runner with the user's credentials, which is the opposite of the Broker's allowlist.

## Consequences

- A Shell has the user's whole environment, so it is exactly as trusted as the user's own terminal. The Sandbox doesn't apply to it.
- The Worktree is written by the Agent. Anything the user's shell loads automatically from the current directory (direnv, mise, a prompt that runs git) or that the command reads from the repo runs unsandboxed. The README says so.
- Shells don't survive a TUI restart, and quitting the TUI kills them. Background jobs a Shell started can outlive it, as they would in a terminal emulator.
