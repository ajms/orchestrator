# Running an Agent inside bubblewrap from its Holder

Research for #97, part of map #96. The question: what does it take for a Holder to launch an Agent (Claude, agy, pi) inside bubblewrap on Linux and keep it working?

## Sources

- **bwrap(1)**, bubblewrap 0.13.0 (`man bwrap` on the test host). Cited as *man*.
- **Holder and Daemon code** at `origin/main` 9df865f:
  - `crates/orch-holder/src/holder.rs` (`spawn_agent`, `signal_agent`, `stop_agent`)
  - `crates/orch-holder/src/paths.rs` and `detach.rs`
  - `crates/orch-daemon/src/lifecycle.rs` (`spawn_holder`, `session_env`)
  - `crates/orch/src/{hook,tap,worktree_hook}.rs`
  - `crates/orch-git/src/{worktree,subagent}.rs`
  - `crates/orch-agent/src/{agent.rs,claude/mod.rs,claude/settings.rs,antigravity/mod.rs}`
- **portable-pty 0.9.0**, `src/unix.rs`: its child calls `setsid()` and then `ioctl(0, TIOCSCTTY)` (lines 257 to 271).
- **Claude Code sandbox docs**, https://code.claude.com/docs/en/sandboxing (fetched 2026-10-12). Cited as *CC docs*.
- **The bwrap command line of Claude Code's own sandbox.** The shell these experiments ran in was inside it, so `/proc/1/cmdline` showed the exact flags.
- **The user's pi sandbox extension**, `~/.pi/agent/extensions/sandbox/`: `index.ts`, `policy.ts`, `hook.ts` and `guard.ts`. Read-only. `auth.json` was not read.
- **Experiments in `$TMPDIR`.** The scripts drive bwrap through a real PTY from Python's `pty.fork()`, the way the Holder does it through portable-pty.

### Test host, and the sandbox that affected the results

- Arch Linux, kernel 7.2.9, bubblewrap 0.13.0, a non-setuid `/usr/bin/bwrap`.
- `kernel.unprivileged_userns_clone=1` and `user.max_user_namespaces=2147483647`. There is no `kernel.apparmor_restrict_unprivileged_userns` key.
- `dev.tty.legacy_tiocsti=0` and `kernel.yama.ptrace_scope=1`.

Every experiment ran inside Claude Code's own bwrap sandbox. That sandbox used `--unshare-user --unshare-pid --unshare-net --new-session --cap-drop ALL --proc /proc`, a read-only `/` and a socat proxy bridge. So each "orch sandbox" below was already one level deep, and each "Agent's own sandbox" was two levels deep. This makes the nesting results conservative: nesting worked even under an outer `--cap-drop ALL` and `--unshare-net`. My shell had no controlling terminal (`--new-session`), but every PTY experiment opened its own PTY, so that did not affect them.

## Short answer

Wrap the Agent's argv as `bwrap <policy> -- <argv>` and spawn it on the Holder's PTY, as today. bwrap becomes the PTY child, and the Agent runs as its grandchild. Five things must be right:

1. **Do not pass `--new-session`.** Without it, the PTY passes through fully, including the controlling TTY, `SIGWINCH` and `^C`. With it, the Agent never learns about resizes.
2. **Signal the Agent itself, not bwrap's process group.** bwrap's outer process takes the default action on `SIGHUP`, `SIGINT` and `SIGTERM`. With `--die-with-parent`, the Agent is then `SIGKILL`ed with no grace. The Holder's current `killpg(agent_pid, SIGHUP)` would lose the Agent's clean shutdown. `--json-status-fd` gives the pid needed to fix this.
3. **Bind-mount only this Session's Holder socket into the sandbox.** Hide the rest of `$XDG_RUNTIME_DIR`. `orch hook` and `orch tap` reach the Holder over a path-based AF_UNIX socket, and that works across `--unshare-net`.
4. **Keep the Agent's whole config tree writable,** plus the Worktree, the git common dir and the Subagent worktrees dir. Adapter `agent_dirs` is too narrow for this.
5. **Leave user namespaces and a real procfs available** (no `--disable-userns`, and mount `/proc` with `--proc`). The Agent's own bwrap sandbox (Claude Code, the pi extension) then nests and works.

The network cannot simply be "off": the Agent process needs its model API. "Off" has to mean `--unshare-net` plus an allowlisting proxy reached through a bind-mounted Unix socket, as Claude Code does. That also cuts the host off from the Session's Port Block.

## PTY

**How it works today.** `spawn_agent` opens a PTY and spawns argv on the slave. portable-pty makes the child a session leader with the slave as its controlling TTY (`setsid` and `TIOCSCTTY`). `agent_pid` is that child's pid. With bwrap, that child is bwrap.

**Experiment.** A Python driver opened a PTY, spawned bwrap with `--unshare-pid --die-with-parent`, resized the PTY with `TIOCSWINSZ` and wrote `\x03`:

| | without `--new-session` | with `--new-session` |
|---|---|---|
| `/dev/tty` opens (controlling TTY) | yes | no |
| size seen by `stty size` | correct | correct (inherited at start) |
| `SIGWINCH` on resize | delivered, new size visible | **not delivered** |
| `^C` in cooked mode | Agent gets `SIGINT`; outer bwrap also dies | only bwrap gets it and dies; Agent killed |

- `--dev /dev` exposes the PTY slave as `/dev/console`, so `tty` prints `/dev/console`. `/dev/tty` works.
- Inside `--unshare-pid`, `getsid` and `getpgrp` return 0, because the leader is outside the namespace. Nothing in orch depends on this.
- *man*, `--new-session`, explains the trade-off: it exists to stop `TIOCSTI` input injection (CVE-2017-5226), and recommends seccomp-blocking `TIOCSTI` instead. That threat does not apply to orch:
  - The PTY belongs to the Holder, and no shell reads from it after the Agent.
  - The kernel already refuses `TIOCSTI` without `CAP_SYS_ADMIN` here (`dev.tty.legacy_tiocsti=0`, the default since Linux 6.2).
  - For older kernels a `--seccomp` filter is the documented alternative.
- **Conclusion:** no `--new-session`. The pi extension (`bwrapArgs`) and Claude Code both use `--new-session`, but only for non-interactive Bash commands that need no terminal.

Cooked-mode `^C` killing the outer bwrap only matters before the Agent switches its TUI into raw mode. In raw mode `^C` is just the byte `0x03`.

## Signals, exit and kill

The Holder stops an Agent like this (`stop_agent`): `killpg(agent_pid, SIGHUP)`, wait 1 s, then `killpg(..., SIGKILL)`. Exit is detected by `child.wait()` on the PTY child.

Inside bwrap, the outer bwrap, bwrap's pid-1 helper (with `--unshare-pid`) and the Agent all share bwrap's process group. Measured outcomes, for an Agent that traps `SIGHUP` and takes 0.5 s to clean up:

| flags | `killpg(bwrap, SIGHUP)` |
|---|---|
| `--unshare-pid --die-with-parent` | bwrap exits at once on `SIGHUP`. Its death `SIGKILL`s the Agent, and cleanup is lost. |
| `--unshare-pid` (no die-with-parent) | bwrap exits at once and the Holder sees "exited". The Agent cleans up as an orphan behind its back. |

- bwrap does not forward signals.
- Signal dispositions pass through bwrap: an ignored `SIGHUP`, `SIGINT` or `SIGTERM` stays ignored in the Agent. So "spawn bwrap with signals ignored" would leak into the Agent and its tools.
- **What works:** pass `--json-status-fd FD`.
  - The first JSON line carries `child-pid` in the Holder's pid namespace. With `--unshare-pid` that is bwrap's **pid-1 helper** (`NSpid: N 1`), not the Agent. A `SIGHUP` to it is ignored.
  - The Agent is that helper's only child, in `/proc/<child-pid>/task/<child-pid>/children`.
  - Sending `SIGHUP` there gave a clean result: the Agent cleaned up, bwrap exited 0 after 0.5 s, and the status fd reported `{"exit-code": 0}`.
  - `SIGKILL` to bwrap's process group still tears everything down, because the pid namespace dies with its init. That is fine for the escalation step.
- *man*: `--die-with-parent` uses `PR_SET_PDEATHSIG`. If the Holder dies, the sandbox dies. That matches today, where losing the PTY master `SIGHUP`s the Agent.
- *man*: `--json-status-fd` also reports `exit-code` (128+n for signal n). bwrap's own exit status mirrors it, so `child.wait()` keeps working.

**Implication for the Holder.** `signal_agent` needs a "sandboxed" variant. It should resolve the inner pid via the status fd, and use killpg on bwrap only for `SIGKILL`. `Hello.agent_pid` would report bwrap's pid unless it is changed.

## Path back to orch: `orch hook`, `orch tap`, `ORCH_SESSION`

How these reach the Holder today:

- `spawn_agent` sets `ORCH_SESSION` and `ORCH_HOLDER_SOCKET` (`paths.rs`) in the Agent's environment.
- The Daemon adds `ORCH_WORKTREE`, `ORCH_PORT_BASE` and display variables through `--env` (`lifecycle.rs`, `session_env`).
- `orch hook` and `orch tap` run as hook and statusline subprocesses of the Agent. They call `locate_socket`, which uses `$ORCH_HOLDER_SOCKET` or falls back to `$XDG_RUNTIME_DIR/orchestrator/holders/<id>.sock`, and talk to the Holder over that socket (`report`, `request_guard`).
- Claude's `WorktreeCreate` hook runs `orch worktree-hook`, which runs `git worktree add` itself.

So inside the sandbox:

- **Environment.** bwrap keeps the environment unless `--clearenv` is passed, so these variables arrive as is. The orch binary (`spec.orch_program`) must be readable and executable, which `--ro-bind / /` covers.
- **Socket across network namespaces.** A path-based AF_UNIX socket bind-mounted into an `--unshare-net` sandbox connected fine. An abstract socket (`\0name`) was **refused**, because abstract sockets are per network namespace. The Holder uses a path socket, so that's fine. Claude Code's sandbox relies on the same property: it bind-mounts `/tmp/claude-http-*.sock` and bridges it to TCP with socat.
- **Hide the runtime dir, re-expose one socket.**
  - Bind-mounting a single socket file under a `--tmpfs` that hides its directory worked.
  - The Holder creates its socket in `bind()` *before* `spawn_agent`, so the file exists when bwrap starts.
  - Expose only `<runtime>/holders/<this id>.sock`, not the holders dir, nor the Daemon socket or other Sessions' sockets.
  - Holder sockets accept `Input`, `Paste`, `Kill`, `GuardAnswer` and more from any client that connects. With every socket visible, an Agent could type into another Session.
  - This is also what the pi extension and Claude Code do for session sockets: pi tmpfs's `$XDG_RUNTIME_DIR` and `/run/dbus` and masks `/run/docker.sock` and sockets in `/tmp`.
- **Remaining gap.** The sandbox cannot stop an Agent from sending its *own* Holder a `GuardAnswer`, because it needs that socket for `orch hook`. That is a protocol question, not a sandbox one.
- **`/proc`.** With `--unshare-pid`, the Daemon can still read sandboxed processes' `/proc/<pid>/environ` from outside. This was checked; `setup.rs` uses this for orphaned setup scripts.

## Writable paths

**Layout, from `orch-git`:**
- Worktree: `<repo>/.orchestrator/worktrees/<slug>`
- Subagent worktrees: `<repo>/.orchestrator/subagents/<slug>/<name>`, on branches created by `git worktree add` from inside the Session worktree
- Git common dir: `<repo>/.git`, outside the Worktree

**Must be writable:**

| path | why | prior art |
|---|---|---|
| Worktree | the work | all |
| `git rev-parse --git-common-dir` (`<repo>/.git`) | commits, refs, index, `worktrees/<name>/` for a linked worktree | pi `resolvePolicy` binds it writable when outside the project. CC docs: "the sandbox also allows writes to the main repository's shared `.git` directory" |
| …but `<common>/hooks` and `<common>/config` **read-only** | otherwise code runs outside the sandbox at the user's next `git` | pi `readOnly`. CC docs: "Writes to `hooks/` and `config` inside that directory remain denied" |
| `<repo>/.orchestrator/subagents/<slug>` | `orch worktree-hook` runs inside the sandbox, as Claude's hook | orch-specific |
| Agent config, session and auth tree | see below | |
| a temp dir | tools, Claude's proxy socket, pi's checkpoint index | pi: host `/tmp`. Claude: a per-user temp dir via `$TMPDIR` |
| caches (`~/.cache`, `~/.npm`, `~/.cargo`, uv …) | builds and tests | pi `sandbox.json` `writable`. Claude `filesystem.allowWrite`. These are Repo config's "extra writable paths" (#96) |

**Agent trees.** `agent_dirs` is not enough. `AgentAdapter::agent_dirs` lists where the *Guard* lets an Agent write without asking: Claude has `<config>/projects` and `<config>/plans`, agy has `~/.gemini/antigravity-cli/brain`, and pi has none yet. A running Agent writes much more:

- **Claude:**
  - `$CLAUDE_CONFIG_DIR` or `~/.claude`, which holds history, todos, `shell-snapshots`, `session-env`, `file-history`, `backups`, the OAuth `.credentials.json`, and more.
  - **`~/.claude.json`, which sits in `$HOME` itself.** If Claude replaces it by temp-file-plus-rename in `$HOME`, binding just that file fails, because rename onto a bind-mount target fails. This was not verified; a spike must check. Mounting `$HOME` writable defeats the sandbox. Setting `CLAUDE_CONFIG_DIR` for sandboxed Sessions may move it, but that is unverified too.
- **agy:** `~/.gemini/…`, since agy uses the Gemini CLI config tree.
- **pi:** `~/.pi/agent/`, which holds `sessions/<cwd>/` JSONL, `models-store.json`, `auth.json` token refresh and `worktrees/`.

So the adapter needs a new, wider method, for example "sandbox-writable dirs". Then `agent_dirs` stays the Guard's list. Secrets inside those trees (auth files) cannot be hidden, because the Agent itself must read and refresh them.

**Order of mounts.** "Later mounts shadow earlier ones" (pi `bwrapArgs` comment, and *man* semantics). Emit them as:

1. `--ro-bind / /`
2. writable `--bind`s
3. read-only carve-outs
4. `/dev/null` file masks and `--tmpfs` dir masks
5. the Holder socket bind-back

## Network

- **On:** omit `--unshare-net`, or use `--share-net` (*man*).
- **Off:** `--unshare-net` gives a new network namespace with only loopback (*man*: `--unshare-net` "Create a new network namespace").

But "off" for the **whole Agent** breaks the Agent: Claude, agy and pi call their model APIs from the Agent process. The prior art sandboxes **commands, not the Agent**, so its Agent keeps the network:

- CC docs: "The sandbox covers shell commands only. Claude's file tools, MCP servers, and hooks run outside it."
- pi wraps `bash` calls only (`hook.ts`, `createSpawnHook`).

For orch, a usable "network off" therefore means restricted network:

- `--unshare-net`.
- An allowlisting HTTP/SOCKS proxy run by orch outside the sandbox.
- The proxy's Unix socket bind-mounted in, with a relay inside that listens on loopback TCP. That is Claude Code's pattern, seen verbatim in its bwrap command: `socat TCP-LISTEN:3128,fork,reuseaddr UNIX-CONNECT:/tmp/claude-http-*.sock` plus `HTTP(S)_PROXY`, `ALL_PROXY`, `NO_PROXY` and `GIT_SSH_COMMAND` with a socat ProxyCommand.

The allowlist must include the Agent's provider hosts. Two side effects:

- **Port Block.** In a separate network namespace, a dev server on `$ORCH_PORT_BASE` is not reachable from the host browser. Port Blocks only work with the network on, unless orch also forwards ports.
- **Hooks still work.** `orch hook` and `orch tap` keep working with the network off (see the Unix-socket result above).

## Nested sandboxes

**Experiments**, all already inside Claude Code's sandbox:

| outer (orch) sandbox | inner `bwrap --unshare-pid --proc /proc --unshare-net` |
|---|---|
| `--unshare-pid --proc /proc` | works (three levels of bwrap in total) |
| `--proc /proc` | works |
| `--ro-bind /proc /proc` | **fails**: `setting up uid map: Read-only file system` |
| `--proc /proc --tmpfs /proc/sys` | **fails**: `Can't read /proc/sys/kernel/overflowuid` |
| `--unshare-user --disable-userns` | **fails**: `Creating new namespace failed: nesting depth or /proc/sys/user/max_*_namespaces exceeded (ENOSPC)` |

**Rules for orch's outer sandbox so that the Agent's own sandbox keeps working:**
- Mount a fresh procfs (`--proc /proc`), and do not mask `/proc/sys`.
- Do not use `--disable-userns`. *man*: it sets `user.max_user_namespaces` to 1 in a nested namespace.
- `--cap-drop ALL` is harmless: Claude Code's sandbox already uses it, and nesting still worked under it.
- *CC docs*, "Bubblewrap fails to start inside a container": where a fresh `/proc` cannot be mounted, Claude needs `enableWeakerNestedSandbox` (bind the existing `/proc`). orch should not force Agents into that.
- **Unprivileged user namespaces:** nested bwrap works with a non-setuid bwrap (`unprivileged_userns_clone=1`). Hosts that restrict them, such as Ubuntu ≥ 24.04 with AppArmor `apparmor_restrict_unprivileged_userns=1`, need the bwrap AppArmor profile from *CC docs*. This matters for an `orch doctor` check.

**Claude Code's own sandbox inside orch's.** It needs `bwrap` and `socat` (*CC docs*), a writable temp dir for its proxy socket, and a writable working directory. It creates 0-byte read-only placeholder files at protected paths there while commands run. Its proxy runs in the Claude process, so under an orch "network off" its allowlist only works if orch's proxy lets those hosts through as well. The tighter of the two layers wins.

**The pi `sandbox` extension inside orch's.** It runs `bash` calls through `/usr/bin/bwrap`, and refuses to run (exit 126) if bwrap is missing. Policy details:
- `--ro-bind / /`, the project and git common dir writable, `<common>/hooks` and `<common>/config` read-only, host `/tmp` writable, `sandbox.json` `writable`.
- SSH private keys and configured secrets masked.
- `$XDG_RUNTIME_DIR`, `/run/dbus`, `/run/docker.sock` and sockets in `/tmp` hidden.
- `--unshare-net` when `"network": false`, and always `--die-with-parent --new-session --unshare-pid`.
- It refuses a project root that contains `$HOME`.
- Its `tool_call` guard also blocks `write` and `edit` outside the roots, reads of masked secrets, and a few destructive commands.

This nests fine under the rules above. Two effects:

- Its tmpfs over `$XDG_RUNTIME_DIR` means **bash commands** inside pi can't reach the Holder socket. That's harmless, because orch's pi hooks run in pi's process through orch's extension, not through bash.
- Its writable list is broader than orch's outer sandbox may be (host `/tmp`, user `writable` entries). Writes there fail at the outer layer, again tightest-wins.

Whether the pi extension stays on is still open on #96.

## Prior art orch can copy

- **Policy shape (both):** read-only root; Worktree and git common dir writable; `.git/hooks` and `.git/config` read-only; mask secrets by binding `/dev/null` over files and tmpfs over dirs; hide session sockets; per-user temp dir; config-supplied extra writable paths. pi's `policy.ts` is a compact reference implementation (`resolvePolicy` and `bwrapArgs`), about 150 lines.
- **Claude Code extras:**
  - protected paths inside the writable Worktree (`.claude/**`, `.mcp.json`, shell rc files, `.gitconfig`, `.vscode` and `.idea`, and files that would turn the Worktree into a bare repo: `HEAD`, `objects`, `refs`)
  - `failIfUnavailable`, which refuses to start rather than run unsandboxed. pi does the same, exit 126.
  - Settings that weaken the sandbox are ignored when they come from repo settings. This matches #96's "committed additions under Trust".
- **Network:** `--unshare-net`, a bind-mounted proxy socket and an in-sandbox socat relay, as in Claude Code.
- **Refusals:** pi refuses a project root that contains `$HOME`. orch Worktrees never do, but a Repo config writable path might.
- **What not to copy:** `--new-session`. It is right for their non-interactive commands and wrong for an interactive Agent on a PTY.

## Open points for later tickets

- **`~/.claude.json` write pattern** under a file bind. Spike: run Claude in the proposed sandbox and watch for `EBUSY`/`EXDEV` on save.
- **Exact writable trees for agy and pi** (one run each under `strace -f -e trace=%file`, or a read-only-home trial).
- **Holder changes:** a sandboxed `signal_agent` (inner pid via `--json-status-fd`), and what `Hello.agent_pid` reports.
- **Proxy for "network off":** whether orch ships one, or whether "off" is simply unavailable for Agents that need a model API.
- **`orch doctor` checks:** `bwrap` present, user namespaces usable (`bwrap --ro-bind / / true`), and `socat` if the proxy design is chosen.
