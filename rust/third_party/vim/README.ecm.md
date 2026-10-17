# Vim for Evans Computer Mod computers

This folder holds Vim **9.2.1202** (tag `v9.2.1202` of
https://github.com/vim/vim), the parts the computers' `vim` program
(`rust/wasm-programs/vim`) is built from, with a few changes for WASI. Vim is
by Bram Moolenaar and the Vim community and is distributed under the Vim
license in `LICENSE` (kept unmodified, as it requires). It ships with the mod
as `server-bin/vim/LICENSE` next to `vim.wasm`.

This is a **modified Vim** (Vim license II.2.a). `:version` and the intro
screen say "Modified by Evans Computer Mod (WASI port)". The changes are
listed below and are all in this repository. Contact for them: the issue
tracker of https://github.com/ewitulsk/EvansComputerMod. The maintainers of
Vim may have them on request, under the Vim license.

## What is here

- `src/`: Vim's C sources and headers for a Unix build (`BASIC_SRC` of
  `src/Makefile`), `proto/` and `xdiff/`. Left out: the test suite
  (`testdir/`), translations (`po/`), the GUI, MS-Windows, macOS, VMS and
  Amiga sources, the language interfaces (`if_perl.c`, `if_python*.c`, ...),
  `libvterm/` (`:terminal`), `xxd/` and other tools.
- `runtime/`: part of Vim's runtime files, chosen by
  `scripts/vendor-vim-runtime.py`: `defaults.vim`, filetype detection,
  syntax, indent and filetype plugins for about fifty common languages,
  every colour scheme, `matchparen`, the `matchit`, `comment`, `nohlsearch`
  and `justify` packages, the English spell file and the main help files
  (`doc/tags` regenerated for them with `:helptags`). Left out: the other
  languages' syntax files, translations (`lang/`), tutor, `version9.txt`
  and the other release notes, `netrw`, keymaps, `print/` and the GUI menus.
- The port's own files are in `rust/wasm-programs/vim`: `port/auto/config.h`
  (what `configure` would generate), `port/pathdef.c`, `port/wasi_shim.c` and
  `port/include/` (termios, window size, keyboard input, no processes),
  `build.rs` (compiles all of it with wasi-sdk) and `vimrc` (the system
  vimrc, `background=dark`).

## Changes to Vim's files

Every change is marked `Evans Computer Mod` in the source and only affects
builds for WASI (`#ifdef __wasi__`):

| File | Change |
| --- | --- |
| `src/os_unix.c` | `mch_call_shell()` gives `E9999` instead of running a shell (there are no processes to start). `mch_set_shellsize()` does nothing (the terminal's size is set by its display panel). `start_timeout()`/`stop_timeout()` have a WASI version without `setitimer`/`SIGALRM` (the flag never fires). |
| `src/ex_docmd.c` | `:suspend`/Ctrl+Z give `E9998` (a program can't stop itself on the computer). |
| `src/errors.h` | The two messages above. |
| `src/term.c` | The builtin `xterm` entry carries what terminfo would add for the computer's console (no terminfo under WASI): the alternate screen (`t_ti`/`t_te` with `?1049`) and 16 colours (`t_Co`, `t_AF`, `t_AB`). |

## Updating

1. Unpack the new Vim release; copy `src/` as above (`*.c` of `BASIC_SRC`
   and `xdiff/`, every `*.h`, `proto/*.pro`) and run
   `python3 scripts/vendor-vim-runtime.py <vim>/runtime rust/third_party/vim/runtime`.
2. Apply the changes in the table again (search the old tree for
   `Evans Computer Mod`) and regenerate `runtime/doc/tags`
   (`vim -u NONE -es -c 'helptags runtime/doc' -c q`).
3. Update the source list in `rust/wasm-programs/vim/build.rs` if
   `BASIC_SRC` changed, the version here and in `man vim`, and build with
   `cargo build --release --target wasm32-wasip1 -p vim` (WASI_SDK_PATH set).
