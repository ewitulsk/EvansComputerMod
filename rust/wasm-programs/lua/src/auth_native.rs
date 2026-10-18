//! `_auth` native module under the Lua `auth` module (lua/auth.lua): accounts on an
//! Auth server (`authd`) or a backend's own realm. Every call goes to
//! `ecm_auth::host::script`, the same functions Python's `auth` calls, so both
//! interpreters pin, save and refuse the same way. Errors are Lua errors starting with
//! `auth: `. The network calls need a computer; off one (host tests) they fail with
//! "auth needs a computer's network".

use ecm_auth::host::script;
use ecm_auth::store::Login;
use mlua::{Lua, Table};

fn err(e: ecm_auth::AuthError) -> mlua::Error {
    mlua::Error::runtime(format!("auth: {e}"))
}

/// {realm, fingerprint, host, user, display, id}, as Python's dict.
fn login_table(lua: &Lua, l: Login) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("realm", l.realm_name)?;
    t.set("fingerprint", l.realm_fp)?;
    t.set("host", l.host)?;
    t.set("user", l.username)?;
    t.set("display", l.display_name)?;
    t.set("id", l.user_id)?;
    Ok(t)
}

fn opt(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

pub fn module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;
    m.set("DEFAULT_PORT", ecm_auth::DEFAULT_PORT)?;
    m.set("signin", lua.create_function(|lua, (dir, host, user, password): (String, String, String, String)| {
        login_table(lua, script::signin(&dir, &host, &user, &password).map_err(err)?)
    })?)?;
    m.set("signup", lua.create_function(|lua, (dir, host, user, password, display): (String, String, String, String, Option<String>)| {
        login_table(lua, script::signup(&dir, &host, &user, &password, opt(&display)).map_err(err)?)
    })?)?;
    m.set("token", lua.create_function(|_, (dir, audience, host): (String, String, Option<String>)| {
        script::token(&dir, &audience, opt(&host)).map_err(err)
    })?)?;
    m.set("prove", lua.create_function(|_, (dir, nonce, audience, host): (String, String, String, Option<String>)| {
        script::prove(&dir, &nonce, &audience, opt(&host)).map_err(err)
    })?)?;
    // fingerprint, realm name, pinned.
    m.set("fingerprint", lua.create_function(|_, (dir, host): (String, String)| script::fingerprint(&dir, &host).map_err(err))?)?;
    m.set("logout", lua.create_function(|_, (dir, host): (String, Option<String>)| script::logout(&dir, opt(&host)).map_err(err))?)?;
    m.set("pins", lua.create_function(|lua, dir: String| {
        let list = lua.create_table()?;
        for (i, p) in script::pins(&dir).into_iter().enumerate() {
            let t = lua.create_table()?;
            t.set("fingerprint", p.fp)?;
            t.set("host", p.host)?;
            t.set("realm", p.name)?;
            list.raw_set(i + 1, t)?;
        }
        Ok(list)
    })?)?;
    m.set("unpin", lua.create_function(|_, (dir, which): (String, String)| script::unpin(&dir, &which).map_err(err))?)?;
    m.set("display_fingerprint", lua.create_function(|_, fp: String| match ecm_auth::parse_fingerprint(&fp) {
        Some(fp) => Ok(ecm_auth::display_fingerprint(&fp)),
        None => Err(mlua::Error::runtime(format!("auth: '{fp}' is not a fingerprint (32 hex digits)"))),
    })?)?;
    Ok(m)
}
