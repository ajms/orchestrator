# pi 1.1 per-launch configuration

Research for #192 (map #191, "pi Agent adapter"). Researched 2026-10-29.

## Answer

pi 1.1 reads all of its user-level state from one **agent dir**. The env var `PI_CODING_AGENT_DIR` moves it, and there is no CLI flag for it. Each file inside it has a fixed name, so you cannot point individual files elsewhere. What you can layer: resources (extensions, skills, prompts, themes) listed in the profile's `settings.json` by absolute path, `-e/--skill/--prompt-template` flags, and sessions moved separately with `--session-dir`. Per launch: `-e <path>` adds an extension (repeatable, and it still loads with `-ne`). `--session-id <id>` opens that exact session or creates it. `--session <path|id>` resumes, `--fork` forks. `--offline` (or `PI_OFFLINE=1`) turns off all startup network activity, including auto-install of missing packages. pi never updates itself without being asked. It only shows a notice.

## Sources

Primary source is the installed release `~/.pi/agent/install/releases/1.1.0/node_modules/@earendil-works/pi-coding-agent/` (written `$PKG` below): `docs/*.md`, `dist/**/*.js`, `pi --help`. The launcher is `~/.pi/agent/bin/pi`. I checked claims hands-on with a throwaway agent dir, an `-e` probe extension that logs `session_start`, and `--mode rpc` with no prompt, so no model was called. Upstream docs were not needed.

## 1. Agent dir / config root

- **Override:** env `PI_CODING_AGENT_DIR` (`~` is expanded). The default is `~/.pi/agent`. See `$PKG/dist/config.js:475-492`: `getAgentDir()` returns `process.env[ENV_AGENT_DIR]`, otherwise `join(homedir(), ".pi", "agent")`. The env name comes from `piConfig.name` (`<NAME>_CODING_AGENT_DIR`). The SDK has an `agentDir` option. **No CLI flag** exists.
- **Verified:** `PI_CODING_AGENT_DIR=<empty dir> pi --offline --help` no longer lists the user's extension flag `--no-worktree-isolation`, so the user's extensions did not load. pi created `<dir>/auth.json` and `<dir>/models-store.json` as empty stores.
- **Files read from `<agent-dir>`.** All are fixed `join(agentDir, name)` paths, with no per-file override (`config.js:493-540`, `core/agent-session-services.js:55-59`, `core/resource-loader.js:753-756,945,956`, `core/trust-manager.js:174`, and the FileModelsStore in the bundle):
  - `settings.json` holds user settings. pi writes to it too, for example `lastChangelogVersion`.
  - `auth.json` holds API keys and OAuth credentials. It is created with mode 0600 (`core/auth-storage.js:25-30`).
  - `models-store.json` is the provider/model store (`FileModelsStore`, default `join(getAgentDir(),"models-store.json")`).
  - Also: `models.json`, `mcp.json`, `mcp-auth.json`, `keybindings.json`, `trust.json` (project-trust decisions), `crashes.json`, `pi-debug.log`, `mcp.log`.
  - `SYSTEM.md` replaces the system prompt, `APPEND_SYSTEM.md` appends to it. A trusted project's `.pi/SYSTEM.md` wins over them (`docs/configuration.md`).
  - `AGENTS.md` / `AGENTS.override.md` / `CLAUDE.md` are user context files.
  - `extensions/`, `skills/`, `prompts/`, `themes/` are auto-discovered resources.
  - `sessions/` is the default session root.
  - `bin/` is where pi downloads `fd`/`rg` if they are missing from the system. Both are in `/usr/bin` on this machine (`utils/tools-manager.js`).
  - `agents/` **is not read by pi core.** The user's `subagent` extension reads it (`~/.pi/agent/extensions/subagent/agents.ts:222`, via `getAgentDir()`), so it follows `PI_CODING_AGENT_DIR`.
- **Project config** lives in `<cwd>/.pi/` (settings, extensions, skills, prompts, SYSTEM.md, mcp.json). It loads only after project trust. The fallback `defaultProjectTrust` (`ask`/`always`/`never`) can only be set in the agent-dir settings. Per run, `-a/--approve` trusts the project and `-na/--no-approve` ignores it (`docs/cli.md`, `docs/settings.md`). For an orch worktree, pass one of these, or set `defaultProjectTrust` in the profile, so pi never stops on a trust prompt.
- **Layering and sharing:**
  - **Resources can be layered.** Profile `settings.json` arrays `extensions`, `skills`, `prompts` and `themes` accept files or directories, `~` and absolute paths. Relative paths resolve from the agent dir (`docs/settings.md` § Resources). Verified: a profile with `{"extensions":["~/.pi/agent/extensions"]}` loads the user's extensions.
  - **Excluding one extension from a listed dir works only with an exact absolute path to its entry file:** `"-/home/albert/.pi/agent/extensions/git-worktree-isolation/index.ts"`. Verified: that removed the flag. These did not work: `-…/git-worktree-isolation` (the directory), `-~/…` (no `~` expansion in `-` patterns), `!git-worktree-isolation/**`, `!**/git-worktree-isolation/**`. The `!` globs go through minimatch against a path inside `.pi/`, a dot-dir (`core/package-manager.js:473-560`). An alternative is a profile `extensions/` dir of symlinks to the chosen extensions (not tested).
  - **Credentials cannot be layered by config.** `auth.json` and `models-store.json` are always `<agent-dir>/<name>`. Options:
    - (a) A symlink `profile/auth.json -> ~/.pi/agent/auth.json`. Writes go through it, because `writeFileSync` writes in place. But the lock is `proper-lockfile` with `realpath:false` (`core/auth-storage.js:39,85`), so the profile locks `profile/auth.json.lock`, not the real file's lock. The user's own pi and orch-pi could race on an OAuth refresh.
    - (b) A bwrap bind-mount of the real file over the profile path. It has the same lock caveat.
    - (c) Provider API-key env vars (`ANTHROPIC_API_KEY` etc., `pi --help`).
  - `SYSTEM.md`, `AGENTS.md` and `keybindings.json` also only come from the agent dir. Copy or symlink them into the profile. For the system prompt there is also `--system-prompt <text|path>` / `--append-system-prompt <text|path>`.
  - **Sessions can be moved separately** (see §4).
- **User extensions that ignore `PI_CODING_AGENT_DIR`** (hardcoded `~/.pi/agent`). They break or point at the real dir under a profile:
  - `sandbox/hook.ts:17`: `sandbox.json`
  - `statusline/codexQuota.ts:22`: `auth.json`
  - `_voice-input/ptt.ts:64`
  - `search/index-store.ts:103`: `~/.pi/index`, or env `PI_INDEX_ROOT`
- **Extensions that overlap with orch.** `git-worktree-isolation` creates `$PI_CODING_AGENT_DIR/worktrees/…` and `sandbox` wraps tools. orch already owns both jobs, so the profile should exclude both.
- Chezmoi source `~/.local/share/chezmoi/dot_pi/agent/` holds `agents/ extensions/ keybindings.json prompts/ sandbox.json settings.json skills/ SYSTEM.md themes/`. It contains no auth, models-store or sessions.

## 2. Per-launch extensions

- `-e, --extension <path>` loads an extension file, a directory with `index.ts`/`index.js`, or `builtin:<name>`. It can be repeated. Relative paths resolve from the cwd (`pi --help`, `docs/cli.md` § Resources). TypeScript loads through jiti, with no build step (`docs/extensions.md`).
- `-ne, --no-extensions` turns off discovered, settings-listed and built-in extensions (`builtin:mcp`, `builtin:llama.cpp`, `builtin:codemode`, `builtin:tool-search`). **Explicit `-e` paths still load.** `--no-mcp` turns off only the built-in MCP extension.
- Settings-level selection: entries in the `extensions` array, `!glob`, `+exact`, `-exact`, and `-builtin:<name>`. `pi config` is the TUI for it (`docs/settings.md`).
- **Verified:** `pi -e probe.ts --help` lists the probe's `pi.registerFlag("orch-probe")` under "Extension CLI Flags". `--orch-probe hello` is then readable inside the extension with `pi.getFlag("orch-probe")`. So orch can pass per-launch parameters to its extension as extension flags, as well as env vars.
- Similar per-launch flags: `--skill`, `--prompt-template`, `--theme` (repeatable), plus `-ns`, `-np`, `--no-themes`, `-nc` (no context files).
- Extension hooks orch can use: `session_start` (reason `startup|reload|new|resume|fork`, `previousSessionFile`), `session_before_switch` (cancellable), `session_before_fork`, `agent_settled`, `session_shutdown`. `ctx.sessionManager.getSessionId()/getSessionFile()/getSessionDir()` give the active session (`dist/core/extensions/types.d.ts:561-585`, `ReadonlySessionManager` in `core/session-manager.d.ts:178`).

## 3. Session id, initial prompt, continue/resume, `/new`

- **Initial prompt:** positional messages, `pi [options] [--] [@files...] [messages...]`. Several messages are sent one after another. `@path` attaches files, piped stdin is prepended, and `--` ends option parsing (`docs/cli.md` § Invocation).
- **Known id:** `--session-id <id>` opens the exact session with that id in the project, or creates it with that id if it is missing. It prints `Warning: No project session found with id '<id>'; creating a new session with that id.` to stderr. Valid ids match `^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?$` (`dist/main.js:344-351`, `core/session-manager.js:15-17`). Verified: `--session-id orch-test-1` gives a session file named `…/<ISO-ts>_orch-test-1.jsonl`. **One flag covers both "new with known id" and "resume by id",** which suits orch, since orch can pick the id up front.
  - It cannot be combined with `--session`, `--continue` or `--resume`. With `--fork` it sets the forked session's id, and errors if that id already exists (`main.js:244-258,290-296`).
- `--session <path|id>`: an argument containing `/` or `\`, or ending in `.jsonl`, is a file path. Otherwise pi tries an exact id in the project, then an id prefix, then a global search across all projects. A match from another project gives an interactive offer to fork (`main.js:192-216,309-321`).
- `-c, --continue` opens the newest session file by mtime in the session dir. With a custom session dir, it only considers sessions whose header `cwd` matches (`core/session-manager.js:445-465,1352-1360`).
- `-r, --resume` opens a picker, which is interactive.
- `--no-session` keeps the session in memory only. `-n, --name <name>` writes a `session_info` entry.
- If the session header's `cwd` no longer exists, interactive mode asks for a cwd and non-interactive modes exit with an error (`main.js:553-563`). This matters if a worktree is gone when resuming.
- **`/new`** (also RPC `new_session` and SDK `newSession`) keeps the same session dir and creates a **new UUIDv7 id** and a new file. `session_before_switch` fires first and can cancel it. Then the runtime is rebuilt and `session_start{reason:"new", previousSessionFile}` fires (`core/agent-session-runtime.js:147-171`). Verified over RPC: after `--session-dir X --session-id orch-test-2`, `new_session` produced id `01a186a3-…` in `X/`, and the `-e` extension was still loaded and saw it. (In that run, the probe's `session_start(new)` handler fired twice, so make handlers idempotent.) **`--session-id` pins only the first Conversation of a launch.** orch has to learn about later ids from an extension (`session_start`), from `PI_SESSION_FILE`, or by scanning the session dir.
- **Lazy file creation:** the `.jsonl` is written only once a user or assistant message exists (`_persist` / `_hasConversation`, `core/session-manager.js:791-810`, uses `openSync(...,"wx")`). Verified: a startup with no prompt leaves the per-cwd session dir empty. orch must not expect the file before the first prompt.
- Child processes run through pi's `bash` tool get `PI_SESSION_ID`, `PI_SESSION_FILE` (unset when ephemeral), `PI_PROVIDER`, `PI_MODEL` and `PI_REASONING_LEVEL`. pi also sets `AI_AGENT=pi` and `PI_CODING_AGENT=true` (`docs/environment-variables.md`).

## 4. Session file location and format

- **Session dir precedence:** `--session-dir` > env `PI_CODING_AGENT_SESSION_DIR` > `sessionDir` setting > default `<agent-dir>/sessions/--<cwd with leading / dropped and / \ : replaced by ->--/` (`main.js:547-551`, `core/session-manager.js:290-295`). Verified: cwd `/tmp/…/scratchpad/work` maps to `sessions/--tmp-…-scratchpad-work--/`. With `--session-dir`, files go directly into that dir, flat, with no cwd subfolder.
- **Filename:** `<ISO timestamp with : and . replaced by ->_<session-id>.jsonl`, for example `2026-10-29T07-49-54-029Z_orch-test-2.jsonl` (`core/session-manager.js:711-714`). Default ids are `uuidv7()` (`:12-14`).
- **Format:** JSONL, version 3. Line 1 is the header `{"type":"session","version":3,"id":"<session-id>","timestamp":"…","cwd":"/abs/cwd","parentSession":"<path>"?}`. `parentSession` is set after `/fork`, `/clone` or `--fork`. Each later line is an entry `{type,id(8-hex),parentId,timestamp,…}` that forms a tree. Types: `message` (roles system/user/assistant/toolResult/custom), `model_change`, `thinking_level_change`, `compaction`, `branch_summary`, `context_edit`, `custom`, `custom_message`, `label`, `session_info`, `usage` (`docs/session-format.md`). **The Conversation id is the header's `id`, and the same id is the filename suffix.**

## 5. Headless modes and forking

- `-p, --print` runs the prompts, prints the final assistant text, then exits. If stdin or stdout is not a TTY and no mode is set, pi uses print mode.
- `--mode json` writes JSONL events to stdout and exits. **The first record is the session header** (with `id`), followed by `agent_start … agent_settled` (`docs/json.md`).
- `--mode rpc` is a long-lived JSONL protocol on stdin/stdout. Commands include `get_state` (returns `sessionId`, `sessionFile`; verified), `new_session`, `switch_session` and prompt (`docs/rpc.md`, `docs/rpc-commands.md`). RPC does not emit the header. Both modes accept the same session flags.
- **Forking:** the CLI has `--fork <path|id>` (new session, optional `--session-id`). Interactive commands: `/fork` (new session from an earlier user message), `/clone` (copy of the active branch), `/tree` (branch within the same file). Forked files carry `parentSession` (`docs/sessions.md`).

## 6. Env vars, launcher, updates

- **Launcher** `~/.pi/agent/bin/pi` (symlinked from `~/.local/bin/pi`). It resolves its own symlinks, takes the agent dir as **its own parent dir, not `PI_CODING_AGENT_DIR`**, reads `install/current-version` (`1.1.0`), and execs `install/releases/<v>/node_modules/.bin/pi`. That file is `#!/usr/bin/env node` pointing at `dist/bundle/cli.js`. The launcher prepends `${XDG_DATA_HOME:-~/.local/share}/pi-node/current/bin` to `PATH` if it exists (it does not exist here, and `/usr/bin/node` v26.12.1 is used). It exports `PI_MANAGED_INSTALL_ROOT=<agent-dir>/install`. Under an orch profile, the Sandbox therefore has to keep `~/.pi/agent/install` (and `bin/pi`) readable even when the rest of `~/.pi/agent` is hidden. **`node` >= 22.19.0 must be on `PATH`** (`package.json` engines).
- `PI_MANAGED_INSTALL_ROOT` is used only by `pi update` (self-update) to recognise a managed install. It checks `<root>/managed-install.json` (`kind: pi-managed-install`, `schemaVersion 1`, `layout releases-v1`) and that the running package is under `<root>/releases` (`dist/package-manager-cli.js:22-40`). Running pi without it is fine. `pi update` would then refuse or fall back.
- **No auto-update.** pi updates itself only via `pi update`. At startup, interactive mode does the following, all in the background:
  - fetches the latest version from pi.dev and shows a notice (`utils/version-check.js`, off with `PI_SKIP_VERSION_CHECK`)
  - checks for package updates (notice only)
  - refreshes model catalogs
  - sends install telemetry (`enableInstallTelemetry`, `PI_TELEMETRY=0`)
- At resource load, pi **does auto-install** npm or git `packages` listed in settings that are missing (`core/package-manager.js:1014-1050`). It may also download `fd`/`rg` into `<agent-dir>/bin`.
- **`--offline` / `PI_OFFLINE=1` turns off all of the above** and also sets `PI_SKIP_VERSION_CHECK=1` (`main.js:452-456`, checked in `version-check.js:37`, `package-manager.js:39`, `tools-manager.js:13`, `interactive-mode.js:830,927,1011`). It does not block provider requests. Recommended for Sandbox launches. It also turns off the model-catalog refresh, which only matters if a catalog is stale.
- Other relevant variables: `PI_CODING_AGENT_DIR`, `PI_CODING_AGENT_SESSION_DIR`, `PI_PACKAGE_DIR`, `PI_TELEMETRY`, `HTTP(S)_PROXY`, provider keys (`docs/environment-variables.md`, `pi --help`).

## Implications for the orch profile

1. Launch: `PI_CODING_AGENT_DIR=<orch-profile> PI_OFFLINE=1 ~/.local/bin/pi -a -e <orch-ext> --session-dir <per-Session dir> --session-id <orch-chosen id> [prompt]`. Use `--session <file>` or `--session-id <id>` to resume.
2. Build the profile's `settings.json` from the user's settings, plus absolute-path `extensions`/`skills`/`prompts`/`themes` entries pointing at `~/.pi/agent/…`. Add exact `-…/index.ts` exclusions for `git-worktree-isolation` and `sandbox`. Copy or symlink `SYSTEM.md`, `AGENTS.md` and `keybindings.json`. Mount or symlink `auth.json` and `models-store.json`, keeping the lock caveat in mind.
3. Track the active Conversation from the orch extension's `session_start` (`ctx.sessionManager.getSessionId/File`), because `/new` changes it. The session file exists only after the first message.
