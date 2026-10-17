//! Compile Vim's C sources (rust/third_party/vim/src) for wasm32-wasip1 with
//! wasi-sdk's clang, using the port's configuration (port/auto/config.h) and
//! WASI glue (port/wasi_shim.c). Host builds compile nothing (main.rs says so).

use std::path::PathBuf;

/// The files of Vim's Unix build (BASIC_SRC in src/Makefile), minus pathdef.c
/// (the port's own). Files for features this build leaves out compile to nothing.
const VIM_SRC: &[&str] = &[
    "alloc.c", "arabic.c", "arglist.c", "autocmd.c", "beval.c", "blob.c", "blowfish.c", "buffer.c",
    "change.c", "charset.c", "cindent.c", "clientserver.c", "clipboard.c", "cmdexpand.c", "cmdhist.c",
    "crypt.c", "crypt_zip.c", "debugger.c", "dict.c", "diff.c", "digraph.c", "drawline.c",
    "drawscreen.c", "edit.c", "eval.c", "evalbuffer.c", "evalfunc.c", "evalvars.c", "evalwindow.c",
    "ex_cmds.c", "ex_cmds2.c", "ex_docmd.c", "ex_eval.c", "ex_getln.c", "fileio.c", "filepath.c",
    "findfile.c", "float.c", "fold.c", "fuzzy.c", "getchar.c", "gc.c", "gui_xim.c", "hardcopy.c",
    "hardcopy_pango.c", "hardcopy_postscript.c", "hashtab.c", "help.c", "highlight.c", "if_cscope.c",
    "if_xcmdsrv.c", "image.c", "image_kitty.c", "image_sixel.c", "indent.c", "insexpand.c", "json.c",
    "linematch.c", "list.c", "locale.c", "logfile.c", "main.c", "map.c", "mark.c", "match.c",
    "mbyte.c", "memfile.c", "memline.c", "menu.c", "message.c", "misc1.c", "misc2.c", "mouse.c",
    "move.c", "normal.c", "ops.c", "option.c", "optionstr.c", "os_unix.c", "popupmenu.c", "popupwin.c",
    "profiler.c", "pty.c", "quickfix.c", "regexp.c", "register.c", "screen.c", "scriptfile.c",
    "search.c", "session.c", "sha256.c", "sign.c", "socketserver.c", "sound.c", "spell.c",
    "spellfile.c", "spellsuggest.c", "strings.c", "syntax.c", "tabpanel.c", "tag.c", "term.c",
    "terminal.c", "testing.c", "textformat.c", "textobject.c", "textprop.c", "time.c", "tuple.c",
    "typval.c", "ui.c", "undo.c", "usercmd.c", "userfunc.c", "version.c", "vim9class.c", "vim9cmds.c",
    "vim9compile.c", "vim9execute.c", "vim9expr.c", "vim9generics.c", "vim9instr.c", "vim9script.c",
    "vim9type.c", "viminfo.c", "window.c", "bufwrite.c",
    "xdiff/xdiffi.c", "xdiff/xemit.c", "xdiff/xprepare.c", "xdiff/xutils.c", "xdiff/xhistogram.c",
    "xdiff/xpatience.c",
];

/// wasi-libc's emulations Vim's Unix code leans on (raise/signal, getpid, clock, mmap).
const EMULATED: &[&str] = &["signal", "getpid", "process-clocks", "mman"];

fn main() {
    let here = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vim = here.join("../../third_party/vim/src");
    let port = here.join("port");
    println!("cargo:rerun-if-changed={}", port.display());
    println!("cargo:rerun-if-changed={}", vim.display());
    println!("cargo:rerun-if-env-changed=WASI_SDK_PATH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("wasi") {
        return;
    }
    let sdk = match std::env::var("WASI_SDK_PATH") {
        Ok(sdk) => sdk,
        Err(_) => panic!(
            "WASI_SDK_PATH is not set: building vim for WASM needs wasi-sdk \
             (https://github.com/WebAssembly/wasi-sdk/releases)"
        ),
    };

    let mut b = cc::Build::new();
    b.include(port.join("include"))
        .include(&port)
        .include(&vim)
        .include(vim.join("proto"))
        .define("HAVE_CONFIG_H", None)
        .define("main", "vim_main")
        .opt_level_str("s")
        .warnings(false)
        .flag_if_supported("-Wno-implicit-function-declaration")
        .flag_if_supported("-Wno-int-conversion")
        .flag_if_supported("-Wno-incompatible-pointer-types")
        .flag_if_supported("-fno-strict-aliasing");
    for e in EMULATED {
        b.define(&format!("_WASI_EMULATED_{}", e.to_uppercase().replace('-', "_")), None);
    }
    for f in VIM_SRC {
        b.file(vim.join(f));
    }
    b.file(port.join("pathdef.c")).file(port.join("wasi_shim.c"));
    b.compile("vim");

    println!("cargo:rustc-link-search=native={sdk}/share/wasi-sysroot/lib/wasm32-wasip1");
    for e in EMULATED {
        println!("cargo:rustc-link-lib=static=wasi-emulated-{e}");
    }
}
