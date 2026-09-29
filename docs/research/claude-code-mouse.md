# How Claude Code handles mouse, selection and copy

Research for [#21](https://github.com/ajms/orchestrator/issues/21) (map: [#20](https://github.com/ajms/orchestrator/issues/20)).
Installed version observed: **Claude Code 2.1.284** (Linux, 2026-09-29).

## Sources

- [FS] Official docs, Fullscreen rendering: https://code.claude.com/docs/en/fullscreen
- [ENV] Official docs, environment variables: https://code.claude.com/docs/en/env-vars
- [SET] Official docs, settings reference: https://code.claude.com/docs/en/settings-reference
- [CL] Changelog: https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md (version cited per entry)
- [OBS] Empirical PTY capture of `claude` 2.1.284 (method at the end)
- [ALA] Alacritty config docs: https://alacritty.org/config-alacritty.html
- [KIT] kitty config docs: https://sw.kovidgoyal.net/kitty/conf/

## TL;DR

- Mouse behaviour depends entirely on the **renderer**. The classic (inline, main-screen) renderer does **not** enable mouse tracking at all; the terminal does selection, scrollback and link clicks natively. The **fullscreen** renderer (alt screen) captures the mouse and implements selection, copy, scrolling and clicks itself.
- Fullscreen requests `?1049h` then `?1000h ?1002h ?1003h ?1006h` (any-motion tracking, SGR encoding). With `CLAUDE_CODE_DISABLE_MOUSE_CLICKS=1` it requests only `?1000h ?1006h` (wheel only); with `CLAUDE_CODE_DISABLE_MOUSE=1` none. [OBS]
- Which renderer you get depends on `tui` setting, `CLAUDE_CODE_NO_FLICKER`, `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN`, and a default table (fullscreen by default for users who started on/after 2026-05-06). [FS]
- Fullscreen implements its own selection with **copy-on-select on mouse release** (setting `copyOnSelect`, default on). Copy path: native tool locally (`wl-copy` / `xclip` / `xsel` on Linux, clipboard + PRIMARY), tmux paste buffer inside tmux, **OSC 52** over SSH or as fallback. A toast reports which path was used. [FS] [CL 2.1.161] [OBS]
- Double-click = word (iTerm2 word boundaries, whole URL incl. scheme), triple-click = line, wheel = scroll transcript (or the list under the pointer). [FS] [CL 2.1.198]
- Links are OSC 8 hyperlinks (with `id=`), in both renderers when the terminal is detected as supporting them. In fullscreen, a link opens only on **Ctrl+click** (Linux/Windows) / Cmd+click (macOS); plain click does not open. [FS] [CL 2.1.2, 2.1.113, 2.1.181] [OBS]
- Shift+drag: Claude Code does nothing special; Kitty and Alacritty suppress mouse reporting while Shift is held, so the terminal does a native selection. Claude Code tells you so in its hint ("hold Shift while selecting for native copy"). [FS] [ALA] [KIT] [OBS]

## 1. Renderers and when the mouse is captured

Claude Code has two renderers [FS]:

| Renderer | Screen | Mouse | Selection / copy | Scrollback |
|---|---|---|---|---|
| Classic (`tui: "default"`) | main screen, inline | not captured | terminal native | terminal native |
| Fullscreen (`tui: "fullscreen"`) | alternate screen (`?1049h`) | captured | in-app, copy-on-select | virtualized, in-app |

Renderer selection, first match wins [FS "Fullscreen by default"]:

1. `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` or `CLAUDE_CODE_NO_FLICKER=0` → classic
2. `CLAUDE_CODE_NO_FLICKER=1` → fullscreen
3. a failed fullscreen start on this machine → classic
4. iTerm2 `tmux -CC`, or SSH into Windows → classic
5. saved `tui` setting → that renderer
6. no feature-flag fetch and startup dialog no longer offered → classic
7. no feature-flag fetch and first launch was ≥ 2.1.239 → fullscreen
8. flags fetched and first used on/after 2026-05-06 → fullscreen
9. otherwise classic

Also: screen-reader mode forces classic; attached background sessions (`claude agents` / `claude attach`) are always fullscreen and ignore `tui` and `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` [FS]. `/tui fullscreen` / `/tui default` switch mid-session by relaunching [FS] [CL 2.1.110].

History: fullscreen began as opt-in `CLAUDE_CODE_NO_FLICKER=1` ("flicker-free alt-screen rendering with virtualized scrollback", [CL 2.1.89]); `/tui` and the `tui` setting came in [CL 2.1.110]; `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` in [CL 2.1.132].

## 2. Terminal modes requested (observed)

Captured with `TERM=xterm-kitty`, 120x40 PTY, cwd a fresh dir, trust dialog accepted by sending `↓ Enter`, then SIGTERM. Only mouse/screen-relevant DECSET/DECRST shown, in order. [OBS]

| Configuration | Sequence |
|---|---|
| Classic (`CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1`) | no `h` at all; on exit defensive reset `?1006l ?1003l ?1002l ?1000l ?1016l` |
| Fullscreen (`CLAUDE_CODE_NO_FLICKER=1`) | `?1049h` → `?1000h ?1002h ?1003h ?1006h` … exit: `?1006l ?1003l ?1002l ?1000l` → `?1049l` → `?1016l …` |
| Fullscreen + `CLAUDE_CODE_DISABLE_MOUSE_CLICKS=1` | `?1049h` → `?1000h ?1006h` (no 1002/1003) |
| Fullscreen + `CLAUDE_CODE_DISABLE_MOUSE=1` | `?1049h`, no mouse `h` |
| Fullscreen, `TERM=xterm-256color` | same mouse modes as kitty; but no OSC 8 and no kitty keyboard push |

Notes:

- Mouse tracking is enabled only **after** the alt screen is entered, and only in fullscreen. The trust dialog and other pre-REPL dialogs render inline without mouse tracking.
- `1003` (any-event motion) is what drives hover highlighting of rows [FS "Hovering highlights the row"]. `1006` = SGR encoding; no `1015`/`1005`. `1016` (SGR-pixels) is only ever reset, never set.
- Other modes seen in every run: `?2004h` bracketed paste, `?1004h` focus events, `?2031h` colour-scheme change notifications, `?2026h/l` synchronized output around frames, `?25h/l` cursor. With `TERM=xterm-kitty` also kitty keyboard protocol `CSI > 5 u` (flags 1+4) plus `CSI < u` pop, and xterm `modifyOtherKeys` `CSI > 4;2 m`.
- Claude Code has had to harden its parser against split mouse reports (`<35;150;7M` leaking into the prompt: [CL 2.1.239], [CL 2.1.247]) and mouse sequences interleaving with bracketed paste [CL 2.1.132]; on exit it resets all mouse modes even in classic mode [CL 2.1.83 "Fixed mouse tracking escape sequences leaking to shell prompt after exit"].

## 3. Selection and copy (fullscreen only)

From [FS "Use the mouse"]:

- **Click and drag** selects text anywhere in the conversation (in-app; the terminal's selection buffer never sees it, so Kitty hints / tmux copy mode don't either [FS "Keep native text selection"]).
- **Double-click** selects a word, "matching iTerm2's word boundaries so a file path selects as one unit"; double-clicking a URL selects the whole URL including the scheme [CL 2.1.198].
- **Triple-click** selects the line.
- **Copy-on-select**: "Selected text copies to your clipboard automatically on mouse release." Toggle in `/config` ("Copy on select"), setting `copyOnSelect` (global config) [SET]. With it off: `Ctrl+Shift+C`, `Cmd+C` on kitty-keyboard terminals, and `Ctrl+C` copies instead of interrupting while a selection is active.
- Shift+arrows extend a selection from the keyboard; Esc keeps it [CL 2.1.234]; `selection:copy` / `selection:clear` keybinding actions exist [CL 2.1.239], [CL 2.1.234].

Copy mechanism [FS "Keep native text selection"], [CL 2.1.161]:

- Local: native tool. Linux: `wl-copy` on Wayland, else `xclip` or `xsel`; writes both CLIPBOARD and PRIMARY (middle-click paste works). macOS `pbcopy`; Windows/WSL PowerShell `Set-Clipboard` [CL 2.1.160].
- Inside tmux: additionally the tmux paste buffer.
- Over SSH: OSC 52.
- A toast after each copy states the path used.

Observed [OBS]: with `WAYLAND_DISPLAY` and `DISPLAY` unset (so no native tool is usable), a synthetic SGR drag (`CSI <0;1;30M`, `CSI <32;…M`, `CSI <0;120;40m`) produced `ESC ] 52 ; c ; <base64>` (clipboard target `c`) and the toast
`sent 715 chars via OSC 52 · if paste fails, hold Shift while selecting for native copy`.
Same with `SSH_CONNECTION` set. So OSC 52 is both the SSH path and the fallback when no native clipboard tool/display is available. (The local wl-copy path was not exercised to avoid overwriting the user's clipboard.)

## 4. Wheel

Fullscreen: wheel scrolls the transcript a few lines per notch (with acceleration; `wheelScrollAccelerationEnabled: false` disables it [SET], [CL 2.1.174]); speed via `CLAUDE_CODE_SCROLL_SPEED` (≤ 20) or `/scroll-speed` [ENV], [CL 2.1.139]. Over a list/select menu/dialog with overflow, the wheel scrolls that list instead [FS], [CL 2.1.280], [CL 2.1.283]. Scrolling up pauses auto-follow (`autoScrollEnabled`) [FS].

Classic: no mouse tracking, so the wheel is the terminal's own scrollback. Dialogs that overflow are "scrollable with … mouse wheel in both fullscreen and non-fullscreen modes" [CL 2.1.121] — presumably via the terminal's alternate-scroll translation to arrow keys rather than mouse reports (not verified).

## 5. Clicks (fullscreen)

Click positions the prompt cursor; clicks choose options in select/multi-select menus, `/` and `@` suggestions, `/config` values, scrollbars; click expands collapsed tool results and `!` output; click on the dim last-prompt header jumps to it; click on "Jump to bottom" [FS]. `CLAUDE_CODE_DISABLE_MOUSE_CLICKS=1` keeps wheel but drops click/drag/hover [ENV], [CL 2.1.195]. A click that merely focuses the terminal window is ignored (uses focus events `?1004`) [CL 2.1.239], [CL 2.1.246].

## 6. Links (OSC 8)

- Markdown links, file paths in tool output, wrapped long URLs and footer PR badges are emitted as OSC 8 hyperlinks when the terminal supports them [CL 2.1.2], [CL 2.1.47], [CL 2.1.113], [CL 2.1.217]; without support they render as `label (url)` [CL 2.1.128]. `FORCE_HYPERLINK=0` opts out [CL 2.1.217], [ENV].
- Observed form: `ESC ] 8 ; id=zaxmda ; https://code.claude.com/docs/en/security ESC \ … ESC ] 8 ; ; ESC \` (with `id=`, ST-terminated) [OBS]. Emitted with `TERM=xterm-kitty`, **not** with `TERM=xterm-256color` (support is detected from the environment).
- Opening: in classic mode the terminal opens OSC 8 links itself. In fullscreen, Claude Code receives the click: plain click does **not** open; since [CL 2.1.181] **Ctrl+click** (Linux/Windows) or Cmd+click (macOS) opens http(s) URLs in the browser and file paths in the default app [FS]. In xterm.js terminals (VS Code) it defers to the terminal's own link handler [FS]. Unsafe targets (UNC/network paths, control chars) render as plain text [CL 2.1.247].

## 7. Shift-held selection

Claude Code does not special-case Shift+drag: in Kitty and Alacritty, holding Shift makes the terminal suppress mouse reporting and do its own selection.

- Alacritty: "When an application running within Alacritty captures the mouse, the `Shift` modifier can be used to suppress mouse reporting." [ALA]
- kitty: `terminal_select_modifiers` defaults to `shift` (select even when the program grabbed the mouse) [KIT].
- Claude Code docs list "Most other terminals: Shift" (Terminal.app `Fn`, iTerm2 `Option`) and show the right key in an on-screen hint [FS], [CL 2.1.161]; observed hint text above [OBS].
- A native selection is invisible to Claude Code; a fix ensures `Ctrl+C` after a native (modifier+drag) selection no longer overwrites the clipboard with the app's previous in-app selection [CL 2.1.181].

## Implications for the orchestrator (inference, not sourced)

- If the embedded Claude session runs fullscreen, the orchestrator's vt100 parser will see `?1049h` + `?1000/1002/1003/1006h`; to give the user Claude's own selection/copy/click/wheel, the orchestrator must forward SGR mouse reports (translated to pane-local coordinates) while those modes are set, including motion events for 1003.
- Copy-on-select from an embedded session will normally go through `wl-copy`/`xclip` (the child inherits `WAYLAND_DISPLAY`/`DISPLAY`), so it works without the orchestrator handling OSC 52. OSC 52 only appears when no display is available or over SSH; then the orchestrator would need to pass it through to the outer terminal.
- OSC 8 emission depends on the `TERM`/terminal detection the orchestrator presents to the child; with `xterm-256color` no hyperlinks were emitted.
- The alternative is forcing classic (`CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1`) or `CLAUDE_CODE_DISABLE_MOUSE=1`, leaving selection/scroll entirely to the orchestrator.

## Method (OBS)

`python3` with `pty.fork()` running `claude` in `/tmp/claude-1000` (outside the repo, to avoid sandbox-blocked project settings), window 120x40, `TERM=xterm-kitty` unless stated, keystrokes `ESC[B` then `\r` at 4-5 s to accept the trust dialog, capture ~13 s, SIGTERM, regex over the byte stream for `ESC[?Nh/l`, `ESC]52;`, `ESC]8;`, `ESC[>Nu`. No prompt was sent to the model. Network was sandboxed (MCP/telemetry hosts blocked), which does not affect terminal setup.
