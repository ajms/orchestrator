# Emulator and terminal capabilities for mouse passthrough and selection

Research for [#22](https://github.com/ajms/orchestrator/issues/22) (map: [#20](https://github.com/ajms/orchestrator/issues/20)).
Researched 2026-09-29 against the versions in the lockfile: vt100 0.16.2, tui-term 0.3.4, crossterm 0.29.0.
Crate claims were checked in the vendored sources under `~/.cargo/registry/src/index.crates.io-*/`; paths below are relative to each crate root.

## TL;DR

- **vt100 0.16 tracks the Agent's mouse mode and encoding** (`Screen::mouse_protocol_mode()` / `mouse_protocol_encoding()`), and the Holder's snapshots already carry them to the TUI mirror. It has **no mouse encoder**; we write one (~40 lines, spec in xterm ctlseqs).
- **OSC 52 from the Agent is surfaced, not swallowed, but only through a `Callbacks` impl.** Both of our parsers use `()` callbacks, so today the Agent's OSC 52 copies are silently dropped.
- **OSC 8 hyperlinks are lost.** vt100 passes them to `unhandled_osc` and stores nothing in cells.
- **Alternate-screen state is exposed** (`Screen::alternate_screen()`), and the Holder already uses it.
- **tui-term has no mouse or selection helpers.** It only renders `Screen`/`Cell` traits. We draw selection highlight ourselves, either by patching the ratatui `Buffer` after render or by wrapping the `Screen` trait.
- **crossterm 0.29 `EnableMouseCapture` sets DECSET 1000, 1002, 1003, 1015 and 1006** (all-motion plus SGR). It decodes SGR into `MouseEvent { kind, column, row, modifiers }` but **cannot re-encode**. Back and forward buttons (cb ≥ 128) are rejected.
- **Kitty and Alacritty both let Shift bypass app mouse capture**, and both allow OSC 52 *write* by default. Reading is ask-first in kitty and disabled in Alacritty. Both support primary selection on X11/Wayland (OSC 52 selector `p`, which Alacritty also accepts as `s`).
- **tmux and zellij use the same pattern.** The multiplexer captures the mouse. If the pane's app has requested mouse tracking, the event is re-encoded pane-relative in the app's requested encoding and forwarded. Otherwise the multiplexer runs its own pane-local selection and copies on release. Outer-terminal Shift-selection is the escape hatch, but it is screen-wide, not pane-local.
- **alacritty_terminal would add** semantic word, line and block selection, `selection_to_string`, and per-cell hyperlinks. It would not add a mouse encoder (that lives in the alacritty app crate), and it lacks vt100's `contents_formatted` / `state_formatted` snapshot serialisation, which the Holder→TUI protocol depends on.

## vt100 0.16.2

### Mouse protocol mode and encoding: exposed

- `src/screen.rs:12` `pub enum MouseProtocolMode { None, Press, PressRelease, ButtonMotion, AnyMotion }` maps DECSET 9, 1000, 1002 and 1003 (`decset`, `src/screen.rs:1139-1173`).
- `src/screen.rs:40` `pub enum MouseProtocolEncoding { Default, Utf8, Sgr }` maps DECSET 1005 and 1006. **Not handled:** 1015 (urxvt), 1016 (SGR-pixels), 1004 (focus events). They fall through to `unhandled`, i.e. `Callbacks::unhandled_csi`.
- Accessors: `Screen::mouse_protocol_mode()` (`src/screen.rs:578`) and `Screen::mouse_protocol_encoding()` (`src/screen.rs:584`).
- Mode and encoding are single slots. `DECRST n` resets the slot only if it currently holds that mode (`clear_mouse_mode`, `src/screen.rs:687`). So `?1000h ?1002h ?1002l` leaves mode `None`, not `PressRelease`. That differs subtly from xterm, which keeps independent flags, but it is fine for Ink-style apps that set and clear symmetrically.
- `Screen::input_mode_formatted()` (`src/screen.rs:379`) re-emits mouse mode and encoding (`src/term.rs:454-540`). The Holder's `ScreenSnapshot.input_modes` is built from it (`crates/orch-holder/src/screen.rs`, `ScreenCopy::capture`). A restored TUI mirror therefore knows the Agent's mouse mode. Live output also flows through `PaneMirror::output`, so `PaneMirror::screen().mouse_protocol_mode()` is always current. **The TUI can decide passthrough per pane with no protocol change.** Only `orch_term::keys::InputModes` would need two more fields if we want it there.
- No encoder exists. vt100 is a parser/screen only.

### OSC 52: surfaced via `Callbacks`, dropped by us today

- `src/perform.rs:198-238` `osc_dispatch`: `OSC 52 ; <sel> ; ?` calls `Callbacks::paste_from_clipboard(screen, sel)`. `OSC 52 ; <sel> ; <base64>` calls `Callbacks::copy_to_clipboard(screen, sel, base64)`, after checking that `sel` is in the clipboard-selector set and the data is valid base64. Anything malformed goes to `unhandled_osc`.
- `src/callbacks.rs` defines the `Callbacks` trait (bell, resize, title, `copy_to_clipboard`, `paste_from_clipboard`, `unhandled_*`). `impl Callbacks for ()` is no-op.
- `Parser::new` uses `()`. `Parser::new_with_callbacks(rows, cols, scrollback, cb)` installs one (`src/parser.rs:29`).
- **Ours:** `orch-holder::Emulator` and `orch-tui::PaneMirror` both use `vt100::Parser::new` (`crates/orch-holder/src/screen.rs:18`, `crates/orch-tui/src/pane.rs:23`). So an Agent's OSC 52 copy never reaches the user's terminal. Forwarding it means giving `PaneMirror` a `Parser<MirrorCallbacks>`. `ScreenSnapshot::restore` currently returns `vt100::Parser` (i.e. `Parser<()>`) and would need a generic or a callbacks argument. Snapshots contain only `contents_formatted` and `input_mode_formatted`, never OSC 52, so a restore will not replay copies.

### OSC 8 hyperlinks: lost

- `osc_dispatch` has no `8` arm, so it goes to `unhandled_osc` (`src/perform.rs:234`). `Cell` (`src/cell.rs`) has no link field, and `grep -ri link src/` finds nothing. Link text renders as plain text. A `Callbacks::unhandled_osc` impl could track open/close against the cursor position, but that is a hack. tui-term cannot render links anyway (see below).

### Alternate screen: exposed

- `Screen::alternate_screen()` (`src/screen.rs:548`) covers DECSET 47 and 1049 (1047 is not handled). `contents_formatted` does **not** emit the alt-screen switch. The Holder works around that with `ALT_SCREEN_ENTER` and a saved `main` screen (`crates/orch-holder/src/screen.rs`).

### Selection helpers

- `Screen::contents_between(start_row, start_col, end_row, end_col)` (`src/screen.rs:167`) returns the text over *visible* rows, honouring the scrollback offset and joining wrapped rows. It is documented as "useful for things like determining the contents of a clipboard selection". `Screen::row_wrapped(row)` and `Screen::cell(row, col)` exist too. **There is no word or line boundary logic.** Double-click word and triple-click line selection must be written by hand over `cell()`. `PaneMirror::text_of_lines` already does multi-page extraction by temporarily moving the scrollback offset.

## tui-term 0.3.4

- The public surface is `PseudoTerminal` (widget), `Cursor`, the `Screen` / `Cell` traits (`src/widget.rs:23,56`), a vt100 impl (`src/vt100_imp.rs`) and a `Controller` (PTY runner, unused by us). `grep -iE 'mouse|select|link'` over `src/` hits only cursor-shape names. **There are no mouse, selection or hyperlink helpers.**
- Two ways to draw a selection highlight:
  1. After `PseudoTerminal` renders, walk the selected cells in the ratatui `Buffer` and add `Modifier::REVERSED` or a style.
  2. Implement `tui_term::widget::Screen` for a wrapper around `&vt100::Screen` plus a selection, whose `Cell::apply` adds the highlight.

  Option 1 is simpler. We probably already do something like it for keyboard-mode selection.

## crossterm 0.29.0

- `EnableMouseCapture::write_ansi` (`src/event.rs:321-335`) writes `CSI ?1000h ?1002h ?1003h ?1015h ?1006h`. `DisableMouseCapture` turns them off in reverse. That means any-motion tracking with SGR, falling back to urxvt. `Command` is just bytes, so we can emit a narrower set (e.g. `?1000h ?1002h ?1006h` without 1003) to cut motion traffic. We would then have to switch 1003 on dynamically when a pane's Agent asks for `AnyMotion`.
- Event types (`src/event.rs:777-830`): `MouseEvent { kind, column, row, modifiers }` (0-based, screen-absolute). `MouseEventKind::{Down(b), Up(b), Drag(b), Moved, ScrollDown, ScrollUp, ScrollLeft, ScrollRight}`, `MouseButton::{Left, Right, Middle}`, and `KeyModifiers` SHIFT, ALT and CONTROL are decoded from cb bits 2, 3 and 4.
- The SGR parser is `parse_csi_sgr_mouse` (`src/event/sys/unix/parse.rs:718`), and `parse_cb` is at `:776`. Losses:
  - Buttons 8–11 (back/forward, cb ≥ 128) make `parse_cb` return an error, so those events are dropped.
  - In non-SGR modes, release is always reported as `Up(Left)`.
  - `Moved` does not say which button (none) was held; that is fine.
- **There is no encoder.** Nothing in the crate turns a `MouseEvent` back into bytes. We re-encode ourselves, per [xterm ctlseqs, "Mouse Tracking"](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Mouse-Tracking):
  - cb = button (0 left, 1 middle, 2 right, 3 release in legacy encodings), 64/65 wheel up/down, 66/67 wheel left/right, +32 for motion, +4 shift, +8 alt (meta), +16 ctrl. Plain motion with no button is 35 (3 + 32).
  - **SGR (1006):** `ESC [ < cb ; x+1 ; y+1 M` for press and motion, `…m` for release (with the real button number).
  - **Default (X10/normal):** `ESC [ M` followed by bytes `32+cb`, `32+x+1`, `32+y+1`. Coordinates above 222 cannot be sent.
  - **UTF-8 (1005):** like Default, but each coordinate byte is UTF-8-encoded, up to 2015.
  - **Filter by `MouseProtocolMode`:**
    - `Press`: presses only (X10: no modifiers, no release).
    - `PressRelease`: presses, releases and wheel.
    - `ButtonMotion`: also motion while a button is held.
    - `AnyMotion`: all motion.
    - `None`: nothing.
  - Translate coordinates to pane-inner coordinates, and drop events outside the pane.

  Alacritty's `mouse_report` / `sgr_mouse_report` / `normal_mouse_report` in [alacritty/src/input/mod.rs](https://github.com/alacritty/alacritty/blob/master/alacritty/src/input/mod.rs) are a compact reference implementation. tmux's `input_key_get_mouse` in [input-keys.c](https://github.com/tmux/tmux/blob/master/input-keys.c) is another.

## Kitty

Sources: [kitty.conf docs](https://sw.kovidgoyal.net/kitty/conf/), [`kitty/options/definition.py`](https://github.com/kovidgoyal/kitty/blob/master/kitty/options/definition.py), and [kitty.conf(5), Ubuntu noble](https://manpages.ubuntu.com/manpages/noble/man5/kitty.conf.5.html).

- **Shift bypass:** `mouse_map` has *grabbed* and *ungrabbed* modes. "`grabbed` refers to when the program running in the terminal has requested mouse events." Default maps bind `shift+left press ungrabbed,grabbed mouse_selection normal` (start selecting even when grabbed), `shift+left doublepress … mouse_selection word`, `shift+left triplepress … mouse_selection line`, and `shift+middle release ungrabbed,grabbed paste_selection`. So Shift-drag selects natively even while orch captures the mouse, but across the whole window, not per pane.
- **OSC 52:** `clipboard_control` defaults to `write-clipboard write-primary read-clipboard-ask read-primary-ask`. Writes are allowed, and reads prompt the user. `clipboard_max_size` defaults to 512 MB. Our current `\e]52;c;…\a` (`crates/orch-tui/src/clipboard.rs`) works by default.
- **Primary selection:** yes, via `write-primary` and `read-primary`, and middle-click `paste_from_selection`. `copy_on_select` defaults to `no` (values `clipboard` or a private buffer name). Native selection already goes to primary on Linux.

## Alacritty

Source: [alacritty config docs](https://alacritty.org/config-alacritty.html).

- **Shift bypass:** "When an application running within Alacritty captures the mouse, the Shift modifier can be used to suppress mouse reporting. If no action is found for the event, actions for the event without the Shift modifier are triggered instead."
- **OSC 52:** `terminal.osc52 = "Disabled" | "OnlyCopy" | "OnlyPaste" | "CopyPaste"`, default `"OnlyCopy"`. Write works and read is refused by default.
- **Primary selection:** selector `c` maps to `ClipboardType::Clipboard`, and `p` or `s` map to `ClipboardType::Selection` (`clipboard_store` / `clipboard_load` in [alacritty_terminal/src/term/mod.rs](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/term/mod.rs)). Native selection goes to primary. `selection.save_to_clipboard` (default `false`) also copies it to the clipboard.
- **Hyperlinks:** OSC 8 links are shown as hints (`hints.enabled[].hyperlinks = true`).

## alacritty_terminal (the fallback emulator)

Sources: docs.rs ([latest is 0.26.0](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/)) and the vendored `vte-0.15.0/src/ansi.rs`, the parser alacritty_terminal is built on.

| Question | vt100 0.16 | alacritty_terminal |
|---|---|---|
| Agent mouse mode/encoding | `mouse_protocol_mode/encoding()` | `Term::mode()` → `TermMode::{MOUSE_REPORT_CLICK, MOUSE_DRAG, MOUSE_MOTION, SGR_MOUSE, UTF8_MOUSE, MOUSE_MODE}` ([TermMode](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/term/struct.TermMode.html)) |
| Mouse encoder | none | none; it is in the alacritty app crate, not `alacritty_terminal` |
| OSC 52 | `Callbacks::copy_to_clipboard/paste_from_clipboard` | `EventListener` gets `Event::ClipboardStore(ClipboardType, String)` / `Event::ClipboardLoad(ClipboardType, formatter)` ([Event](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/event/enum.Event.html)) |
| OSC 8 | dropped | kept per cell: `Cell::hyperlink() -> Option<Hyperlink{id, uri}>` ([Cell](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/term/cell/struct.Cell.html)); vte `ansi.rs:1393` parses it |
| Alt screen | `alternate_screen()` | `TermMode::ALT_SCREEN` |
| Selection | `contents_between` only | `Term.selection: Option<Selection>`, `SelectionType::{Simple, Block, Semantic, Lines}`, `selection_to_string()`, `bounds_to_string()`, `semantic_search_left/right()`, `line_search_left/right()` ([Term](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/term/struct.Term.html), [SelectionType](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/selection/enum.SelectionType.html)) |
| State as escape bytes (our snapshot/mirror protocol) | `state_formatted`, `contents_formatted`, `rows_formatted`, `input_mode_formatted` | **none**; it would need our own serialiser or a different mirror protocol |
| tui-term rendering | built in | needs our own `tui_term::widget::Screen` impl |

**Verdict:** switching would give us word, line and block selection plus hyperlinks for free. The things this ticket needs (mode detection, OSC 52 surfacing, alt-screen) are already answered by vt100. Switching would cost us the byte-level snapshot serialisation that the Holder→TUI mirror is built on, plus a tui-term adapter. Semantic and line selection over `vt100::Screen::cell()` is small. **Nothing here forces the fallback.** The one thing vt100 cannot do at all is OSC 8.

## How multiplexers do it

### tmux

Sources: tmux(1) 3.7c "MOUSE SUPPORT" and `set-clipboard`, [key-bindings.c](https://github.com/tmux/tmux/blob/master/key-bindings.c) and [input-keys.c](https://github.com/tmux/tmux/blob/master/input-keys.c).

- With `mouse on`, tmux captures the outer mouse and turns events into bindable keys with a location suffix (`MouseDrag1Pane`, `WheelUpPane`, …). `send-keys -M` forwards the current mouse event to the pane.
- The default bindings are the whole policy:
  - `MouseDown1Pane { select-pane -t=; send -M }` always focuses the pane and forwards the press.
  - `MouseDrag1Pane { if -F '#{||:#{pane_in_mode},#{mouse_any_flag}}' { send -M } { copy-mode -M } }` forwards the drag if the app wants the mouse; otherwise it starts **pane-local copy mode** with a mouse selection.
  - `WheelUpPane { if -F '#{||:#{alternate_on},#{pane_in_mode},#{mouse_any_flag}}' { send -M } { copy-mode -e } }` forwards the wheel to mouse apps *and to alt-screen apps*; otherwise it scrolls tmux's own history.
  - `DoubleClick1Pane` / `TripleClick1Pane` use the same test, else `copy-mode -H; send -X select-word|select-line; run -d0.3; send -X copy-pipe-and-cancel`.
  - `MouseDown2Pane` forwards or `paste -p`.
  - In copy mode: `MouseDragEnd1Pane { send -X copy-pipe-and-cancel }`. That is **copy-on-release**, which stores the text in a tmux buffer and sets the outer clipboard via OSC 52.
- `input_key_get_mouse` re-encodes per pane:
  - It drops motion unless the pane is in button or all mode, and drops releases in X10 mode.
  - It writes SGR `\033[<%u;%u;%u%c` when the pane set 1006, else UTF-8 or legacy `\033[M`, clamping legacy coordinates.
  - Coordinates are made pane-relative.
- `set-clipboard on|external|off`: `on` accepts the app's OSC 52 into a tmux buffer *and* forwards it to the outer terminal. `external` only forwards it. The man page does not state the default; I did not verify it.

### zellij

Sources: [Options](https://zellij.dev/documentation/options.html), [Compatibility](https://zellij.dev/documentation/compatibility), and source files [`panes/terminal_pane.rs`](https://github.com/zellij-org/zellij/blob/main/zellij-server/src/panes/terminal_pane.rs) and [`panes/grid.rs`](https://github.com/zellij-org/zellij/blob/main/zellij-server/src/panes/grid.rs).

- Options:
  - `mouse_mode` (default true): zellij captures the mouse.
  - `copy_on_select` (default true): "automatic copy of selection when releasing mouse".
  - `copy_command`: pipe to e.g. `wl-copy` instead of OSC 52.
  - `copy_clipboard = system | primary`.
- Its own emulator (`grid.rs`) tracks `MouseTracking::{Off, Normal, ButtonEventTracking, AnyEventTracking}` and `MouseMode::{NoEncoding, Utf8, Sgr}` from DECSET 1000/1002/1003/1005/1006. It re-encodes per pane (`Grid::mouse_event_signal`, `mouse_left_click_signal`).
- `TerminalPane::terminal_emulator_wants_mouse()` is `grid.mouse_tracking != Off`. That is the switch between forwarding to the app and doing zellij's own pane-local selection. zellij also tracks OSC 8 (`link_handler`) and OSC 52 (`pending_clipboard_update`).
- The escape hatch is the *outer* terminal's Shift: "If `mouse_mode` is turned on zellij handles these events, zellij provides an escape mechanism in the form of the `SHIFT` Key, once it is pressed zellij lets the terminal handle selection, clicking on links, copying, scrolling."
- OSC 52 write is passed through and advertised in DA1. OSC 52 *read* is disabled by default since 0.45.0 (`dangerously_enable_paste_buffer_read`).

## Implications for orch (inputs to #23, not decisions)

1. Orch does not enable mouse capture today (`crates/orch-tui/src/runtime.rs` enters the alt screen and raw mode only). Native terminal selection therefore works now, but it spans panes and the sidebar.
2. The tmux/zellij pattern maps directly onto what we have:
   - Capture the mouse.
   - Hit-test the pane.
   - If `PaneMirror::screen().mouse_protocol_mode() != None` (and, for the wheel, maybe also if `alternate_screen()`), re-encode pane-relative per `mouse_protocol_encoding()` and send it as Agent input.
   - Otherwise run orch's own pane-local selection on the vt100 mirror and copy on release via our existing OSC 52 effect.
   - Shift remains the user's native escape hatch in kitty and Alacritty.
3. Forwarding the Agent's own OSC 52 needs a `Callbacks` impl on the TUI mirror. It then works by default in kitty and Alacritty (both allow write). Answering OSC 52 *reads* should stay unsupported, as Alacritty, zellij and kitty all refuse or ask.
4. For primary-selection parity on Linux, emit `OSC 52 ; p ; …` alongside `c`. Kitty (`write-primary`) and Alacritty (`p` → Selection) both honour it by default.
5. OSC 8 hyperlinks are not achievable with vt100 without a hack. They are the only capability here that argues for alacritty_terminal.

## Open questions

- Which DECSET mouse modes does Claude Code's TUI actually request, if any, and when? This was not verified. The next step is to record a session's PTY output with `script` and grep for `\e[?100[0-6]h`.
