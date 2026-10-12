# Claude's `~/.claude.json` inside the Sandbox

Research for #107 (map #96). Builds on `docs/research/bubblewrap-holder.md` (#97).

Measured on Arch, kernel 7.2.9, bwrap 0.13.0, Claude Code 2.1.296. The
experiments used a throwaway `$HOME` with a minimal `.claude.json`, ran
`strace -f` on file syscalls, and ran `claude -p hi` without credentials. Claude
exits with "Not logged in", but only after its startup config saves, so no model
calls were made. The user's real `~/.claude.json` and `~/.claude` were not
touched. Everything ran nested inside Claude Code's own bwrap sandbox.

The sandbox policy for each run: `--ro-bind / /`, `--tmpfs /tmp`, a fake
`HOME=/tmp/home`, then the variant's binds.

## How Claude saves `~/.claude.json`

Source: strace of an unsandboxed run, plus the bundle strings (`Writing to temp
file`, `Renaming`, `written via in-place fallback`).

1. **Lock:** `mkdir("$HOME/.claude.json.lock")`, then `rmdir` when done. This is
   a directory lock that sits *next to* the config file, in `$HOME`.
2. **Backup:** `~/.claude/backups/.claude.json.backup.<ms>`.
3. **Temp file:** `open("$HOME/.claude.json.tmp.<pid>.<12 hex>", O_CREAT|O_EXCL)`,
   also next to the file, in `$HOME`.
4. **Replace:** `rename(tmp, "$HOME/.claude.json")`.
5. **Fallback:** if the rename fails, Claude opens the target
   `O_WRONLY|O_CREAT`, calls `ftruncate(fd, 0)`, writes, and unlinks the temp
   file. The bundle calls this "written via in-place fallback". It isn't atomic.

One startup saves about 9 times. If `.claude.json` is a symlink, Claude resolves
it and stages the temp file next to the *target*. It doesn't refuse.

## Results per layout

| Layout | Lock | Temp file | Outcome |
|---|---|---|---|
| ro `$HOME`, writable **file bind** `~/.claude.json` + dir bind `~/.claude` | `EROFS` | `EROFS` on create | **Save silently lost.** No rename is tried, there's no in-place fallback, and nothing on stderr. The file is unchanged. |
| **tmpfs `$HOME`**, file bind `~/.claude.json` + dir bind `~/.claude` | ok, but on the sandbox's private tmpfs | ok, on tmpfs | `rename` → **`EBUSY`**, then the in-place fallback. **Save persists**, and the JSON is valid, including when it shrinks from 20 KB to 0.9 KB, since `ftruncate` is used. |
| ro `$HOME`, `~/.claude.json` → symlink into the dir bind `~/.claude/` | `EROFS`, ignored | in `~/.claude/` | `rename` ok. **Save persists, but without a lock.** |
| ro `$HOME`, `CLAUDE_CONFIG_DIR=<dir>`, dir bind of `<dir>` | `<dir>/.claude.json.lock` ok | `<dir>/` | `rename` ok. **Atomic and locked. Clean.** |

Notes:
- The EBUSY rename failure only appears once the temp file can be created, which
  needs a writable `$HOME`. With a read-only `$HOME`, the earlier `EROFS` loses
  the save first.
- With `CLAUDE_CONFIG_DIR` set, **`~/.claude.json` is never read**. Claude uses
  `$CLAUDE_CONFIG_DIR/.claude.json` and created a fresh one (mode 0600), even
  when `CLAUDE_CONFIG_DIR=~/.claude` and `~/.claude.json` exists. So orch can't
  set it only inside the Sandbox without forking the Agent's state from the
  user's normal Claude: onboarding, `oauthAccount`, per-project trust and MCP
  servers. Some paths stay hard-wired to `$HOME` even then; it still reads
  `~/.claude/state/…`, `~/.config/anthropic/…` and `~/.gitconfig`.
- In the tmpfs layout the lock lives on the sandbox's private tmpfs, so it
  excludes nobody outside: not the host's Claude, not other Agents. With the
  symlink there's no lock at all. In both, concurrent writers can lose each
  other's updates, which is a read-modify-write race. The in-place fallback is
  also not crash-atomic, but `~/.claude/backups/` mitigates that.

## Other files Claude touches in `$HOME`

In an unauthenticated `-p` startup, every write under `$HOME` was:
- `~/.claude.json`, `~/.claude.json.lock/` and `~/.claude.json.tmp.*`, all in
  `$HOME` itself
- `~/.claude/backups/`, `~/.claude/sessions/<pid>.json` (created and unlinked),
  and `~/.claude/projects/<cwd-slug>/…jsonl` plus `memory/`

No writes landed in `~/.cache`, `~/.config` or `~/.local`.

Not exercised: an authenticated session and tool use. Those add
`.credentials.json`, `shell-snapshots/`, `file-history/`, `todos/` and similar.
The real `~/.claude` listing shows they all live inside `~/.claude`, which is
already a directory bind, so they need nothing extra.

## Recommendation

A plain file bind of `~/.claude.json` on a read-only `$HOME` **doesn't work**:
saves vanish silently. Pick one of these:

1. **Preferred: tmpfs `$HOME` plus selective binds.** Mount `--tmpfs $HOME`,
   then `--bind ~/.claude`, `--bind ~/.claude.json` (a file bind), and read-only
   binds for whatever else in `$HOME` the Agent needs (`.gitconfig`,
   `.config/git`, toolchains). Claude's EBUSY fallback keeps saves, and the
   Agent keeps sharing state with the user's Claude. The cost is that the
   `.claude.json` lock is per-Sandbox. This tmpfs also hides the rest of
   `$HOME`, which matches the #97 policy.
2. **Symlink: avoid.** It needs a host-side change to the user's `$HOME`
   (`~/.claude.json` → `~/.claude/…`), and it drops the lock.
3. **`CLAUDE_CONFIG_DIR`: only for an isolated Agent identity.** It is clean
   and atomic, but it forks the config from the user's `~/.claude.json`. That
   suits a dedicated, orch-owned config dir, not "the user's Claude, sandboxed".
