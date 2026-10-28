# orch

`orch` is a terminal UI for running and supervising many coding-agent Sessions in parallel, across several git repositories. Each Session is one Agent (Claude Code) working on its own Worktree and Branch. You see every Session in a sidebar next to the focused Agent's real, live TUI. You get notified when a Session needs you, you review the diff, and you land the work as a local squash or a PR.

Sessions outlive the TUI. A background Daemon owns all state, and one Holder process per Session keeps its Agent alive. Closing the TUI, or upgrading `orch`, never interrupts an Agent.

The domain vocabulary (Session, Repo, Worktree, Landing, Preset, Guard, Trust…) is defined in [CONTEXT.md](CONTEXT.md).

## Requirements

- Linux (other platforms are out of scope)
- git ≥ 2.40
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude` on `PATH`), set up as you normally use it; `orch` uses your own `~/.claude` configuration
- [`gh`](https://cli.github.com/), authenticated, for PR Landing and PR status
- [bubblewrap](https://github.com/containers/bubblewrap) 0.5.0 or later (`bwrap` on `PATH`) with unprivileged user namespaces, for the Sandbox. On Ubuntu 24.04 and later, `kernel.apparmor_restrict_unprivileged_userns = 1` blocks them, so add an AppArmor profile for `bwrap`. `orch doctor` checks this.
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

**Upgrading to the Sandbox.** From this release every Claude Session runs in the [Sandbox](#sandbox) unless its Preset opts out. Running Agents keep running unsandboxed; each Session is sandboxed the next time its Agent starts (resume, restart or `:preset`), with no exemption for Sessions created before the upgrade. Restart the Daemon when the TUI offers it, since the old Daemon keeps spawning unsandboxed. Then:

1. Run `orch doctor`. If the Sandbox can't run (no `bwrap`, user namespaces off, Ubuntu's AppArmor restriction), fix the host, or turn the Sandbox off for every Preset that doesn't say otherwise with `sandbox = false` under `[defaults]` in `~/.config/orchestrator/config.toml`. Until then, `orch` refuses to start sandboxed Agents rather than run them unsandboxed.
2. Expect writes outside the Worktree to fail. Most visible are tool caches in `$HOME`: `~/.cargo`, `~/.rustup`, `~/.local/share/uv`, `~/.nvm` and the other toolchain dirs are read-only, so `cargo` can't download crates and `rustup`, `uv` or `nvm` can't install anything, and other caches such as `~/.cache` or `~/.npm` start empty in the Sandbox's private `$HOME` and are thrown away when the Agent stops. Fetch dependencies in the Repo's Setup script (it runs outside the Sandbox), e.g. `cargo fetch`, `npm ci` or `uv sync`, open the caches with a [`[sandbox]` table](#sandbox), or give the Session a Preset with `sandbox = false`.
3. `git push` and `gh` keep working through the [Broker](#broker), but only for the Session's own Branch, its own PR, its Repo's issues and reads of its Repo. Your ssh keys, `gh` token and cloud credentials are not visible inside.

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

Type a Branch name into the form's Branch field to pick it yourself. A name that exists nowhere creates that Branch from the Base branch. A name that already exists, locally or on origin, starts the Session on that **Existing Branch** under its own name, with or without the `orch/` prefix, so the Agent's pushes update its PR. The Worktree's slug is the name without the prefix. A Branch only on origin is fetched, and a local one behind origin is fast-forwarded; one ahead of origin starts as it is. `orch` refuses the Branch if it has diverged from origin, origin can't be reached, it is checked out in the main checkout or any other worktree (Subagent worktrees included), another Session owns it, or it is origin's default branch, the configured `base` (without an origin, the Repo's current branch) or the new Session's own Base branch. Another Session's Base branch is fine.

When Claude runs a Subagent in its own worktree, `orch` creates that worktree at `.orchestrator/subagents/<slug>/<name>` on Branch `worktree-<name>`, starting from the Session's HEAD. Guards treat it as part of the Session, so the Subagent can edit there without a prompt. It is removed with the Session. A Subagent Branch with commits that were never merged into the Session is kept.

When the Agent is done, press `d` to review, then run `:land` to squash onto the Base branch or open a PR. If the Branch already has an open PR, `:land` follows that one instead. Landing, a merged PR and Discarding delete the Branch along with the Worktree, except an Existing Branch, which `orch` always keeps.

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
- `:d`, `:discard` (an Existing Branch is kept: only the Worktree is removed, and the preview warns only about uncommitted changes)
- `:review`
- `:resume`
- `:retry`, `:start` (after Setup failed)
- `:p <name>`, `:preset <name>`
- `:guards on|off`
- `:m`, `:mute`
- `:u`, `:usage`
- `:reconcile`
- `:sh`, `:shell`, `:!<command>` (your shell, or one command, in the Session's Worktree outside the Sandbox; `:sh` again switches back)
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

[signing]
key = "~/.ssh/orch_signing"   # signs sandboxed Agents' commits (see Broker)

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
| `sandbox` | The global switch: whether Presets that don't set `sandbox` themselves run in the Sandbox (default `true`); not allowed in `.orchestrator.toml` |
| `[sandbox]` | `readable`, `writable` and `hidden` path lists for the [Sandbox](#sandbox); they append across layers, and a layer can't hold both this table and the `sandbox` switch |
| `[agent]` | `name` (default `claude`), `binary`, `args` |
| `[presets.<name>]` | `mode`, `allow`, `deny`, `sandbox` (run the Agent in the Sandbox; default `true`) |
| `[notifications.desktop]`, `[notifications.bell]` | Per-attention toggles |

**A new Session starts from origin's Base branch.** Before cutting the Worktree, `orch` fetches the Base branch from origin and fast-forwards your local one to it. It refuses to create the Session if origin can't be reached, if your Base branch is ahead of or diverged from origin's, or if it can't be fast-forwarded because it is checked out with local changes or in another Worktree. A Base branch origin doesn't have, such as a stacked Session's, and a Repo without an origin start from the local branch.

**Setup and Teardown scripts** run with `sh -c` in the Worktree, with stdin closed. Setup output streams into the TUI; a non-zero exit puts the Session in *Setup failed*. Both scripts get these variables:

| Variable | Value |
| --- | --- |
| `ORCH_SESSION` | The Session id |
| `ORCH_WORKTREE` | Absolute path of the Worktree |
| `ORCH_PORT_BASE` | First port of the Session's block (see `ports`), e.g. `PORT=$((ORCH_PORT_BASE + 1))` |

State lives in `~/.local/state/orchestrator/state.db`. Runtime sockets live under `$XDG_RUNTIME_DIR/orchestrator`.

### Sandbox

Every Preset, built-in or your own, runs its Agent in the Sandbox unless it sets `sandbox = false` ([ADR 0008](docs/adr/0008-sandbox-on-by-default.md)). A sandboxed Agent runs as `bwrap <Sandbox args> -- claude …` (needs [bubblewrap](https://github.com/containers/bubblewrap) 0.5.0 or later), and so does everything it runs. Drafts and the Setup and Teardown scripts are not sandboxed. The setting applies the next time the Agent starts, and `:preset` restarts it. Sandboxed is the normal case, so only a Session whose Agent runs unsandboxed is marked, with `unsandboxed` in its sidebar row and pane title. While the Agent's Holder runs, the marker shows how that Agent was started; once the Holder is gone, it shows whether the Session's Preset would run it unsandboxed. A write the Sandbox blocks shows only as the Agent's own error.

Inside, `$HOME` is a private tmpfs with only an allowlist bound back ([ADR 0009](docs/adr/0009-sandbox-home-is-an-allowlist.md)), so ssh keys, the `gh` token, cloud credentials and keyrings are invisible without you listing them. The allowlist is Claude's own paths (`~/.claude`, `~/.claude.json`, and read-only `~/.local/share/claude` and `~/.local/bin`), your git config (`~/.gitconfig`, `~/.config/git`) and these toolchains, read-only: `~/.cargo`, `~/.rustup`, `~/.local/share/uv`, `~/.nvm`, `~/.pyenv`, `~/.rbenv`, `~/.volta`, `~/.bun`, `~/.deno`, `~/.local/share/mise`, `~/.local/share/fnm`, `~/.asdf`, `~/.sdkman` and `~/go`. Anything else written under `$HOME` lands in the tmpfs and is gone when the Agent stops. `~/.claude` and `~/.claude.json` stay writable, except Claude's settings, hooks, skills, agents, commands, rules, plugins and `CLAUDE.md`. The rest of the system is read-only, and other homes, `/mnt` and `/media` are hidden. The Agent can write its Worktree, its Subagent worktrees and the git common dir, but not `.git/hooks` or `.git/config`. If one of these read-only paths in `~/.claude` or `.git` doesn't exist yet, `orch` creates it empty before the Agent starts, so the Agent can't plant it. `/tmp` is the Session's own directory, removed when the Session is Landed or Discarded. The Agent starts with a cleared environment ([ADR 0013](docs/adr/0013-sandbox-environment-is-an-allowlist.md)): only `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `LANG`, `LANGUAGE`, `LC_*`, `TZ`, `COLORTERM`, the XDG base and runtime dirs, Claude's own `ANTHROPIC_*` and `CLAUDE_*`, and what `orch` sets for the Session come back, so tokens in the Daemon's environment, `SSH_AUTH_SOCK` and the display don't reach it. In a sandboxed Session the Guard for writes outside the Worktree no longer asks, and the git Guards stay. The network stays open.

**Opting out:** give a Preset `sandbox = false` and pick it with `:preset open`, or make it the default with `preset = "open"`:

```toml
[defaults.presets.open]
mode = "acceptEdits"
sandbox = false
```

In `.orchestrator.toml` that opt-out needs your Trust. `sandbox = false` under `[defaults]` in your global config is the global switch, for hosts where bubblewrap can't run: it turns the Sandbox off for every Preset that doesn't set `sandbox` itself.

**Running things yourself:** `:sh` opens your `$SHELL` in the selected Session's Worktree, shown in the Session pane in place of the Agent, with the sidebar still beside it. In Insert mode your keys go to it, as they would to the Agent. `:sh` again switches back to the Agent, and the next `:sh` back to the shell, which keeps running while you look at other Sessions. Each Session has its own, and `exit` closes it. `:!<command>` runs one command the same way and keeps its output and exit status in the pane until you press Enter or Esc; it is refused while the Session's shell runs. Shells run from the TUI, outside the Sandbox, with your own `$HOME` and credentials, while the Agent stays sandboxed. A shell ends when you quit the TUI or when its Session is suspended, Landed or Discarded. Only you can start one: the Agent has no way to ask for it ([ADR 0012](docs/adr/0012-user-shells-run-outside-the-sandbox-from-the-tui.md)). The Worktree is written by the Agent, so anything your shell or the command loads from it (a direnv `.envrc`, a mise config, a prompt that runs git) runs unsandboxed too.

**Widening and tightening:** a `[sandbox]` table opens more of `$HOME` or hides more of the system. It goes in `[defaults.sandbox]`, `[repos."<path>".sandbox]` or `.orchestrator.toml`, and the lists from every layer append. Paths are absolute or start with `~/`.

```toml
[defaults.sandbox]
readable = ["~/.m2"]                                   # bound read-only
writable = ["~/.cache", "~/.npm", "~/.cargo/registry"] # bound writable
hidden = ["~/.sdkman", "/srv/data"]                    # masked with an empty dir
```

- `hidden` beats `readable`, `writable` and the shipped toolchains. A deeper `readable` or `writable` entry wins over the one it sits in, so `writable = ["~/x"]` with `readable = ["~/x/sub"]` leaves `sub` read-only.
- Claude's own paths beat all three lists: an entry at or inside `~/.claude` is ignored, so the Agent can't be broken that way.
- A `writable` entry can be a file or a directory; a missing one is created as a directory before the Agent starts. Missing `readable` and `hidden` paths are skipped. A `hidden` path must be a directory.
- `readable` and `writable` may not name `/`, `$HOME` or a directory that contains it, `orch`'s config or state dirs (or a directory containing them), `.git/hooks`, `.git/config`, or Claude's read-only config (`settings.json`, `hooks/`, `skills/` and the rest), also when reached through a symlink. Doing so is a config error.
- In `.orchestrator.toml` each `readable` and `writable` path needs your Trust; `hidden` paths only tighten and need none. Until you approve, a sandboxed spawn is refused with the Trust prompt.
- A change applies the next time the Agent starts.

The Sandbox fails closed: before each sandboxed spawn, `orch` runs `bwrap` with the Session's Sandbox args around `true`, and if that fails, it never runs the Agent unsandboxed. A new Session, a resume and *start anyway* are refused, and a restart leaves the Agent Errored. The message names the cause (`bwrap` missing, `kernel.unprivileged_userns_clone` or `user.max_user_namespaces` at 0, or Ubuntu's AppArmor restriction) and the ways out: fix the host, set `sandbox = false` under `[defaults]` in your global config, or `:preset` to a Preset with `sandbox = false`. A Preset's own `sandbox` setting wins over `[defaults] sandbox`. `orch` also runs a nested `bwrap` inside that Sandbox; if that fails, the Agent still starts and its pane warns that Claude's own Bash sandbox won't work. If `bwrap` fails while starting the Agent, the error reads "The Sandbox failed to start: …".

### Broker

A sandboxed Agent gets none of your GitHub credentials, so `orch` fetches and pushes for it ([ADR 0010](docs/adr/0010-sandboxed-agents-reach-github-through-the-broker.md)). Inside the Sandbox, origin's URL and each `remote.origin.pushurl` the repo sets are rewritten to `orch::origin` (with `url.orch::origin.insteadOf`, so repos the Agent makes itself, such as test fixtures, reach their own remotes), which a `git-remote-orch` helper on `PATH` forwards over the Session's Holder socket to the Daemon, so the Agent keeps typing `git fetch`, `git pull` and `git push`. For a fetch, the Daemon fetches the refs the Agent asked for from origin, objects only, into the object store the Worktree shares with the main checkout, and the Agent's git then updates the refs, so your own `origin/*` refs move too. Any ref on origin can be fetched, a fetch never asks, and shallow fetches aren't supported. The Daemon pushes from the main checkout with your credentials, but only this Session's Branch, to origin under the same name. `--force-with-lease` works; a plain `--force`, other branches, tags, deletions, `--all`, `--mirror` and the Base branch are refused with a reason. It runs with `--no-verify`, `core.hooksPath=/dev/null`, and `core.sshCommand` taken from your global git config (or `ssh`), so nothing the Agent plants in the repo runs. With the `plan` and `ask` Presets (or any Preset whose `mode` is `plan` or `default`) each push asks first, like a Guard; *allow for Session* stops the asking. Every brokered fetch and push, and every refusal, shows in the TUI's status line, named after its Session when another one is shown. Unsandboxed Sessions push with your own credentials as before.

A `gh` shim on the Sandbox's `PATH` forwards `gh` the same way, for this Repo's GitHub remote only (origin, passed to `gh` as `--repo`). Reads go through: `issue view/list`, `pr view/list/diff/checks`, `run view/list`, `repo view` (of this Repo only), and GET-only `gh api` under `repos/<owner>/<repo>`. The Agent can also `pr create` for its own Branch and `pr edit` its title and body or `pr ready [--undo]` its draft state; the base is always the Session's Base branch, and `--body-file` is read inside the Sandbox. A PR the Agent creates is followed like one from `:land`: the Session moves to PR open and `orch` polls it. It can file and update issues in this Repo: `issue create` (title, body, labels), `issue comment`, `issue edit` (title, body, labels), `issue close [--comment --reason]` and `issue reopen [--comment]`, assigning or unassigning only you, as `@me`. For linking issues, `gh api -X POST` reaches exactly `repos/<owner>/<repo>/issues/<n>/sub_issues` with one numeric `sub_issue_id` and `.../issues/<n>/dependencies/blocked_by` with one numeric `issue_id`. The Daemon rebuilds every call from an allowlist of flags, so anything else (merging or closing PRs, PR comments, deleting or transferring issues, another repo, a changed base, `--web`, other `gh api` writes, `--input`, or a jq query that reads the environment) fails with a reason. PR and issue changes ask first under `plan` and `ask`, like a push, as separate Guards; reads never ask. Each call shows in the TUI's status line. `gh auth token` and `gh auth status` answer as a `gh` that isn't logged in, without reaching the Daemon.

**Signed commits:** a sandboxed Agent commits as you, with ` (orch)` after your name and your email, so its commits stand apart from yours. It never holds a signing key ([ADR 0011](docs/adr/0011-sandboxed-agents-sign-commits-through-the-broker.md)). With a key configured, git's `gpg.ssh.program` in the Sandbox is an `orch-ssh-keygen` shim, which signs over the Holder socket: the Daemon signs with `ssh-keygen -Y sign`, but only a commit whose committer is the Agent, and it never asks. Other `ssh-keygen` calls, such as verifying, go to the real one. The key file reads as empty inside the Sandbox wherever it lives. The Agent's name and email are fixed when the Agent starts, so a change to your git identity applies after a restart. Tags aren't signed, so a host `tag.gpgsign=true` makes the Agent's tags fail. Without one, the Sandbox sets `commit.gpgsign=false`, so your own `commit.gpgsign` doesn't break the Agent's commits. To have Agent commits show as Verified on GitHub:

1. Generate a dedicated key without a passphrase: `ssh-keygen -t ed25519 -N '' -C orch-agent -f ~/.ssh/orch_signing`.
2. Point `orch` at it in your global config:
   ```toml
   [signing]
   key = "~/.ssh/orch_signing"
   ```
3. Add `~/.ssh/orch_signing.pub` on GitHub under *Settings → SSH and GPG keys → New SSH key*, with key type *Signing Key*.
4. To check signatures locally, add `<your email> <contents of orch_signing.pub>` to the file your `gpg.ssh.allowedSignersFile` names.

A change applies the next time the Agent starts. Unsandboxed Sessions, Landing and snapshots commit with your own config and key as before.

### Trust

Scripts (`setup`, `teardown`), the Agent command, permission-loosening Presets (including a Preset with `sandbox = false`) and `readable` or `writable` Sandbox paths that come from a Repo's own file only run after you approve them, in the TUI's Trust prompt or with `orch trust <repo>`. If they change, you have to approve them again: the action that needs them (new Session, resume, `:preset`, Setup retry, Landing, Discarding) asks first and then continues. An untrusted Teardown can also be skipped for one Landing or Discard. Trust is stored in `orch`'s own state and never in your Claude configuration.

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
- [ADR 0008](docs/adr/0008-sandbox-on-by-default.md): every Preset runs its Agent in the Sandbox unless it opts out.
- [ADR 0009](docs/adr/0009-sandbox-home-is-an-allowlist.md): the Sandbox's `$HOME` is an allowlist.
- [ADR 0010](docs/adr/0010-sandboxed-agents-reach-github-through-the-broker.md): sandboxed Agents reach GitHub through the Broker.
- [ADR 0011](docs/adr/0011-sandboxed-agents-sign-commits-through-the-broker.md): sandboxed Agents sign commits through the Broker.
- [ADR 0012](docs/adr/0012-user-shells-run-outside-the-sandbox-from-the-tui.md): user Shells run outside the Sandbox, from the TUI.
- [ADR 0013](docs/adr/0013-sandbox-environment-is-an-allowlist.md): the Sandbox's environment is an allowlist.
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
