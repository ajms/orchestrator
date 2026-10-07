# orch

`orch` is a terminal UI for running and supervising many coding-agent Sessions in parallel, across several git repositories. Each Session is one Agent (Claude Code) working on its own Worktree and Branch. You see every Session in a sidebar next to the focused Agent's real, live TUI. You get notified when a Session needs you, you review the diff, and you land the work as a local squash or a PR.

Sessions outlive the TUI. A background Daemon owns all state, and one Holder process per Session keeps its Agent alive. Closing the TUI, or upgrading `orch`, never interrupts an Agent.

The domain vocabulary (Session, Repo, Worktree, Landing, Preset, Guard, Trust…) is defined in [CONTEXT.md](CONTEXT.md).

## Requirements

- Linux (other platforms are out of scope)
- git ≥ 2.40
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude` on `PATH`), set up as you normally use it; `orch` uses your own `~/.claude` configuration. Since Antigravity support, Claude Sessions show a few visible changes: the statusline always shows Claude's 5h/7d usage windows, `mcp__*` tools raise the new ExternalTool Guard, and the stricter shell parsing may bring new Guard prompts
- [Antigravity CLI](https://antigravity.google/) (`agy` on `PATH`), optional, for Antigravity Sessions. It can't take orch's hooks per launch, so install the **Agent hookup** once with `orch agent install antigravity` (see [Commands](#commands)); until then `orch` refuses to create Antigravity Sessions. `orch` never answers agy's workspace-trust screen for you, so agy shows it once per Worktree: answer it in the Session's pane. agy reports nothing while it waits there (its statusline only says `initializing`), so the Session shows Starting meanwhile, not Needs input. agy also reports `initializing` with no Conversation until its first turn, so a Session started without a prompt stays Starting until you send one (both found in live checks against agy 1.3.1).
- [`gh`](https://cli.github.com/), authenticated, for PR Landing and PR status
- a freedesktop notification daemon for desktop notifications (optional)
- Rust 1.88+ to build (development uses the version pinned in `rust-toolchain.toml`)

## Install

```sh
cargo install --path crates/orch      # or: cargo build --release → target/release/orch
```

Or download a Linux x86_64 build from [Releases](https://github.com/ajms/orchestrator/releases).

The Daemon starts automatically the first time you run `orch`, and exits after a few idle minutes once no Client, Holder or open PR needs it. If you'd rather have it start with your login:

```sh
orch daemon install                   # writes ~/.config/systemd/user/orch-daemon.service
systemctl --user daemon-reload
systemctl --user enable --now orch-daemon
```

The unit runs the `orch` binary you ran `install` with, so run it again after moving or reinstalling `orch`. It also captures the `PATH` of the shell you ran it from, because the systemd user manager's own `PATH` often lacks `claude`, `gh` or `git`. Re-run `install` if they move. `orch daemon uninstall` removes the unit, and its `default.target.wants` link if there is one. After an upgrade, the TUI offers to restart a Daemon that speaks an older protocol; a Daemon run by the unit is restarted with `systemctl --user restart orch-daemon.service`.

## Quick start

```sh
cd ~/src/my-repo
orch                                  # opens the TUI; this Repo is preselected
```

In the TUI, type `:new`, write a prompt (`Ctrl+g` opens `$EDITOR`, `Ctrl+r` picks another Repo), Tab to the other fields if needed and press `Ctrl+s`. `orch` then:

1. creates `.orchestrator/worktrees/<slug>` on Branch `orch/<slug>`;
2. runs the Repo's Setup script;
3. starts Claude in the pane.

When the Agent is done, press `d` to review, then run `:land` to squash onto the Base branch or open a PR.

## Commands

| Command | |
|---|---|
| `orch` | TUI Client (auto-starts the Daemon) |
| `orch agent install <agent> [--yes]` | Install the Agent hookup for an Agent that can't take orch's hooks per launch (`antigravity`). Shows the diff of the Agent's global config and asks y/N. For agy it adds an `"orch"` entry to `~/.gemini/config/hooks.json` and points `statusLine` in `~/.gemini/antigravity-cli/settings.json` at `orch tap`, saving your own statusline command in `orch`'s state, where the tap keeps running it. Outside an `orch` Session the hook lets agy decide as usual and the tap only runs your own statusline. Running it again changes nothing. |
| `orch agent uninstall <agent> [--yes]` | Remove the `"orch"` hook and restore your previous statusline. |
| `orch doctor [--json]` | Run Reconciliation and print findings per Repo with the fixes available, and check that an installed Agent hookup is still in place. Exits 1 if there are findings. |
| `orch repo move <old> <new>` | Tell `orch` a known Repo now lives at a new path. Its Sessions follow it, and Agents keep running if you already moved the directory. |
| `orch repo forget <path>` | Make `orch` forget a known Repo. This is refused while it has Sessions or running Agents. |
| `orch trust <repo> [--yes]` | Show what in the Repo's config needs Trust and approve it after a y/N prompt. |
| `orch daemon [--no-idle-exit]` | Run the Daemon in the foreground |
| `orch daemon install` / `uninstall` | Manage the systemd user unit |

`orch hold`, `orch hook` and `orch tap` are internal plumbing. The Daemon and the Agent's hooks call them. `orch hook` and `orch tap` take the Session from `--session`, or from `ORCH_SESSION` for an Agent hookup's global commands; without either they do nothing.

## Keymap

The TUI is modal, like nvim's `:terminal`. In **Insert** mode every key except `Ctrl-h` goes to the Agent, Esc included.

| Keys | Action |
|---|---|
| `Ctrl-\ Ctrl-n` | Insert → Normal mode |
| `Ctrl-h` | Insert → Normal mode, focus sidebar |
| `i` / `a` | Normal → Insert mode (focused Session) |
| `Ctrl-c` | Normal mode: send Ctrl-c to the selected Session's Agent (interrupt) |
| `j` / `k` | Sidebar: select a Session, skipping Subagent rows. Pane: scroll. |
| `J` / `K` | Step through the selected Session's Subagent rows |
| `Enter` / `l` on a Subagent | Focus its Subagent transcript (`j` / `k`, `Ctrl-d` / `Ctrl-u`, `gg` / `G` scroll it) |
| `Esc` / `h` on a Subagent | Back to its parent Session in the sidebar |
| `i` on a Subagent | Insert into its parent Session's Agent |
| `o` on a Subagent | Show full tool results in the Subagent transcript (until you leave it) |
| `Enter` on `↳ N done` | Expand / collapse that Session's finished Subagents |
| `za`, or `Enter` on a folded heading | Fold / unfold the selected Repo |
| `Ctrl-d` / `Ctrl-u`, `gg` / `G` | Scroll history |
| `Ctrl-w h` / `l` / `w` | Focus sidebar / pane / other |
| `v` / `V`, then `y` | Select and yank text |
| `d` / `D` | Built-in Review / external Review command |
| `o` | Review view: open the first hunk in `$EDITOR` |
| `:` | Command line |

The mouse works the same in Insert and Normal mode and never switches between them. When the Agent asks for the mouse (Claude Code's fullscreen renderer does), clicks, drags, the wheel and motion over the Session pane go to the Agent, so its own selection, copy, scrolling and links work as in a plain terminal. A drag that starts in the pane stays with the pane until you release, even if it wanders onto the sidebar. Hold Shift while dragging to get your terminal's native selection instead; it spans the whole window, sidebar included.

Otherwise the pane has its own selection. Drag to select; dragging past the top or bottom edge scrolls the history. The selection is copied to the clipboard and PRIMARY when you release. Double-click selects a word, triple-click a line. The wheel scrolls the history.

Hold Ctrl over a URL or a file path to underline it, and Ctrl+click to open it: URLs in your browser, files in `$EDITOR` at the line.

In the sidebar, click a Session to show it, click a Subagent to select it and show its Subagent transcript (the wheel scrolls it), click `↳ N done` to expand the finished Subagents (`Enter` collapses them), click a Repo heading to fold or unfold it, and the wheel scrolls the list.

In the Review view the wheel scrolls the column under it, a click on a file shows it, a drag in the diff selects and copies on release, and Ctrl+click on a diff line opens `$EDITOR` at that line.

Commands:

- `:new`
- `:land`
- `:discard`
- `:review`
- `:resume`
- `:retry`, `:start` (after Setup failed)
- `:preset <name>`
- `:guards on|off`
- `:mute`
- `:usage`
- `:reconcile`
- `:refresh`, `:abandon` (PRs)
- `:q`

## Configuration

Global settings live in `~/.config/orchestrator/config.toml` (`$XDG_CONFIG_HOME` is honoured). They are re-read every time they're used, so you never need a restart.

```toml
branch_prefix = "orch/"
stalled_minutes = 10
ports = { start = 20000, end = 29999, block_size = 10 }   # ORCH_PORT_BASE per Session

[notifications.desktop]
turn_ended = false            # needs_input, errored, setup_failed, checks_failing, …

[defaults]                    # defaults for every Repo
preset = "edits"
review_command = "git -p diff \"$ORCH_MERGE_BASE\" \"$ORCH_REVIEW_TREE\""

[defaults.presets.careful]
mode = "default"
deny = ["WebFetch"]

[defaults.agents.claude]      # where to find an Agent, resolved at every launch
binary = "/opt/claude/bin/claude"

[repos."~/src/my-repo"]       # personal overrides; these win over the Repo's file
setup = "direnv allow && make deps"
```

Shared per-Repo settings go in a committed `.orchestrator.toml` at the Repo root:

```toml
setup = "npm ci"
teardown = "docker compose down"
base = "main"
preset = "ask"
agent = "claude"

[presets.tight]
mode = "plan"
deny = ["Bash(rm *)"]                  # top-level rules are Claude's

[presets.tight.antigravity]            # Antigravity's rules, in agy's syntax
allow = ["command(cargo test)", "mcp(github/*)"]
deny = ["command(rm)", "write_file(.env)"]
```

Precedence is: personal override > Repo file > global defaults.

The old `[agent]` table is no longer read; `orch` reports it as a config error and shows the `agent =` / `[agents.<name>]` form to use instead. A Session whose Agent is unknown, or whose binary is missing, fails as Errored with a message; `orch` never falls back to Claude.

Per-Repo keys (valid in `[defaults]`, `[repos."<path>"]` and `.orchestrator.toml` unless noted):

| Key | Meaning |
| --- | --- |
| `setup` | Setup script, run in each new Worktree before its Agent starts |
| `teardown` | Teardown script, run in a Worktree just before it is removed |
| `base` | Base branch for new Sessions; unset means origin's default branch, else the Repo's current branch |
| `preset` | Default Preset for new Sessions (built-in default: `edits`) |
| `review_command` | Command shown when reviewing a Session; not allowed in `.orchestrator.toml` |
| `agent` | Default Agent for new Sessions (built-in default: `claude`); the `:new` form can pick another. A Session keeps its Agent for good. |
| `[agents.<name>]` | `binary` (default: the Agent's own, e.g. `claude` on `PATH`; a path with a `/` is relative to the Repo root) and `args` for that Agent, read at every launch, resume and Draft |
| `[presets.<name>]` | `mode` (`default`, `acceptEdits`, `plan`, `auto`, `dontAsk`, `bypassPermissions` or `inherit`) and Claude's `allow`/`deny` rules. Only Presets whose mode the Session's Agent can express are offered; a default Preset it can't express falls back to `edits` (or `inherit`, if the Agent can't express `edits` either), and the `:new` form says so. |
| `[presets.<name>.<agent>]` | `allow`/`deny` in that Agent's own syntax, e.g. `[presets.tight.antigravity]`. Each Agent reads only its own rules; a Preset without rules for the chosen Agent is still offered with its mode only, marked "no <agent> rules". `<agent>` must be a known Agent (`claude` or `antigravity`), and `[presets.<name>.claude]` is a config error, since Claude's rules are the top-level `allow`/`deny`.<br><br>**Antigravity:** agy's settings are global, so `orch` enforces these rules itself in its `PreToolUse` hook. A matching `deny` blocks the tool call at once (no Guard prompt), a Guard hit then asks you, a matching `allow` lets the call run without agy asking, and anything else is left to agy's own permissions. If a restarted `orch` can't load a running Session's Preset, or a tool call's target can't be read while the Preset has `deny` rules, agy is told to ask you, since those rules can't be checked. The forms: `command(<words>)` matches a command whose words start with those words, so `command(git)` covers `git status` but not `gitk`. `write_file(<path>)` covers that path and everything under it, relative to the Worktree unless absolute, and takes `*` within a name and `**` for any number of directories (`write_file(src/*.ts)`, `write_file(**/.env)`). `mcp(<server>/<tool>)` or `mcp(<server>/*)` matches agy's MCP tools. `*` alone matches everything of its kind (`command(*)`, `write_file(*)`, `mcp(*)`), except that a `write_file` allow rule (`*` and `**` included) never covers a `.git` directory, in any letter case, unless the rule names `.git` itself. Other forms, like `read_file(…)` and `read_url(…)`, are ignored.<br><br>**MCP allow rules are limited:** agy names an MCP tool `mcp_<server>_<tool>` (with `-` turned into `_`) and its hook doesn't say which server a tool belongs to, so `orch` can't tell `chrome` + `devtools_snapshot` from `chrome-devtools` + `snapshot`. An `mcp(<server>/…)` allow therefore only applies when exactly one server in agy's MCP config (`~/.gemini/config/mcp_config.json` and `~/.gemini/config/plugins/*/mcp_config.json`, under the Daemon's `HOME`) owns the name, and only to tools whose own name has no `_` or `-`; for any other tool agy asks you as usual. `mcp(*)` and `deny` rules aren't limited.<br><br>**`allow` is strict:** every part of a command line (split on `;`, `&&`, `\|\|`, `\|` and newlines) must match, and only plain words are allowed: letters, digits and `_./:=@%+,-`, a leading `~` or `~/`, and quoted text without `$`, backticks or `\` escapes. A part with anything else (braces, globs, `~user`, `!`, `$`/backtick expansions, …), a `VAR=` prefix, a wrapper (`sudo`, `env`, `nohup`, `eval`, `xargs`, `timeout`, …) or a shell keyword is never allowed. `git` is never allowed with the global options `-c`, `--config-env` or `--exec-path`, with `--output…`, as `git config`, `filter-branch` or `bundle`, `rebase -x/--exec`, `difftool -x/--extcmd`, `submodule foreach`, `archive`/`format-patch -o`, or `clone`/`fetch`/`pull`/`ls-remote`/`push` with `-u`, `--upload-pack` or `--receive-pack`. A redirect must go to a harmless file (`/dev/null`, …) or one a `write_file` allow rule covers from every directory the line has been in. A `cd` other than into one fixed directory (bare, with a flag, or into a `$`-expanded path), or any `pushd`/`popd`, refuses the whole line. **`deny` is best-effort:** a command matches if any part does, looking through `VAR=` prefixes, keywords, function bodies, wrappers and their own short and long flags (`sudo -iu root`, `sudo --user root`, `command -p`, `timeout --signal KILL 10`, …), program paths (`/bin/rm`), `git`'s global options (`-C`, `-c`, `--git-dir=`, …), line continuations, `$(…)`, backticks, `eval`, `env -S`/`--split-string`, `find -exec` and `sh -c`/`bash -c` scripts, but a determined command line can still hide a call. |
| `[notifications.desktop]`, `[notifications.bell]` | Per-attention toggles |

**Setup and Teardown scripts** run with `sh -c` in the Worktree, with stdin closed. Setup output streams into the TUI; a non-zero exit puts the Session in *Setup failed*. Both scripts get these variables:

| Variable | Value |
| --- | --- |
| `ORCH_SESSION` | The Session id |
| `ORCH_WORKTREE` | Absolute path of the Worktree |
| `ORCH_PORT_BASE` | First port of the Session's block (see `ports`), e.g. `PORT=$((ORCH_PORT_BASE + 1))` |

**Trust:** scripts (`setup`, `teardown`), an Agent's `binary` and `args` and permission-loosening Presets (a loosening mode, or `allow` rules for any Agent) that come from a Repo's own file only run after you approve them (a committed `agent =` needs no Trust, since it only picks a built-in Agent), in the TUI's Trust prompt or with `orch trust <repo>`. If they change, you have to approve them again: the action that needs them (new Session, resume, `:preset`, Setup retry, Landing, Discarding) asks first and then continues. An untrusted Teardown can also be skipped for one Landing or Discard. Trust is stored in `orch`'s own state and never in your Claude configuration.

State lives in `~/.local/state/orchestrator/state.db`. Runtime sockets live under `$XDG_RUNTIME_DIR/orchestrator`.

## Development

The toolchain is pinned in `rust-toolchain.toml`. Hooks are managed with [pre-commit](https://pre-commit.com):

```sh
uv tool install pre-commit            # or: pipx install pre-commit
pre-commit install                    # fmt + clippy on commit, tests on push
pre-commit run --all-files
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` on every PR and on pushes to `main`.

Every green push to `main` is released by `.github/workflows/release.yml`: it tags the commit with the UTC date (`2026.09.30`, then `2026.09.30.1`, `2026.09.30.2`, … for later merges that day) and publishes a GitHub Release with generated notes and a Linux x86_64 tarball. The tag is baked into `orch --version` via `ORCH_VERSION`; local builds report the Cargo version.

## Architecture

- [ADR 0001](docs/adr/0001-embed-agent-tui.md): embed the Agent's own TUI instead of rendering the conversation.
- [ADR 0002](docs/adr/0002-daemon-with-session-holders.md): a Daemon plus one Holder per Session, not tmux.
- [Agent adapters](docs/agent-adapters.md): the adapter trait, capabilities, and a checklist for adding another Agent.

The crates under `crates/`:

- `orch`: the binary
- `orch-daemon`: the Daemon
- `orch-tui`: the TUI Client
- `orch-protocol`: the Client ↔ Daemon protocol
- `orch-holder`: the Holder
- `orch-agent`: Agent adapters, Presets and Guards
- `orch-core`: the status model
- `orch-git`: git and Worktree flows
- `orch-store`: SQLite state
- `orch-config`: config and Trust
- `orch-notify`: notifications
- `orch-term`: key encoding
