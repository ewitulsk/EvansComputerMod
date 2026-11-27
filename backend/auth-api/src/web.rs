//! The web pages: plain server-rendered HTML forms, no script. Every form carries a
//! CSRF token (double-submit cookie) and posts to the path it was served from.

use crate::{db, hash_password, verify_password, App, GRANTABLE_SCOPES};
use axum::extract::{ConnectInfo, Form, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use backend_common::html::{esc, page, respond, see_other, url_encode};
use backend_common::http::{client_ip, cookie};
use backend_common::{rfc3339, token};
use std::collections::HashMap;
use std::net::SocketAddr;

type Fields = HashMap<String, String>;

const SESSION_COOKIE: &str = "ecm_session";
const STYLESHEET: &str = "/static/auth/style.css";
pub const MIN_PASSWORD: usize = 10;
const MAX_PASSWORD: usize = 256;
const MAX_KEYS: usize = 50;

// ------------------------------------------------------------------ sessions and CSRF

pub struct Session {
    pub token_id: i64,
    pub user: db::User,
}

fn session(app: &App, headers: &HeaderMap) -> Option<Session> {
    let presented = cookie(headers, SESSION_COOKIE)?;
    let found = db::find_token(&app.db.lock().unwrap(), presented, &["session"]).ok()??;
    Some(Session { token_id: found.0.id, user: found.1 })
}

fn session_cookie(app: &App, value: &str, max_age: i64) -> String {
    let secure = if app.secure() { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}")
}

/// `__Host-` pins the cookie to this host over https, so a sibling host cannot plant one.
fn csrf_cookie_name(app: &App) -> &'static str {
    if app.secure() {
        "__Host-ecm_csrf"
    } else {
        "ecm_csrf"
    }
}

/// The CSRF token of this browser: the cookie it already has, or a new one to set.
struct Csrf {
    token: String,
    set_cookie: Option<String>,
}

fn csrf(app: &App, headers: &HeaderMap) -> Csrf {
    let name = csrf_cookie_name(app);
    match cookie(headers, name) {
        Some(v) if v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()) => Csrf { token: v.to_string(), set_cookie: None },
        _ => {
            let t = token::random_hex(32);
            let secure = if app.secure() { "; Secure" } else { "" };
            let set = format!("{name}={t}; Path=/; HttpOnly; SameSite=Lax{secure}");
            Csrf { token: t, set_cookie: Some(set) }
        }
    }
}

/// A form post must come from our own page: same origin when the browser says where it
/// came from, and the hidden field must match the cookie.
fn check_csrf(app: &App, headers: &HeaderMap, form: &Fields) -> Result<(), Response> {
    let origin_ok = match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(o) => o == app.cfg.public_origin,
        None => true,
    };
    let sent = form.get("csrf").map(String::as_str).unwrap_or("");
    let have = cookie(headers, csrf_cookie_name(app)).unwrap_or("");
    if origin_ok && !sent.is_empty() && token::same(sent, have) {
        return Ok(());
    }
    let body = "<h1>That form expired</h1><p class=\"error\">Go back, reload the page and try again.</p>";
    Err(respond(StatusCode::FORBIDDEN, page("Form expired", STYLESHEET, "", body)))
}

fn hidden_csrf(c: &Csrf) -> String {
    format!("<input type=\"hidden\" name=\"csrf\" value=\"{}\">", esc(&c.token))
}

fn add_cookie(mut r: Response, value: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(value) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

/// Render a page with the layout, the navigation for this visitor and the CSRF cookie.
fn render(status: StatusCode, title: &str, body: &str, c: &Csrf, sess: Option<&Session>) -> Response {
    let nav = match sess {
        Some(s) => format!(
            "<span>{}</span><a href=\"/account\">Account</a>\
             <form method=\"post\" action=\"/signout\">{}<button class=\"plain\">Sign out</button></form>",
            esc(&s.user.handle),
            hidden_csrf(c)
        ),
        None => "<a href=\"/signin\">Sign in</a><a href=\"/signup\">Sign up</a>".to_string(),
    };
    let r = respond(status, page(title, STYLESHEET, &nav, body));
    match &c.set_cookie {
        Some(v) => add_cookie(r, v),
        None => r,
    }
}

fn error_box(message: &str) -> String {
    if message.is_empty() {
        String::new()
    } else {
        format!("<p class=\"error\">{}</p>", esc(message))
    }
}

fn too_many(c: &Csrf, sess: Option<&Session>) -> Response {
    let body = "<h1>Too many tries</h1><p class=\"error\">Wait a while and try again.</p>";
    render(StatusCode::TOO_MANY_REQUESTS, "Too many tries", body, c, sess)
}

fn oops(c: &Csrf) -> Response {
    render(StatusCode::INTERNAL_SERVER_ERROR, "Error", "<h1>Something went wrong</h1><p>Try again in a moment.</p>", c, None)
}

/// Where to go after signing in: only a path on this site.
fn safe_next(next: Option<&String>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") && !n.contains('\\') && !n.chars().any(|c| c.is_control()) => n.clone(),
        _ => "/account".into(),
    }
}

fn field<'a>(form: &'a Fields, name: &str) -> &'a str {
    form.get(name).map(String::as_str).unwrap_or("")
}

pub fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else { return false };
    e.len() <= 254
        && !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !e.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"'))
}

pub fn valid_handle(h: &str) -> bool {
    (3..=32).contains(&h.len()) && h.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

pub fn password_problem(p: &str) -> Option<String> {
    if p.chars().count() < MIN_PASSWORD {
        Some(format!("The password must have at least {MIN_PASSWORD} characters."))
    } else if p.len() > MAX_PASSWORD {
        Some(format!("The password must have at most {MAX_PASSWORD} bytes."))
    } else {
        None
    }
}

fn send_verification(app: &App, user_id: i64, email: &str) {
    let made = db::create_email_token(&app.db.lock().unwrap(), user_id, "verify", db::VERIFY_SECONDS);
    if let Ok(t) = made {
        let text = format!(
            "Welcome to ECM packages.\n\nConfirm this email address by opening:\n{}/verify?token={}\n\n\
             The link works for 24 hours. If you did not make an account, ignore this mail.\n",
            app.cfg.public_origin, t
        );
        app.send_mail(email, "Confirm your email address", text);
    }
}

// ------------------------------------------------------------------ sign up

fn signup_form(c: &Csrf, error: &str, email: &str, handle: &str, next: &str) -> String {
    format!(
        "<h1>Sign up</h1>{err}<section><form method=\"post\" action=\"/signup\">{csrf}\
         <input type=\"hidden\" name=\"next\" value=\"{next}\">\
         <label for=\"email\">Email</label><input id=\"email\" name=\"email\" type=\"email\" required autocomplete=\"email\" value=\"{email}\">\
         <label for=\"handle\">Handle</label><input id=\"handle\" name=\"handle\" type=\"text\" required minlength=\"3\" maxlength=\"32\" pattern=\"[a-z0-9_\\-]{{3,32}}\" autocomplete=\"username\" value=\"{handle}\">\
         <p class=\"hint\">3 to 32 of a-z, 0-9, _ and -. It is shown as the publisher of your packages.</p>\
         <label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" required minlength=\"{min}\" autocomplete=\"new-password\">\
         <p class=\"hint\">At least {min} characters.</p>\
         <button>Create account</button></form></section>\
         <p>Already have an account? <a href=\"/signin?next={next_q}\">Sign in</a>.</p>",
        err = error_box(error),
        csrf = hidden_csrf(c),
        next = esc(next),
        next_q = url_encode(next),
        email = esc(email),
        handle = esc(handle),
        min = MIN_PASSWORD,
    )
}

pub async fn signup_page(State(app): State<App>, headers: HeaderMap, Query(q): Query<Fields>) -> Response {
    let c = csrf(&app, &headers);
    let next = safe_next(q.get("next"));
    render(StatusCode::OK, "Sign up", &signup_form(&c, "", "", "", &next), &c, None)
}

pub async fn signup(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<Fields>,
) -> Response {
    if let Err(r) = check_csrf(&app, &headers, &form) {
        return r;
    }
    let c = csrf(&app, &headers);
    let ip = client_ip(&headers, peer, app.cfg.trust_proxy);
    if !app.limiter.hit(&format!("signup:{ip}"), app.cfg.limits.signup) {
        return too_many(&c, None);
    }
    let email = field(&form, "email").trim().to_lowercase();
    let handle = field(&form, "handle").trim().to_string();
    let password = field(&form, "password").to_string();
    let next = safe_next(form.get("next"));
    let again = |msg: &str| render(StatusCode::BAD_REQUEST, "Sign up", &signup_form(&c, msg, &email, &handle, &next), &c, None);
    if !valid_email(&email) {
        return again("That does not look like an email address.");
    }
    if !valid_handle(&handle) {
        return again("The handle must be 3 to 32 of a-z, 0-9, _ and -.");
    }
    if let Some(problem) = password_problem(&password) {
        return again(&problem);
    }
    let Ok(hash) = hash_password(password).await else { return oops(&c) };
    let created = {
        let conn = app.db.lock().unwrap();
        match db::create_user(&conn, &email, &handle, &hash) {
            Ok(Some(id)) => db::create_token(&conn, id, "session", "web", &[]).map(|t| Some((id, t))),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    };
    match created {
        Ok(Some((id, session_token))) => {
            send_verification(&app, id, &email);
            add_cookie(see_other(&next), &session_cookie(&app, &session_token, db::SESSION_SECONDS))
        }
        Ok(None) => again("That email or handle is already in use."),
        Err(_) => oops(&c),
    }
}

// ------------------------------------------------------------------ sign in and out

fn signin_form(c: &Csrf, error: &str, email: &str, next: &str) -> String {
    format!(
        "<h1>Sign in</h1>{err}<section><form method=\"post\" action=\"/signin\">{csrf}\
         <input type=\"hidden\" name=\"next\" value=\"{next}\">\
         <label for=\"email\">Email</label><input id=\"email\" name=\"email\" type=\"email\" required autocomplete=\"email\" value=\"{email}\">\
         <label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" required autocomplete=\"current-password\">\
         <button>Sign in</button></form></section>\
         <p><a href=\"/forgot\">Forgot your password?</a> No account yet? <a href=\"/signup?next={next_q}\">Sign up</a>.</p>",
        err = error_box(error),
        csrf = hidden_csrf(c),
        next = esc(next),
        next_q = url_encode(next),
        email = esc(email),
    )
}

pub async fn signin_page(State(app): State<App>, headers: HeaderMap, Query(q): Query<Fields>) -> Response {
    let next = safe_next(q.get("next"));
    if session(&app, &headers).is_some() {
        return see_other(&next);
    }
    let c = csrf(&app, &headers);
    render(StatusCode::OK, "Sign in", &signin_form(&c, "", "", &next), &c, None)
}

pub async fn signin(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<Fields>,
) -> Response {
    if let Err(r) = check_csrf(&app, &headers, &form) {
        return r;
    }
    let c = csrf(&app, &headers);
    let ip = client_ip(&headers, peer, app.cfg.trust_proxy);
    let email = field(&form, "email").trim().to_lowercase();
    let next = safe_next(form.get("next"));
    // Both counters count every try, so neither one address nor one account can be hammered.
    let by_ip = app.limiter.hit(&format!("signin:ip:{ip}"), app.cfg.limits.signin);
    let by_account = app.limiter.hit(&format!("signin:account:{email}"), app.cfg.limits.signin);
    if !by_ip || !by_account {
        return too_many(&c, None);
    }
    let user = match db::user_by_email(&app.db.lock().unwrap(), &email) {
        Ok(u) => u,
        Err(_) => return oops(&c),
    };
    let password: String = field(&form, "password").chars().take(MAX_PASSWORD).collect();
    if !verify_password(password, user.as_ref().map(|u| u.password_hash.clone())).await {
        let body = signin_form(&c, "Wrong email or password.", &email, &next);
        return render(StatusCode::UNAUTHORIZED, "Sign in", &body, &c, None);
    }
    let user = user.expect("verified password implies a user");
    match db::create_token(&app.db.lock().unwrap(), user.id, "session", "web", &[]) {
        Ok(t) => add_cookie(see_other(&next), &session_cookie(&app, &t, db::SESSION_SECONDS)),
        Err(_) => oops(&c),
    }
}

pub async fn signout(State(app): State<App>, headers: HeaderMap, Form(form): Form<Fields>) -> Response {
    if let Err(r) = check_csrf(&app, &headers, &form) {
        return r;
    }
    if let Some(s) = session(&app, &headers) {
        let _ = db::revoke_token(&app.db.lock().unwrap(), s.user.id, s.token_id);
    }
    add_cookie(see_other("/signin"), &session_cookie(&app, "", 0))
}

// ------------------------------------------------------------------ email verification

pub async fn verify(State(app): State<App>, headers: HeaderMap, Query(q): Query<Fields>) -> Response {
    let c = csrf(&app, &headers);
    let sess = session(&app, &headers);
    let done = {
        let conn = app.db.lock().unwrap();
        match db::take_email_token(&conn, field(&q, "token"), "verify") {
            Ok(Some(user_id)) => conn.execute("UPDATE users SET email_verified = 1 WHERE id = ?1", [user_id]).is_ok(),
            _ => false,
        }
    };
    if done {
        let body = "<h1>Email confirmed</h1><p class=\"ok\">Thank you. You can publish packages now.</p><p><a href=\"/account\">Go to your account</a></p>";
        render(StatusCode::OK, "Email confirmed", body, &c, sess.as_ref())
    } else {
        let body = "<h1>That link does not work</h1><p class=\"error\">It was already used or it is older than 24 hours.</p>\
                    <p>You can send a new one from <a href=\"/account\">your account</a>.</p>";
        render(StatusCode::BAD_REQUEST, "Link expired", body, &c, sess.as_ref())
    }
}

// ------------------------------------------------------------------ forgotten password

pub async fn forgot_page(State(app): State<App>, headers: HeaderMap) -> Response {
    let c = csrf(&app, &headers);
    let body = format!(
        "<h1>Forgot your password?</h1><section><form method=\"post\" action=\"/forgot\">{}\
         <label for=\"email\">Email</label><input id=\"email\" name=\"email\" type=\"email\" required autocomplete=\"email\">\
         <button>Send a reset link</button></form></section>",
        hidden_csrf(&c)
    );
    render(StatusCode::OK, "Forgot password", &body, &c, None)
}

pub async fn forgot(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<Fields>,
) -> Response {
    if let Err(r) = check_csrf(&app, &headers, &form) {
        return r;
    }
    let c = csrf(&app, &headers);
    let ip = client_ip(&headers, peer, app.cfg.trust_proxy);
    if !app.limiter.hit(&format!("forgot:ip:{ip}"), app.cfg.limits.forgot) {
        return too_many(&c, None);
    }
    let email = field(&form, "email").trim().to_lowercase();
    // The answer is the same whether or not the account exists (and whether or not
    // its own limit is used up): nothing here tells an asker which emails have accounts.
    if valid_email(&email) && app.limiter.hit(&format!("forgot:account:{email}"), app.cfg.limits.forgot) {
        let made = {
            let conn = app.db.lock().unwrap();
            match db::user_by_email(&conn, &email) {
                Ok(Some(u)) => db::create_email_token(&conn, u.id, "reset", db::RESET_SECONDS).ok(),
                _ => None,
            }
        };
        if let Some(t) = made {
            let text = format!(
                "Someone asked to reset the password of your ECM packages account.\n\nChoose a new password here:\n\
                 {}/reset?token={}\n\nThe link works for one hour. If it was not you, ignore this mail.\n",
                app.cfg.public_origin, t
            );
            app.send_mail(&email, "Reset your password", text);
        }
    }
    let body = "<h1>Check your mail</h1><p class=\"note\">If that address has an account, we sent it a link to choose a new password.</p>";
    render(StatusCode::OK, "Check your mail", body, &c, None)
}

fn reset_form(c: &Csrf, error: &str, link_token: &str) -> String {
    format!(
        "<h1>Choose a new password</h1>{err}<section><form method=\"post\" action=\"/reset\">{csrf}\
         <input type=\"hidden\" name=\"token\" value=\"{tok}\">\
         <label for=\"password\">New password</label><input id=\"password\" name=\"password\" type=\"password\" required minlength=\"{min}\" autocomplete=\"new-password\">\
         <p class=\"hint\">At least {min} characters.</p><button>Change password</button></form></section>",
        err = error_box(error),
        csrf = hidden_csrf(c),
        tok = esc(link_token),
        min = MIN_PASSWORD,
    )
}

fn bad_reset_link(c: &Csrf) -> Response {
    let body = "<h1>That link does not work</h1><p class=\"error\">It was already used or it is older than one hour.</p>\
                <p><a href=\"/forgot\">Ask for a new one</a></p>";
    render(StatusCode::BAD_REQUEST, "Link expired", body, c, None)
}

pub async fn reset_page(State(app): State<App>, headers: HeaderMap, Query(q): Query<Fields>) -> Response {
    let c = csrf(&app, &headers);
    let link = field(&q, "token");
    let good = db::peek_email_token(&app.db.lock().unwrap(), link, "reset").ok().flatten().is_some();
    if good {
        render(StatusCode::OK, "New password", &reset_form(&c, "", link), &c, None)
    } else {
        bad_reset_link(&c)
    }
}

pub async fn reset(State(app): State<App>, headers: HeaderMap, Form(form): Form<Fields>) -> Response {
    if let Err(r) = check_csrf(&app, &headers, &form) {
        return r;
    }
    let c = csrf(&app, &headers);
    let link = field(&form, "token").to_string();
    let password = field(&form, "password").to_string();
    if db::peek_email_token(&app.db.lock().unwrap(), &link, "reset").ok().flatten().is_none() {
        return bad_reset_link(&c);
    }
    if let Some(problem) = password_problem(&password) {
        return render(StatusCode::BAD_REQUEST, "New password", &reset_form(&c, &problem, &link), &c, None);
    }
    let Ok(hash) = hash_password(password).await else { return oops(&c) };
    let changed = {
        let conn = app.db.lock().unwrap();
        match db::take_email_token(&conn, &link, "reset") {
            // Whoever holds the mailbox owns the account: old sessions end, and the address counts as confirmed.
            Ok(Some(user_id)) => db::set_password(&conn, user_id, &hash)
                .and_then(|_| db::revoke_sessions(&conn, user_id, None))
                .and_then(|_| conn.execute("UPDATE users SET email_verified = 1 WHERE id = ?1", [user_id]))
                .is_ok(),
            _ => false,
        }
    };
    if !changed {
        return bad_reset_link(&c);
    }
    let body = "<h1>Password changed</h1><p class=\"ok\">You can sign in with the new password now.</p><p><a href=\"/signin\">Sign in</a></p>";
    add_cookie(render(StatusCode::OK, "Password changed", body, &c, None), &session_cookie(&app, "", 0))
}

// ------------------------------------------------------------------ account

fn signin_redirect(path_and_query: &str) -> Response {
    see_other(&format!("/signin?next={}", url_encode(path_and_query)))
}

fn account_body(app: &App, c: &Csrf, s: &Session, notice: &str, error: &str, new_key: Option<&str>) -> String {
    let tokens = db::list_tokens(&app.db.lock().unwrap(), s.user.id).unwrap_or_default();
    let mut out = String::from("<h1>Your account</h1>");
    if !notice.is_empty() {
        out.push_str(&format!("<p class=\"ok\">{}</p>", esc(notice)));
    }
    out.push_str(&error_box(error));
    if let Some(k) = new_key {
        out.push_str(&format!(
            "<section><h2>Your new API key</h2><p>Copy it now. It is not shown again.</p><code class=\"token\">{}</code></section>",
            esc(k)
        ));
    }
    let verified = if s.user.email_verified {
        "<span class=\"badge\">confirmed</span>".to_string()
    } else {
        format!(
            "<span class=\"badge\">not confirmed</span></p><p class=\"hint\">Confirm your email address before you publish packages.</p>\
             <form method=\"post\" action=\"/account\">{}<input type=\"hidden\" name=\"action\" value=\"resend\">\
             <button>Send the confirmation mail again</button></form><p>",
            hidden_csrf(c)
        )
    };
    out.push_str(&format!(
        "<section><h2>Profile</h2><p>Handle: <strong>{}</strong>{}</p><p>Email: {} {}</p></section>",
        esc(&s.user.handle),
        if app.is_admin(&s.user) { " <span class=\"badge\">admin</span>" } else { "" },
        esc(&s.user.email),
        verified
    ));

    out.push_str("<section><h2>API keys and devices</h2>");
    if tokens.is_empty() {
        out.push_str("<p class=\"hint\">None yet. A computer in the game asks for access with <code>pkg login</code>; a key can be made below.</p>");
    } else {
        out.push_str("<div class=\"scroll\"><table><thead><tr><th>Name</th><th>Kind</th><th>Scopes</th><th>Created</th><th>Last used</th><th></th></tr></thead><tbody>");
        for t in &tokens {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"meta\">{}</td><td class=\"meta\">{}</td>\
                 <td><form method=\"post\" action=\"/account\">{}<input type=\"hidden\" name=\"action\" value=\"revoke\">\
                 <input type=\"hidden\" name=\"token_id\" value=\"{}\"><button class=\"danger\">Revoke</button></form></td></tr>",
                esc(&t.name),
                if t.kind == "key" { "API key" } else { "device" },
                esc(&t.scopes.join(", ")),
                esc(&rfc3339(t.created_at)),
                t.last_used_at.map(|u| esc(&rfc3339(u))).unwrap_or_else(|| "never".into()),
                hidden_csrf(c),
                t.id
            ));
        }
        out.push_str("</tbody></table></div>");
    }
    out.push_str(&format!(
        "<h2>New API key</h2><form method=\"post\" action=\"/account\">{}<input type=\"hidden\" name=\"action\" value=\"create_key\">\
         <label for=\"name\">Name</label><input id=\"name\" name=\"name\" type=\"text\" required maxlength=\"64\" placeholder=\"build server\">\
         <p><label class=\"check\"><input type=\"checkbox\" name=\"scope_social\" checked> social (like, favorite, report)</label>\
         <label class=\"check\"><input type=\"checkbox\" name=\"scope_publish\" checked> publish (publish, yank)</label></p>\
         <button>Create key</button></form></section>",
        hidden_csrf(c)
    ));

    out.push_str(&format!(
        "<section><h2>Change password</h2><form method=\"post\" action=\"/account\">{}<input type=\"hidden\" name=\"action\" value=\"password\">\
         <label for=\"current\">Current password</label><input id=\"current\" name=\"current\" type=\"password\" required autocomplete=\"current-password\">\
         <label for=\"new\">New password</label><input id=\"new\" name=\"new\" type=\"password\" required minlength=\"{}\" autocomplete=\"new-password\">\
         <button>Change password</button></form></section>",
        hidden_csrf(c),
        MIN_PASSWORD
    ));
    out
}

pub async fn account_page(State(app): State<App>, headers: HeaderMap, Query(q): Query<Fields>) -> Response {
    let Some(s) = session(&app, &headers) else { return signin_redirect("/account") };
    let c = csrf(&app, &headers);
    let notice = match field(&q, "done") {
        "revoked" => "Revoked.",
        "sent" => "We sent the confirmation mail.",
        "password" => "Password changed. Other browsers were signed out.",
        _ => "",
    };
    render(StatusCode::OK, "Account", &account_body(&app, &c, &s, notice, "", None), &c, Some(&s))
}

pub async fn account(State(app): State<App>, headers: HeaderMap, Form(form): Form<Fields>) -> Response {
    if let Err(r) = check_csrf(&app, &headers, &form) {
        return r;
    }
    let Some(s) = session(&app, &headers) else { return signin_redirect("/account") };
    let c = csrf(&app, &headers);
    let fail = |msg: &str| render(StatusCode::BAD_REQUEST, "Account", &account_body(&app, &c, &s, "", msg, None), &c, Some(&s));
    match field(&form, "action") {
        "create_key" => {
            let name: String = field(&form, "name").chars().filter(|ch| !ch.is_control()).take(64).collect();
            let name = name.trim();
            let scopes: Vec<String> =
                GRANTABLE_SCOPES.iter().filter(|sc| form.contains_key(&format!("scope_{sc}"))).map(|sc| sc.to_string()).collect();
            if name.is_empty() {
                return fail("Give the key a name.");
            }
            if scopes.is_empty() {
                return fail("Choose at least one scope.");
            }
            let made = {
                let conn = app.db.lock().unwrap();
                match db::list_tokens(&conn, s.user.id) {
                    Ok(list) if list.len() >= MAX_KEYS => Ok(None),
                    Ok(_) => db::create_token(&conn, s.user.id, "key", name, &scopes).map(Some),
                    Err(e) => Err(e),
                }
            };
            match made {
                Ok(Some(key)) => render(StatusCode::OK, "Account", &account_body(&app, &c, &s, "", "", Some(&key)), &c, Some(&s)),
                Ok(None) => fail("You have too many keys and devices. Revoke one first."),
                Err(_) => oops(&c),
            }
        }
        "revoke" => {
            let id: i64 = field(&form, "token_id").parse().unwrap_or(-1);
            match db::revoke_token(&app.db.lock().unwrap(), s.user.id, id) {
                Ok(_) => see_other("/account?done=revoked"),
                Err(_) => oops(&c),
            }
        }
        "resend" => {
            if s.user.email_verified {
                return see_other("/account");
            }
            if !app.limiter.hit(&format!("resend:{}", s.user.id), app.cfg.limits.forgot) {
                return too_many(&c, Some(&s));
            }
            send_verification(&app, s.user.id, &s.user.email);
            see_other("/account?done=sent")
        }
        "password" => {
            if !app.limiter.hit(&format!("signin:account:{}", s.user.email), app.cfg.limits.signin) {
                return too_many(&c, Some(&s));
            }
            let current: String = field(&form, "current").chars().take(MAX_PASSWORD).collect();
            let new = field(&form, "new").to_string();
            if let Some(problem) = password_problem(&new) {
                return fail(&problem);
            }
            if !verify_password(current, Some(s.user.password_hash.clone())).await {
                return fail("The current password is wrong.");
            }
            let Ok(hash) = hash_password(new).await else { return oops(&c) };
            let conn = app.db.lock().unwrap();
            match db::set_password(&conn, s.user.id, &hash).and_then(|_| db::revoke_sessions(&conn, s.user.id, Some(s.token_id))) {
                Ok(_) => see_other("/account?done=password"),
                Err(_) => oops(&c),
            }
        }
        _ => fail("Unknown action."),
    }
}

// ------------------------------------------------------------------ device approval

/// `abcd efgh`, `abcdefgh` and `ABCD-EFGH` are the same code.
fn normal_user_code(s: &str) -> String {
    let raw: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).take(16).collect();
    if raw.len() == 8 {
        format!("{}-{}", &raw[..4], &raw[4..])
    } else {
        raw
    }
}

fn device_code_form(error: &str) -> String {
    format!(
        "<h1>Connect a computer</h1>{}<section><form method=\"get\" action=\"/device\">\
         <label for=\"code\">Code shown on the computer</label>\
         <input id=\"code\" name=\"code\" type=\"text\" required autocomplete=\"off\" autocapitalize=\"characters\" placeholder=\"ABCD-EFGH\">\
         <button>Continue</button></form></section>",
        error_box(error)
    )
}

pub async fn device_page(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<Fields>,
) -> Response {
    let code = normal_user_code(field(&q, "code"));
    let Some(s) = session(&app, &headers) else {
        let back = if code.is_empty() { "/device".to_string() } else { format!("/device?code={}", url_encode(&code)) };
        return signin_redirect(&back);
    };
    let c = csrf(&app, &headers);
    if code.is_empty() {
        return render(StatusCode::OK, "Connect a computer", &device_code_form(""), &c, Some(&s));
    }
    // Codes are short: guessing them is limited per address.
    let ip = client_ip(&headers, peer, app.cfg.trust_proxy);
    if !app.limiter.hit(&format!("devicecode:{ip}"), app.cfg.limits.device_start) {
        return too_many(&c, Some(&s));
    }
    let found = db::pending_device(&app.db.lock().unwrap(), &code).ok().flatten();
    let Some(d) = found else {
        let body = device_code_form("That code is unknown or it expired. Start again on the computer.");
        return render(StatusCode::NOT_FOUND, "Connect a computer", &body, &c, Some(&s));
    };
    let body = format!(
        "<h1>Connect a computer</h1><section><p><strong>{name}</strong> asks to use your account <strong>{handle}</strong>.</p>\
         <p>It will be allowed to: {scopes}.</p><p class=\"hint\">Only approve this if you just started it yourself, and the code on the computer is <code>{code}</code>.</p>\
         <div class=\"actions\"><form method=\"post\" action=\"/device\">{csrf}<input type=\"hidden\" name=\"code\" value=\"{code}\">\
         <input type=\"hidden\" name=\"action\" value=\"approve\"><button>Approve</button></form>\
         <form method=\"post\" action=\"/device\">{csrf}<input type=\"hidden\" name=\"code\" value=\"{code}\">\
         <input type=\"hidden\" name=\"action\" value=\"deny\"><button class=\"danger\">Deny</button></form></div></section>",
        name = esc(&d.client_name),
        handle = esc(&s.user.handle),
        scopes = esc(&d.scopes.join(", ")),
        code = esc(&d.user_code),
        csrf = hidden_csrf(&c),
    );
    render(StatusCode::OK, "Connect a computer", &body, &c, Some(&s))
}

pub async fn device(State(app): State<App>, headers: HeaderMap, Form(form): Form<Fields>) -> Response {
    if let Err(r) = check_csrf(&app, &headers, &form) {
        return r;
    }
    let Some(s) = session(&app, &headers) else { return signin_redirect("/device") };
    let c = csrf(&app, &headers);
    let code = normal_user_code(field(&form, "code"));
    let approve = match field(&form, "action") {
        "approve" => true,
        "deny" => false,
        _ => return render(StatusCode::BAD_REQUEST, "Connect a computer", &device_code_form("Choose approve or deny."), &c, Some(&s)),
    };
    let decided = db::decide_device(&app.db.lock().unwrap(), &code, s.user.id, approve).unwrap_or(false);
    let (status, body) = match (decided, approve) {
        (true, true) => (StatusCode::OK, "<h1>Approved</h1><p class=\"ok\">The computer is connected. You can close this page.</p>".to_string()),
        (true, false) => (StatusCode::OK, "<h1>Denied</h1><p class=\"note\">The computer was not connected.</p>".to_string()),
        (false, _) => (StatusCode::NOT_FOUND, device_code_form("That code is unknown or it expired. Start again on the computer.")),
    };
    render(status, "Connect a computer", &body, &c, Some(&s))
}

pub async fn not_found(headers: HeaderMap, uri: axum::http::Uri) -> Response {
    if uri.path().starts_with("/auth/") || uri.path().starts_with("/internal/") {
        return axum::response::IntoResponse::into_response(backend_common::ApiError::not_found("no such endpoint"));
    }
    let _ = headers;
    respond(StatusCode::NOT_FOUND, page("Not found", STYLESHEET, "", "<h1>Not found</h1><p><a href=\"/account\">Your account</a></p>"))
}
