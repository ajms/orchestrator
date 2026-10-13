# Claude Code inside bubblewrap from the Holder

Research for [#127](https://github.com/ajms/orchestrator/issues/127), part of map [#126](https://github.com/ajms/orchestrator/issues/126). The question: what do the facts say about running today's Claude Code as `bwrap <policy> -- claude …` on a Holder's PTY?

It re-verifies the earlier research on `research/bubblewrap-holder` (#97) and `research/claude-json-sandbox` (#107). Every result below was measured again. **Contradicts or extends earlier research** marks the places where the new results differ.

## Sources

- **Host:** Arch Linux, kernel 7.2.9, `bubblewrap 0.13.0` (non-setuid `/usr/bin/bwrap`), `socat` present, Claude Code **2.1.296** (native build, `~/.local/share/claude/versions/2.1.296`).
  - Sysctls: `kernel.unprivileged_userns_clone=1`, `user.max_user_namespaces=2147483647`, `dev.tty.legacy_tiocsti=0`.
  - No `kernel.apparmor_restrict_unprivileged_userns` key.
- **bwrap(1)** man page and `bwrap --help` of 0.13.0. Cited as *man*.
- **bubblewrap release notes**, from `gh api repos/containers/bubblewrap/releases`. Cited as *NEWS*.
- **Claude Code docs**, https://code.claude.com/docs/en/sandboxing, fetched 2026-10-13. Cited as *CC docs*.
- **The Claude 2.1.296 binary:** `strings` of the bundle, `claude --help`, and `claude sandbox status`.
- **orch code on `main` 408402e:**
  - `crates/orch-holder/src/holder.rs` (`spawn_agent`, `signal_agent`, `stop_agent`) and `paths.rs`
  - `crates/orch-daemon/src/lifecycle.rs` (`session_env`)
  - `crates/orch/src/{hook,tap,worktree_hook}.rs`
  - `crates/orch-agent/src/claude/{mod,settings}.rs`
- **Distro package pages:** Launchpad `bubblewrap` and tracker.debian.org `bubblewrap`. Upstream AppArmor `profiles/apparmor.d/unprivileged_userns` (gitlab.com/apparmor). The Ubuntu blog post on restricted unprivileged user namespaces.
- **Experiments in `$TMPDIR`:**
  - A Python driver opens a PTY with `pty.fork()`. Like portable-pty, it calls `setsid()` and sets the controlling TTY. The driver spawns `bwrap … -- <agent>`, resizes, types and signals.
  - The agent is either a probe script or the real `claude` with a throwaway `$HOME` (`env -i`, no credentials). Claude reached its idle prompt ("Not logged in"), and no prompt was ever sent, so **no model calls were made**.
  - Traces used `strace -f`. The user's real `~/.claude.json` and `~/.claude` were never touched.
  - Everything ran nested inside Claude Code's own Bash sandbox, which is `bwrap --unshare-user --unshare-pid --unshare-net --cap-drop ALL --proc /proc --new-session`. So each "orch sandbox" was already one level deep, as in the earlier research. Nested bwrap worked without disabling that sandbox.

## Short answer

Spawn `bwrap <policy> -- claude …` as the PTY child, as the Holder does today, with these choices:

1. **No `--new-session`.** Then `/dev/tty`, `SIGWINCH` and `^C` pass through. Confirmed again.
2. **Spawn bwrap with `SIGHUP` ignored (`SIG_IGN`)**, and keep the Holder's existing `killpg(pid, SIGHUP)` → `SIGKILL` stop. Claude installs its own `SIGHUP` handler, so the ignore doesn't reach it or its children. bwrap then survives both the Holder's `SIGHUP` and a PTY hangup, and Claude exits cleanly with 129.
   - *Contradicts earlier research:* that research proposed a "sandboxed `signal_agent`" that finds the inner pid through `--json-status-fd`. That works, but it isn't needed.
   - *Contradicts earlier research:* that research said `--die-with-parent` "matches today" when the Holder dies. It doesn't. A PTY hangup kills bwrap at once, and Claude is `SIGKILL`ed before it can clean up.
3. **Prefer no `--unshare-pid`** (or accept the cost). Inside a pid namespace every sandboxed Claude is pid 2. Claude names `~/.claude/sessions/<pid>.json` after its pid, so concurrent sandboxed Sessions overwrite each other's registration. *New finding.*
4. **Nesting:** Claude's own Bash sandbox runs inside orch's if orch's sandbox:
   - mounts a fresh `--proc /proc` and leaves `/proc/sys` alone
   - does not pass `--disable-userns`
   - uses no seccomp filter that blocks creating user namespaces

   `--cap-drop ALL`, `--unshare-user`, `--unshare-net` and a TIOCSTI-only seccomp filter are fine. **Claude does not detect a broken nesting at startup.** `failIfUnavailable` doesn't catch it, so orch must check it itself.
5. **`~/.claude.json`:** the results of #107 hold exactly on 2.1.296. A file bind on a read-only `$HOME` silently loses saves. A tmpfs `$HOME` plus a file bind persists them through the EBUSY in-place fallback. `CLAUDE_CONFIG_DIR` forks the config.
6. **Writable `$TMPDIR` (or `/tmp`)** is needed for Claude's messaging sockets and its temp dirs. Without one, Claude warns "Cross-session messaging is off". Startup also runs `git`, `ssh`, `gh`, `ps` and `pacman`. See *What Claude touches*.
7. **Hosts:** bwrap ≥ 0.3.3 covers every flag used here (Ubuntu 22.04 ships 0.6.1, and 24.04 ships 0.9.0). On Ubuntu ≥ 24.04 with `kernel.apparmor_restrict_unprivileged_userns=1`, bwrap needs an AppArmor profile with `userns`, or no bwrap works at all.

## 1. Process and signals

### PTY and job control

Driver: open a PTY, run `bwrap --ro-bind / / --dev /dev --proc /proc --unshare-pid --die-with-parent [--new-session] -- probe`, resize to 40×120, and type `^C` in cooked mode.

| | without `--new-session` | with `--new-session` |
|---|---|---|
| `/dev/tty` opens | yes | no (`ENXIO`) |
| `SIGWINCH` on resize | delivered, sees 40x120 | not delivered |
| `^C` (cooked) | probe gets `SIGINT`; bwrap also dies of `SIGINT` | only bwrap gets it; probe is killed |

- The same results as #97. Real Claude inside bwrap without `--new-session` drew its full TUI, and the trace shows it opening `/dev/tty`.
- *man*, `--new-session`: without it, "it is recommended to use seccomp to disallow the TIOCSTI ioctl" (CVE-2017-5226). On this kernel `dev.tty.legacy_tiocsti=0` already refuses TIOCSTI without `CAP_SYS_ADMIN`. For older kernels, a TIOCSTI-only `--seccomp` filter was built and tested: TIOCSTI → `EPERM`, and nesting still worked (section 2).
- **Process groups.** Without `--unshare-pid`, Claude stays in bwrap's process group (pgrp = bwrap's pid). With `--unshare-pid`, `getpgrp`/`getsid` return 0 inside, but bwrap, its pid-1 helper and the Agent still share one process group.

### What Claude 2.1.296 does with signals

From `/proc/<pid>/status` `SigCgt` of the running `claude`:
- **Caught:** `SIGHUP`, `SIGINT`, `SIGQUIT`, `SIGTERM`, `SIGWINCH`, `SIGCONT`, `SIGUSR2` and the fault signals.
- **Ignored:** `SIGPIPE`.

Claude exits with **code 129** about 0.6–0.8 s after `SIGHUP`. It removes `~/.claude/sessions/<pid>.json` on the way out.

### How signals reach Claude inside bwrap

Measured with real Claude, unless the table says "probe":

| how the Holder signals | flags | result |
|---|---|---|
| `killpg(bwrap, SIGHUP)` (today's code) | `--unshare-pid --die-with-parent` | bwrap dies of `SIGHUP` in 10 ms. Claude is SIGKILLed, and **its cleanup is lost** |
| `kill(<inner pid>, SIGHUP)`, pid found via `--json-status-fd` | `--unshare-pid --die-with-parent` | clean: bwrap exits 129 after 0.79 s; status fd `{"exit-code": 129}` |
| `kill(<json child-pid>, SIGHUP)` | `--unshare-pid` | `child-pid` is bwrap's **pid-1 helper** (`NSpid: N 1`); `SIGHUP` is ignored, nothing happens |
| `kill(<json child-pid>, SIGHUP)` (probe) | no `--unshare-pid`, or `--unshare-pid --as-pid-1` | `child-pid` **is** the Agent; clean exit |
| PTY master closed (Holder dies) | `--unshare-pid --die-with-parent` | bwrap (session leader) dies of `SIGHUP` in 50 ms. Claude is killed, and **`sessions/2.json` is left behind** |
| `killpg(bwrap, SIGHUP)`, **bwrap spawned with `SIGHUP` = `SIG_IGN`** | `--unshare-pid --die-with-parent` and `--die-with-parent` | **clean: exit 129 after 0.67–0.8 s, sessions file removed** |
| PTY master closed, **bwrap spawned with `SIG_IGN`** | `--unshare-pid --die-with-parent` | **clean: exit 0 after 0.45 s, sessions file removed** |
| `killpg(bwrap, SIGKILL)` (escalation) | any | everything dies at once |

Why `SIG_IGN` works:
- bwrap installs no `SIGHUP` handler (its `SigCgt` is empty), so an ignored `SIGHUP` stays ignored in bwrap and in its pid-1 helper.
- Ignored dispositions are inherited through `exec`, but Claude installs its own handler. Handled signals reset to the default at the next `exec`.
- A shell command run by Claude under this setup showed `SigIgn: 0000000000000000`, so nothing leaks to Claude's children.

The only window is Claude's first moments before it installs the handler. A `SIGHUP` there is ignored, and the Holder's `SIGKILL` after `KILL_GRACE` covers it.

**Why the lost cleanup matters.** *CC docs* (troubleshooting): when a session is killed "for example by SIGKILL", Claude's Bash sandbox leaves **0-byte read-only placeholder files** at protected paths in the working directory. Later sessions then fail to save settings there. With `--die-with-parent`, every Holder death or `SIGHUP` to bwrap is such a `SIGKILL`.

### Exit codes

*man*: the status fd reports `exit-code` "in the usual shell encoding (n …, or 128+n if it was killed by signal n)". Measured with and without `--unshare-pid`:

| Agent | bwrap's own wait status | status fd |
|---|---|---|
| `exit 7` | exit 7 | `{"exit-code": 7}` |
| killed by `SIGKILL` | **exit 137** (not a signal) | `{"exit-code": 137}` |
| killed by `SIGABRT` | **exit 134** | `{"exit-code": 134}` |
| Claude after `SIGHUP` | exit 129 | `{"exit-code": 129}` |

- **Implication:** `AgentExit` comes from `child.wait()` on bwrap. A signal death of Claude now shows up as `code: 128+n` with `signal: None`. A bwrap setup failure (bad bind and so on) exits 1 with a `bwrap: …` message, before any `child-pid` line.
- If orch needs to tell these apart, read the status fd. No `child-pid` line means bwrap itself failed.
- bwrap itself is killed by a signal only when it receives one directly. In those cases `child.wait()` reports a signal (rows above).

### Holder code today

- `spawn_agent` spawns argv on the PTY slave and keeps `agent_pid = child.process_id()`. That pid is bwrap once argv is wrapped.
- `signal_agent` sends `killpg(agent_pid, …)`, and `stop_agent` sends `SIGHUP`, waits `KILL_GRACE`, then sends `SIGKILL`.
- With bwrap spawned with `SIGHUP` ignored, none of this needs to change.
- portable-pty's `CommandBuilder` has no `pre_exec` hook. Ways to set `SIG_IGN`:
  - set it in the Holder around the spawn, since ignored dispositions survive fork and `exec`
  - use a tiny exec wrapper
  - spawn without portable-pty

  This is an implementation choice for the spec.

## 2. Nesting Claude's own Bash sandbox

**How Claude's sandbox works** (from the bwrap command line of this very sandbox, and from strings in the 2.1.296 binary):
- It runs `bwrap --new-session --die-with-parent --unshare-net --ro-bind / / … --dev /dev --unshare-pid --unshare-user --cap-drop ALL --proc /proc`.
- It routes network traffic through a `socat` bridge from `$TMPDIR/claude-http-*.sock` to a host proxy.
- It runs an embedded **`apply-seccomp`** helper (a multicall of the Claude binary). Its error strings show that it calls `unshare(CLONE_NEWUSER)`, writes `/proc/self/uid_map`/`gid_map`, calls `unshare(CLONE_NEWPID|CLONE_NEWNS)`, `mount(/proc)` and `prctl(PR_SET_SECCOMP)`.
- So it needs one more user namespace and procfs mount *inside* its own bwrap. *CC docs*: the seccomp filter "adds Unix domain socket blocking".

**Matrix.** Each row is an outer (orch) bwrap running the inner command `bwrap --new-session --die-with-parent --unshare-net --ro-bind / / --dev /dev --unshare-pid --unshare-user --cap-drop ALL --proc /proc -- sh -c 'echo INNER-OK'`. All of this is already inside Claude Code's sandbox, so it is three levels deep.

| outer flags | inner |
|---|---|
| `--unshare-pid --proc /proc` | OK |
| `--proc /proc` (no pid ns) | OK |
| `--unshare-pid --as-pid-1 --proc /proc` | OK |
| `--unshare-all --share-net --proc /proc` | OK |
| `--unshare-user --unshare-pid --cap-drop ALL --proc /proc` | OK |
| `--new-session` added | OK |
| `--seccomp` filter that only blocks `ioctl(TIOCSTI)` | OK |
| `--ro-bind /proc /proc` (no fresh procfs) | **fails**: `setting up uid map: Read-only file system` |
| `--proc /proc --tmpfs /proc/sys` | **fails**: `Can't read /proc/sys/kernel/overflowuid` |
| `--proc /proc --ro-bind /proc/sys /proc/sys` | **fails**: `Can't mount proc on /proc: Operation not permitted`. *Extends earlier research:* any overmount inside `/proc` blocks the inner procfs mount |
| `--unshare-user --disable-userns` | **fails**: `Creating new namespace failed: nesting depth or /proc/sys/user/max_*_namespaces exceeded (ENOSPC)` |
| `--seccomp` filter that blocks `unshare`, `clone(CLONE_NEWUSER)` and `clone3` | **fails**: `No permissions to create a new namespace…` |

*man*, `--disable-userns`: it is "implemented by setting the user.max_user_namespaces sysctl to 1", which explains the `ENOSPC`.

**Claude does not notice a broken nesting.** *Contradicts the earlier assumption that `failIfUnavailable` helps here.*
- Inside an outer bwrap with `--unshare-user --disable-userns`, `claude sandbox status` still printed `"enabled":true,"unavailableReason":null`, the same as under a working outer sandbox.
- `/sandbox` showed its normal mode menu with no Dependencies tab.
- *CC docs*: the dependency check covers `ripgrep`, `bubblewrap`, `socat` and the seccomp filter. It runs at startup and doesn't try a namespace.
- So a broken nesting surfaces only when the first sandboxed Bash command fails. `failIfUnavailable` doesn't catch it, and orch must not rely on it. A check such as `bwrap --unshare-user --unshare-pid --proc /proc --ro-bind / / true` *inside* orch's policy is what proves nesting.

**What runs outside Claude's sandbox (but inside orch's).**
- *CC docs*: "The sandbox covers shell commands only. Claude's file tools, MCP servers, and hooks run outside it." The status line command and `apiKeyHelper` also run outside it.
- So `orch hook`, `orch tap` and `orch worktree-hook` run in orch's sandbox only, and they reach the Holder socket normally.
- Shell commands Claude runs inside its own sandbox get the Unix-socket seccomp block. An `orch` CLI call from Claude's Bash tool can't reach the Holder unless allowed.
- **`!` shell-mode commands are not sandboxed by Claude.** *CC docs* say so, and a trace confirmed it: `! echo …` ran `zsh -c …` directly, with no bwrap. orch's sandbox is the only boundary for them.
- *CC docs*: Claude's sandbox allows writes to the main repo's shared `.git` for linked worktrees, except `hooks/` and `config`. orch's sandbox must allow the same writes, or `git commit` fails at the outer layer (tightest wins).

**Opt-in subprocess isolation.** 2.1.296 also contains "subprocess env scrubbing and isolation". It requires bubblewrap and is controlled by `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB` and `sandbox.bwrapPath`. It is on only when that variable is truthy or under `GITHUB_ACTIONS`, so orch can leave it off. If it is turned on, it is one more nested bwrap with the same needs.

## 3. Config saves: `~/.claude.json` and `~/.claude/`

Re-run of #107 on 2.1.296. `claude -p hi` without credentials exits "Not logged in · Please run /login" *after* its startup saves. The seed config was 20 KB.

| `$HOME` layout | what strace shows | result |
|---|---|---|
| ro `$HOME`, file bind `~/.claude.json`, dir bind `~/.claude` | `mkdir .claude.json.lock` → `EROFS` (10×), temp `open(O_EXCL)` → `EROFS` (9×), no rename, no fallback | **save silently lost**: file unchanged at 20061 bytes, nothing on stderr |
| tmpfs `$HOME` + file bind `~/.claude.json` + dir bind `~/.claude` | lock ok (on tmpfs), temp ok, `rename` → **`EBUSY`** (8×), then `open(.claude.json, O_WRONLY\|O_CREAT)` in place | **save persists** (20061 → 20919 bytes, valid JSON) |
| writable `$HOME` | lock, temp, `rename` = 0 | persists, atomic, locked |
| `CLAUDE_CONFIG_DIR=$HOME/.claude`, dir bind | lock, temp and rename all inside `$HOME/.claude/` | atomic and locked, but it wrote a **fresh** `$HOME/.claude/.claude.json` (852 bytes, mode 0600) and **never read `~/.claude.json`** |

All four agree with #107. Notes:
- In the tmpfs layout the lock dir sits on the sandbox's private tmpfs, so it excludes neither the host's Claude nor other Sandboxes. The in-place write isn't crash-atomic, though `~/.claude/backups/` holds copies.
- Writes under `~/.claude/` during startup and idle: `backups/`, `sessions/<pid>.json` plus a `<pid>.<hash>.key`, `cache/`, `plugins/` (a first start clones the official marketplace into `plugins/marketplaces/`) and `projects/<slug>/`.
- A logged-in session adds `.credentials.json` (OAuth refresh), `history.jsonl`, `shell-snapshots/`, `session-env/`, `file-history/`, `todos/` and `paste-cache/` (from the user's real `~/.claude` listing). All of them live inside `~/.claude`.
- `~/.claude` must be writable. A real `~/.claude` also holds files Claude must not be able to rewrite from inside the Sandbox, such as `settings.json`, `hooks/`, `skills/`, `agents/` and `commands/`. That is a policy decision for the grilling tickets.
- **Session registry collision** (*new*):
  - Claude writes `~/.claude/sessions/<pid>.json` with `pid`, `pidDomain` (`linux:<machine-id>:pid:[<pidns inode>]`) and `messagingSocketPath`.
  - With `--unshare-pid` the pid is always 2. Two concurrent sandboxed Claudes that shared `~/.claude` left **one** `2.json` and two `2.<hash>.key` files: the second overwrote the first.
  - Claude is pid-namespace-aware (`pidDomain`), but the filename is not.
  - Without `--unshare-pid`, pids are unique host pids, and the earlier rows show that nothing else needs the pid namespace.

## 4. What else Claude touches at runtime

From `strace -f -e trace=%file,connect,bind` of an interactive start that reached the idle prompt inside bwrap. `HOME` was fake, and `env -i` left only `PATH`, `TERM`, `USER`, `TMPDIR` and the proxy variables.

**Reads (read-only is enough):**
- the binary: `~/.local/bin/claude` → `~/.local/share/claude/versions/<v>`
- `/etc/claude-code/` (managed settings, `CLAUDE.md`, `.claude/…`)
- `CLAUDE.md`/`AGENTS.md`/`.claude/` in the cwd and **every parent directory up to `/`**
- `~/.config/anthropic/…`, `~/.gitconfig`, `~/.config/git/*`, `~/.netrc`
- `/etc/ssl`, `/etc/resolv.conf`, `/etc/passwd`, `/etc/os-release`, `/etc/machine-id`, `/proc/self/ns/pid`

**Writes:**
- `~/.claude.json` and its lock and temp files in `$HOME` (section 3)
- `~/.claude/**`
- **`$TMPDIR`** (falls back to `/tmp` when unset):
  - `$TMPDIR/claude-<uid>/` (its temp dir)
  - `$TMPDIR/cc-socks/<pid>.sock` (the cross-session messaging socket; it bound there)
  - `$TMPDIR/claude-ssh-config-*`
  - `$TMPDIR/claude-http-*.sock` (Bash-sandbox proxy bridge, when the sandbox is on)
  - When `/tmp` was read-only and `TMPDIR` unset, `mkdir /tmp/cc-socks` → `EROFS`, and the TUI showed "⚠ Cross-session messaging is off: its socket directory could not be set up".
  - The bundle also lists `/run/user/<uid>/cc-socks` and says "Point XDG_RUNTIME_DIR or CLAUDE_CODE_TMPDIR at a private (0700) directory". A private tmpfs on `$XDG_RUNTIME_DIR`, or a per-Session `TMPDIR`, keeps that working and isolated.
- The auto-updater targets `~/.local/share/claude/`. Read-only (or blocked) → the TUI shows "✘ Auto-update failed". Set `DISABLE_AUTOUPDATER=1` (an env var in the bundle) for sandboxed Sessions.

**Subprocesses Claude 2.1.296 starts on its own at startup** (they all run inside orch's sandbox):
- `git …` (repo status)
- `rg` (the bundled ripgrep, as a multicall)
- `ps aux | grep …` (IDE detection)
- `uname`, `pacman -Qo` (install-method check), `xprop`, `xdg-mime`
- `gh auth token` and `gh issue list …`
- `ssh -T git@github.com` and `git clone` of the plugin marketplace (on first start)

Consequences:
- **`ssh` ignores `$HOME`**: it opened the *real* `/home/<user>/.ssh/config` and private key files (from `getpwuid`), even with a fake `HOME`. If orch masks `~/.ssh`, it must mask the real path.
- **`gh auth token` connected to the D-Bus session bus `/run/user/<uid>/bus`** (keyring lookup). The traced thread belonged to `gh`; Claude itself didn't touch D-Bus. Hiding `$XDG_RUNTIME_DIR` breaks gh's keyring token, but it doesn't break Claude.
- With `--unshare-pid`, `ps aux` sees only the Sandbox's processes. That is harmless.

**Auth and keychain.**
- On Linux, Claude looked for `~/.claude/.credentials.json` (`ENOENT` here) and the `~/.config/anthropic/` config. No keyring or Secret Service access by Claude was observed.
- `apiKeyHelper`, if configured, runs outside Claude's own sandbox (*CC docs*) but inside orch's.

**orch's own paths inside the Sandbox** (code on `main`):
- `spawn_agent` sets `ORCH_SESSION` and `ORCH_HOLDER_SOCKET`. `session_env` adds `ORCH_WORKTREE` and `ORCH_PORT_BASE`.
- bwrap keeps the environment unless `--clearenv` is passed.
- `orch hook` and `orch tap` → `locate_socket` → `$ORCH_HOLDER_SOCKET`.
  - Re-verified: a path socket bind-mounted over a `--tmpfs` of the holders dir works across `--unshare-net`.
  - A sibling Session's socket in the same dir was `ENOENT`.
- **`orch tap` also runs the user's own status line command** with `sh -c` (`user_statusline_command`, read from managed, project and `$CLAUDE_CONFIG_DIR`/`~/.claude` settings). That command runs inside the Sandbox too and needs whatever it reads.
- `orch worktree-hook` runs `git worktree add` itself. It needs `<repo>/.orchestrator/subagents/<slug>` and the git common dir writable (as in #97).

**Env vars that matter:** `HOME`, `TMPDIR`, `CLAUDE_CODE_TMPDIR`, `XDG_RUNTIME_DIR`, `CLAUDE_CONFIG_DIR` (forks the config, section 3), `DISABLE_AUTOUPDATER`, `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB`, the proxy variables, `ORCH_*`, and `TERM`.

## 5. Host prerequisites

**bwrap versions** (*NEWS*):

| feature | since |
|---|---|
| `--new-session` optional (before it, bwrap always called `setsid()`) | 0.1.7 |
| `--die-with-parent` | 0.1.8 |
| `--as-pid-1` | 0.2.0 |
| `--json-status-fd`, with the child's `exit-code` | 0.3.3 |
| `--disable-userns`, `--assert-userns-disabled` | 0.8.0 |

**Distro packages:**

| distro | version |
|---|---|
| Ubuntu 18.04 | 0.2.1 |
| Ubuntu 20.04 | 0.4.0 |
| Ubuntu 22.04 | 0.6.1 |
| Ubuntu 24.04 | 0.9.0 |
| Ubuntu (current stable, Resolute) | 0.11.1 |
| Debian 12 | 0.8.0 |
| Debian 13 | 0.11.0 |
| Debian testing | 0.13.0 |

Every supported distro has what this design needs (≥ 0.3.3). Do not rely on `--new-session` being opt-in on anything older than 0.1.7.

**Unprivileged user namespaces:**
- Without setuid bwrap, the kernel must allow them. That means `kernel.unprivileged_userns_clone=1` where the key exists (Debian/Arch), and `user.max_user_namespaces` > 0.
- Nesting Claude's sandbox needs **at least 3 levels** of user namespaces: orch's bwrap, Claude's bwrap and `apply-seccomp`'s own `unshare(CLONE_NEWUSER)`. That's 4 levels when orch itself already runs under a sandbox like this one, which is how these experiments ran.
- **Ubuntu ≥ 24.04 (AppArmor):**
  - *CC docs*: "On Ubuntu 24.04 and later, the default AppArmor policy prevents bubblewrap from creating the user namespaces it needs." Check with `sysctl kernel.apparmor_restrict_unprivileged_userns`.
  - If it is `1`, install a profile `profile bwrap /usr/bin/bwrap flags=(unconfined) { userns, … }` and reload AppArmor.
  - Ubuntu blog: processes can create user namespaces only "if they are confined and have the "userns," rule in their AppArmor profile (or if they have CAP_SYS_ADMIN)".
  - Upstream AppArmor's `unprivileged_userns` profile is the fallback for namespace creators without such a rule. It has `audit deny capability` and stacks children (`allow pix /** -> &unprivileged_userns`), so bwrap can't set up its mounts there.
  - One profile for `/usr/bin/bwrap` covers orch's bwrap and Claude's nested bwrap, because both exec `/usr/bin/bwrap`.
  - **Not verified on a real Ubuntu host:** whether `apply-seccomp`'s own `unshare(CLONE_NEWUSER)` inside the nested sandbox is allowed under that profile. It runs from the Claude binary, not `/usr/bin/bwrap`. It should work, because children of a `flags=(unconfined)` profile stay unconfined, but an `orch doctor` check on Ubuntu should run the real nesting probe rather than trust this.
- **Docker and other unprivileged containers:** *CC docs*: bwrap "can't mount a fresh `/proc`" there (`Can't mount proc on /newroot/proc`). Claude's escape hatch is `enableWeakerNestedSandbox`. The same limit hits orch's own `--proc /proc`.

**Suggested `orch doctor` checks:**
- `bwrap --version` ≥ 0.3.3
- `bwrap --ro-bind / / --unshare-user --proc /proc true` succeeds (userns, AppArmor)
- the same command run inside orch's actual policy succeeds (nesting)
- `socat` present if Claude's sandbox is wanted
- warn on `kernel.apparmor_restrict_unprivileged_userns=1` without a bwrap profile

## Where this differs from the earlier research

| earlier claim | now |
|---|---|
| #97: Holder needs a "sandboxed `signal_agent`" using the inner pid from `--json-status-fd` | Works, but unnecessary. Spawn bwrap with `SIGHUP` ignored, and the existing `killpg(SIGHUP)` → `SIGKILL` stop gives a clean Claude exit. |
| #97: with `--die-with-parent`, Holder death "matches today, where losing the PTY master SIGHUPs the Agent" | Not the same. The hangup kills bwrap first, and Claude is SIGKILLed without cleanup: a stale `sessions/<pid>.json`, plus the Bash sandbox placeholders *CC docs* warns about. Fixed by the same `SIG_IGN`. |
| #97: use `--unshare-pid` | Costs session-registry collisions (every Claude is pid 2). Nothing measured needs it. |
| #97: rule "don't mask `/proc/sys`" | Wider: **no overmount anywhere in `/proc`**; `--ro-bind /proc/sys` breaks nesting too. |
| #97: Claude's sandbox needs bwrap and socat | It also needs a further `unshare(CLONE_NEWUSER)` plus a procfs mount by `apply-seccomp`, and Claude does **not** detect a broken nesting at startup (`unavailableReason: null`). |
| #97: bwrap's exit status mirrors the Agent's | True. The new implication: a signal death becomes exit `128+n`, so `AgentExit.signal` is lost. |
| #107: all four `$HOME` layouts | Unchanged on 2.1.296. |
| (not covered) | `$TMPDIR` needs (`cc-socks`, `claude-<uid>`, ssh config dirs), startup subprocesses (`ssh` reads the real `~/.ssh`, `gh` uses D-Bus), the auto-updater failing visibly, `!` commands being unsandboxed, and `orch tap` running the user's status line inside the Sandbox. |
