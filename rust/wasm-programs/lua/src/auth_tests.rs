//! lua/auth.lua on a real Lua with [`install`]: the module's names (the same as
//! Python's auth.py), the pins in the script's `.auth/` folder, and the network calls
//! failing off a computer (they need its sockets).

use super::*;

fn lua() -> Lua {
    let lua = Lua::new();
    install(&lua).expect("install");
    lua
}

fn eval(lua: &Lua, code: &str) -> String {
    lua.load(code).set_name("=test").eval::<String>().unwrap_or_else(|e| panic!("lua raised:\n{e}\nin:\n{code}"))
}

#[test]
fn the_module_has_its_functions() {
    let l = lua();
    let missing = eval(
        &l,
        r#"local t = {}
for _, n in ipairs({"signin", "signup", "token", "prove", "fingerprint", "logout", "pins", "unpin", "display_fingerprint"}) do
  if type(auth[n]) ~= "function" then t[#t + 1] = n end
end
return table.concat(t, ",") .. "|" .. auth.DEFAULT_PORT"#,
    );
    assert_eq!(missing, "|7600");
    assert_eq!(eval(&l, "return auth.display_fingerprint('3fa19c0d00000000000000000000beef')"), "3F:A1:9C:0D:00:00:00:00:00:00:00:00:00:00:BE:EF");
    let e = eval(&l, "local ok, e = pcall(auth.display_fingerprint, 'nope') return tostring(ok) .. ' ' .. tostring(e)");
    assert!(e.starts_with("false ") && e.contains("auth: 'nope' is not a fingerprint"), "{e}");
}

#[test]
fn pins_live_next_to_the_script_and_are_removed_by_hand() {
    let d = std::env::temp_dir().join(format!("ecm-lua-auth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join(".auth")).unwrap();
    let (a, b) = ("a".repeat(32), "b".repeat(32));
    std::fs::write(d.join(".auth/pins"), format!("{a} 10.0.8.1:7600 Tech Village\n{b} 10.0.9.1:7600 Music\n")).unwrap();
    let l = lua();
    l.globals().set("SCRIPT", format!("{}/app.lua", d.to_string_lossy().replace('\\', "/"))).unwrap();
    let out = eval(
        &l,
        &format!(
            r#"arg = {{[0] = SCRIPT}}
local function show(ps)
  local t = {{}}
  for _, p in ipairs(ps) do t[#t + 1] = p.fingerprint:sub(1, 1) .. "@" .. p.host .. "=" .. p.realm end
  return table.concat(t, ";")
end
local before = show(auth.pins())
return table.concat({{before, tostring(auth.unpin("10.0.9.1")), tostring(auth.unpin("{a}")), tostring(auth.unpin("{a}")), show(auth.pins()), tostring(auth.logout())}}, " | ")"#
        ),
    );
    assert_eq!(out, "a@10.0.8.1:7600=Tech Village;b@10.0.9.1:7600=Music | true | true | false |  | false");
    assert_eq!(std::fs::read_to_string(d.join(".auth/pins")).unwrap(), "");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn network_calls_fail_off_a_computer() {
    let l = lua();
    let e = eval(&l, "local ok, e = pcall(auth.signin, '10.0.8.1', 'alice', 'pw') return tostring(ok) .. ' ' .. tostring(e)");
    assert!(e.starts_with("false ") && e.contains("auth: network: auth needs a computer's network"), "{e}");
    let e = eval(&l, "local ok, e = pcall(auth.token, string.rep('ab', 16)) return tostring(e)");
    assert!(e.contains("auth: not signed in"), "{e}");
}
