//! auth.py on a real RustPython interpreter, loaded as the bootstrap loads it: the
//! module's names, the pins in the script's `.auth/` folder, and the network calls
//! refusing off a computer (they need its sockets).

use super::*;
use rustpython_vm::{compiler::Mode, Interpreter, Settings};

const SETUP: &str = "import sys, _auth\nM = type(sys)('auth')\nexec(compile(_auth._source, '<auth>', 'exec'), M.__dict__)\nsys.modules['auth'] = M\nimport auth\n";

/// Run `code` and return the value of `OUT` (a str).
fn run(code: &str) -> String {
    let interp = Interpreter::with_init(Settings::default(), |vm| {
        vm.add_native_module("_auth".to_owned(), Box::new(auth_native::make_module));
    });
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let src = format!("{SETUP}{code}");
        let obj = vm.compile(&src, Mode::Exec, "<test>".to_owned()).unwrap_or_else(|e| panic!("{e:?} in:\n{src}"));
        if let Err(e) = vm.run_code_obj(obj, scope.clone()) {
            let mut s = String::new();
            vm.write_exception(&mut s, &e).unwrap();
            panic!("python raised:\n{s}");
        }
        let out = scope.globals.get_item("OUT", vm).expect("OUT");
        out.str(vm).unwrap().as_str().to_string()
    })
}

#[test]
fn the_module_has_its_functions() {
    let out = run("OUT = repr([n for n in ['signin', 'signup', 'token', 'prove', 'fingerprint', 'logout', 'pins', 'unpin', 'display_fingerprint', 'AuthError'] if not hasattr(auth, n)] + [auth.DEFAULT_PORT])");
    assert_eq!(out, "[7600]");
    assert_eq!(run("OUT = auth.display_fingerprint('3fa19c0d00000000000000000000beef')"), "3F:A1:9C:0D:00:00:00:00:00:00:00:00:00:00:BE:EF");
    assert!(run("try:\n    auth.display_fingerprint('nope')\n    OUT = 'no error'\nexcept ValueError as e:\n    OUT = str(e)").contains("not a fingerprint"));
}

#[test]
fn pins_live_next_to_the_script_and_are_removed_by_hand() {
    let d = std::env::temp_dir().join(format!("ecm-py-auth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join(".auth")).unwrap();
    let (a, b) = ("a".repeat(32), "b".repeat(32));
    std::fs::write(d.join(".auth/pins"), format!("{a} 10.0.8.1:7600 Tech Village\n{b} 10.0.9.1:7600 Music\n")).unwrap();
    let script = format!("{}/app.py", d.to_string_lossy().replace('\\', "/"));
    let out = run(&format!(
        "sys.argv = ['{script}']\nbefore = auth.pins()\nOUT = repr([before, auth.unpin('10.0.9.1'), auth.unpin('{a}'), auth.unpin('{a}'), auth.pins(), auth.logout()])"
    ));
    assert_eq!(
        out,
        format!(
            "[[{{'fingerprint': '{a}', 'host': '10.0.8.1:7600', 'realm': 'Tech Village'}}, {{'fingerprint': '{b}', 'host': '10.0.9.1:7600', 'realm': 'Music'}}], True, True, False, [], False]"
        )
    );
    assert_eq!(std::fs::read_to_string(d.join(".auth/pins")).unwrap(), "");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn network_calls_raise_auth_error_off_a_computer() {
    let out = run("try:\n    auth.signin('10.0.8.1', 'alice', 'pw')\n    OUT = 'signed in?'\nexcept auth.AuthError as e:\n    OUT = str(e)");
    assert_eq!(out, "network: auth needs a computer's network");
    let out = run("try:\n    auth.token('ab' * 16)\n    OUT = 'token?'\nexcept auth.AuthError as e:\n    OUT = str(e)");
    assert!(out.starts_with("not signed in"), "{out}");
}
