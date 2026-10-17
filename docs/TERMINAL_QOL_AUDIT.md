# Terminal quality-of-life audit

Audit of the in-world Terminal block (the kernel console drawn by
`TerminalScreen`) and the GUI Terminal app (`ecm_apps::terminal`, used by the
desktop and the ePhone) against what a player expects from a real terminal
and shell (xterm/VTE + bash/readline + man/less). Written 2026-10-16 against
`staging` @ eb20e09c.

## Where each piece lives

| Piece | Code | Used by |
|---|---|---|
| Line editor (prompt input) | `rust/operating-system/rust/src/lineedit.rs` | Block console, GUI Terminal app and SSH (all shell sessions) |
| Shell (builtins, launch, pipes) | `operating-system/.../shell.rs`, `parse.rs` | same |
| Job control, Ctrl+T / Ctrl+C | `kernel.rs` (console), `sessions.rs` (GUI Terminal app, SSH), `jobs.rs` | |
| Terminal emulator (VT parser, scrollback rows) | `rust/crates/ecm-vte` | Console (cells in the shared framebuffer), GUI Terminal app (virtual grid) |
| Block console keys, mouse, clipboard | `src/main/java/.../block/TerminalScreen.java` | Terminal block |
| GUI Terminal window | `rust/crates/ecm-apps/src/terminal.rs` | Desktop and phone Terminal app |
| Python / Lua REPL input | `read_line_echo` in `wasm-programs/python` and `wasm-programs/lua` | REPLs |
| `help` | `wasm-programs/help` (one flat `println!` list) | |

Legend: **OK** works like a real terminal, **Partial** some of it, **Missing**.
"Console" = the Terminal block, "App" = the GUI Terminal app (desktop/phone).

## 1. Line editing at the prompt

| Feature (bash/readline) | Console | App | Notes |
|---|---|---|---|
| Left/Right move the cursor inside the line | Missing | Missing | The editor ignores CSI C/D; the line is append-only. |
| Insert in the middle, Backspace/Delete at the cursor | Missing | Missing | Backspace only pops the last char; Delete (`ESC[3~`) is ignored. |
| Home/End, Ctrl+A / Ctrl+E | Missing | Missing | The block screen doesn't send Home/End at all; Ctrl+A/E reach the editor and are dropped. |
| Word jumps: Ctrl+Left/Right, Alt+B/F | Missing | Missing | The block screen sends plain arrows for Ctrl+arrows; the app too. |
| Kill/yank: Ctrl+U, Ctrl+K, Ctrl+W, Alt+D, Alt+Backspace, Ctrl+Y | Missing | Missing | The block screen doesn't even send Ctrl+U/W/Y (only A D E F K R S T X are mapped). |
| Ctrl+L clears the screen keeping the line | Missing | Missing | |
| Ctrl+C abandons the line | Missing | OK | The console ignores 0x03 at the prompt. |
| Long lines wrapping past the right edge | Partial | Partial | Typing wraps, but Backspace can't go back up a row. |
| Ctrl+D on an empty line / delete char | Missing | Missing | |
| Transpose (Ctrl+T) | n/a | n/a | Ctrl+T is the mod's "terminate"; keep it. |

## 2. History

| Feature | Console | App | Notes |
|---|---|---|---|
| Up/Down recall | OK | OK | 100 entries, duplicates of the last entry skipped. |
| Ctrl+P / Ctrl+N | Missing | Missing | |
| Ctrl+R reverse incremental search | Missing | Missing | |
| `history` builtin, `!!`, `!n`, `!prefix` | Missing | Missing | |
| History kept across reboots / sessions | Missing | Missing | Lost on every reboot; each session has its own. |
| Edited history entry keeps the draft | OK | OK | |

## 3. Tab completion

| Feature | Console | App | Notes |
|---|---|---|---|
| Complete a command name (builtins, `bin/`, `server-bin/`) | Missing | Missing | Tab is ignored by the editor. |
| Complete file and directory paths | Missing | Missing | |
| Double Tab lists the candidates | Missing | Missing | |
| Completion of `help`/`man` topics | Missing | Missing | |

## 4. Selection, copy and paste

| Feature | Console | App | Notes |
|---|---|---|---|
| Drag to select | Partial | OK | Console: the selection is tracked but **never drawn** (`SELECTION_COLOR` is unused), so the player can't see it. |
| Double-click word, triple-click line | Missing | Missing | |
| Copy to the real clipboard | Partial | OK | Console: Ctrl+C copies only while a selection exists (invisible). No Ctrl+Shift+C, no Ctrl+Insert, no right-click menu. App: Ctrl+Shift+C and right-click Copy. |
| Paste from the real clipboard | Partial | OK | Console: Ctrl+V only (no Ctrl+Shift+V / Shift+Insert / right click). CRLF pastes run an extra empty line; a Tab in a paste would act as a key. |
| Selecting in the scrollback | Missing | OK | The console has no scrollback view to select from. |
| Selection survives new output / is cleared sensibly | Partial | OK | |
| Bracketed paste (pasted text never triggers keys) | Missing | Missing | |

## 5. Scrollback and scrolling

| Feature | Console | App | Notes |
|---|---|---|---|
| Mouse wheel scrolls back | Missing | OK | Console: the VTE keeps 500 rows but nothing can show them (`mouseScrolled` is a TODO). |
| Shift+PgUp / Shift+PgDn | Missing | OK | |
| Shift+Home / Shift+End (top / bottom) | Missing | Missing | |
| New output keeps a scrolled-back view in place | n/a | OK | |
| Typing jumps back to the live view | n/a | OK | |
| A scroll position indicator | Missing | Missing | |
| Wheel in full-screen programs (alternate screen) sends arrows | Missing | Missing | Real terminals send Up/Down to `less`/editors. |

## 6. Terminal emulation (ecm-vte, shared)

| Feature | Status | Notes |
|---|---|---|
| Cursor moves, CUP, erase line/display, SGR 16 colours, bold/underline/inverse/blink, alt screen, save/restore, scroll S/T, IND/RI | OK | |
| 256-colour and truecolor SGR (`38;5;n`, `38;2;r;g;b`, `48;…`) | Missing (buggy) | The parameters are read as separate SGRs: `38;5;196` turns **blink** on. Map to the nearest of the 16 colours. |
| OSC (`ESC ] … BEL/ST`, window titles) | Missing (buggy) | `]` is treated as an unknown escape and the rest of the title is **printed**. |
| Insert/delete characters and lines (ICH `@`, DCH `P`, ECH `X`, IL `L`, DL `M`) | Missing | Needed by any curses-style program. |
| Scroll region (DECSTBM `r`) | Missing | |
| `CSI ? a;b h` with several modes | Partial | Only the first parameter is used. |
| Device status report (`ESC[6n`), cursor/size queries | Missing | No reply path to the program. |
| VPA/HPA/REP/CBT (`d`, `` ` ``, `b`, `Z`) | Missing | |
| UTF-8 output | Missing | Each byte of a multi-byte character becomes its own blank cell, so lines drift. Decode and draw one cell (ASCII fallback for box drawing, `?` otherwise). |
| Resize: text kept, prompt redrawn | OK | Rows are cut, not reflowed. |
| Reflow of wrapped lines on resize | Missing | Needs soft-wrap flags per row. |
| Bell (BEL) | Missing | Ignored; could click on the Speaker or flash. |

## 7. Programs, job control and keys

| Feature | Console | App | Notes |
|---|---|---|---|
| Ctrl+C stops the foreground program | Missing | OK | Console: only Ctrl+T works; Ctrl+C is sent to the program, which ignores it. |
| Full-screen programs get Ctrl+C themselves | n/a | Missing | The app kills on Ctrl+C even while `edit` (alternate screen) uses Ctrl+C for copy. |
| `&`, `jobs`, `fg`, `ps`, `kill` | OK | OK | |
| Pipes `|` and redirects `>` `>>` `<` `2>` | Missing (silently) | Missing (silently) | The shell parses them, but the Java host's `pipe_create` and `fd_open` were stubs and `process_spawn` ignored the stdio it was given: `ls > f` printed to the screen and made no file, `a | b` said "Pipes are not supported by this host". `grep` and `wc` (stdin only) could never get input. |
| Ctrl+Z suspend, `bg` resume | Missing | Missing | `bg` says jobs already run in the background. |
| Exit status in the prompt / `$?` | Partial | Partial | "Process exited with code N" is printed; no `$?`. |
| Esc reaches the program | Missing | OK | Console: Esc always closes the screen (kept: players expect Esc to close a GUI). |
| F1–F12, Insert, PgUp/PgDn to programs | Missing | Partial | |
| REPL line editing (Python, Lua) | Missing | Missing | `read_line_echo` handles Backspace only: no cursor, no history. |

## 8. Help, manual pages, pager

| Feature | Status | Notes |
|---|---|---|
| `help` overview | Partial | One long flat list (~100 lines) that scrolls off a 25-row console, no headings styling, partly out of date (no `lua`, `dig`, `named`, `pkg`, `store`, `chat`…). |
| `help <topic>` | Missing | Arguments are ignored. |
| `man <command>` | Missing | No `man` program and no pages. |
| `man -k` / apropos search | Missing | |
| A pager (`less`/`more`) | Missing | Long output can't be paged; with no scrollback on the console it is simply lost. |
| `<command> --help` | Partial | 23 of ~100 programs answer `--help`. |

## What to build, in order

1. **Line editor rewrite** (shared by console, GUI Terminal and SSH): a cursor inside the line with
   wrapping redraw, Left/Right, Home/End, Ctrl+A/E/B/F, word jumps (Ctrl+arrows, Alt+B/F), Delete,
   Ctrl+U/K/W/Y, Alt+D, Ctrl+L, Ctrl+C abandons the line, Ctrl+D, history with Ctrl+P/N and Ctrl+R
   search, `history`, `!!`/`!n`, history saved to disk, and Tab completion of commands and paths
   (double Tab lists). Put it in a crate (`ecm-lineedit`) so the Python and Lua REPLs get the same
   editing and history.
2. **Console scrollback**: the console keeps a scrolled view of its 500 rows; Shift+PgUp/PgDn,
   Shift+Up/Down, Shift+Home/End and the mouse wheel scroll it; typing returns to the live view; a
   position indicator in the corner. The wheel sends arrows to full-screen programs.
3. **Block screen selection and clipboard**: draw the selection, double/triple click, Ctrl+Shift+C,
   Ctrl+Insert, Ctrl+Shift+V, Shift+Insert, right click copies or pastes; every Ctrl+letter sent;
   xterm sequences for Home/End/PgUp/PgDn/Insert/F-keys and modified arrows; bracketed paste.
4. **Ctrl+C** stops the foreground program on the console as in the app; a program on the alternate
   screen receives Ctrl+C itself (both).
5. **`help`** with headings and `help <topic>`, and **`man`** with a page per command and per
   feature (shell keys, terminal, networking, radio, GUI, phone, scripting), `man -k`, a built-in
   pager (Space/b/arrows/`/` search/`q`) and a `less` program for files.
6. **VTE fixes**: OSC, 256/truecolor SGR, ICH/DCH/ECH/IL/DL, DECSTBM, multi-mode `?h/l`, UTF-8.
7. Later: reflow on resize, Ctrl+Z/`bg`, DSR replies, `$?`, bell.

## What this branch built (feature/terminal-qol)

| Area | Where | Done |
|---|---|---|
| Line editor | `rust/crates/ecm-lineedit` | Cursor, word jumps, Home/End, Delete, Ctrl+W/U/K/Y, Alt+D/B/F, Ctrl+L, Ctrl+C/D, wrapping redraw, history with Ctrl+P/N/Ctrl+R and `!!`/`!N`/`!-N`/`!text`, Tab completion with listing, bracketed paste. Used by the console, every shell session (GUI Terminal, SSH) and the Python and Lua REPLs (`stdio` module). |
| Shell | `operating-system/.../lineedit.rs`, `shell.rs` | Completion of commands and paths, `history [N|-c]`, history saved to `/.history` and shared, Ctrl+C stops the foreground program on the console too; a program on the alternate screen or with mode `?7727` gets Ctrl+C as a key. |
| Console scrollback | `console.rs`, `vte.rs` | Shift+PgUp/PgDn/Up/Down/Home/End and the wheel scroll the 500 rows; "-N" tag; live screen held while scrolled; typing returns. Full-screen programs get the wheel as arrows. |
| Terminal block screen | `TerminalScreen.java` | Selection drawn, double/triple click, Ctrl+Shift+C / Ctrl+Insert / right click copy, Ctrl+Shift+V / Shift+Insert / right click paste (bracketed), every Ctrl+letter, Alt+letter, xterm sequences with modifiers for arrows, Home/End, PgUp/PgDn, Insert/Delete, F1-F12, Shift+Tab, UTF-8 typing. |
| GUI Terminal app | `ecm-apps/src/terminal.rs` | Same key sequences, Ctrl+Insert copy, double/triple click, Shift+Up/Down/Home/End, "-N" tag, wheel as arrows in full-screen programs, bracketed paste, replies to cursor/size queries. |
| Emulator | `ecm-vte` | OSC/DCS skipped, 256/truecolor, ICH/DCH/ECH/IL/DL, DECSTBM, REP/VPA/HPA/CBT/CHT, several `?` modes at once, no scrollback on the alternate screen, DSR/size replies, UTF-8 one cell per character, mode tracking (`Modes`, `ModeScanner`). |
| Pipes and redirects | `ComputerInstance.java`, `wasi/SharedFd.java`, `ProcessManager.java` | `fd_open`, `pipe_create`, `fd_close` and the stdio of `process_spawn` are real; a pipe's reader gets end of input when the last writer closes; a redirected stdout reports a file / stream type. |
| Help and manual | `rust/crates/ecm-man`, `wasm-programs/{help,man,less}` | 115 pages (every command and feature), `help` with headings, `help <name>`, `man`, `man -k`, `man -l`, `man -P`, `/usr/share/man` overrides, a pager (Space/b/arrows/g/G//, n/N, h) and `less <file>`. |

Still open: reflow on resize, Ctrl+Z/`bg`, `$?`, the bell, Esc reaching programs on the Terminal block (Esc closes the screen; Shift+Esc sends Esc), and `less` at the end of a pipeline (keyboard input goes to a pipeline's first program).
