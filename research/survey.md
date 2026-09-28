# Survey: existing parallel coding-agent orchestrators

Ticket: [#2](https://github.com/ajms/orchestrator/issues/2) (map [#1](https://github.com/ajms/orchestrator/issues/1)). Surveyed 2026-09-28.
Vocabulary follows `CONTEXT.md` (Orchestrator, Repo, Session, Agent, Worktree, Base branch, Landing, Discarding).

## TL;DR

1. **The biggest competitor is Claude Code itself.** Claude Code now ships *agent view* (`claude agents`, `claude --bg`, `claude attach`) with a supervisor daemon, automatic per-session worktrees under `.claude/worktrees/`, session states (working / needs input / idle / completed / failed / stopped) and PR badges. It does **not** survive reboot, has no per-Session cost display, and is scoped to one directory tree. The Orchestrator must justify itself against this: multi-Repo overview, Landing, cost, reboot recovery, setup hooks. ([agent view docs](https://code.claude.com/docs/en/agent-view), [worktrees docs](https://code.claude.com/docs/en/worktrees))
2. **Two families for the live view.** (a) *Wrap the real interactive TUI* in a PTY (tmux or own daemon) — full Claude Code feature parity, status must be inferred. (b) *Drive headless* `claude -p --output-format stream-json` and render a custom UI — clean structured status/cost, but chronically lags new Claude Code features (slash commands, `AskUserQuestion`, project MCP settings). Every custom-UI tool has issues about missing features.
3. **Persistence = the Agent process must not be a child of the UI.** Tools that got this right use tmux (claude-squad, Agent of Empires, agent-deck, workmux, uzi) or a dedicated PTY-owning daemon (Superset, Claude Code's own supervisor). Electron/in-process tools (Crystal, ccmanager, Conductor local) lose running Agents on quit and at best `--resume` later. **Nobody survives reboot with a live process**; the best do "record Session ↔ Claude session id, respawn with `--resume`".
4. **Status detection has converged on Claude Code hooks** (`UserPromptSubmit`/`PreToolUse` → running, `Stop` → idle, `Notification{permission_prompt,…}` → waiting), with screen-scraping as a fallback. Pure screen-scraping (claude-squad: `strings.Contains(content, "No, and tell Claude what to do differently")`) is fragile and breaks on every Claude Code UI change.
5. **Cost tracking is weak everywhere.** Conductor *removed* its cost display ("wasn't accurate enough"). Sources that exist: statusline JSON (`cost.total_cost_usd`, `context_window.*`, `rate_limits.*`), stream-json `result` messages (`total_cost_usd`, `usage`), and transcript JSONL. Subscription users care more about rate-limit % than USD.
6. **Top user complaints across tools:** worktrees start "broken" (no `.env`, `node_modules`, port clashes) → setup hooks are table stakes; TUI freezes from synchronous subprocess calls in the render loop; embedded-terminal input bugs (escape sequences, Shift/Shift+Enter, no scrollback); no multi-Repo view; no resume after reboot; worktree disk bloat / orphan cleanup; permissions forced to `--dangerously-skip-permissions`; hooks installed into the user's global config leaking into non-orchestrated sessions; projects abandoned or pivoted (Crystal → Nimbalyst, vibe-kanban sunsetting, uzi dormant, claude-squad maintainers "working full time jobs").

## Tools surveyed

| Tool | Form | Lang | Stars / last push | State |
|---|---|---|---|---|
| [Claude Code agent view](https://code.claude.com/docs/en/agent-view) | built into `claude` | – | – | research preview |
| [claude-squad](https://github.com/smtg-ai/claude-squad) | TUI (Bubble Tea) | Go | 8.5k / 2026-08 | slow maintenance ([#250](https://github.com/smtg-ai/claude-squad/issues/250)) |
| [Crystal](https://github.com/stravu/crystal) | Electron desktop | TS | 3.1k / 2026-02 | deprecated Feb 2026 → [Nimbalyst](https://github.com/nimbalyst/nimbalyst) |
| [Conductor](https://www.conductor.build/docs) | macOS desktop, closed source | – | – | active, adding cloud |
| [vibe-kanban](https://github.com/BloopAI/vibe-kanban) | local web app + Tauri | Rust + TS | 28k / 2026-09 | **sunsetting** ([blog](https://www.vibekanban.com/blog/shutdown)) |
| [uzi](https://github.com/devflowinc/uzi) | CLI | Go | 0.6k / 2025-06 | dormant |
| [Agent of Empires (aoe)](https://github.com/agent-of-empires/agent-of-empires) | TUI (ratatui) + web | **Rust** | 3.3k / 2026-09 | very active |
| [agent-deck](https://github.com/asheshgoplani/agent-deck) | TUI | Go | 1k / 2026-09 | active |
| [ccmanager](https://github.com/kbwo/ccmanager) | TUI (Ink) | TS | 1.3k / 2026-09 | active |
| [workmux](https://github.com/raine/workmux) | CLI glue over tmux | Rust | 2.7k / 2026-09 | active |
| [Superset](https://github.com/superset-sh/superset) | Electron desktop | TS | 14.7k / 2026-09 | active, macOS-first, Linux AppImage "experimental" |
| [Emdash](https://github.com/generalaction/emdash) | Electron desktop | TS | 5.9k / 2026-09 | active, Linux builds |
| [cmux](https://github.com/manaflow-ai/cmux) | Ghostty-based macOS terminal | Swift | 27k | macOS only, out of scope for Linux |
| [container-use](https://github.com/dagger/container-use), [Sculptor](https://github.com/imbue-ai/sculptor) | container isolation per agent | Go / Py | 4k / 0.2k | adjacent (sandboxing, not session mgmt) |

Plain tmux + worktree setups are represented by workmux, which is explicitly "build on tools you already use" glue ([README](https://github.com/raine/workmux)).

## 1. Process model and persistence

| Tool | Agent runs as | Survives UI close | Survives reboot |
|---|---|---|---|
| Claude Code agent view | one `claude` process per session under a **supervisor daemon** (`claude daemon status`); restarts crashed sessions; stops idle-unattached ones after ~1h, conversation kept on disk | yes | **no** — shows `failed`/`stopped`; `claude respawn` resumes conversation ([docs](https://code.claude.com/docs/en/agent-view)) |
| claude-squad | interactive `claude` in a **tmux session** per instance; state in `~/.claude-squad/`; an optional daemon only auto-presses Enter in auto-yes mode ([daemon.go](https://github.com/smtg-ai/claude-squad/blob/ce1ffb4/daemon/daemon.go)) | yes | no; users ask ([#212](https://github.com/smtg-ai/claude-squad/issues/212), unanswered) and the workaround is manual `/resume` ([#189](https://github.com/smtg-ai/claude-squad/issues/189)) |
| Agent of Empires | tmux session per agent, or ACP "structured" mode; has a daemon/REST server for the web UI | yes ("What happens when I close aoe? Nothing.") | via stored agent session ids + resume ([#343](https://github.com/agent-of-empires/agent-of-empires/issues/343)) |
| agent-deck, workmux, uzi | tmux | yes | no |
| Superset | own **"terminal host daemon"** (persistent Node process owning PTYs + headless xterm), Electron talks to it over a Unix socket with NDJSON ([plan](https://github.com/superset-sh/superset/blob/3b60241/apps/desktop/plans/done/20260106-1800-terminal-host-control-stream-sockets.md)) | yes ("Sessions survive app restarts") | relaunches agent with its resume command ([docs](https://docs.superset.sh/agent-integration)) |
| Crystal | `claude -p … --output-format stream-json` via node-pty, **one process per turn**, `--resume <id>` for follow-ups ([claudeCodeManager.ts](https://github.com/stravu/crystal/blob/1e18e0b/main/src/services/panels/claude/claudeCodeManager.ts)); SQLite state | no (child of Electron) | conversation only |
| vibe-kanban | `claude -p --output-format=stream-json --input-format=stream-json` long-lived, bidirectional control protocol ([claude.rs](https://github.com/BloopAI/vibe-kanban/blob/d5cbb53/crates/executors/src/executors/claude.rs)); SQLite | no (child of server) | conversation only |
| ccmanager | node-pty in-process, no tmux | no; offers "restore sessions after a restart" from a crash-safe record ([README](https://github.com/kbwo/ccmanager)) | conversation only |
| Conductor | desktop app; local workspaces stop when the app closes, only paid Conductor Cloud keeps running ([search/FAQ](https://www.conductor.build/docs)) | no (local) | – |

Lessons from Superset's own daemon: multiplexing high-volume PTY output and RPC on one socket caused head-of-line blocking and UI freezes, and stale daemons from older app versions held the socket after updates — they needed split control/stream sockets and protocol-version negotiation ([plan](https://github.com/superset-sh/superset/blob/3b60241/apps/desktop/plans/done/20260106-1800-terminal-host-control-stream-sockets.md)). Claude Code's supervisor handles the same version issue by restarting sessions onto the new version after auto-update.

## 2. Live Agent view: embedded PTY vs custom UI

**Wrap the real TUI (PTY):** claude-squad (preview via `tmux capture-pane -p -e -J` polling, full attach via creack/pty, [tmux.go](https://github.com/smtg-ai/claude-squad/blob/ce1ffb4/session/tmux/tmux.go)), agent-deck (tmux control mode), Agent of Empires (tmux + its own VT parser, `src/tmux/vt.rs` is 225 kB), ccmanager, Superset/Emdash (xterm.js), workmux (you just use tmux). Advantage: every Claude Code feature works on day one. Costs: terminal-emulation bugs dominate issue trackers:
- agent-deck: arrow keys/mouse broken ([#539](https://github.com/asheshgoplani/agent-deck/issues/539), [#544](https://github.com/asheshgoplani/agent-deck/issues/544)), raw escape sequences leaking into input when switching sessions ([#585](https://github.com/asheshgoplani/agent-deck/issues/585)), Shift/Shift+Enter broken ([#397](https://github.com/asheshgoplani/agent-deck/issues/397), [#445](https://github.com/asheshgoplani/agent-deck/issues/445)), no scrollback in control-mode attach ([#1491](https://github.com/asheshgoplani/agent-deck/issues/1491)), 2–11 s freezes after detaching from long sessions ([#39](https://github.com/asheshgoplani/agent-deck/issues/39)).
- claude-squad: "Unable to type in preview" ([#53](https://github.com/smtg-ai/claude-squad/issues/53)), detach key conflicts in IDE terminals ([#57](https://github.com/smtg-ai/claude-squad/issues/57), [#119](https://github.com/smtg-ai/claude-squad/issues/119)), multi-second keystroke lag because `tmux capture-pane`/`git diff` ran synchronously inside the Bubble Tea `Update()` loop ([#215](https://github.com/smtg-ai/claude-squad/issues/215)), freeze on huge diffs ([#64](https://github.com/smtg-ai/claude-squad/issues/64)).
- ccmanager must force `--teammate-mode in-process` so Claude's agent-teams feature does not fight its PTY ([README](https://github.com/kbwo/ccmanager)).

**Custom UI over stream-json:** Crystal, vibe-kanban, Conductor, AoE's "structured view". Advantage: structured events, native status and usage, mobile/web friendly. Costs: permanent feature lag and a large parser to maintain (vibe-kanban's `claude.rs` is 131 kB):
- vibe-kanban: `AskUserQuestion` unsupported ([#1220](https://github.com/BloopAI/vibe-kanban/issues/1220)); it passes `--disallowedTools=AskUserQuestion` when approvals are off.
- Crystal: slash commands impossible in SDK mode ([#51](https://github.com/stravu/crystal/issues/51)), project `.claude/settings.json`/MCP ignored ([#13](https://github.com/stravu/crystal/issues/13), [#144](https://github.com/stravu/crystal/issues/144)), forced `--dangerously-skip-permissions` until a toggle was added ([#178](https://github.com/stravu/crystal/issues/178)), permissions later routed through an MCP `--permission-prompt-tool`.
- vibe-kanban's UI redesign produced its most-reacted issues ("Bring back old UI", [#2687](https://github.com/BloopAI/vibe-kanban/issues/2687), [#2288](https://github.com/BloopAI/vibe-kanban/issues/2288)).

**Hybrid** (Agent of Empires): tmux terminal view *and* ACP structured view, user picks. Claude Code agent view does the same trick natively: a compact list plus `claude attach` into the full TUI.

## 3. Status detection

| Approach | Used by | Notes |
|---|---|---|
| Screen-scrape + content hash | claude-squad (`HasUpdated`: sha256 of pane; "waiting" = pane contains `"No, and tell Claude what to do differently"`), uzi, agent-deck (polling), ccmanager ("state detection strategies" per agent) | breaks when Claude Code changes wording; many "error capturing pane content" issues ([#51](https://github.com/smtg-ai/claude-squad/issues/51), [#189](https://github.com/smtg-ai/claude-squad/issues/189), [#216](https://github.com/smtg-ai/claude-squad/issues/216)) |
| Claude Code **hooks** writing a per-session status file | Agent of Empires (`/tmp/aoe-hooks-<euid>/<instance>`; mapping: `UserPromptSubmit`/`PreToolUse` → Running, `Stop`/`StopFailure` → Idle, `Notification` `permission_prompt\|elicitation_dialog\|agent_needs_input` → Waiting, `idle_prompt\|agent_completed` → Idle, `PreToolUse(AskUserQuestion)` → Waiting until `PostToolUse` — [agents.rs](https://github.com/agent-of-empires/agent-of-empires/blob/e019ffd/src/agents.rs)), workmux (`workmux setup` installs hooks), Superset (`notify.sh`), agent-deck | robust and cheap; AoE still keeps a 131 kB scrape fallback (`status_detection.rs`) and debounces transitions before firing user hooks |
| Structured stream events | Crystal, vibe-kanban | exact, but only in headless mode |
| Native | Claude Code agent view: working / needs input / idle / completed / failed / stopped, plus process-alive glyph | the reference state set |

Hook installation pitfall: Superset registers hooks in the user's **global** `~/.claude/settings.json`, so they fire in every Claude session on the machine, including ones Superset did not start; they had to add env-var guards ([HOOKS_INVESTIGATION.md](https://github.com/superset-sh/superset/blob/3b60241/HOOKS_INVESTIGATION.md)). Relevant Claude Code hook events and `Notification` types are listed in the [hooks reference](https://code.claude.com/docs/en/hooks).

## 4. Worktree and branch management

- **Location**: sibling dirs, `~/.claude-squad/worktrees`, `.claude/worktrees/<name>` (Claude Code, branch `worktree-<name>`), `~/conductor/…`; configurability is a repeated request ([claude-squad #86](https://github.com/smtg-ai/claude-squad/issues/86), [vibe-kanban #1830](https://github.com/BloopAI/vibe-kanban/issues/1830)); agent-deck offers `sibling` / `subdirectory` / custom root namespaced by repo.
- **Branch naming**: prefix + sanitized title (claude-squad `branch_prefix`); requests for templates ([#88](https://github.com/smtg-ai/claude-squad/issues/88)); Conductor lets the agent rename the branch to match the work ([docs](https://www.conductor.build/docs/concepts/workspaces-and-branches)).
- **Setup is table stakes**: Conductor `scripts.setup` / `scripts.run` / `scripts.archive` with `CONDUCTOR_PORT` (10 ports per workspace), `CONDUCTOR_ROOT_PATH`, `CONDUCTOR_WORKSPACE_NAME`, `run_mode = concurrent|nonconcurrent` ([scripts](https://www.conductor.build/docs/reference/scripts)); uzi `devCommand` + `portRange`; `.worktreeinclude` (Claude Code, ccmanager, requested in vibe-kanban [#1947](https://github.com/BloopAI/vibe-kanban/issues/1947)). claude-squad's most-reacted open issue is exactly this ([#260](https://github.com/smtg-ai/claude-squad/issues/260): missing `.env`, `node_modules`, port and docker-compose collisions), which spawned standalone helpers (workz, wtpool).
- **Pause / cleanup**: claude-squad "pause" = commit, remove worktree, keep branch; "resume" re-adds it. vibe-kanban users hit 26 GB of worktrees in two days and asked for auto-cleanup ([#765](https://github.com/BloopAI/vibe-kanban/issues/765)). Claude Code sweeps old background worktrees but keeps any with uncommitted/unpushed work, holds `git worktree lock` while an agent runs, and marks worktrees it created so it never deletes user-made ones ([worktrees docs](https://code.claude.com/docs/en/worktrees)). agent-deck has `worktree cleanup` for orphans.
- **Base branch**: Claude Code `worktree.baseRef = fresh|head` (fresh = fetched `origin/HEAD`); starting from a PR (`--worktree "#1234"`); requested in claude-squad [#124](https://github.com/smtg-ai/claude-squad/issues/124).
- **Multi-Repo**: missing in most tools and repeatedly requested (claude-squad [#56](https://github.com/smtg-ai/claude-squad/issues/56), [#89](https://github.com/smtg-ai/claude-squad/issues/89), [#299](https://github.com/smtg-ai/claude-squad/issues/299); ccmanager [#174](https://github.com/kbwo/ccmanager/issues/174); workmux [#161](https://github.com/raine/workmux/issues/161); vibe-kanban [#705](https://github.com/BloopAI/vibe-kanban/issues/705), [#1106](https://github.com/BloopAI/vibe-kanban/issues/1106)). This is the Orchestrator's clearest differentiator.

## 5. Landing

| Tool | Local merge | Push + PR |
|---|---|---|
| claude-squad | `c` = commit + pause (checkout branch yourself) | `s` = commit + push, `gh` required; `c` unexpectedly pushed ([#122](https://github.com/smtg-ai/claude-squad/issues/122)) |
| uzi | `checkpoint` = commit + rebase onto current branch | – |
| Crystal | "rebase main into worktree", "squash and rebase to main", abort-rebase-and-let-Claude-fix ([worktreeManager.ts](https://github.com/stravu/crystal/blob/1e18e0b/main/src/services/worktreeManager.ts)) | requested ([#65](https://github.com/stravu/crystal/issues/65)) |
| vibe-kanban | `git merge --squash` into base ([cli.rs](https://github.com/BloopAI/vibe-kanban/blob/d5cbb53/crates/git/src/cli.rs)) | PR with AI description via `git-host` crate (GitHub; GitLab requested [#1697](https://github.com/BloopAI/vibe-kanban/issues/1697)); users want branch deletion after merge ([#2680](https://github.com/BloopAI/vibe-kanban/issues/2680)) |
| workmux | `workmux merge`: merge, delete worktree, close window, delete branch | – |
| agent-deck | `worktree finish`: merge, remove worktree, delete session | – |
| ccmanager | merge (squash requested and added, [#223](https://github.com/kbwo/ccmanager/issues/223)) | – |
| Conductor | review diff, merge, archive | open PR, shows checks |
| Claude Code agent view | – | agent is instructed to commit + push + open draft PR; PR status badge on the row (checks yellow/green, merged purple) |

Recurring asks: squash option, conflict handling (Crystal lets Claude resolve rebase conflicts), commit-message generation ([Crystal #48](https://github.com/stravu/crystal/issues/48), [claude-squad #182](https://github.com/smtg-ai/claude-squad/issues/182)), cleanup of worktree **and** branch after Landing.

## 6. Cost / token tracking

- **Conductor removed Claude Code cost display because it "wasn't accurate enough"** ([0.39.0 changelog](https://www.conductor.build/changelog/0.39.0-insta-summarize-command-palette-opus-4-6)); it also warns that a stray `ANTHROPIC_API_KEY` silently switches subscription users to API billing ([harness docs](https://www.conductor.build/docs/reference/harnesses/claude-code)).
- Crystal sums `input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens` from stream-json and shows `cost_usd` per message.
- agent-deck: `$` cost dashboard, SQLite store, budgets, pricing table with user overrides; Claude events are parsed from a `Stop` hook payload and non-hook tools by scraping the pane ([internal/costs](https://github.com/asheshgoplani/agent-deck/tree/035fd60/internal/costs)). Note: the documented `Stop` input fields do not include usage ([hooks](https://code.claude.com/docs/en/hooks)), so that source looks unreliable.
- Claude Code agent view shows no per-session cost ([agent view](https://code.claude.com/docs/en/agent-view)).
- **Authoritative sources available to us:** the statusline JSON exposes `cost.total_cost_usd` (client-side estimate at list price), `context_window.{total_input_tokens,total_output_tokens,used_percentage,context_window_size}`, `rate_limits.five_hour/seven_day.{used_percentage,resets_at}` for Pro/Max ([statusline docs](https://code.claude.com/docs/en/statusline)); stream-json `result` messages; the transcript JSONL at `transcript_path`.
- Rate-limit pain: users want "rate-limited, resumes at HH:MM" status and auto-resume ([Crystal #18](https://github.com/stravu/crystal/issues/18)); Claude Code now emits `quota_auto_resume_*` notifications ([hooks](https://code.claude.com/docs/en/hooks)).

## 7. What users complain about (ranked by recurrence)

1. **Fresh worktrees are unusable** without setup (deps, `.env`, ports, containers).
2. **No multi-Repo overview**; must relaunch per Repo.
3. **Embedded terminal glitches** (keys, escape sequences, scrollback, resize, focus) and **UI freezes** from blocking subprocess calls.
4. **Custom UIs missing Claude Code features** (slash commands, `AskUserQuestion`, project MCP/settings, skills).
5. **Sessions lost on reboot / app quit**; no automatic `--resume`.
6. **Permissions**: forced YOLO or fragile auto-Enter (claude-squad auto-yes broken [#151](https://github.com/smtg-ai/claude-squad/issues/151); uzi `auto` panics [#9](https://github.com/devflowinc/uzi/issues/9)).
7. **Disk bloat / orphan worktrees and branches** after Landing or Discarding.
8. **Global side effects**: hooks written into `~/.claude/settings.json`; tmux config coupling.
9. **Project churn**: pivots, shutdowns, maintainers disappearing, heavy UX rewrites.

## Lessons for the Orchestrator

**Adopt**
- **Agent processes outside the TUI.** Either tmux as the PTY host or an own small PTY-owning daemon (Superset-style, or reuse Claude Code's supervisor). Record Session ↔ Claude session id ↔ Worktree durably so a reboot becomes "respawn with `--resume`" (AoE #343, `claude respawn`).
- **Hook-based status**, mapped to a state set close to Claude Code's (working / needs input / idle / done / failed / stopped), debounced, with a process-liveness check. Inject hooks **per Session** (e.g. a generated settings file passed at launch), never into the user's global config.
- **Embedded real Claude Code TUI for the live view** (full feature parity), plus a light structured summary/list from hooks and transcripts. Consider "attach" (hand the terminal over) instead of re-rendering a terminal inside ratatui, the source of most terminal bugs.
- **Setup/teardown hooks with per-Session env** (`ORCH_PORT` block, root path, Session name), `.worktreeinclude` compatibility, concurrent/non-concurrent run scripts.
- **Landing as one command** that also removes the Worktree and Branch, with squash option, PR via `gh`, and PR/check status on the Session row.
- **Safe cleanup rules** borrowed from Claude Code: never delete Worktrees with uncommitted or unpushed work without confirmation, lock Worktrees while an Agent runs, mark Orchestrator-created Worktrees, sweep orphans on start.
- **Usage from structured sources** (statusline JSON / transcript JSONL / stream-json result), shown as tokens + context % + rate-limit %, with USD labelled as an estimate.

**Avoid**
- Screen-scraping as the primary status signal (keep only as fallback).
- Synchronous `git`/`tmux` calls on the UI thread; do all polling off-thread with bounded diff sizes.
- Rebuilding Claude Code's UI from stream-json as the main view; it guarantees feature lag.
- Making the Agent a child of the TUI process.
- Defaulting to `--dangerously-skip-permissions` or auto-pressing Enter; map autonomy presets to real `--permission-mode` values.
- Writing into the user's global Claude or tmux config.

**Open questions raised for later tickets**
- Build on Claude Code's own supervisor (`claude --bg`, `claude agents --json`, `claude attach`) or run an independent tmux/daemon layer? The former gives persistence and state for free but is a research preview with a moving interface.
- tmux dependency vs own PTY daemon in Rust (tmux: proven, free persistence, but user-config coupling and capture-pane cost; daemon: control, but Superset-style protocol/versioning work).
