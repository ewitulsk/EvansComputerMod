# Vim on the computers

`vim` is the real Vim 9.2 (patch 1202), compiled from its C sources to a WASI
program (`wasm32-wasip1`) and run like every other program on a computer. The
player's guide is `man vim` / `man vi`. This page describes how the port is put
together and what it needed from the computer.

## Building

- Sources: `rust/third_party/vim` (Vim's `src/`, part of `runtime/`, the
  licence, and `README.ecm.md`, which lists the changes to Vim's files).
- Program: `rust/wasm-programs/vim`. `build.rs` compiles Vim's C files with
  wasi-sdk's clang (like `lua` and the browser's QuickJS: `WASI_SDK_PATH`,
  wasi-sdk 30) together with the port's files. `main.rs` only sets `HOME=/`
  and `TERM=xterm` when unset and calls Vim's `main`.
- `port/auto/config.h` replaces `configure`'s output: a Unix "huge" build
  without terminfo, X11, GUI, `:terminal`, jobs/channels, netbeans,
  clientserver, sound, language interfaces or `dlopen`.
- `port/wasi_shim.c` provides what wasi-libc lacks (see below).
  `port/include/` has a `termios.h`, `pwd.h` and `grp.h` for it.
- `scripts/stage-wasm.sh` builds `vim.wasm` and stages Vim's runtime files,
  the licence and the system vimrc (`rust/wasm-programs/vim/vimrc`) as
  `wasm-bin/vim/`. The jar carries them (`build.gradle`), `WasmManager`
  extracts them next to the programs (manifest lines under `vim/`), and every
  computer sees them read-only at `/server-bin/vim` (`$VIM`) and
  `/server-bin/vim/runtime` (`$VIMRUNTIME`, `port/pathdef.c`).
- `vim.wasm` is about 2.8 MB. Chicory now compiles modules up to 4 MiB to
  bytecode before they run (on the program's own thread, about 1.5 s for vim;
  before it was 2 MiB). A Vim started interpreted would stay interpreted, and
  would then be too slow to notice Ctrl+C between its own work. `WasmManager`
  also prewarms `vim.wasm` when the first computer boots, so the first launch
  doesn't wait.
- `main.rs` sets `VIM`/`VIMRUNTIME` when they are unset. Otherwise Vim would
  derive `$VIM` from `argv[0]` (a relative path to the `.wasm`) when the
  runtime is missing.

## What Vim needed from the computer

| Need | Primitive |
| --- | --- |
| Raw keys | Programs already get the keyboard raw from the kernel (no echo, no line editing). The shim keeps a `struct termios` for Vim's mode switches. In raw mode it maps the console's Enter (`\n`) to `\r`, as a tty without `ICRNL` would. While `ISIG` is off (or the tty isn't cooked), it sets the terminal's `?7727` mode, so Ctrl+C and Ctrl+Z stay Vim's keys even when Vim briefly leaves the alternate screen around `:!` and the hit-enter prompt. |
| Esc vs. key sequences (`ttimeout`) | `poll()` on stdin with a timeout. `PipeFd` (a program's stdin) is now a `Pollable`, so `poll_oneoff` wakes on a key instead of only timing out (`StdinPollAndReaddirTest`). |
| Window size and resizing | `TIOCGWINSZ` answers from `COLUMNS`/`LINES`, then from in-band resize reports. `ecm-vte` implements DEC mode `?2048` (`ESC[48;rows;cols;0;0t`, sent when set and on every resize). The kernel console passes the VTE's replies (`6n`, `18t`, `2048`) to the foreground program's stdin, as does the desktop's Terminal app (`session_write`). The shim takes the reports out of the key stream and raises `SIGWINCH` (wasi-libc's emulated signals), and Vim redraws. |
| Escape sequences | The xterm subset `ecm-vte` draws (`man terminal`). The builtin `xterm` termcap entry carries `t_ti`/`t_te` (`?1049`) and 16 colours, since there is no terminfo. Sequences the VTE doesn't know (`ESC[>4;2m`, cursor shapes) are ignored. |
| `isatty` | wasi-libc's `isatty` also wants the fd without seek rights. The shim checks for a character device, the way `ecm_man::tty::is_terminal` does. |
| File permissions | wasi-libc's `stat` gives only the file type. The shim adds `0644`/`0755`, read-only under `/server-bin`, otherwise every file opens `'readonly'`. |
| Telling files apart | `path_filestat_get`/`fd_filestat_get` now fill dev, inode (from the real path), nlink and the times. Vim keeps one buffer per dev/inode. `path_filestat_get` also finds `server-bin/` files, as `path_open` already did. |
| WASI calls the host lacked | `path_readlink` (no symlinks: EINVAL), `path_rename`, `fd_sync`/`fd_datasync`, `fd_filestat_set_size` and `path_filestat_set_times` now exist. Imports a host doesn't supply used to be stubbed to return 0, which WASI reads as success: `readlink` "succeeded" with a garbage length and Vim wrote a NUL far past its buffer, and `rename` (`mv`, Files, Python, Lua) silently did nothing. A missing WASI preview1 function now returns ENOSYS (`WasmRuntime.stubResult`, both runtimes). |
| Directory listings (`:e *`, `:runtime!`, completion) | `fd_readdir` now fills the buffer and cuts the last entry, as WASI specifies. Before, a short buffer made wasi-libc's `readdir` stop early. |
| `vi`, `view`, `vimdiff`, `ex` | Shell aliases (`PROGRAM_ALIASES` in `shell.rs`) run `vim` with the alias as `argv[0]`. A program of that name still wins. |

## Not available

- No processes: `:!`, `:shell`, `system()`, filters and `:make` give
  `E9999`.
- `:suspend`/Ctrl+Z give `E9998`.
- Ctrl+T is the computer's kill key and never reaches Vim.
- No clipboard registers, no mouse (the VTE has no mouse reporting), no
  `:terminal`.
- `'redrawtime'`/search timeouts never fire (no `SIGALRM`).
- At the Terminal, plain Esc closes the screen and Shift+Esc sends Esc to
  Vim (man keys).
- Keys typed within Vim's Esc timeout (`ttimeoutlen`, 100 ms with
  defaults.vim) after Esc join it, as on any terminal: Esc j is Alt+J. A
  Ctrl+C right after Esc only leaves Insert mode.
- Vim can't query the background colour (no termresponse without
  terminfo), so the system vimrc sets `background=dark`.

## Tests

- `cargo test -p terminal-os`: `aliases_start_their_program_under_their_own_name`,
  `console_replies_reach_the_foreground_program`.
- `cargo test -p ecm-vte`: `in_band_resize_reports`.
- `cargo test -p ecm-apps --test apps`: the Terminal app forwards resize
  reports.
- `cargo test -p ecm-man`: the `vim` and `vi` pages.
- JUnit `StdinPollAndReaddirTest`, and `KernelHostIntegrationTest.vimEditsAFileOnTheRealHost`
  (Vim on the real kernel and WASI host: editing, Ctrl+C, a console resize,
  Ctrl+Z, `:wq`).
- GameTests `ecm_vim`, the scenarios `vim_editing`, `vim_windows`,
  `vim_runtime` and `vim_resize` (`/ecm scenario spawn vim_editing`; see
  TESTING.md).
