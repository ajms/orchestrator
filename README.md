# orch

`orch` is a terminal UI for running and supervising many coding-agent Sessions in parallel, across several git repositories. Each Session is one Agent (Claude Code) working on its own Worktree and Branch. You see every Session in a sidebar next to the focused Agent's real, live TUI. You get notified when a Session needs you, you review the diff, and you land the work as a local squash or a PR.

Sessions outlive the TUI. A background Daemon owns all state, and one Holder process per Session keeps its Agent alive. Closing the TUI, or upgrading `orch`, never interrupts an Agent.

The domain vocabulary (Session, Repo, Worktree, Landing, Preset, Guard, Trust…) is defined in [CONTEXT.md](CONTEXT.md).

## Requirements

- Linux (other platforms are out of scope)
- git ≥ 2.40
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude` on `PATH`), set up as you normally use it; `orch` uses your own `~/.claude` configuration
- [`gh`](https://cli.github.com/), authenticated, for PR Landing and PR status
- [bubblewrap](https://github.com/containers/bubblewrap) 0.3.3 or later (`bwrap` on `PATH`) with unprivileged user namespaces, for the Sandbox. On Ubuntu 24.04 and later, `kernel.apparmor_restrict_unprivileged_userns = 1` blocks them, so add an AppArmor profile for `bwrap`. `orch doctor` checks this.
- a freedesktop notification daemon for desktop notifications (optional)
- Rust 1.88+ to build (development uses the version pinned in `rust-toolchain.toml`)

## Install

Install the latest Linux x86_64 build from [Releases](https://github.com/ajms/orchestrator/releases) into `~/.local/bin`:

```sh
mkdir -p ~/.local/bin
gh release download --repo ajms/orchestrator --pattern '*x86_64-unknown-linux-gnu.tar.gz' -O - \
  | tar -xz -C ~/.local/bin orch
orch --version
```

Or build from source into `~/.local/bin`:

```sh
cargo install --path crates/orch --root ~/.local   # installs ~/.local/bin/orch
```

Make sure `~/.local/bin` is on your `PATH` (e.g. `export PATH="$HOME/.local/bin:$PATH"` in your shell profile). To upgrade, run the same command again; running Agents are not interrupted.

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

When Claude runs a Subagent in its own worktree, `orch` creates that worktree at `.orchestrator/subagents/<slug>/<name>` on Branch `worktree-<name>`, starting from the Session's HEAD. Guards treat it as part of the Session, so the Subagent can edit there without a prompt. It is removed with the Session. A Subagent Branch with commits that were never merged into the Session is kept.

When the Agent is done, press `d` to review, then run `:land` to squash onto the Base branch or open a PR.

## Commands

| Command | |
|---|---|
| `orch` | TUI Client (auto-starts the Daemon) |
| `orch doctor [--json]` | Run Reconciliation and print findings per Repo with the fixes available, and check that the Sandbox runs and can nest. Exits 1 if there are findings or the Sandbox can't run; a Sandbox that can't nest is only a warning. |
| `orch repo move <old> <new>` | Tell `orch` a known Repo now lives at a new path. Its Sessions follow it, and Agents keep running if you already moved the directory. |
| `orch repo forget <path>` | Make `orch` forget a known Repo. This is refused while it has Sessions or running Agents. |
| `orch trust <repo> [--yes]` | Show what in the Repo's config needs Trust and approve it after a y/N prompt. |
| `orch daemon [--no-idle-exit]` | Run the Daemon in the foreground |
| `orch daemon install` / `uninstall` | Manage the systemd user unit |

`orch hold`, `orch hook`, `orch tap` and `orch worktree-hook` are internal plumbing. The Daemon and the Agent's hooks call them.

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

- `:n`, `:new`
- `:l`, `:land`
- `:d`, `:discard`
- `:review`
- `:resume`
- `:retry`, `:start` (after Setup failed)
- `:p <name>`, `:preset <name>`
- `:guards on|off`
- `:m`, `:mute`
- `:u`, `:usage`
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

[repos."~/src/my-repo"]       # personal overrides; these win over the Repo's file
setup = "direnv allow && make deps"
```

Shared per-Repo settings go in a committed `.orchestrator.toml` at the Repo root:

```toml
setup = "npm ci"
teardown = "docker compose down"
base = "main"
preset = "ask"

[agent]
name = "claude"

[presets.tight]
mode = "plan"
deny = ["Bash(rm *)"]
```

Precedence is: personal override > Repo file > global defaults.

Per-Repo keys (valid in `[defaults]`, `[repos."<path>"]` and `.orchestrator.toml` unless noted):

| Key | Meaning |
| --- | --- |
| `setup` | Setup script, run in each new Worktree before its Agent starts |
| `teardown` | Teardown script, run in a Worktree just before it is removed |
| `base` | Base branch for new Sessions; unset means origin's default branch, else the Repo's current branch |
| `preset` | Default Preset for new Sessions (built-in default: `edits`) |
| `review_command` | Command shown when reviewing a Session; not allowed in `.orchestrator.toml` |
| `sandbox` | Sandbox for Presets that don't set `sandbox` themselves (built-in default: `false` for now); not allowed in `.orchestrator.toml` |
| `[agent]` | `name` (default `claude`), `binary`, `args` |
| `[presets.<name>]` | `mode`, `allow`, `deny`, `sandbox` (run the Agent in the Sandbox; default `false` for now) |
| `[notifications.desktop]`, `[notifications.bell]` | Per-attention toggles |

**Setup and Teardown scripts** run with `sh -c` in the Worktree, with stdin closed. Setup output streams into the TUI; a non-zero exit puts the Session in *Setup failed*. Both scripts get these variables:

| Variable | Value |
| --- | --- |
| `ORCH_SESSION` | The Session id |
| `ORCH_WORKTREE` | Absolute path of the Worktree |
| `ORCH_PORT_BASE` | First port of the Session's block (see `ports`), e.g. `PORT=$((ORCH_PORT_BASE + 1))` |

**Sandbox:** a Preset with `sandbox = true` runs its Agent as `bwrap <policy> -- claude …` (needs [bubblewrap](https://github.com/containers/bubblewrap) 0.3.3 or later). Inside, `$HOME` is a private tmpfs with only Claude's own paths, your git config and common toolchains (`~/.cargo`, `~/.rustup`, `~/.nvm`, …) bound back read-only, so ssh keys, the `gh` token and cloud credentials are invisible. `~/.claude` and `~/.claude.json` stay writable, except Claude's settings, hooks, skills, agents, commands, rules, plugins and `CLAUDE.md`. The rest of the system is read-only, and other homes, `/mnt` and `/media` are hidden. The Agent can write its Worktree, its Subagent worktrees and the git common dir, but not `.git/hooks` or `.git/config`. `/tmp` is the Session's own directory, removed when the Session is Landed or Discarded. In a sandboxed Session the Guard for writes outside the Worktree no longer asks, and the git Guards stay.

The Sandbox fails closed: before each sandboxed spawn, `orch` runs the Session's policy around `true`, and if that fails, it never runs the Agent unsandboxed. A new Session, a resume and *start anyway* are refused, and a restart leaves the Agent Errored. The message names the cause (`bwrap` missing, `kernel.unprivileged_userns_clone` or `user.max_user_namespaces` at 0, or Ubuntu's AppArmor restriction) and the ways out: fix the host, set `sandbox = false` under `[defaults]` in your global config, or `:preset` to a Preset with `sandbox = false`. A Preset's own `sandbox` setting wins over `[defaults] sandbox`. `orch` also runs a nested `bwrap` inside the policy; if that fails, the Agent still starts and its pane warns that Claude's own Bash sandbox won't work. If `bwrap` fails while starting the Agent, the error reads "The Sandbox failed to start: …".

**Broker:** a sandboxed Agent gets none of your GitHub credentials, so `orch` pushes for it ([ADR 0010](docs/adr/0010-sandboxed-agents-reach-github-through-the-broker.md)). Inside the Sandbox, origin's push URL is `orch::origin`, which a `git-remote-orch` helper on `PATH` forwards over the Session's Holder socket to the Daemon, so the Agent keeps typing `git push`. The Daemon pushes from the main checkout with your credentials, but only this Session's Branch, to origin under the same name. `--force-with-lease` works; a plain `--force`, other branches, tags, deletions, `--all`, `--mirror` and the Base branch are refused with a reason. It runs with `--no-verify`, `core.hooksPath=/dev/null`, and `core.sshCommand` taken from your global git config (or `ssh`), so nothing the Agent plants in the repo runs. With the `plan` and `ask` Presets (or any Preset whose `mode` is `plan` or `default`) each push asks first, like a Guard; *allow for Session* stops the asking. Every brokered push, and every refusal, shows in the Session's pane. Unsandboxed Sessions push with your own credentials as before.

**Trust:** scripts (`setup`, `teardown`), the Agent command and permission-loosening Presets (including a Preset with `sandbox = false`) that come from a Repo's own file only run after you approve them, in the TUI's Trust prompt or with `orch trust <repo>`. If they change, you have to approve them again: the action that needs them (new Session, resume, `:preset`, Setup retry, Landing, Discarding) asks first and then continues. An untrusted Teardown can also be skipped for one Landing or Discard. Trust is stored in `orch`'s own state and never in your Claude configuration.

State lives in `~/.local/state/orchestrator/state.db`. Runtime sockets live under `$XDG_RUNTIME_DIR/orchestrator`.

## Development

The toolchain is pinned in `rust-toolchain.toml`. Hooks are managed with [pre-commit](https://pre-commit.com):

```sh
uv tool install pre-commit            # or: pipx install pre-commit
pre-commit install                    # fmt + clippy on commit, tests on push
pre-commit run --all-files
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` on every PR and on pushes to `main`.

Releases follow [SemVer](https://semver.org) and are driven by [release-plz](https://release-plz.dev) (`release-plz.toml`, `.github/workflows/release.yml`). All crates share `workspace.package.version`; none is published to crates.io. After every green push to `main`, release-plz opens or updates a release PR that bumps the version from the [Conventional Commits](https://www.conventionalcommits.org) since the last `v*` tag (`fix` → patch, `feat` → minor; a breaking `!` is a minor bump until 1.0, a major one after) and updates `CHANGELOG.md`. Merging that PR tags `vX.Y.Z` and publishes a GitHub Release with the changelog and a Linux x86_64 tarball. The older date-tagged releases (`2026.09.30` … `2026.10.13`) predate this scheme.

To let CI run on the release PR, add a `RELEASE_PLZ_TOKEN` secret (a fine-grained PAT with contents and pull-requests write); without it the PR is opened with `GITHUB_TOKEN`, which doesn't trigger workflows. The repository must also allow GitHub Actions to create pull requests.

## Architecture

- [ADR 0001](docs/adr/0001-embed-agent-tui.md): embed the Agent's own TUI instead of rendering the conversation.
- [ADR 0002](docs/adr/0002-daemon-with-session-holders.md): a Daemon plus one Holder per Session, not tmux.
- [ADR 0010](docs/adr/0010-sandboxed-agents-reach-github-through-the-broker.md): sandboxed Agents reach GitHub through the Broker.
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
