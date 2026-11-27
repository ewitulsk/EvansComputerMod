//! A publish round trip against a RUNNING stack (the compose stack: Caddy, both
//! services, MinIO). Ignored by `cargo test`; run it with the stack up:
//!
//!   ECM_STACK_URL=http://localhost:8080 \
//!   ECM_MAIL_LOG_CMD="docker compose -f $PWD/docker-compose.yml logs --no-color auth-api" \
//!   cargo test -p repo-api --test stack -- --ignored --nocapture
//!
//! `ECM_MAIL_LOG_CMD` prints what auth-api logged (EMAIL_PROVIDER=log), which is
//! where the verification link is. Give the compose file as an absolute path: cargo
//! runs the test from the crate directory.

#[path = "../../auth-api/tests/common/mod.rs"]
mod common;

use common::*;
use ecm_pkg::package;
use std::net::ToSocketAddrs;

fn verify_link(log_cmd: &str, email: &str) -> Option<String> {
    let out = std::process::Command::new("sh").arg("-c").arg(log_cmd).output().expect("ECM_MAIL_LOG_CMD runs");
    let log = String::from_utf8_lossy(&out.stdout).to_string();
    let after = &log[log.rfind(&format!("To: {email}"))?..];
    let at = after.find("/verify?token=")?;
    let token: String = after[at + 14..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    Some(format!("/verify?token={token}"))
}

#[tokio::test]
#[ignore = "needs the compose stack: see the top of this file"]
async fn publish_round_trip_through_the_proxy() {
    let base = std::env::var("ECM_STACK_URL").expect("ECM_STACK_URL").trim_end_matches('/').to_string();
    let log_cmd = std::env::var("ECM_MAIL_LOG_CMD").expect("ECM_MAIL_LOG_CMD");
    let unique = &backend_common::token::random_hex(4);
    let handle = format!("stack{unique}");
    let email = format!("{handle}@example.com");

    // Sign up, confirm the address with the mailed (logged) link, make a key: all through the proxy.
    let b = Browser::new(&base);
    let p = b.sign_up(&email, &handle).await;
    assert_eq!((p.status, p.location()), (303, "/account"), "{}", p.body);
    let mut link = None;
    for _ in 0..50 {
        link = verify_link(&log_cmd, &email);
        if link.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let link = link.expect("the verification mail is in the auth-api log");
    assert_eq!(b.get(&link).await.status, 200);
    let token = b.api_key("stack test", &["publish", "social"]).await;

    // Publish.
    let id = format!("io.{handle}.hello");
    let toml = format!("id = \"{id}\"\nname = \"Hello\"\nversion = \"1.0.0\"\nentry = \"main.py\"\nicon = \"icon.png\"\n");
    let (built, bytes) = package::build(vec![
        ("app.toml".into(), toml.into_bytes()),
        ("main.py".into(), b"print('hello')\n".to_vec()),
        ("icon.png".into(), b"\x89PNG stack icon".to_vec()),
    ])
    .unwrap();
    let http = reqwest::Client::new();
    let p = page(http.put(format!("{base}/v1/publish")).bearer_auth(&token).header("Content-Type", "application/octet-stream").body(bytes.clone())).await;
    assert_eq!(p.status, 201, "{}", p.body);
    let file = p.json()["file"].as_str().unwrap().to_string();

    // Index, download and verification against the repository key.
    let index = package::parse_index(&page(http.get(format!("{base}/r/community/index.json"))).await.body).unwrap();
    let key = page(http.get(format!("{base}/r/community/publisher.pub"))).await.body.trim().to_string();
    assert_eq!(index.publisher_key.as_deref(), Some(key.as_str()));
    let entry = index.packages.iter().find(|e| e.id == id).expect("the package is in the index");
    assert_eq!((entry.file.as_str(), entry.sha256.as_str(), entry.publisher.as_str()), (file.as_str(), built.sha256.as_str(), handle.as_str()));
    let downloaded = http.get(format!("{base}/r/community/{file}")).send().await.unwrap().bytes().await.unwrap().to_vec();
    assert_eq!(downloaded, bytes);
    assert_eq!(package::verify(entry, index.publisher_key.as_deref(), Some(&key), &downloaded).unwrap(), package::Trust::Verified);
    let icon = http.get(format!("{base}/r/community/{file}/icon.png")).send().await.unwrap().bytes().await.unwrap().to_vec();
    assert_eq!(icon, b"\x89PNG stack icon");

    // The way the game asks: HTTP/1.0, Connection: close, on a plain socket, through the proxy.
    let authority = base.split("://").nth(1).unwrap();
    // `localhost` may resolve to ::1 first, where the stack does not listen.
    let addrs: Vec<_> = authority.to_socket_addrs().unwrap().collect();
    let addr = *addrs.iter().find(|a| a.is_ipv4()).unwrap_or(&addrs[0]);
    for path in ["/v1/health".to_string(), "/r/community/index.json".to_string(), format!("/r/community/{file}"), "/v1/packages".to_string(), "/v1/nothing".to_string()] {
        for version in ["1.0", "1.1"] {
            let request = format!("GET {path} HTTP/{version}\r\nHost: {authority}\r\nUser-Agent: ecm-pkg/1\r\nConnection: close\r\n\r\n");
            let (head, body) = raw(addr, request.as_bytes()).await;
            assert_plain_framing(&head, &body);
            assert!(raw_header(&head, "content-encoding").is_none(), "{head}");
            if path.ends_with(".ecmpkg") {
                assert_eq!(body, bytes);
            }
        }
    }
    let start = r#"{"client_name":"stack test"}"#;
    let request = format!("POST /auth/v1/device/start HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{start}", start.len());
    let (head, body) = raw(addr, request.as_bytes()).await;
    assert_eq!(raw_status(&head), 200, "{head}");
    assert_plain_framing(&head, &body);

    // The proxy does not route the internal call.
    let p = page(http.post(format!("{base}/internal/introspect")).header("X-Internal-Secret", "local-development-internal-secret").body(format!("{{\"token\":\"{token}\"}}"))).await;
    assert_eq!(p.status, 404, "{}", p.body);
    let who = page(http.get(format!("{base}/auth/v1/whoami")).bearer_auth(&token)).await.json();
    assert_eq!((who["handle"].as_str(), who["email_verified"].as_bool()), (Some(handle.as_str()), Some(true)));
    let liked = page(http.put(format!("{base}/v1/packages/{id}/like")).bearer_auth(&token)).await;
    assert_eq!(liked.status, 204);
    let d = page(http.get(format!("{base}/v1/packages/{id}")).bearer_auth(&token)).await.json();
    assert_eq!((d["likes"].as_i64(), d["liked"].as_bool(), d["owner"].as_str()), (Some(1), Some(true), Some(handle.as_str())));
    assert!(page(http.get(format!("{base}/"))).await.body.contains(&id));
    println!("published {id} 1.0.0 as {handle}; repository key {key}");
}
