//! `auth-api admin <verb> ...`: account setup on the server, against the same database
//! as the running service. For first-time setup and for automated tests that cannot
//! click through a browser.

use crate::{api, db, hash_password, web, App, Config};

pub const USAGE: &str = "\
usage: auth-api admin <verb> ...
  user-add <email> <handle> <password>     create an account whose email is already confirmed
  key-add <handle> <name> [scope,scope]    make an API key (scopes: social,publish; default both);
                                           prints only the key
  users                                    list accounts: handle, email, verified, admin
";

/// Run an admin command with the service's environment; the text to print, or the error.
pub async fn run(args: &[String]) -> Result<String, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    if matches!(words.as_slice(), [] | ["help"] | ["--help"] | ["-h"]) {
        return Ok(USAGE.to_string());
    }
    let app = crate::build(Config::from_env()?)?;
    run_on(&app, &words).await
}

pub async fn run_on(app: &App, words: &[&str]) -> Result<String, String> {
    let failed = |e: rusqlite::Error| format!("database: {e}");
    match words {
        ["user-add", email, handle, password] => {
            let email = email.trim().to_lowercase();
            if !web::valid_email(&email) {
                return Err(format!("'{email}' is not an email address"));
            }
            if !web::valid_handle(handle) {
                return Err(format!("the handle '{handle}' must be 3 to 32 of a-z, 0-9, _ and -"));
            }
            if let Some(problem) = web::password_problem(password) {
                return Err(problem);
            }
            {
                let conn = app.db.lock().unwrap();
                if db::user_by_email(&conn, &email).map_err(failed)?.is_some() {
                    return Err(format!("an account with the email {email} exists already"));
                }
                if db::user_by_handle(&conn, handle).map_err(failed)?.is_some() {
                    return Err(format!("the handle {handle} is taken"));
                }
            }
            let hash = hash_password(password.to_string()).await.map_err(|e| e.message)?;
            let conn = app.db.lock().unwrap();
            let Some(id) = db::create_user(&conn, &email, handle, &hash).map_err(failed)? else {
                return Err(format!("the email {email} or the handle {handle} is taken"));
            };
            conn.execute("UPDATE users SET email_verified = 1 WHERE id = ?1", [id]).map_err(failed)?;
            Ok(format!("created {handle} <{email}> (email confirmed)\n"))
        }
        ["key-add", handle, name] => key_add(app, handle, name, "social,publish"),
        ["key-add", handle, name, scopes] => key_add(app, handle, name, scopes),
        ["users"] => {
            let conn = app.db.lock().unwrap();
            let mut out = String::new();
            for u in db::list_users(&conn).map_err(failed)? {
                let yes_no = |b: bool| if b { "yes" } else { "no" };
                out.push_str(&format!("{}\t{}\tverified={}\tadmin={}\n", u.handle, u.email, yes_no(u.email_verified), yes_no(app.is_admin(&u))));
            }
            if out.is_empty() {
                out.push_str("no accounts\n");
            }
            Ok(out)
        }
        _ => Err(USAGE.to_string()),
    }
}

fn key_add(app: &App, handle: &str, name: &str, scopes: &str) -> Result<String, String> {
    let asked: Vec<String> = scopes.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let scopes = api::check_scopes(&asked).map_err(|e| e.message)?;
    let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    let name = name.trim();
    if name.is_empty() {
        return Err("give the key a name".into());
    }
    let conn = app.db.lock().unwrap();
    let user = db::user_by_handle(&conn, handle).map_err(|e| format!("database: {e}"))?.ok_or_else(|| format!("no account with the handle {handle}"))?;
    let key = db::create_token(&conn, user.id, "key", name, &scopes).map_err(|e| format!("database: {e}"))?;
    Ok(format!("{key}\n"))
}
