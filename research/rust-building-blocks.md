# Rust building blocks for an agent-hosting TUI

Research for [#4](https://github.com/ajms/orchestrator/issues/4) (map [#1](https://github.com/ajms/orchestrator/issues/1)). Checked on 2026-09-28. Crate versions and dates are taken from the crates.io API (`https://crates.io/api/v1/crates/<name>/versions`). Repo activity comes from the GitHub API.

## TL;DR

| Concern | Recommendation | Why |
|---|---|---|
| TUI | **ratatui 0.30 + crossterm 0.29** | This is the de facto standard. Both are actively maintained. |
| PTY spawning | **portable-pty 0.9** (sync) or **pty-process 0.5** (tokio-native) | Both are mature. portable-pty releases rarely. |
| Terminal emulation in a pane | **vt100 0.16 via tui-term 0.3** to start. Keep **alacritty_terminal 0.26** as the upgrade path. | tui-term renders vt100 straight into ratatui. alacritty_terminal is more complete but has more API surface. |
| Persistence | **Own daemon** that owns the PTYs, with a Unix-socket IPC client (`interprocess`/tokio `UnixStream` + serde/prost). Alternative: **tmux as the host** via control mode. | The zellij model (daemonized server holds the PTYs) is proven. tmux gives persistence for free, but everything goes through a text protocol. |
| Git | **Shell out to `git` and `gh`** for worktree/merge/push/PR. Use **git2** or **gix** only for fast read-only queries (status, diffs). | Neither library covers the full worktree + merge + push + PR workflow as well as the CLIs do. |

## 1. TUI framework

- **ratatui**: 0.30.2 (2026-06-19). 0.30.0 (2025-12-26) split the crate into `ratatui-core` and `ratatui-widgets`, among others. About 19M recent downloads. Repo: https://github.com/ratatui/ratatui. It is immediate-mode: you redraw the whole frame each tick from app state, which fits a dashboard of Sessions well. Limitations: it has no built-in async runtime or event loop and no built-in focus or component model, so you write your own.
- **crossterm**: 0.29.0 (2025-04-05). The repo is active (last push 2026-09-14). This is ratatui's default backend. It supports an async `EventStream` (feature `event-stream`), bracketed paste, keyboard enhancement flags and mouse. https://github.com/crossterm-rs/crossterm
- Alternatives: **termion** 4.0.6 (2025-11-21, Redox GitLab) is Unix-only and a smaller project. **termwiz** 0.23.3 (2025-03-20, part of the wezterm monorepo) is also possible. Neither has an advantage over crossterm on Linux.

## 2. Spawning Agents in PTYs

- **portable-pty** 0.9.0 (2025-02-11; the previous release was 0.8.1 in 2023-03). It lives in the wezterm monorepo (https://github.com/wezterm/wezterm/tree/main/pty). The repo is active (pushed 2026-09-28), but wezterm's last tagged app release is 20240203, so crate releases are infrequent. About 9M recent downloads. The API is sync (blocking `Read`/`Write` plus a `MasterPty::resize`), so it needs a reader thread per PTY or `spawn_blocking`. It is cross-platform, including ConPTY, which Linux-only doesn't need.
- **pty-process** 0.5.3 (2025-07-12), https://git.tozt.net/pty-process, by the same author as vt100. It wraps `std`/`tokio::process::Command` and has native tokio `AsyncRead`/`AsyncWrite` on the PTY (feature `async`). It is Unix-only, which is acceptable here, and a lighter fit for a tokio daemon.
- **nix** 0.31.3 (2026-05-11) / **rustix** 1.1.5 (2026-09-16): raw `openpty`/`forkpty`/`setsid`/`TIOCSWINSZ`. This is what zellij does itself (zellij-server depends on `nix` with the `term` feature).
- **alacritty_terminal** also ships its own `tty` + `event_loop` modules that spawn a PTY and pump I/O into its `Term` (https://docs.rs/alacritty_terminal).
- **expectrl** 0.9.0 (2026-05-11) is expect-style automation on a PTY. It can be useful for tests that drive `claude` interactively.

## 3. Terminal emulation to render an Agent TUI inside a pane

Claude Code is a full-screen Ink/React TUI that uses the alternate screen, 256/truecolor, bracketed paste and mouse. The emulator has to handle these well.

- **vt100** 0.16.2 (2025-07-12; before that 0.15.2 in 2023-02). https://github.com/doy/vt100-rust has 122 stars and a single maintainer, with bursty releases. It provides `Parser::process(bytes)` → `Screen`, `Screen::cell()`, scrollback, `contents_formatted()` / `contents_diff()` (useful for re-attaching a client: send the formatted screen, then stream diffs), and a `Callbacks` trait for extra sequences (https://docs.rs/vt100). Limitations: it is a smaller feature set than a real terminal emulator (it is a parser plus a grid, with no reflow on resize). There is an active fork, **vt100-ctt** 0.17.1 (2026-02-07, https://github.com/ChrisTitusTech/vt100-rust).
- **tui-term** 0.3.4 (2026-04-07), https://github.com/a-kenji/tui-term. It is a `PseudoTerminal` ratatui widget and the README describes it as "work in progress". The only built-in backend is vt100 (default feature), but the widget is generic over a public `Screen`/`Cell` trait (`src/widget.rs`), so you can plug in another emulator by implementing that trait. The `unstable` controller feature uses portable-pty and supports only one-shot commands.
- **alacritty_terminal** 0.26.0 (2026-04-06), https://github.com/alacritty/alacritty. This is the emulator core of Alacritty, which has 65k stars and an active repo. It includes `Term`, `Grid` (with reflow and scrollback), selection, the vte parser, and its own tty/event_loop. It is the most complete and battle-tested option, used by e.g. Zed's terminal. Limitations: the API has no stability promise (breaking minor bumps), docs cover only about 63%, and you must write the ratatui adapter yourself (map `Term::renderable_content()` cells to ratatui `Buffer`, or implement tui-term's `Screen` trait).
- **wezterm-term** is **not published on crates.io** as `wezterm-term`. It is only available as a git dependency from the wezterm monorepo, or as the fork **tattoy-wezterm-term** 0.1.0-fork.5 (2025-07-11), used by tattoy/**shadow-terminal** 0.2.3 (2025-07-28). It is very complete (it underpins WezTerm), but depending on it is awkward.
- **vte** 0.15.0 (2025-02-02) is only the escape-sequence parser (the state machine), with no grid. zellij builds its own grid on top of vte.

Recommendation: start with tui-term + vt100 because it is the least code. If Claude Code rendering glitches appear (wide chars, resize, scroll regions), swap in alacritty_terminal behind tui-term's `Screen` trait.

## 4. tmux control mode as an alternative host

- tmux is actively maintained: 3.7c (2026-08-17) and 3.8-rc2 (2026-09-09), https://github.com/tmux/tmux. Protocol docs: https://github.com/tmux/tmux/wiki/Control-Mode.
- How it works: `tmux -CC attach` / `new-session`. Commands are answered inside `%begin … %end|%error` blocks. Pane output arrives as `%output %<pane> <octal-escaped bytes>`. Flow control uses `refresh-client -f pause-after=N`, which produces `%pause` / `%extended-output` and is resumed with `refresh-client -A '%pane:continue'`. `refresh-client -B` subscribes to format changes (`%subscription-changed`, at most once a second). Initial pane contents come from `capture-pane -p -e`. iTerm2 uses this protocol.
- Pros: Sessions survive the TUI closing for free, and the user can also `tmux attach` directly, which is a useful escape hatch. There is no daemon to write.
- Cons: you still need a terminal emulator on the client side (vt100/alacritty_terminal) to render `%output`. You depend on the tmux version and parse a text protocol. Output from tmux's own modes (copy mode, choose) is not sent to control clients.
- Rust support: **tmux_interface** 0.4.0 (2026-03-10, https://github.com/AntonGepting/tmux-interface-rs) wraps tmux *commands* (the CLI) but not the control-mode notification stream. I found no maintained Rust control-mode client crate, so you would write the `%`-line parser yourself (it is small).
- A simpler hybrid: run each Agent in a detached tmux session, use `tmux send-keys` / `capture-pane` for automation, and have the TUI attach to or embed the pane only when needed.

## 5. Background daemon + IPC (Session persistence)

How zellij does it (from https://github.com/zellij-org/zellij, 0.45.1, 2026-08-28):
- The first `zellij` invocation forks a **server**, daemonized with the `daemonize` crate (zellij-server Cargo.toml). The server owns the PTYs (via `nix`) and the terminal grid (its own, built on `vte`). Clients connect over a **Unix domain socket** using the `interprocess` crate, with messages encoded as protobuf via `prost` (zellij-utils `ipc.rs`, `client_server_contract`). A client closing means detach, and the server and processes keep running. Reattaching re-renders the server-side grid to the client.
- **Session resurrection** (https://zellij.dev/documentation/session-resurrection.html) is a *different* feature. The layout, pane commands and cwd (optionally with viewport/scrollback) are serialized to the cache dir every second. After a crash or reboot the **processes are not preserved**: commands are recreated behind a "Press ENTER to run" banner. For us this means re-launching `claude --resume <id>` in the same Worktree, which Claude Code supports.

Building blocks for our daemon:
- **tokio** `UnixListener`/`UnixStream`, or **interprocess** 2.4.4 (2026-09-03, actively maintained) for local sockets.
- Framing and serialization: `tokio-util` `LengthDelimitedCodec` + **serde_json** (debuggable) or **postcard** 1.1.3 (2025-07-24) or **prost**. **Avoid bincode**: 3.0.0 (2025-12-16) is a tombstone release that says "Bincode is now unmaintained" and contains only a compile error (https://docs.rs/crate/bincode/latest). **tarpc** 0.38.0 (2026-08-12) is an option if you want typed RPC. **tokio-serde** 0.9.0 dates from 2024-02 and is stale.
- Daemonization: **daemonize** 0.5.0 dates from 2023-02 and is stale but trivial. Alternatives are a manual `fork` + `setsid` via nix/rustix, or running the daemon as a **systemd user service** (socket activation). On Linux-only, systemd `--user` is arguably the cleanest way to get auto-start and restart.
- Protocol shape: control RPC (list/create/land Sessions) plus a per-Session output stream. On attach, send a snapshot (`vt100::Screen::contents_formatted()` or the rendered grid), then the live bytes or diffs. Keep the terminal-emulator state **in the daemon** so a fresh TUI can render immediately.

## 6. Git operations

- **git2** 0.21.0 (2026-05-18), https://github.com/rust-lang/git2-rs (libgit2 bindings, rust-lang org). It has worktree support (`Repository::worktree` (add), `worktrees`, `find_worktree`, `Worktree::prune`/`validate`), `merge_commits`/`merge_trees`/`merge_analysis`, and tree/index/workdir diffs, plus push via `Remote::push` (https://docs.rs/git2/latest/git2/struct.Repository.html). Limitations: it depends on C libgit2 (a build dependency, or the vendored feature). libgit2 lags git on features (e.g. no hooks, partial support for sparse-checkout and some config), and push/credentials handling is fiddly compared with the user's configured `git`/ssh-agent setup.
- **gix (gitoxide)** 0.88.0 (2026-09-25), https://github.com/GitoxideLabs/gitoxide. It is pure Rust, very active with monthly releases, and pre-1.0 with frequent breaking changes. From `crate-status.md`: it can *create* linked worktrees, but "move, remove, and repair linked worktrees" is not done. Blob/tree diff and `status` are complete. Three-way tree/commit merge is done, but "merge workflow orchestration" is not. Checkout orchestration is not done. **Push is not implemented.** It is good for fast read-only work (status, diffs, log) and not sufficient for the full workflow.
- **Shell out to `git`**: `git worktree add -b <branch> <path> <base>` / `remove` / `prune`, `git diff`, `git merge`, and `git push`. This path gives full fidelity with the user's config, hooks, credential helpers and signing. The costs are process spawn and parsing output (use porcelain/`-z` formats).
- **PRs**: shell out to **`gh pr create/view/merge`**, which reuses the user's auth. Alternatively use **octocrab** 0.54.2 (2026-09-14), an active GitHub API client, but then you have to handle tokens yourself.

Recommendation: model Git as a small trait. Use a `git`/`gh` CLI implementation for mutations (worktree add/remove, merge, push, PR). Optionally use gix for fast status/diff polling across many Worktrees.

## Sources

- crates.io API (versions and dates): https://crates.io/api/v1/crates/{ratatui,crossterm,portable-pty,pty-process,vt100,alacritty_terminal,tui-term,tmux_interface,interprocess,git2,gix,zellij,bincode,octocrab,tarpc,postcard,daemonize,termwiz,tattoy-wezterm-term}/versions
- ratatui: https://github.com/ratatui/ratatui
- crossterm: https://github.com/crossterm-rs/crossterm
- portable-pty: https://github.com/wezterm/wezterm/tree/main/pty
- pty-process: https://git.tozt.net/pty-process
- vt100: https://docs.rs/vt100, https://github.com/doy/vt100-rust
- tui-term: https://github.com/a-kenji/tui-term (README, docs/ARCHITECTURE.md, Cargo.toml, src/widget.rs)
- alacritty_terminal: https://docs.rs/alacritty_terminal
- tmux control mode: https://github.com/tmux/tmux/wiki/Control-Mode
- tmux_interface: https://github.com/AntonGepting/tmux-interface-rs
- zellij: https://github.com/zellij-org/zellij (zellij-utils/Cargo.toml, zellij-server/Cargo.toml, zellij-utils/src/ipc.rs), https://zellij.dev/documentation/session-resurrection.html
- bincode status: https://docs.rs/crate/bincode/latest
- git2: https://docs.rs/git2/latest/git2/struct.Repository.html
- gitoxide status: https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md
