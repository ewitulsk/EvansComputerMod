//! `vim` (and `vi`): Vim 9.2, the real one, for the computer's terminal.
//! Everything is Vim's C code (rust/third_party/vim) and the WASI port in
//! `port/`; this only hands Vim its arguments.

#[cfg(target_os = "wasi")]
fn main() {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};

    extern "C" {
        fn vim_main(argc: c_int, argv: *mut *mut c_char) -> c_int;
    }

    // Vim reads ~/.vimrc and ~/.vim: the computer's home is its root folder.
    if std::env::var_os("HOME").is_none() {
        std::env::set_var("HOME", "/");
    }
    // Vim's files are on the server's program folder (port/pathdef.c). Naming
    // them here keeps Vim from guessing $VIM from argv[0] (a relative path to
    // the .wasm) when they are missing.
    if std::env::var_os("VIM").is_none() {
        std::env::set_var("VIM", "/server-bin/vim");
    }
    if std::env::var_os("VIMRUNTIME").is_none() {
        std::env::set_var("VIMRUNTIME", "/server-bin/vim/runtime");
    }
    // The console is an xterm-like VTE (crates/ecm-vte); Vim's builtin xterm
    // termcap matches it.
    if std::env::var_os("TERM").is_none() {
        std::env::set_var("TERM", "xterm");
    }
    let args: Vec<CString> = std::env::args().map(|a| CString::new(a).unwrap_or_default()).collect();
    let mut argv: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr() as *mut c_char).collect();
    argv.push(std::ptr::null_mut());
    // SAFETY: argv is a NULL-terminated array of NUL-terminated strings that
    // outlive the call; Vim may permute the pointers but not free them.
    let code = unsafe { vim_main((argv.len() - 1) as c_int, argv.as_mut_ptr()) };
    std::process::exit(code);
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("vim: built only for the computer (cargo build --target wasm32-wasip1 -p vim)");
    std::process::exit(1);
}
