//! auth-api, end to end: the real app on an ephemeral port with a temporary SQLite file.

mod common;

use backend_common::ratelimit::Limit;
use common::*;
use serde_json::json;

fn api(auth: &Auth) -> reqwest::Client {
    let _ = auth;
    reqwest::Client::new()
}

async fn post_json(auth: &Auth, path: &str, body: serde_json::Value) -> Page {
    page(api(auth).post(format!("{}{path}", auth.url)).json(&body)).await
}

async fn whoami(auth: &Auth, token: &str) -> Page {
    page(api(auth).get(format!("{}/auth/v1/whoami", auth.url)).bearer_auth(token)).await
}

async fn introspect(auth: &Auth, secret: &str, token: &str) -> Page {
    page(api(auth).post(format!("{}/internal/introspect", auth.url)).header("X-Internal-Secret", secret).json(&json!({ "token": token }))).await
}

#[tokio::test]
async fn signup_verify_signin_and_reset() {
    let auth = start_auth(|_| {}).await;
    let b = Browser::new(&auth.url);

    // Refusals leave no account behind.
    for (email, handle, password) in [
        ("not-an-email", "evan", PASSWORD),
        ("evan@example.com", "Ev", PASSWORD),
        ("evan@example.com", "has space", PASSWORD),
        ("evan@example.com", "evan", "too short"),
    ] {
        let p = b.submit("/signup", "/signup", &[("email", email), ("handle", handle), ("password", password)]).await;
        assert_eq!(p.status, 400, "{email} {handle} {password}");
        assert!(p.body.contains("class=\"error\""));
    }

    let p = b.sign_up("Evan@Example.com", "evan").await;
    assert_eq!((p.status, p.location()), (303, "/account"));
    let session = p.cookies().into_iter().find(|c| c.starts_with("ecm_session=ecm_s_")).expect("a session cookie");
    assert!(session.contains("HttpOnly") && session.contains("SameSite=Lax") && session.contains("Path=/") && session.contains("Max-Age=2592000"));
    assert!(!session.contains("Secure"), "http origin: {session}");

    // The password is stored as Argon2id, never as text.
    let stored: String = auth.app.db.lock().unwrap().query_row("SELECT password_hash FROM users WHERE email = 'evan@example.com'", [], |r| r.get(0)).unwrap();
    assert!(stored.starts_with("$argon2id$") && !stored.contains(PASSWORD));

    let account = b.get("/account").await;
    assert_eq!(account.status, 200);
    assert!(account.body.contains("not confirmed") && account.body.contains("evan@example.com"));

    // The same email or handle again is refused.
    let other = Browser::new(&auth.url);
    assert_eq!(other.sign_up("evan@example.com", "evan2").await.status, 400);
    assert_eq!(other.sign_up("evan2@example.com", "evan").await.status, 400);

    // Verify by the mailed link; a link works once.
    let link = mailed_link(&auth, "evan@example.com", "/verify").await.unwrap();
    let verified = Browser::new(&auth.url).get(&link).await;
    assert_eq!(verified.status, 200);
    assert!(verified.body.contains("Email confirmed"));
    assert_eq!(b.get(&link).await.status, 400);
    assert_eq!(b.get("/verify?token=nonsense").await.status, 400);
    let account = b.get("/account").await;
    assert!(account.body.contains("confirmed") && !account.body.contains("not confirmed"));

    // Sign out, then the account page asks to sign in.
    let out = b.submit("/account", "/signout", &[]).await;
    assert_eq!((out.status, out.location()), (303, "/signin"));
    let p = b.get("/account").await;
    assert_eq!((p.status, p.location()), (303, "/signin?next=%2Faccount"));

    // Sign in: a wrong password and an unknown account look the same.
    let wrong = b.sign_in("evan@example.com", "wrong password!").await;
    let unknown = b.sign_in("nobody@example.com", "wrong password!").await;
    assert_eq!((wrong.status, unknown.status), (401, 401));
    assert!(wrong.body.contains("Wrong email or password.") && unknown.body.contains("Wrong email or password."));
    let p = b.sign_in("evan@example.com", PASSWORD).await;
    assert_eq!((p.status, p.location()), (303, "/account"));
    assert_eq!(b.get("/account").await.status, 200);

    // A signed-in visitor is sent on from the sign-in page.
    assert_eq!(b.get("/signin").await.location(), "/account");
    // `next` only leads to this site.
    let p = Browser::new(&auth.url).submit("/signin", "/signin", &[("email", "evan@example.com"), ("password", PASSWORD), ("next", "https://evil.example/")]).await;
    assert_eq!(p.location(), "/account");
    let p = Browser::new(&auth.url).submit("/signin", "/signin", &[("email", "evan@example.com"), ("password", PASSWORD), ("next", "//evil.example/")]).await;
    assert_eq!(p.location(), "/account");
    let p = Browser::new(&auth.url).submit("/signin", "/signin", &[("email", "evan@example.com"), ("password", PASSWORD), ("next", "/device?code=ABCD-EFGH")]).await;
    assert_eq!(p.location(), "/device?code=ABCD-EFGH");

    // Forgot: the answer does not tell whether the account exists; only a real account gets mail.
    let anon = Browser::new(&auth.url);
    let known = anon.submit("/forgot", "/forgot", &[("email", "evan@example.com")]).await;
    let unknown = anon.submit("/forgot", "/forgot", &[("email", "nobody@example.com")]).await;
    assert_eq!((known.status, unknown.status), (200, 200));
    assert_eq!(known.body, unknown.body);
    let reset = mailed_link(&auth, "evan@example.com", "/reset").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!auth.app.outbox.lock().unwrap().iter().any(|m| m.to == "nobody@example.com"));

    // Reset: the link shows a form, refuses a short password, works once and ends old sessions.
    assert_eq!(anon.get("/reset?token=nonsense").await.status, 400);
    assert_eq!(anon.get(&reset).await.status, 200);
    let link_token = reset.split("token=").nth(1).unwrap();
    assert_eq!(anon.submit(&reset, "/reset", &[("token", link_token), ("password", "short")]).await.status, 400);
    let done = anon.submit(&reset, "/reset", &[("token", link_token), ("password", "a brand new password")]).await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(anon.get(&reset).await.status, 400);
    assert_eq!(b.get("/account").await.status, 303, "the old session ended");
    assert_eq!(b.sign_in("evan@example.com", PASSWORD).await.status, 401);
    assert_eq!(b.sign_in("evan@example.com", "a brand new password").await.status, 303);

    // Change the password on the account page: the current one is needed; other browsers are signed out.
    let second = Browser::new(&auth.url);
    assert_eq!(second.sign_in("evan@example.com", "a brand new password").await.status, 303);
    let p = b.submit("/account", "/account", &[("action", "password"), ("current", "not it at all"), ("new", "yet another password")]).await;
    assert_eq!(p.status, 400);
    let p = b.submit("/account", "/account", &[("action", "password"), ("current", "a brand new password"), ("new", "yet another password")]).await;
    assert_eq!((p.status, p.location()), (303, "/account?done=password"));
    assert_eq!(b.get("/account").await.status, 200);
    assert_eq!(second.get("/account").await.status, 303);
    assert_eq!(second.sign_in("evan@example.com", "yet another password").await.status, 303);
}

#[tokio::test]
async fn resend_verification_replaces_the_old_link() {
    let auth = start_auth(|_| {}).await;
    let b = new_user(&auth, "rhea", false).await;
    let first = mailed_link(&auth, "rhea@example.com", "/verify").await.unwrap();
    let p = b.submit("/account", "/account", &[("action", "resend")]).await;
    assert_eq!((p.status, p.location()), (303, "/account?done=sent"));
    let mut second = first.clone();
    for _ in 0..100 {
        second = mailed_link(&auth, "rhea@example.com", "/verify").await.unwrap();
        if second != first {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_ne!(first, second);
    assert_eq!(b.get(&first).await.status, 400);
    assert_eq!(b.get(&second).await.status, 200);
}

#[tokio::test]
async fn forms_need_a_csrf_token_and_pages_carry_security_headers() {
    let auth = start_auth(|_| {}).await;
    let b = Browser::new(&auth.url);
    let shown = b.get("/signup").await;
    let csrf = hidden(&shown.body, "csrf").unwrap();
    let fields = [("email", "mallory@example.com"), ("handle", "mallory"), ("password", PASSWORD)];

    // No token, a wrong token, a token without the cookie, and a foreign Origin are all refused.
    let p = page(b.http.post(format!("{}/signup", auth.url)).form(&fields)).await;
    assert_eq!(p.status, 403);
    let mut with = fields.to_vec();
    with.push(("csrf", "0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(page(b.http.post(format!("{}/signup", auth.url)).form(&with)).await.status, 403);
    let mut with = fields.to_vec();
    with.push(("csrf", &csrf));
    let no_cookie = reqwest::Client::new();
    assert_eq!(page(no_cookie.post(format!("{}/signup", auth.url)).form(&with)).await.status, 403);
    assert_eq!(page(b.http.post(format!("{}/signup", auth.url)).header("Origin", "https://evil.example").form(&with)).await.status, 403);
    let users: i64 = auth.app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap();
    assert_eq!(users, 0);
    // Our own origin is fine.
    let ok = page(b.http.post(format!("{}/signup", auth.url)).header("Origin", "http://repo.test").form(&with)).await;
    assert_eq!(ok.status, 303, "{}", ok.body);
    // Signing out needs the token too.
    assert_eq!(page(b.http.post(format!("{}/signout", auth.url)).form(&[("x", "y")])).await.status, 403);
    assert_eq!(b.get("/account").await.status, 200);

    for path in ["/signin", "/signup", "/forgot", "/account", "/nothing-here"] {
        let p = Browser::new(&auth.url).get(path).await;
        if p.status == 303 {
            continue;
        }
        let h = |n: &str| p.headers.get(n).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        assert!(h("content-security-policy").contains("default-src 'none'") && h("content-security-policy").contains("frame-ancestors 'none'"), "{path}");
        assert_eq!(h("x-content-type-options"), "nosniff", "{path}");
        assert_eq!(h("x-frame-options"), "DENY", "{path}");
        assert_eq!(h("referrer-policy"), "no-referrer", "{path}");
        assert_eq!(h("cache-control"), "no-store", "{path}");
        assert!(h("content-type").starts_with("text/html"), "{path}");
        assert!(p.body.contains("name=\"viewport\""), "{path}");
    }
    let css = b.get("/static/auth/style.css").await;
    assert_eq!(css.status, 200);
    assert!(css.headers["content-type"].to_str().unwrap().starts_with("text/css"));
}

#[tokio::test]
async fn cookies_are_secure_on_an_https_origin() {
    let auth = start_auth(|c| c.public_origin = "https://repo.test".into()).await;
    let shown = page(reqwest::Client::new().get(format!("{}/signup", auth.url))).await;
    let cookie = shown.cookies().into_iter().find(|c| c.starts_with("__Host-ecm_csrf=")).expect("a CSRF cookie");
    assert!(cookie.contains("Secure") && cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax") && cookie.contains("Path=/"));
    // A browser would refuse to send Secure cookies over this plain test connection, so send them by hand.
    let csrf = hidden(&shown.body, "csrf").unwrap();
    let p = page(
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
            .post(format!("{}/signup", auth.url))
            .header("Cookie", format!("__Host-ecm_csrf={csrf}"))
            .form(&[("csrf", csrf.as_str()), ("email", "s@example.com"), ("handle", "secure"), ("password", PASSWORD)]),
    )
    .await;
    assert_eq!(p.status, 303, "{}", p.body);
    let session = p.cookies().into_iter().find(|c| c.starts_with("ecm_session=")).unwrap();
    assert!(session.contains("; Secure") && session.contains("HttpOnly") && session.contains("SameSite=Lax"));
}

#[tokio::test]
async fn device_flow_pending_approve_deny_and_expiry() {
    let auth = start_auth(|_| {}).await;
    let user = new_user(&auth, "dana", true).await;

    let start = post_json(&auth, "/auth/v1/device/start", json!({ "client_name": "<b>Workshop</b> PC", "scopes": ["social", "publish"] })).await;
    assert_eq!(start.status, 200, "{}", start.body);
    let s = start.json();
    let (device_code, user_code) = (s["device_code"].as_str().unwrap().to_string(), s["user_code"].as_str().unwrap().to_string());
    assert_eq!((s["interval"].as_i64(), s["expires_in"].as_i64()), (Some(3), Some(600)));
    assert_eq!(s["verification_uri"], "http://repo.test/device");
    assert_eq!(s["verification_uri_complete"], format!("http://repo.test/device?code={user_code}"));
    let shape: Vec<bool> = user_code.chars().map(|c| c == '-').collect();
    assert_eq!(shape, [false, false, false, false, true, false, false, false, false], "{user_code}");

    // Pending until someone decides.
    let poll = post_json(&auth, "/auth/v1/device/token", json!({ "device_code": device_code })).await;
    assert_eq!((poll.status, poll.json()["error"].as_str().unwrap().to_string()), (428, "pending".to_string()));

    // The page signs in first, then shows who asks (escaped) and for what.
    let stranger = Browser::new(&auth.url);
    let p = stranger.get(&format!("/device?code={user_code}")).await;
    assert_eq!((p.status, p.location().to_string()), (303, format!("/signin?next=%2Fdevice%3Fcode%3D{user_code}")));
    let lower = user_code.to_lowercase().replace('-', "");
    let shown = user.get(&format!("/device?code={lower}")).await;
    assert_eq!(shown.status, 200);
    assert!(shown.body.contains("&lt;b&gt;Workshop&lt;/b&gt; PC") && !shown.body.contains("<b>Workshop"));
    assert!(shown.body.contains("social, publish") && shown.body.contains(&user_code));
    assert_eq!(user.get("/device?code=ZZZZ-ZZZZ").await.status, 404);

    let page_path = format!("/device?code={user_code}");
    let approved = user.submit(&page_path, "/device", &[("code", &user_code), ("action", "approve")]).await;
    assert_eq!(approved.status, 200);
    assert!(approved.body.contains("Approved"));

    let poll = post_json(&auth, "/auth/v1/device/token", json!({ "device_code": device_code })).await;
    assert_eq!(poll.status, 200, "{}", poll.body);
    let t = poll.json();
    let token = t["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("ecm_d_"));
    assert_eq!((t["handle"].as_str(), t["scopes"].clone()), (Some("dana"), json!(["social", "publish"])));
    // The token is handed out once.
    assert_eq!(post_json(&auth, "/auth/v1/device/token", json!({ "device_code": device_code })).await.status, 410);
    // It works, carries the device's name and is listed on the account page.
    let who = whoami(&auth, &token).await.json();
    assert_eq!(who, json!({ "handle": "dana", "email_verified": true, "scopes": ["social", "publish"], "token_name": "<b>Workshop</b> PC" }));
    let account = user.get("/account").await;
    assert!(account.body.contains("&lt;b&gt;Workshop&lt;/b&gt; PC") && account.body.contains("device"));
    // Only its hash is stored.
    let hashes: Vec<String> = {
        let conn = auth.app.db.lock().unwrap();
        let mut st = conn.prepare("SELECT hash FROM tokens").unwrap();
        let rows = st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        rows
    };
    assert!(hashes.contains(&backend_common::token::hash(&token)) && !hashes.contains(&token));

    // Denied.
    let s = post_json(&auth, "/auth/v1/device/start", json!({ "client_name": "Laptop", "scopes": ["social"] })).await.json();
    let (device_code, user_code) = (s["device_code"].as_str().unwrap().to_string(), s["user_code"].as_str().unwrap().to_string());
    let denied = user.submit(&format!("/device?code={user_code}"), "/device", &[("code", &user_code), ("action", "deny")]).await;
    assert!(denied.status == 200 && denied.body.contains("Denied"));
    let poll = post_json(&auth, "/auth/v1/device/token", json!({ "device_code": device_code })).await;
    assert_eq!((poll.status, poll.json()["error"].as_str().unwrap().to_string()), (403, "forbidden".to_string()));
    // A decision is final.
    assert_eq!(user.submit("/account", "/device", &[("code", &user_code), ("action", "approve")]).await.status, 404);

    // Expired: neither approvable nor pollable.
    let s = post_json(&auth, "/auth/v1/device/start", json!({ "client_name": "Old" })).await.json();
    let (device_code, user_code) = (s["device_code"].as_str().unwrap().to_string(), s["user_code"].as_str().unwrap().to_string());
    auth.app.db.lock().unwrap().execute("UPDATE device_codes SET expires_at = ?1 WHERE user_code = ?2", rusqlite::params![backend_common::now() - 1, user_code]).unwrap();
    assert_eq!(user.get(&format!("/device?code={user_code}")).await.status, 404);
    assert_eq!(user.submit("/account", "/device", &[("code", &user_code), ("action", "approve")]).await.status, 404);
    let poll = post_json(&auth, "/auth/v1/device/token", json!({ "device_code": device_code })).await;
    assert_eq!((poll.status, poll.json()["error"].as_str().unwrap().to_string()), (410, "gone".to_string()));

    // Bad requests are JSON errors.
    let p = post_json(&auth, "/auth/v1/device/start", json!({ "client_name": "x", "scopes": ["admin"] })).await;
    assert_eq!((p.status, p.json()["error"].as_str().unwrap().to_string()), (400, "bad_request".to_string()));
    let p = post_json(&auth, "/auth/v1/device/start", json!({ "client_name": "x", "scopes": [] })).await;
    assert_eq!(p.status, 400);
    let p = page(api(&auth).post(format!("{}/auth/v1/device/start", auth.url)).body("{not json")).await;
    assert_eq!((p.status, p.json()["error"].as_str().unwrap().to_string()), (400, "bad_request".to_string()));
    let p = post_json(&auth, "/auth/v1/device/token", json!({ "device_code": "ecm_dc_unknown" })).await;
    assert_eq!(p.status, 410);
    let p = page(api(&auth).post(format!("{}/auth/v1/device/start", auth.url)).body(vec![b' '; 100_000])).await;
    assert_eq!((p.status, p.json()["error"].as_str().unwrap().to_string()), (413, "too_large".to_string()));
    let p = page(api(&auth).get(format!("{}/auth/v1/nothing", auth.url))).await;
    assert_eq!((p.status, p.json()["error"].as_str().unwrap().to_string()), (404, "not_found".to_string()));
}

#[tokio::test]
async fn api_keys_are_created_shown_once_and_revoked() {
    let auth = start_auth(|_| {}).await;
    let user = new_user(&auth, "kira", true).await;
    let key = user.api_key("build <server>", &["publish"]).await;
    assert!(key.starts_with("ecm_k_"));

    let who = whoami(&auth, &key).await;
    assert_eq!(who.json(), json!({ "handle": "kira", "email_verified": true, "scopes": ["publish"], "token_name": "build <server>" }));
    // The account page lists it by name (escaped) but never shows the key again.
    let account = user.get("/account").await;
    assert!(account.body.contains("build &lt;server&gt;") && !account.body.contains(&key));
    // A key needs a name and a scope.
    assert_eq!(user.submit("/account", "/account", &[("action", "create_key"), ("name", "x")]).await.status, 400);
    assert_eq!(user.submit("/account", "/account", &[("action", "create_key"), ("name", " "), ("scope_social", "on")]).await.status, 400);

    // Someone else cannot revoke it.
    let token_id = hidden(&account.body, "token_id").unwrap();
    let other = new_user(&auth, "omar", true).await;
    other.submit("/account", "/account", &[("action", "revoke"), ("token_id", &token_id)]).await;
    assert_eq!(whoami(&auth, &key).await.status, 200);

    // Revoke on the account page.
    let p = user.submit("/account", "/account", &[("action", "revoke"), ("token_id", &token_id)]).await;
    assert_eq!((p.status, p.location()), (303, "/account?done=revoked"));
    let gone = whoami(&auth, &key).await;
    assert_eq!((gone.status, gone.json()["error"].as_str().unwrap().to_string()), (401, "unauthorized".to_string()));
    assert!(!user.get("/account").await.body.contains("build &lt;server&gt;"));

    // Revoke the presented token over the API.
    let key = user.api_key("second", &["social", "publish"]).await;
    let p = page(api(&auth).post(format!("{}/auth/v1/token/revoke", auth.url)).bearer_auth(&key)).await;
    assert_eq!((p.status, p.body.as_str()), (204, ""));
    assert_eq!(whoami(&auth, &key).await.status, 401);
    assert_eq!(page(api(&auth).post(format!("{}/auth/v1/token/revoke", auth.url)).bearer_auth(&key)).await.status, 401);

    // No token and a made-up token are refused.
    assert_eq!(page(api(&auth).get(format!("{}/auth/v1/whoami", auth.url))).await.status, 401);
    assert_eq!(whoami(&auth, "ecm_k_0000").await.status, 401);
}

#[tokio::test]
async fn introspection_answers_for_tokens_and_needs_the_secret() {
    let auth = start_auth(|_| {}).await;
    let user = new_user(&auth, "ines", false).await;
    let key = user.api_key("k", &["social"]).await;

    let p = introspect(&auth, "wrong secret", &key).await;
    assert_eq!((p.status, p.json()["error"].as_str().unwrap().to_string()), (403, "forbidden".to_string()));
    let p = page(api(&auth).post(format!("{}/internal/introspect", auth.url)).json(&json!({ "token": key }))).await;
    assert_eq!(p.status, 403);

    let p = introspect(&auth, SECRET, &key).await;
    assert_eq!(p.status, 200);
    let a = p.json();
    let id: i64 = auth.app.db.lock().unwrap().query_row("SELECT id FROM users WHERE handle = 'ines'", [], |r| r.get(0)).unwrap();
    assert_eq!(a, json!({ "active": true, "user_id": id, "handle": "ines", "email_verified": false, "scopes": ["social"], "admin": false }));
    assert_eq!(introspect(&auth, SECRET, "ecm_k_nope").await.json(), json!({ "active": false }));

    // A web session is not an API token.
    let b = Browser::new(&auth.url);
    let signed_in = b.sign_in("ines@example.com", PASSWORD).await;
    let cookie = signed_in.cookies().into_iter().find(|c| c.starts_with("ecm_session=")).unwrap();
    let session = cookie.trim_start_matches("ecm_session=").split(';').next().unwrap().to_string();
    assert!(session.starts_with("ecm_s_"));
    assert_eq!(introspect(&auth, SECRET, &session).await.json(), json!({ "active": false }));

    // Verification shows up; the last use is remembered.
    let link = mailed_link(&auth, "ines@example.com", "/verify").await.unwrap();
    assert_eq!(user.get(&link).await.status, 200);
    assert_eq!(introspect(&auth, SECRET, &key).await.json()["email_verified"], true);
    let used: Option<i64> = auth.app.db.lock().unwrap().query_row("SELECT last_used_at FROM tokens WHERE kind = 'key'", [], |r| r.get(0)).unwrap();
    assert!(used.is_some());

    // An ADMIN_EMAILS account is an admin only once its address is confirmed.
    let admin = new_user(&auth, "admin", false).await;
    let admin_key = admin.api_key("k", &["social", "publish"]).await;
    let a = introspect(&auth, SECRET, &admin_key).await.json();
    assert_eq!((a["admin"].clone(), a["scopes"].clone()), (json!(false), json!(["social", "publish"])));
    let link = mailed_link(&auth, ADMIN_EMAIL, "/verify").await.unwrap();
    assert_eq!(admin.get(&link).await.status, 200);
    let a = introspect(&auth, SECRET, &admin_key).await.json();
    assert_eq!((a["admin"].clone(), a["scopes"].clone()), (json!(true), json!(["social", "publish", "admin"])));
    assert_eq!(whoami(&auth, &admin_key).await.json()["scopes"], json!(["social", "publish", "admin"]));
    // Or when the database says so.
    auth.app.db.lock().unwrap().execute("UPDATE users SET is_admin = 1 WHERE handle = 'ines'", []).unwrap();
    assert_eq!(introspect(&auth, SECRET, &key).await.json()["admin"], true);

    // Revoked means inactive.
    let p = page(api(&auth).post(format!("{}/auth/v1/token/revoke", auth.url)).bearer_auth(&key)).await;
    assert_eq!(p.status, 204);
    assert_eq!(introspect(&auth, SECRET, &key).await.json(), json!({ "active": false }));
}

#[tokio::test]
async fn rate_limits_per_address_and_per_account() {
    // The contract's limits: sign-in 10/min, sign-up 5/hour, forgot 5/hour, device start 20/min.
    let auth = start_auth(|c| c.limits = auth_api::Limits::default()).await;
    assert_eq!(auth.app.cfg.limits.signin, Limit::per_minute(10));
    assert_eq!(auth.app.cfg.limits.signup, Limit::per_hour(5));
    assert_eq!(auth.app.cfg.limits.forgot, Limit::per_hour(5));
    assert_eq!(auth.app.cfg.limits.device_start, Limit::per_minute(20));

    for i in 0..20 {
        assert_eq!(post_json(&auth, "/auth/v1/device/start", json!({ "client_name": "pc" })).await.status, 200, "start {i}");
    }
    let p = post_json(&auth, "/auth/v1/device/start", json!({ "client_name": "pc" })).await;
    assert_eq!((p.status, p.json()["error"].as_str().unwrap().to_string()), (429, "rate_limited".to_string()));

    let b = Browser::new(&auth.url);
    for i in 0..10 {
        assert_eq!(b.sign_in("a@example.com", "wrong password!").await.status, 401, "sign-in {i}");
    }
    assert_eq!(b.sign_in("a@example.com", "wrong password!").await.status, 429);

    for i in 0..5 {
        assert_eq!(b.submit("/forgot", "/forgot", &[("email", "a@example.com")]).await.status, 200, "forgot {i}");
    }
    assert_eq!(b.submit("/forgot", "/forgot", &[("email", "a@example.com")]).await.status, 429);

    for i in 0..5 {
        let fresh = Browser::new(&auth.url);
        assert_eq!(fresh.sign_up(&format!("u{i}@example.com"), &format!("user{i}")).await.status, 303, "sign-up {i}");
    }
    assert_eq!(Browser::new(&auth.url).sign_up("u9@example.com", "user9").await.status, 429);
}

#[tokio::test]
async fn account_limits_hold_across_addresses_and_forwarded_for_needs_trust() {
    // Behind the proxy: each forwarded address has its own budget, each account too.
    let auth = start_auth(|c| {
        c.trust_proxy = true;
        c.limits = auth_api::Limits::default();
    })
    .await;
    let start = |ip: &'static str| {
        let req = reqwest::Client::new().post(format!("{}/auth/v1/device/start", auth.url)).header("X-Forwarded-For", ip).json(&json!({ "client_name": "pc" }));
        async move { page(req).await.status }
    };
    for _ in 0..20 {
        assert_eq!(start("203.0.113.1").await, 200);
    }
    assert_eq!(start("203.0.113.1").await, 429);
    assert_eq!(start("203.0.113.2").await, 200);
    // The proxy appends the real address: a client-supplied first entry does not help.
    assert_eq!(start("198.51.100.77, 203.0.113.1").await, 429);

    // One account, many addresses: the per-account limit stops password guessing.
    let user = new_user(&auth, "vic", false).await;
    let _ = user;
    let mut statuses = Vec::new();
    for i in 0..11 {
        let b = Browser::new(&auth.url);
        let shown = page(b.http.get(format!("{}/signin", auth.url))).await;
        let csrf = hidden(&shown.body, "csrf").unwrap();
        let p = page(
            b.http
                .post(format!("{}/signin", auth.url))
                .header("X-Forwarded-For", format!("192.0.2.{i}"))
                .form(&[("csrf", csrf.as_str()), ("email", "vic@example.com"), ("password", "wrong password!")]),
        )
        .await;
        statuses.push(p.status);
    }
    assert_eq!(statuses[..10], [401; 10]);
    assert_eq!(statuses[10], 429);

    // Not behind a proxy: the header is ignored, so it cannot be used to dodge the limit.
    let direct = start_auth(|c| c.limits = auth_api::Limits::default()).await;
    let mut last = 0;
    for i in 0..21 {
        let req = reqwest::Client::new()
            .post(format!("{}/auth/v1/device/start", direct.url))
            .header("X-Forwarded-For", format!("203.0.113.{i}"))
            .json(&json!({ "client_name": "pc" }));
        last = page(req).await.status;
    }
    assert_eq!(last, 429);
}

#[tokio::test]
async fn api_responses_have_content_length_on_a_raw_socket() {
    let auth = start_auth(|_| {}).await;
    let user = new_user(&auth, "sock", true).await;
    let key = user.api_key("k", &["social"]).await;
    let body = r#"{"client_name":"pc","scopes":["social"]}"#;

    for version in ["1.0", "1.1"] {
        // 200 with a JSON body.
        let request = format!(
            "POST /auth/v1/device/start HTTP/{version}\r\nHost: repo.test\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (head, got) = raw(auth.addr, request.as_bytes()).await;
        assert_eq!(raw_status(&head), 200, "{head}");
        assert_plain_framing(&head, &got);
        let device_code = serde_json::from_slice::<serde_json::Value>(&got).unwrap()["device_code"].as_str().unwrap().to_string();

        // 428 (an error body), 401, 404 and 400.
        let poll = format!(r#"{{"device_code":"{device_code}"}}"#);
        let cases = [
            (format!("POST /auth/v1/device/token HTTP/{version}\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{poll}", poll.len()), 428),
            (format!("GET /auth/v1/whoami HTTP/{version}\r\nHost: x\r\nConnection: close\r\n\r\n"), 401),
            (format!("GET /auth/v1/whoami HTTP/{version}\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {key}\r\n\r\n"), 200),
            (format!("GET /auth/v1/missing HTTP/{version}\r\nHost: x\r\nConnection: close\r\n\r\n"), 404),
            (format!("POST /auth/v1/device/token HTTP/{version}\r\nHost: x\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{{]"), 400),
            (format!("GET /auth/v1/device/start HTTP/{version}\r\nHost: x\r\nConnection: close\r\n\r\n"), 405),
            (format!("GET /auth/v1/health HTTP/{version}\r\nHost: x\r\nConnection: close\r\n\r\n"), 200),
            (format!("GET /healthz HTTP/{version}\r\nHost: x\r\nConnection: close\r\n\r\n"), 200),
        ];
        for (request, status) in cases {
            let (head, got) = raw(auth.addr, request.as_bytes()).await;
            assert_eq!(raw_status(&head), status, "{request}\n{head}");
            assert_plain_framing(&head, &got);
        }
    }

    // 204: no body, not chunked, and the connection closes.
    let key = user.api_key("to revoke", &["social"]).await;
    let request = format!("POST /auth/v1/token/revoke HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {key}\r\nContent-Length: 0\r\n\r\n");
    let (head, got) = raw(auth.addr, request.as_bytes()).await;
    assert_eq!(raw_status(&head), 204, "{head}");
    assert!(got.is_empty() && raw_header(&head, "transfer-encoding").is_none());
    println!("204 head:\n{head}");
}

#[tokio::test]
async fn admin_command_adds_users_and_keys() {
    let auth = start_auth(|_| {}).await;
    let run = |words: &'static [&'static str]| {
        let app = auth.app.clone();
        async move { auth_api::admin::run_on(&app, words).await }
    };
    assert!(auth_api::admin::run(&[]).await.unwrap().contains("user-add <email> <handle> <password>"));
    assert!(run(&["nonsense"]).await.unwrap_err().contains("usage: auth-api admin"));
    assert_eq!(run(&["users"]).await.unwrap(), "no accounts\n");

    // The password policy and the handle and email rules hold here too.
    assert!(run(&["user-add", "bot@example.com", "bot", "short"]).await.unwrap_err().contains("at least 10"));
    assert!(run(&["user-add", "not-an-email", "bot", PASSWORD]).await.unwrap_err().contains("not an email"));
    assert!(run(&["user-add", "bot@example.com", "B", PASSWORD]).await.unwrap_err().contains("handle"));
    assert_eq!(run(&["users"]).await.unwrap(), "no accounts\n");

    assert_eq!(run(&["user-add", "Bot@Example.com", "bot", PASSWORD]).await.unwrap(), "created bot <bot@example.com> (email confirmed)\n");
    // Duplicates are refused, by email and by handle.
    assert!(run(&["user-add", "bot@example.com", "bot2", PASSWORD]).await.unwrap_err().contains("email bot@example.com exists"));
    assert!(run(&["user-add", "bot2@example.com", "bot", PASSWORD]).await.unwrap_err().contains("handle bot is taken"));
    run(&["user-add", ADMIN_EMAIL, "admin", PASSWORD]).await.unwrap();
    assert_eq!(run(&["users"]).await.unwrap(), "admin\tadmin@example.com\tverified=yes\tadmin=yes\nbot\tbot@example.com\tverified=yes\tadmin=no\n");

    // The account works in the browser, with an Argon2id hash like any other.
    let b = Browser::new(&auth.url);
    assert_eq!(b.sign_in("bot@example.com", PASSWORD).await.status, 303);
    assert!(!b.get("/account").await.body.contains("not confirmed"));

    // key-add prints only the key; it works on whoami with the default and the given scopes.
    let out = run(&["key-add", "bot", "ci runner"]).await.unwrap();
    let key = out.strip_suffix('\n').unwrap();
    assert!(key.starts_with("ecm_k_") && key.len() == 70 && !key.contains(char::is_whitespace), "{out:?}");
    assert_eq!(whoami(&auth, key).await.json(), json!({ "handle": "bot", "email_verified": true, "scopes": ["social", "publish"], "token_name": "ci runner" }));
    let social = run(&["key-add", "bot", "reader", "social"]).await.unwrap();
    assert_eq!(whoami(&auth, social.trim()).await.json()["scopes"], json!(["social"]));
    assert!(b.get("/account").await.body.contains("ci runner"));
    assert!(run(&["key-add", "bot", "x", "admin"]).await.unwrap_err().contains("unknown scope"));
    assert!(run(&["key-add", "nobody", "x"]).await.unwrap_err().contains("no account with the handle nobody"));
    assert!(run(&["key-add", "bot", " "]).await.unwrap_err().contains("name"));
}
