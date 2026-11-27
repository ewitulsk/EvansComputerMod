//! repo-api, end to end: the real repo-api and auth-api on ephemeral ports, temporary
//! SQLite files and the `fs` blob store. Tokens are checked through introspection.

#[path = "../../auth-api/tests/common/mod.rs"]
mod common;

use backend_common::ratelimit::Limit;
use common::*;
use ecm_pkg::package;
use repo_api::{Config, StorageConfig};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;

/// The seed of the cross-check vector in `ecm_pkg::sign`; its public key is known.
const SEED: &str = "0707070707070707070707070707070707070707070707070707070707070707";
const REPO_KEY: &str = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";

struct Stack {
    auth: Auth,
    app: repo_api::App,
    addr: SocketAddr,
    url: String,
    blobs: PathBuf,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

async fn start(tweak: impl FnOnce(&mut Config)) -> Stack {
    let auth = start_auth(|_| {}).await;
    let dir = tempfile::tempdir().unwrap();
    let blobs = dir.path().join("blobs");
    let mut cfg = Config {
        public_origin: "http://repo.test".into(),
        listen: "127.0.0.1:0".into(),
        database_path: dir.path().join("repo.db").to_str().unwrap().into(),
        internal_secret: SECRET.into(),
        auth_internal_url: auth.url.clone(),
        storage: StorageConfig::Fs { dir: blobs.to_str().unwrap().into() },
        repo_signing_key: SEED.into(),
        max_package_bytes: 8 * 1024 * 1024,
        account_quota_bytes: 200 * 1024 * 1024,
        trust_proxy: false,
        upload_limit: Limit::per_hour(30),
        report_limit: Limit::per_hour(20),
    };
    tweak(&mut cfg);
    let app = repo_api::build(cfg).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(repo_api::serve(app.clone(), listener));
    Stack { auth, app, addr, url: format!("http://{addr}"), blobs, http: reqwest::Client::new(), _dir: dir }
}

impl Stack {
    /// A verified account and a token of its with these scopes.
    async fn token(&self, handle: &str, scopes: &[&str]) -> String {
        new_user(&self.auth, handle, true).await.api_key("test", scopes).await
    }

    async fn get(&self, path: &str, token: Option<&str>) -> Page {
        let mut r = self.http.get(format!("{}{path}", self.url));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        page(r).await
    }

    async fn bytes(&self, path: &str) -> (u16, Vec<u8>) {
        let r = self.http.get(format!("{}{path}", self.url)).send().await.unwrap();
        (r.status().as_u16(), r.bytes().await.unwrap().to_vec())
    }

    async fn send(&self, method: reqwest::Method, path: &str, token: Option<&str>, body: Option<Value>) -> Page {
        let mut r = self.http.request(method, format!("{}{path}", self.url));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        page(r).await
    }

    async fn post(&self, path: &str, token: &str, body: Value) -> Page {
        self.send(reqwest::Method::POST, path, Some(token), Some(body)).await
    }

    async fn publish(&self, token: Option<&str>, bytes: Vec<u8>) -> Page {
        let mut r = self.http.put(format!("{}/v1/publish", self.url)).header("Content-Type", "application/octet-stream").body(bytes);
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        page(r).await
    }

    async fn index(&self, channel: &str) -> package::Index {
        let p = self.get(&format!("/r/{channel}/index.json"), None).await;
        assert_eq!(p.status, 200, "{}", p.body);
        package::parse_index(&p.body).unwrap()
    }

    /// `id version` of every entry of a channel's index, in order.
    async fn listed(&self, channel: &str) -> Vec<String> {
        self.index(channel).await.packages.iter().map(|e| format!("{} {}", e.id, e.version)).collect()
    }
}

fn code(p: &Page) -> (u16, String) {
    (p.status, p.json()["error"].as_str().unwrap_or("").to_string())
}

fn err(status: u16, code: &str) -> (u16, String) {
    (status, code.to_string())
}

/// An app package.
fn app(id: &str, version: &str, name: &str, summary: &str) -> Vec<u8> {
    let toml = format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"{version}\"\nentry = \"main.py\"\nsummary = \"{summary}\"\n");
    package::build(vec![("app.toml".into(), toml.into_bytes()), ("main.py".into(), b"print('hi')\n".to_vec())]).unwrap().1
}

/// A package of another kind: `rest` is the manifest's tail, `files` the other members.
fn other(id: &str, version: &str, name: &str, rest: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let toml = format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"{version}\"\n{rest}");
    let mut all = vec![("app.toml".to_string(), toml.into_bytes())];
    all.extend(files.iter().map(|(p, d)| (p.to_string(), d.to_vec())));
    package::build(all).unwrap().1
}

/// The test the brief asks for: build with `ecm_pkg`, publish over HTTP, read the
/// index the way `pkg` does, download, and verify against the repository key.
#[tokio::test]
async fn publish_round_trip_verifies_against_the_repository_key() {
    let s = start(|_| {}).await;
    let token = s.token("evan", &["publish"]).await;

    let toml = "id = \"io.evan.notes\"\nname = \"Notes\"\nversion = \"1.2.0\"\nentry = \"main.py\"\nsummary = \"Take notes\"\n\
                publisher = \"Somebody Else\"\nicon = \"icon.png\"\npermissions = [\"network\"]\ndepends = [\"io.evan.lib >= 1.0.0\"]\n";
    let (built, bytes) = package::build(vec![
        ("app.toml".into(), toml.as_bytes().to_vec()),
        ("main.py".into(), b"print('notes')\n".to_vec()),
        ("icon.png".into(), b"\x89PNG icon".to_vec()),
        ("screenshots/one.png".into(), b"\x89PNG shot".to_vec()),
    ])
    .unwrap();

    // The author signs too.
    let (author_key, author_secret) = ecm_pkg::sign::keypair_from_seed(&[3u8; 32]);
    let author_signature = ecm_pkg::sign::sign(&author_secret, &package::signed_message("io.evan.notes", "1.2.0", &built.sha256));
    let p = page(
        s.http
            .put(format!("{}/v1/publish", s.url))
            .bearer_auth(&token)
            .header("Content-Type", "application/octet-stream")
            .header("X-Ecm-Author-Key", &author_key)
            .header("X-Ecm-Author-Signature", &author_signature)
            .body(bytes.clone()),
    )
    .await;
    assert_eq!(p.status, 201, "{}", p.body);
    assert_eq!(
        p.json(),
        json!({ "id": "io.evan.notes", "version": "1.2.0", "kind": "app", "channel": "community", "file": "io.evan.notes-1.2.0.ecmpkg", "sha256": built.sha256 })
    );

    // The index is an ecm-repo-1 index that `pkg` parses.
    let raw_index = s.get("/r/community/index.json", None).await;
    assert_eq!(raw_index.status, 200);
    let index = package::parse_index(&raw_index.body).unwrap();
    assert_eq!(index.format, package::INDEX_FORMAT);
    assert_eq!(index.publisher_key.as_deref(), Some(REPO_KEY));
    assert_eq!(index.publisher_key, ecm_pkg::sign::public_of(SEED));
    assert_eq!(index.packages.len(), 1);
    let entry = &index.packages[0];
    assert_eq!((entry.id.as_str(), entry.version.as_str(), entry.file.as_str()), ("io.evan.notes", "1.2.0", "io.evan.notes-1.2.0.ecmpkg"));
    assert_eq!((entry.size, entry.sha256.as_str()), (bytes.len() as u64, built.sha256.as_str()));
    // As `index_entry` writes it, except that the publisher is the account's handle.
    let mut expected = package::index_entry(&built, "io.evan.notes-1.2.0.ecmpkg", bytes.len() as u64, entry.signature.clone());
    expected.publisher = "evan".into();
    expected.author_key = Some(author_key.clone());
    expected.author_signature = Some(author_signature.clone());
    assert_eq!(entry, &expected);
    assert_eq!(entry.depends, vec!["io.evan.lib >= 1.0.0"]);
    assert_eq!(entry.author(), Some(author_key.as_str()));
    assert!(ecm_pkg::sign::verify(REPO_KEY, &package::signed_message(&entry.id, &entry.version, &entry.sha256), entry.signature.as_deref().unwrap()));

    // Download and verify with the repository key pinned, as `pkg repo add --key` does.
    let (status, downloaded) = s.bytes(&format!("/r/community/{}", entry.file)).await;
    assert_eq!(status, 200);
    assert_eq!(downloaded, bytes);
    assert_eq!(package::verify(entry, index.publisher_key.as_deref(), Some(REPO_KEY), &downloaded).unwrap(), package::Trust::Verified);
    let (wrong_key, _) = ecm_pkg::sign::keypair_from_seed(&[1u8; 32]);
    assert!(package::verify(entry, index.publisher_key.as_deref(), Some(&wrong_key), &downloaded).is_err());
    assert_eq!(package::open(&downloaded).unwrap().manifest.id, "io.evan.notes");

    // The key file, the icon and the screenshot are served; other members are not.
    assert_eq!(s.get("/r/community/publisher.pub", None).await.body.trim(), REPO_KEY);
    assert_eq!(s.bytes("/r/community/io.evan.notes-1.2.0.ecmpkg/icon.png").await, (200, b"\x89PNG icon".to_vec()));
    assert_eq!(s.bytes("/r/community/io.evan.notes-1.2.0.ecmpkg/screenshots/one.png").await, (200, b"\x89PNG shot".to_vec()));
    assert_eq!(s.get("/r/community/io.evan.notes-1.2.0.ecmpkg/main.py", None).await.status, 404);
    assert_eq!(s.get("/r/community/io.evan.notes-1.2.0.ecmpkg/app.toml", None).await.status, 404);
    // Not in other channels, and unknown channels and files are 404s.
    assert!(s.index("stable").await.packages.is_empty());
    assert_eq!(code(&s.get("/r/stable/io.evan.notes-1.2.0.ecmpkg", None).await), err(404, "not_found"));
    assert_eq!(code(&s.get("/r/nightly/index.json", None).await), err(404, "not_found"));
    assert_eq!(code(&s.get("/r/community/other-1.0.0.ecmpkg", None).await), err(404, "not_found"));
    assert_eq!(s.get("/r/community/..%2F..%2Frepo.db", None).await.status, 404);

    // The fs store holds the blob under the contract's key.
    assert_eq!(std::fs::read(s.blobs.join("packages/io.evan.notes/1.2.0/io.evan.notes-1.2.0.ecmpkg")).unwrap(), bytes);

    // The one successful package download was counted; the detail shows the rest.
    let d = s.get("/v1/packages/io.evan.notes", None).await.json();
    assert_eq!(d["downloads"], 1);
    assert_eq!(d["owner"], "evan");
    assert_eq!(d["permissions"], json!(["network"]));
    assert_eq!(d["depends"], json!(["io.evan.lib >= 1.0.0"]));
    assert_eq!(d["icon"], "/r/community/io.evan.notes-1.2.0.ecmpkg/icon.png");
    assert_eq!(d["screenshots"], json!(["/r/community/io.evan.notes-1.2.0.ecmpkg/screenshots/one.png"]));
    assert_eq!(d["versions"][0]["sha256"], built.sha256);
    assert_eq!(s.get("/v1/health", None).await.body, "ok\n");
    assert_eq!(s.get("/healthz", None).await.status, 200);
}

#[tokio::test]
async fn publish_refusals_in_the_contracts_order() {
    let s = start(|c| {
        c.max_package_bytes = 64 * 1024;
        c.account_quota_bytes = 20 * 1024;
        c.upload_limit = Limit::per_hour(4);
    })
    .await;
    let owner = s.token("owner", &["publish", "social"]).await;
    let social_only = new_user(&s.auth, "liker", true).await.api_key("social", &["social"]).await;
    let unverified = new_user(&s.auth, "newbie", false).await.api_key("k", &["publish"]).await;
    let rival = s.token("rival", &["publish"]).await;
    let admin = s.token("admin", &["publish"]).await;

    let good = app("io.owner.app", "1.0.0", "App", "An app");
    let huge = vec![0u8; 64 * 1024 + 1];
    let garbage = b"this is not a tar archive".to_vec();

    // 1. Token and scope: 401 without a good token, 403 without `publish` or a verified email.
    //    These come before everything else, even for a body that is too large.
    assert_eq!(code(&s.publish(None, good.clone()).await), err(401, "unauthorized"));
    assert_eq!(code(&s.publish(Some("ecm_k_made_up"), good.clone()).await), err(401, "unauthorized"));
    assert_eq!(code(&s.publish(Some("not even a token"), huge.clone()).await), err(401, "unauthorized"));
    assert_eq!(code(&s.publish(Some(&social_only), good.clone()).await), err(403, "forbidden"));
    assert_eq!(code(&s.publish(Some(&social_only), huge.clone()).await), err(403, "forbidden"));
    let p = s.publish(Some(&unverified), good.clone()).await;
    assert_eq!(code(&p), err(403, "forbidden"));
    assert!(p.json()["message"].as_str().unwrap().contains("email"));

    // 2. Size: 413, before the archive is looked at.
    assert_eq!(code(&s.publish(Some(&owner), huge.clone()).await), err(413, "too_large"));

    // 3. The archive must open with `ecm_pkg::package::open`.
    assert_eq!(code(&s.publish(Some(&owner), garbage.clone()).await), err(400, "bad_request"));
    assert_eq!(code(&s.publish(Some(&owner), Vec::new()).await), err(400, "bad_request"));
    let no_entry = ecm_pkg::tar::write(&[ecm_pkg::tar::Entry {
        path: "app.toml".into(),
        data: b"id = \"io.owner.app\"\nname = \"A\"\nversion = \"1.0.0\"\nentry = \"main.py\"\n".to_vec(),
        is_dir: false,
    }])
    .unwrap();
    let p = s.publish(Some(&owner), no_entry).await;
    assert_eq!(code(&p), err(400, "bad_request"));
    assert!(p.json()["message"].as_str().unwrap().contains("entry main.py"), "{}", p.body);

    // 4. Reserved prefixes are for admins (checked before the signature: a bad one still gets 403).
    for id in ["dev.ecm.chat", "com.evanwitulski.tool", "dev.ecm"] {
        let p = page(
            s.http.put(format!("{}/v1/publish", s.url)).bearer_auth(&owner).header("X-Ecm-Author-Key", "zz").header("X-Ecm-Author-Signature", "zz")
                .body(app(id, "1.0.0", "Reserved", "")),
        )
        .await;
        assert_eq!(code(&p), err(403, "forbidden"), "{id}");
    }
    assert_eq!(s.publish(Some(&admin), app("dev.ecm.chat", "1.0.0", "Chat", "")).await.status, 201);
    // A look-alike prefix is not reserved.
    assert_eq!(s.publish(Some(&rival), app("dev.ecmx.chat", "1.0.0", "Chat", "")).await.status, 201);

    // The happy path, then: 5. the id belongs to its first publisher.
    assert_eq!(s.publish(Some(&owner), good.clone()).await.status, 201);
    let p = s.publish(Some(&rival), app("io.owner.app", "2.0.0", "Stolen", "")).await;
    assert_eq!(code(&p), err(403, "forbidden"));
    // (Before the version check: the rival gets 403 for the existing version too. And an admin is not the owner either.)
    assert_eq!(code(&s.publish(Some(&rival), good.clone()).await), err(403, "forbidden"));
    assert_eq!(code(&s.publish(Some(&admin), app("io.owner.app", "3.0.0", "Admin", "")).await), err(403, "forbidden"));

    // 6. Versions are immutable (checked before the signature).
    let signed = |body: Vec<u8>, key: &str, signature: &str| {
        s.http.put(format!("{}/v1/publish", s.url)).bearer_auth(&owner).header("X-Ecm-Author-Key", key).header("X-Ecm-Author-Signature", signature).body(body)
    };
    assert_eq!(code(&s.publish(Some(&owner), good.clone()).await), err(409, "conflict"));
    assert_eq!(code(&s.publish(Some(&owner), app("io.owner.app", "1.0.0", "Different bytes", "")).await), err(409, "conflict"));
    assert_eq!(code(&page(signed(good.clone(), "zz", "zz")).await), err(409, "conflict"));

    // 7. The author signature, when given, must verify (checked before the quota).
    let (key, secret) = ecm_pkg::sign::keypair_from_seed(&[4u8; 32]);
    let v2 = app("io.owner.app", "2.0.0", "App", "An app");
    let sha = package::sha256_hex(&v2);
    let good_sig = ecm_pkg::sign::sign(&secret, &package::signed_message("io.owner.app", "2.0.0", &sha));
    let wrong_sig = ecm_pkg::sign::sign(&secret, &package::signed_message("io.owner.app", "2.0.1", &sha));
    let over_quota = other("io.owner.big", "1.0.0", "Big", "kind = \"source\"\n", &[("data.bin", &[7u8; 30 * 1024])]);
    assert_eq!(code(&page(signed(v2.clone(), &key, &wrong_sig)).await), err(400, "bad_request"));
    assert_eq!(code(&page(signed(v2.clone(), &key, "not hex")).await), err(400, "bad_request"));
    assert_eq!(code(&page(signed(v2.clone(), &REPO_KEY.replace('e', "f"), &good_sig)).await), err(400, "bad_request"));
    assert_eq!(code(&page(s.http.put(format!("{}/v1/publish", s.url)).bearer_auth(&owner).header("X-Ecm-Author-Key", &key).body(v2.clone())).await), err(400, "bad_request"));
    assert_eq!(code(&page(signed(over_quota.clone(), &key, &wrong_sig)).await), err(400, "bad_request"));
    assert_eq!(page(signed(v2.clone(), &key, &good_sig)).await.status, 201);

    // 8. The account quota (20 KiB here): 413. Refused uploads did not use any of it.
    let p = s.publish(Some(&owner), over_quota.clone()).await;
    assert_eq!(code(&p), err(413, "too_large"));
    assert!(p.json()["message"].as_str().unwrap().contains("quota"));
    // Someone else's quota is their own.
    assert_eq!(s.publish(Some(&rival), other("io.rival.big", "1.0.0", "Big", "kind = \"source\"\n", &[("data.bin", &[7u8; 12 * 1024])])).await.status, 201);

    // 9. The upload rate (4 an hour here). Only uploads that passed every other check count:
    //    the owner has made 2, so 2 more pass and the next is refused.
    assert_eq!(s.publish(Some(&owner), app("io.owner.app", "2.0.1", "App", "")).await.status, 201);
    assert_eq!(s.publish(Some(&owner), app("io.owner.app", "2.0.2", "App", "")).await.status, 201);
    assert_eq!(code(&s.publish(Some(&owner), app("io.owner.app", "2.0.3", "App", "")).await), err(429, "rate_limited"));
    // The quota is checked before the rate: an over-quota upload is still 413 now.
    assert_eq!(code(&s.publish(Some(&owner), over_quota).await), err(413, "too_large"));

    // Nothing refused reached the index or the store.
    let expect = ["dev.ecm.chat 1.0.0", "dev.ecmx.chat 1.0.0", "io.owner.app 2.0.2", "io.owner.app 2.0.1", "io.owner.app 2.0.0", "io.owner.app 1.0.0", "io.rival.big 1.0.0"];
    assert_eq!(s.listed("community").await, expect);
    assert!(!s.blobs.join("packages/io.owner.big").exists());
    assert!(!s.blobs.join("packages/io.owner.app/2.0.3").exists());
    assert!(s.blobs.join("packages/io.owner.app/2.0.2/io.owner.app-2.0.2.ecmpkg").is_file());
}

#[tokio::test]
async fn file_names_cannot_collide() {
    let s = start(|_| {}).await;
    let a = s.token("anna", &["publish"]).await;
    // `io.anna.x` `1.0.0-1.0.0` and `io.anna.x-1.0.0` `1.0.0` would both be io.anna.x-1.0.0-1.0.0.ecmpkg.
    assert_eq!(s.publish(Some(&a), app("io.anna.x", "1.0.0-1.0.0", "X", "")).await.status, 201);
    assert_eq!(code(&s.publish(Some(&a), app("io.anna.x-1.0.0", "1.0.0", "X", "")).await), err(409, "conflict"));
}

#[tokio::test]
async fn yank_leaves_the_index_but_stays_downloadable_for_thirty_days() {
    let s = start(|_| {}).await;
    let owner = s.token("owner", &["publish"]).await;
    let rival = s.token("rival", &["publish"]).await;
    let social = new_user(&s.auth, "liker", true).await.api_key("s", &["social"]).await;
    for v in ["1.0.0", "1.1.0"] {
        assert_eq!(s.publish(Some(&owner), app("io.owner.app", v, "App", "")).await.status, 201);
    }
    assert_eq!(s.listed("community").await, ["io.owner.app 1.1.0", "io.owner.app 1.0.0"]);

    // Who may yank what.
    assert_eq!(code(&s.send(reqwest::Method::POST, "/v1/packages/io.owner.app/yank", None, Some(json!({ "version": "1.1.0" }))).await), err(401, "unauthorized"));
    assert_eq!(code(&s.post("/v1/packages/io.owner.app/yank", &social, json!({ "version": "1.1.0" })).await), err(403, "forbidden"));
    assert_eq!(code(&s.post("/v1/packages/io.owner.app/yank", &rival, json!({ "version": "1.1.0" })).await), err(403, "forbidden"));
    assert_eq!(code(&s.post("/v1/packages/io.owner.app/yank", &owner, json!({ "version": "9.9.9" })).await), err(404, "not_found"));
    assert_eq!(code(&s.post("/v1/packages/io.nobody.app/yank", &owner, json!({ "version": "1.0.0" })).await), err(404, "not_found"));
    assert_eq!(code(&s.post("/v1/packages/io.owner.app/yank", &owner, json!({})).await), err(400, "bad_request"));
    assert_eq!(s.listed("community").await.len(), 2);

    // Yank (twice: it is idempotent).
    for _ in 0..2 {
        let p = s.post("/v1/packages/io.owner.app/yank", &owner, json!({ "version": "1.1.0" })).await;
        assert_eq!((p.status, p.body.as_str()), (204, ""));
    }
    assert_eq!(s.listed("community").await, ["io.owner.app 1.0.0"]);
    // The catalogue falls back to the newest live version and marks the yanked one.
    let d = s.get("/v1/packages/io.owner.app", None).await.json();
    assert_eq!(d["version"], "1.0.0");
    let versions: Vec<(String, bool)> = d["versions"].as_array().unwrap().iter().map(|v| (v["version"].as_str().unwrap().to_string(), v["yanked"].as_bool().unwrap())).collect();
    assert_eq!(versions, [("1.1.0".to_string(), true), ("1.0.0".to_string(), false)]);
    // Still downloadable by exact name; and the version cannot be published again.
    assert_eq!(s.bytes("/r/community/io.owner.app-1.1.0.ecmpkg").await.0, 200);
    assert_eq!(code(&s.publish(Some(&owner), app("io.owner.app", "1.1.0", "App", "")).await), err(409, "conflict"));

    // 29 days later it is still there; after 30 it is gone, and the purge deletes the file.
    let backdate = |days: i64| {
        s.app.db.lock().unwrap().execute("UPDATE versions SET yanked_at = ?1 WHERE version = '1.1.0'", [backend_common::now() - days * 24 * 3600]).unwrap();
    };
    backdate(29);
    assert_eq!(repo_api::ops::purge_yanked(&s.app).await.unwrap(), 0);
    assert_eq!(s.bytes("/r/community/io.owner.app-1.1.0.ecmpkg").await.0, 200);
    backdate(31);
    assert_eq!(code(&s.get("/r/community/io.owner.app-1.1.0.ecmpkg", None).await), err(410, "gone"));
    let blob = s.blobs.join("packages/io.owner.app/1.1.0/io.owner.app-1.1.0.ecmpkg");
    assert!(blob.is_file());
    assert_eq!(repo_api::ops::purge_yanked(&s.app).await.unwrap(), 1);
    assert!(!blob.exists());
    assert!(s.blobs.join("packages/io.owner.app/1.0.0/io.owner.app-1.0.0.ecmpkg").is_file());
    assert_eq!(code(&s.get("/r/community/io.owner.app-1.1.0.ecmpkg", None).await), err(410, "gone"));

    // With every version yanked the package leaves the catalogue, but its owner still sees it.
    assert_eq!(s.post("/v1/packages/io.owner.app/yank", &owner, json!({ "version": "1.0.0" })).await.status, 204);
    assert!(s.listed("community").await.is_empty());
    assert_eq!(s.get("/v1/packages", None).await.json()["total"], 0);
    assert_eq!(s.get("/v1/me/packages", Some(&owner)).await.json()[0]["id"], "io.owner.app");
}

#[tokio::test]
async fn likes_favorites_and_reports() {
    let s = start(|c| c.report_limit = Limit::per_hour(3)).await;
    let owner = s.token("owner", &["publish", "social"]).await;
    let fan = s.token("fan", &["social"]).await;
    let publish_only = new_user(&s.auth, "quiet", true).await.api_key("p", &["publish"]).await;
    let admin = s.token("admin", &["social"]).await;
    for id in ["io.owner.one", "io.owner.two"] {
        assert_eq!(s.publish(Some(&owner), app(id, "1.0.0", id, "")).await.status, 201);
    }
    let put = reqwest::Method::PUT;
    let delete = reqwest::Method::DELETE;
    let summary = |list: Value, id: &str| list["packages"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();

    // Scope and existence.
    assert_eq!(code(&s.send(put.clone(), "/v1/packages/io.owner.one/like", None, None).await), err(401, "unauthorized"));
    assert_eq!(code(&s.send(put.clone(), "/v1/packages/io.owner.one/like", Some(&publish_only), None).await), err(403, "forbidden"));
    assert_eq!(code(&s.send(put.clone(), "/v1/packages/io.none.x/like", Some(&fan), None).await), err(404, "not_found"));
    assert_eq!(code(&s.send(put.clone(), "/v1/packages/io.none.x/favorite", Some(&fan), None).await), err(404, "not_found"));

    // Liking twice counts once; two people count twice.
    for _ in 0..2 {
        let p = s.send(put.clone(), "/v1/packages/io.owner.one/like", Some(&fan), None).await;
        assert_eq!((p.status, p.body.as_str()), (204, ""));
    }
    assert_eq!(s.send(put.clone(), "/v1/packages/io.owner.one/like", Some(&owner), None).await.status, 204);
    assert_eq!(s.send(put.clone(), "/v1/packages/io.owner.two/favorite", Some(&fan), None).await.status, 204);
    assert_eq!(s.send(put.clone(), "/v1/packages/io.owner.two/favorite", Some(&fan), None).await.status, 204);

    // `liked` and `favorited` are the caller's; without a token they are false.
    let anonymous = s.get("/v1/packages", None).await.json();
    let one = summary(anonymous.clone(), "io.owner.one");
    assert_eq!((one["likes"].clone(), one["liked"].clone(), one["favorited"].clone()), (json!(2), json!(false), json!(false)));
    let as_fan = s.get("/v1/packages", Some(&fan)).await.json();
    assert_eq!(summary(as_fan.clone(), "io.owner.one")["liked"], true);
    assert_eq!(summary(as_fan.clone(), "io.owner.one")["favorited"], false);
    assert_eq!(summary(as_fan.clone(), "io.owner.two")["favorited"], true);
    assert_eq!(summary(as_fan, "io.owner.two")["likes"], 0);
    let d = s.get("/v1/packages/io.owner.one", Some(&fan)).await.json();
    assert_eq!((d["likes"].clone(), d["liked"].clone()), (json!(2), json!(true)));
    // A bad token on a public call is just nobody.
    assert_eq!(summary(s.get("/v1/packages", Some("ecm_k_bogus")).await.json(), "io.owner.one")["liked"], false);

    // The caller's lists.
    let ids = |p: Page| -> Vec<String> { p.json().as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect() };
    assert_eq!(ids(s.get("/v1/me/likes", Some(&fan)).await), ["io.owner.one"]);
    assert_eq!(ids(s.get("/v1/me/favorites", Some(&fan)).await), ["io.owner.two"]);
    assert!(ids(s.get("/v1/me/packages", Some(&fan)).await).is_empty());
    assert_eq!(ids(s.get("/v1/me/packages", Some(&owner)).await), ["io.owner.one", "io.owner.two"]);
    assert_eq!(code(&s.get("/v1/me/likes", None).await), err(401, "unauthorized"));
    assert_eq!(code(&s.get("/v1/me/favorites", Some("ecm_k_bogus")).await), err(401, "unauthorized"));

    // Unliking twice is fine too.
    for _ in 0..2 {
        assert_eq!(s.send(delete.clone(), "/v1/packages/io.owner.one/like", Some(&fan), None).await.status, 204);
        assert_eq!(s.send(delete.clone(), "/v1/packages/io.owner.two/favorite", Some(&fan), None).await.status, 204);
    }
    assert_eq!(s.get("/v1/packages/io.owner.one", Some(&fan)).await.json()["likes"], 1);
    assert!(ids(s.get("/v1/me/likes", Some(&fan)).await).is_empty());
    assert!(ids(s.get("/v1/me/favorites", Some(&fan)).await).is_empty());

    // Reports: a reason of at most 1000 characters, a limit per account, readable by admins.
    assert_eq!(code(&s.post("/v1/packages/io.owner.one/report", &publish_only, json!({ "reason": "x" })).await), err(403, "forbidden"));
    assert_eq!(code(&s.post("/v1/packages/io.owner.one/report", &fan, json!({ "reason": "y".repeat(1001) })).await), err(400, "bad_request"));
    assert_eq!(code(&s.post("/v1/packages/io.none.x/report", &fan, json!({ "reason": "spam" })).await), err(404, "not_found"));
    let p = s.post("/v1/packages/io.owner.one/report", &fan, json!({ "reason": "é".repeat(1000) })).await;
    assert_eq!((p.status, p.body.as_str()), (204, ""));
    assert_eq!(code(&s.post("/v1/packages/io.owner.one/report", &fan, json!({ "reason": "again" })).await), err(429, "rate_limited"));
    assert_eq!(s.post("/v1/packages/io.owner.two/report", &owner, json!({ "reason": "it is <b>mine</b>" })).await.status, 204);
    assert_eq!(code(&s.get("/v1/admin/reports", Some(&fan)).await), err(403, "forbidden"));
    let reports = s.get("/v1/admin/reports", Some(&admin)).await.json();
    let reports = reports.as_array().unwrap();
    assert_eq!(reports.len(), 2);
    assert_eq!((reports[0]["id"].clone(), reports[0]["reporter"].clone(), reports[0]["reason"].clone()), (json!("io.owner.two"), json!("owner"), json!("it is <b>mine</b>")));
    assert_eq!(reports[1]["reporter"], "fan");
}

#[tokio::test]
async fn catalogue_search_sort_kind_and_channel() {
    let s = start(|_| {}).await;
    let ana = s.token("ana", &["publish", "social"]).await;
    let ben = s.token("ben", &["publish", "social"]).await;
    let admin = s.token("admin", &["social"]).await;

    assert_eq!(s.publish(Some(&ana), app("io.ana.notes", "1.0.0", "Notes", "Write things down")).await.status, 201);
    assert_eq!(s.publish(Some(&ana), other("io.ana.hexdump", "0.3.0", "Hexdump", "kind = \"program\"\nbin = [\"hexdump.wasm\"]\nsummary = \"Dump bytes\"\n", &[("hexdump.wasm", b"\0asm")])).await.status, 201);
    assert_eq!(s.publish(Some(&ben), other("io.ben.geo", "2.0.0", "Geometry", "kind = \"library\"\nprovides = [\"geo\"]\nsummary = \"Shapes for notes apps\"\n", &[("python/geo.py", b"x = 1\n")])).await.status, 201);
    assert_eq!(s.publish(Some(&ben), other("io.ben.examples", "1.0.0", "Examples", "kind = \"source\"\n", &[("a.py", b"pass\n")])).await.status, 201);
    assert_eq!(s.publish(Some(&ben), app("io.ben.lua", "1.0.0", "Abacus", "Count")).await.status, 201);
    // Later versions: the newest represents the package.
    assert_eq!(s.publish(Some(&ana), app("io.ana.notes", "1.10.0", "Notes Pro", "Write things down")).await.status, 201);
    assert_eq!(s.publish(Some(&ana), app("io.ana.notes", "1.2.0", "Notes", "Write things down")).await.status, 201);

    // Popularity: likes (geo 2, notes 1) and downloads (hexdump 2, geo 1).
    let put = reqwest::Method::PUT;
    for (who, id) in [(&ana, "io.ben.geo"), (&ben, "io.ben.geo"), (&ben, "io.ana.notes")] {
        assert_eq!(s.send(put.clone(), &format!("/v1/packages/{id}/like"), Some(who), None).await.status, 204);
    }
    for file in ["io.ana.hexdump-0.3.0.ecmpkg", "io.ana.hexdump-0.3.0.ecmpkg", "io.ben.geo-2.0.0.ecmpkg"] {
        assert_eq!(s.bytes(&format!("/r/community/{file}")).await.0, 200);
    }
    // Recency, set by hand so the order does not depend on the clock's resolution.
    for (i, id) in ["io.ben.examples", "io.ana.hexdump", "io.ben.lua", "io.ben.geo", "io.ana.notes"].iter().enumerate() {
        s.app.db.lock().unwrap().execute("UPDATE versions SET published_at = ?1 WHERE pkg_id = ?2", rusqlite::params![1_800_000_000 + i as i64, id]).unwrap();
    }

    let ids = |v: &Value| -> Vec<String> { v["packages"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect() };
    let list = |query: &'static str| {
        let s = &s;
        async move { s.get(&format!("/v1/packages{query}"), None).await.json() }
    };

    // The whole list, in the default order (likes, then name).
    let all = list("").await;
    assert_eq!((all["total"].clone(), all["page"].clone(), all["per_page"].clone()), (json!(5), json!(1), json!(50)));
    assert_eq!(ids(&all), ["io.ben.geo", "io.ana.notes", "io.ben.lua", "io.ben.examples", "io.ana.hexdump"]);
    assert_eq!(ids(&list("?sort=likes").await), ids(&all));
    assert_eq!(ids(&list("?sort=name").await), ["io.ben.lua", "io.ben.examples", "io.ben.geo", "io.ana.hexdump", "io.ana.notes"]);
    assert_eq!(ids(&list("?sort=downloads").await), ["io.ana.hexdump", "io.ben.geo", "io.ben.lua", "io.ben.examples", "io.ana.notes"]);
    assert_eq!(ids(&list("?sort=new").await), ["io.ana.notes", "io.ben.geo", "io.ben.lua", "io.ana.hexdump", "io.ben.examples"]);

    // A Summary has exactly the contract's fields; the newest version (1.10.0 > 1.2.0) describes the package.
    let notes = all["packages"].as_array().unwrap().iter().find(|p| p["id"] == "io.ana.notes").unwrap().clone();
    let mut keys: Vec<&str> = notes.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["channels", "downloads", "favorited", "id", "kind", "liked", "likes", "name", "owner", "summary", "updated", "version"]);
    assert_eq!((notes["version"].clone(), notes["name"].clone(), notes["kind"].clone(), notes["owner"].clone()), (json!("1.10.0"), json!("Notes Pro"), json!("app"), json!("ana")));
    assert_eq!((notes["summary"].clone(), notes["channels"].clone(), notes["likes"].clone()), (json!("Write things down"), json!(["community"]), json!(1)));
    assert_eq!(notes["updated"], "2027-01-15T08:00:04Z");

    // Kinds.
    assert_eq!(ids(&list("?kind=app&sort=name").await), ["io.ben.lua", "io.ana.notes"]);
    assert_eq!(ids(&list("?kind=program").await), ["io.ana.hexdump"]);
    assert_eq!(ids(&list("?kind=library").await), ["io.ben.geo"]);
    assert_eq!(ids(&list("?kind=source").await), ["io.ben.examples"]);
    assert_eq!(list("?kind=library").await["packages"][0]["kind"], "library");

    // Search: every word, in the id, name, summary or owner, any case.
    assert_eq!(ids(&list("?q=NOTES&sort=name").await), ["io.ben.geo", "io.ana.notes"]);
    assert_eq!(ids(&list("?q=notes+write").await), ["io.ana.notes"]);
    assert_eq!(ids(&list("?q=ben&kind=app").await), ["io.ben.lua"]);
    assert_eq!(ids(&list("?q=hexdump").await), ["io.ana.hexdump"]);
    assert_eq!(list("?q=nothing-like-this").await["total"], 0);
    assert_eq!(list("?q=%25").await["total"], 0);

    // Channels: only what was promoted, described by the version that is in the channel.
    assert_eq!(list("?channel=stable").await["total"], 0);
    assert_eq!(s.post("/v1/admin/promote", &admin, json!({ "id": "io.ana.notes", "version": "1.2.0", "channel": "stable" })).await.status, 200);
    let stable = list("?channel=stable").await;
    assert_eq!(ids(&stable), ["io.ana.notes"]);
    assert_eq!((stable["packages"][0]["version"].clone(), stable["packages"][0]["name"].clone()), (json!("1.2.0"), json!("Notes")));
    assert_eq!(stable["packages"][0]["channels"], json!(["community", "stable"]));
    assert_eq!(list("?channel=community").await["total"], 5);
    assert_eq!(list("?channel=nope").await["total"], 0);

    // Pages, and bad parameters.
    let page2 = list("?page=2").await;
    assert_eq!((page2["total"].clone(), page2["page"].clone(), page2["packages"].as_array().unwrap().len()), (json!(5), json!(2), 0));
    for bad in ["?sort=random", "?kind=gadget", "?page=0", "?page=x"] {
        assert_eq!(code(&s.get(&format!("/v1/packages{bad}"), None).await), err(400, "bad_request"), "{bad}");
    }
    assert_eq!(code(&s.get("/v1/packages/io.none.x", None).await), err(404, "not_found"));
    assert_eq!(code(&s.get("/v1/nothing", None).await), err(404, "not_found"));

    // Detail: a Summary plus the rest.
    let d = s.get("/v1/packages/io.ana.notes", None).await.json();
    let versions: Vec<&str> = d["versions"].as_array().unwrap().iter().map(|v| v["version"].as_str().unwrap()).collect();
    assert_eq!(versions, ["1.10.0", "1.2.0", "1.0.0"]);
    for key in ["id", "name", "summary", "kind", "version", "owner", "channels", "likes", "downloads", "updated", "liked", "favorited", "versions", "permissions", "depends", "icon", "screenshots"] {
        assert!(d.get(key).is_some(), "detail lacks {key}");
    }
    assert!(d["versions"][0]["size"].as_u64().unwrap() > 0 && d["versions"][0]["published"].as_str().unwrap().ends_with('Z'));
    assert_eq!((d["icon"].clone(), d["screenshots"].clone()), (Value::Null, json!([])));
    // The index has kinds, commands, provides and levels as `index_entry` writes them.
    let index = s.index("community").await;
    let entry = |id: &str| index.packages.iter().find(|e| e.id == id).unwrap().clone();
    assert_eq!((entry("io.ana.hexdump").kind, entry("io.ana.hexdump").commands, entry("io.ana.hexdump").level), ("program".to_string(), vec!["hexdump".to_string()], 2));
    assert_eq!((entry("io.ben.geo").kind, entry("io.ben.geo").provides), ("library".to_string(), vec!["geo".to_string()]));
    assert_eq!((entry("io.ben.lua").kind, entry("io.ben.lua").level), (String::new(), 0));
    let order: Vec<String> = index.packages.iter().filter(|e| e.id == "io.ana.notes").map(|e| e.version.clone()).collect();
    assert_eq!(order, ["1.10.0", "1.2.0", "1.0.0"]);
}

#[tokio::test]
async fn admin_promote_demote_channels_and_takedown() {
    let s = start(|_| {}).await;
    let owner = s.token("owner", &["publish", "social"]).await;
    let admin = s.token("admin", &["publish"]).await;
    for v in ["1.0.0", "2.0.0"] {
        assert_eq!(s.publish(Some(&owner), app("io.owner.app", v, "App", "")).await.status, 201);
    }
    assert_eq!(s.publish(Some(&owner), app("io.owner.bad", "1.0.0", "Bad", "")).await.status, 201);
    let promote = |id: &'static str, version: &'static str, channel: &'static str| json!({ "id": id, "version": version, "channel": channel });

    // Admin calls need an admin.
    for path in ["/v1/admin/promote", "/v1/admin/demote"] {
        assert_eq!(code(&s.post(path, &owner, promote("io.owner.app", "1.0.0", "stable")).await), err(403, "forbidden"), "{path}");
        assert_eq!(code(&s.send(reqwest::Method::POST, path, None, Some(promote("io.owner.app", "1.0.0", "stable"))).await), err(401, "unauthorized"));
    }
    assert_eq!(code(&s.post("/v1/admin/takedown", &owner, json!({ "id": "io.owner.app", "version": null })).await), err(403, "forbidden"));
    assert_eq!(code(&s.post("/v1/admin/channels", &owner, json!({ "name": "nightly", "title": "Nightly" })).await), err(403, "forbidden"));
    assert_eq!(code(&s.get("/v1/admin/reports", Some(&owner)).await), err(403, "forbidden"));
    assert!(s.listed("stable").await.is_empty());

    // Promote to stable: that channel's index lists it, signed with the same repository key.
    assert_eq!(s.post("/v1/admin/promote", &admin, promote("io.owner.app", "1.0.0", "stable")).await.status, 200);
    assert_eq!(s.post("/v1/admin/promote", &admin, promote("io.owner.app", "1.0.0", "stable")).await.status, 200);
    assert_eq!(s.listed("stable").await, ["io.owner.app 1.0.0"]);
    let stable = s.index("stable").await;
    assert_eq!((stable.name.as_str(), stable.publisher_key.as_deref()), ("Stable", Some(REPO_KEY)));
    let (status, bytes) = s.bytes("/r/stable/io.owner.app-1.0.0.ecmpkg").await;
    assert_eq!(status, 200);
    assert_eq!(package::verify(&stable.packages[0], stable.publisher_key.as_deref(), Some(REPO_KEY), &bytes).unwrap(), package::Trust::Verified);
    assert_eq!(s.get("/r/stable/io.owner.app-2.0.0.ecmpkg", None).await.status, 404);
    assert_eq!(s.listed("beta").await.len(), 0);
    assert_eq!(code(&s.post("/v1/admin/promote", &admin, promote("io.owner.app", "9.0.0", "stable")).await), err(404, "not_found"));
    assert_eq!(code(&s.post("/v1/admin/promote", &admin, promote("io.owner.app", "1.0.0", "nightly")).await), err(404, "not_found"));

    // A new channel.
    assert_eq!(code(&s.post("/v1/admin/channels", &admin, json!({ "name": "Not Valid", "title": "x" })).await), err(400, "bad_request"));
    assert_eq!(s.post("/v1/admin/channels", &admin, json!({ "name": "nightly", "title": "Nightly builds" })).await.status, 201);
    assert_eq!(code(&s.post("/v1/admin/channels", &admin, json!({ "name": "nightly", "title": "Again" })).await), err(409, "conflict"));
    assert!(s.listed("nightly").await.is_empty());
    assert_eq!(s.post("/v1/admin/promote", &admin, promote("io.owner.app", "2.0.0", "nightly")).await.status, 200);
    assert_eq!(s.index("nightly").await.name, "Nightly builds");
    assert_eq!(s.listed("nightly").await, ["io.owner.app 2.0.0"]);

    // Demote.
    assert_eq!(s.post("/v1/admin/demote", &admin, promote("io.owner.app", "1.0.0", "stable")).await.status, 200);
    assert!(s.listed("stable").await.is_empty());
    assert_eq!(s.get("/r/stable/io.owner.app-1.0.0.ecmpkg", None).await.status, 404);
    assert_eq!(s.listed("community").await.len(), 3);

    // The same operations as `repo-api admin <verb>`; the running server sees them.
    let cli = |words: &'static [&'static str]| {
        let app = s.app.clone();
        async move { repo_api::admin::run_on(&app, words).await }
    };
    assert_eq!(cli(&["promote", "io.owner.app", "2.0.0", "beta"]).await.unwrap(), "io.owner.app 2.0.0 is in beta\n");
    assert_eq!(s.listed("beta").await, ["io.owner.app 2.0.0"]);
    cli(&["demote", "io.owner.app", "2.0.0", "beta"]).await.unwrap();
    assert!(s.listed("beta").await.is_empty());
    cli(&["channel", "lab", "The", "Lab"]).await.unwrap();
    assert_eq!(s.index("lab").await.name, "The Lab");
    assert!(cli(&["promote", "io.owner.app", "2.0.0", "nowhere"]).await.unwrap_err().contains("no channel nowhere"));
    assert!(cli(&["frobnicate"]).await.unwrap_err().contains("usage: repo-api admin"));
    assert_eq!(cli(&["reports"]).await.unwrap(), "no reports\n");
    assert_eq!(s.post("/v1/packages/io.owner.bad/report", &owner, json!({ "reason": "testing" })).await.status, 204);
    assert!(cli(&["reports"]).await.unwrap().contains("io.owner.bad by owner: testing"));

    // Take down one version: out of every index, file deleted, never publishable again.
    let blob = |id: &str, v: &str| s.blobs.join(format!("packages/{id}/{v}/{id}-{v}.ecmpkg"));
    assert!(blob("io.owner.app", "2.0.0").is_file());
    assert_eq!(code(&s.post("/v1/admin/takedown", &admin, json!({ "id": "io.owner.app", "version": "7.0.0" })).await), err(404, "not_found"));
    assert_eq!(s.post("/v1/admin/takedown", &admin, json!({ "id": "io.owner.app", "version": "2.0.0" })).await.status, 200);
    assert_eq!(s.listed("community").await, ["io.owner.app 1.0.0", "io.owner.bad 1.0.0"]);
    assert!(s.listed("nightly").await.is_empty());
    assert_eq!(s.get("/r/community/io.owner.app-2.0.0.ecmpkg", None).await.status, 404);
    assert!(!blob("io.owner.app", "2.0.0").exists() && blob("io.owner.app", "1.0.0").is_file());
    let d = s.get("/v1/packages/io.owner.app", None).await.json();
    assert_eq!((d["version"].clone(), d["versions"].as_array().unwrap().len()), (json!("1.0.0"), 1));
    assert_eq!(code(&s.publish(Some(&owner), app("io.owner.app", "2.0.0", "App", "")).await), err(409, "conflict"));
    assert_eq!(s.publish(Some(&owner), app("io.owner.app", "2.0.1", "App", "")).await.status, 201);

    // Take down a whole package (`version: null`, or left out): gone everywhere, and the id stays taken.
    assert_eq!(code(&s.post("/v1/admin/takedown", &admin, json!({ "id": "io.none.x", "version": null })).await), err(404, "not_found"));
    assert_eq!(s.post("/v1/admin/takedown", &admin, json!({ "id": "io.owner.bad", "version": null })).await.status, 200);
    assert_eq!(s.listed("community").await, ["io.owner.app 2.0.1", "io.owner.app 1.0.0"]);
    assert_eq!(s.get("/r/community/io.owner.bad-1.0.0.ecmpkg", None).await.status, 404);
    assert!(!blob("io.owner.bad", "1.0.0").exists());
    assert_eq!(code(&s.get("/v1/packages/io.owner.bad", None).await), err(404, "not_found"));
    assert_eq!(s.get("/v1/packages", None).await.json()["total"], 1);
    assert_eq!(code(&s.publish(Some(&owner), app("io.owner.bad", "1.0.1", "Bad", "")).await), err(403, "forbidden"));
    assert_eq!(code(&s.publish(Some(&admin), app("io.owner.bad", "1.0.1", "Bad", "")).await), err(403, "forbidden"));
    assert_eq!(code(&s.send(reqwest::Method::PUT, "/v1/packages/io.owner.bad/like", Some(&owner), None).await), err(404, "not_found"));
    // The takedown freed the owner's quota, and the CLI takes down as well.
    cli(&["takedown", "io.owner.app", "1.0.0"]).await.unwrap();
    assert_eq!(s.listed("community").await, ["io.owner.app 2.0.1"]);
    assert!(!blob("io.owner.app", "1.0.0").exists());
    let used: i64 = s.app.db.lock().unwrap().query_row("SELECT COALESCE(SUM(size), 0) FROM versions WHERE blob_deleted = 0", [], |r| r.get(0)).unwrap();
    assert_eq!(used as usize, app("io.owner.app", "2.0.1", "App", "").len());
}

#[tokio::test]
async fn admin_changes_from_another_process_reach_the_running_server() {
    let s = start(|_| {}).await;
    let owner = s.token("owner", &["publish"]).await;
    assert_eq!(s.publish(Some(&owner), app("io.owner.app", "1.0.0", "App", "")).await.status, 201);
    assert!(s.listed("stable").await.is_empty());
    // A second connection to the same database file, as `repo-api admin` has.
    let cli = repo_api::build(s.app.cfg.clone()).unwrap();
    repo_api::admin::run_on(&cli, &["promote", "io.owner.app", "1.0.0", "stable"]).await.unwrap();
    assert_eq!(s.listed("stable").await, ["io.owner.app 1.0.0"]);
    repo_api::admin::run_on(&cli, &["takedown", "io.owner.app"]).await.unwrap();
    assert!(s.listed("stable").await.is_empty() && s.listed("community").await.is_empty());
}

#[tokio::test]
async fn admin_cli_prints_and_makes_keys() {
    let made = repo_api::admin::run(&["keygen".to_string()]).await.unwrap();
    let seed = made.lines().next().unwrap().strip_prefix("REPO_SIGNING_KEY=").expect("an env line").to_string();
    let public = made.lines().nth(1).unwrap().rsplit(' ').next().unwrap().to_string();
    assert_eq!((seed.len(), public.len()), (64, 64));
    assert_eq!(repo_api::signing_key(&seed).unwrap(), (seed.clone(), public.clone()));
    assert_eq!(ecm_pkg::sign::public_of(&seed), Some(public));
    assert_ne!(made, repo_api::admin::run(&["keygen".to_string()]).await.unwrap());
    assert_eq!(repo_api::signing_key(SEED).unwrap().1, REPO_KEY);
    assert!(repo_api::signing_key("abc").is_err() && repo_api::signing_key(&"zz".repeat(32)).is_err());
    assert!(repo_api::admin::run(&[]).await.unwrap().contains("pubkey"));
}

#[tokio::test]
async fn the_catalogue_page_lists_searches_and_escapes() {
    let s = start(|_| {}).await;
    let owner = s.token("owner", &["publish"]).await;
    assert_eq!(s.publish(Some(&owner), app("io.owner.app", "1.0.0", "<script>alert(1)</script>", "Fish & <i>chips</i>")).await.status, 201);
    assert_eq!(s.publish(Some(&owner), other("io.owner.tool", "1.0.0", "Tool", "kind = \"program\"\nbin = [\"tool.wasm\"]\n", &[("tool.wasm", b"\0asm")])).await.status, 201);

    let p = s.get("/", None).await;
    assert_eq!(p.status, 200);
    assert!(p.body.contains("&lt;script&gt;alert(1)&lt;/script&gt;") && !p.body.contains("<script>"));
    assert!(p.body.contains("Fish &amp; &lt;i&gt;chips&lt;/i&gt;"));
    assert!(p.body.contains("href=\"/account\"") && p.body.contains("Tool") && p.body.contains("0 likes"));
    assert!(p.body.contains(&format!("pkg repo add official http://repo.test/r/stable/ --key {REPO_KEY}")));
    let h = |n: &str| p.headers.get(n).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    assert!(h("content-security-policy").contains("default-src 'none'"));
    assert_eq!((h("x-content-type-options"), h("x-frame-options")), ("nosniff".to_string(), "DENY".to_string()));
    assert!(h("content-type").starts_with("text/html"));

    let programs = s.get("/?kind=program", None).await;
    assert!(programs.body.contains("io.owner.tool") && !programs.body.contains("io.owner.app"));
    let searched = s.get("/?q=%22%3E%3Cb%3Efish", None).await;
    assert!(searched.body.contains("value=\"&quot;&gt;&lt;b&gt;fish\"") && searched.body.contains("No packages found."));
    assert!(s.get("/?q=fish&sort=name", None).await.body.contains("io.owner.app"));
    assert_eq!(s.get("/static/repo/style.css", None).await.status, 200);
}

#[tokio::test]
async fn responses_have_content_length_on_a_raw_socket() {
    let s = start(|_| {}).await;
    let token = s.token("sock", &["publish", "social"]).await;
    let bytes = app("io.sock.app", "1.0.0", "App", "");

    for version in ["1.0", "1.1"] {
        // Publishing: a 201 with a JSON body (and a 409 the second time round).
        let mut request = format!(
            "PUT /v1/publish HTTP/{version}\r\nHost: repo.test\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
            bytes.len()
        )
        .into_bytes();
        request.extend_from_slice(&bytes);
        let (head, body) = raw(s.addr, &request).await;
        assert_eq!(raw_status(&head), if version == "1.0" { 201 } else { 409 }, "{head}");
        assert_plain_framing(&head, &body);

        let get = |path: &str, auth: bool| {
            let bearer = if auth { format!("Authorization: Bearer {token}\r\n") } else { String::new() };
            format!("GET {path} HTTP/{version}\r\nHost: repo.test\r\nUser-Agent: ecm-pkg/1\r\nConnection: close\r\n{bearer}\r\n")
        };
        let cases = [
            (get("/v1/health", false), 200),
            (get("/healthz", false), 200),
            (get("/r/community/index.json", false), 200),
            (get("/r/community/publisher.pub", false), 200),
            (get("/r/community/io.sock.app-1.0.0.ecmpkg", false), 200),
            (get("/r/community/missing-1.0.0.ecmpkg", false), 404),
            (get("/r/nowhere/index.json", false), 404),
            (get("/v1/packages", false), 200),
            (get("/v1/packages?sort=bogus", false), 400),
            (get("/v1/packages/io.sock.app", true), 200),
            (get("/v1/packages/io.none.x", false), 404),
            (get("/v1/me/packages", true), 200),
            (get("/v1/me/likes", false), 401),
            (get("/v1/admin/reports", true), 403),
            (get("/v1/unknown", false), 404),
            (get("/v1/publish", false), 405),
        ];
        for (request, status) in cases {
            let (head, body) = raw(s.addr, request.as_bytes()).await;
            assert_eq!(raw_status(&head), status, "{request}\n{head}");
            assert_plain_framing(&head, &body);
            if request.contains("io.sock.app-1.0.0.ecmpkg") {
                assert_eq!(body, bytes);
                assert_eq!(raw_header(&head, "content-type").as_deref(), Some("application/octet-stream"));
            }
            if request.starts_with("GET /v1/health ") {
                assert_eq!(body, b"ok\n");
                assert!(raw_header(&head, "content-type").unwrap().starts_with("text/plain"));
            }
        }
        // A body that is too large and a body that is not JSON.
        let big = format!("PUT /v1/publish HTTP/{version}\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nContent-Length: 9000000\r\n\r\n");
        let (head, body) = raw(s.addr, big.as_bytes()).await;
        assert_eq!(raw_status(&head), 413, "{head}");
        assert_plain_framing(&head, &body);
        let bad = format!("POST /v1/packages/io.sock.app/report HTTP/{version}\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nContent-Length: 3\r\n\r\n{{x}}");
        let (head, body) = raw(s.addr, bad.as_bytes()).await;
        assert_eq!(raw_status(&head), 400, "{head}");
        assert_plain_framing(&head, &body);

        // 204: an empty body, not chunked, and the server closes the connection.
        let like = format!("PUT /v1/packages/io.sock.app/like HTTP/{version}\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n");
        let (head, body) = raw(s.addr, like.as_bytes()).await;
        assert_eq!(raw_status(&head), 204, "{head}");
        assert!(body.is_empty() && raw_header(&head, "transfer-encoding").is_none(), "{head}");
    }
}

#[tokio::test]
async fn tokens_are_checked_with_auth_api_and_its_absence_is_an_error() {
    let s = start(|_| {}).await;
    let token = s.token("ivy", &["social"]).await;
    assert_eq!(s.get("/v1/me/likes", Some(&token)).await.status, 200);
    // A session cookie's token, a device code and an empty bearer are not API tokens.
    for bad in ["ecm_s_0000000000000000000000000000000000000000000000000000000000000000", "ecm_dc_1234", "x"] {
        assert_eq!(code(&s.get("/v1/me/likes", Some(bad)).await), err(401, "unauthorized"), "{bad}");
    }
    // Without auth-api, a token nobody has vouched for yet cannot be checked: 500, not a silent yes or no.
    let lonely = start(|c| c.auth_internal_url = "http://127.0.0.1:1".into()).await;
    let p = lonely.get("/v1/me/likes", Some("ecm_k_0000000000000000000000000000000000000000000000000000000000000000")).await;
    assert_eq!(code(&p), err(500, "internal"));
    assert!(!p.body.contains("127.0.0.1"), "{}", p.body);
    // Public reads still work.
    assert_eq!(lonely.get("/r/community/index.json", None).await.status, 200);
    assert_eq!(lonely.get("/v1/packages", Some("ecm_k_0000000000000000000000000000000000000000000000000000000000000000")).await.status, 200);
    // A wrong shared secret is our failure too, not the caller's.
    let wrong = start(|c| c.internal_secret = "another-secret-entirely".into()).await;
    let token = wrong.token("joe", &["social"]).await;
    assert_eq!(code(&wrong.get("/v1/me/likes", Some(&token)).await), err(500, "internal"));
}

#[tokio::test]
async fn a_key_from_auth_api_admin_publishes() {
    let s = start(|_| {}).await;
    // No browser: the account and its key come from `auth-api admin`.
    auth_api::admin::run_on(&s.auth.app, &["user-add", "ci@example.com", "ci-bot", PASSWORD]).await.unwrap();
    let key = auth_api::admin::run_on(&s.auth.app, &["key-add", "ci-bot", "game tests"]).await.unwrap();
    let key = key.trim_end();
    let p = s.publish(Some(key), app("io.ci.app", "1.0.0", "App", "")).await;
    assert_eq!(p.status, 201, "{}", p.body);
    assert_eq!(s.index("community").await.packages[0].publisher, "ci-bot");
    assert_eq!(s.send(reqwest::Method::PUT, "/v1/packages/io.ci.app/like", Some(key), None).await.status, 204);
    // A key without `publish` does not publish.
    let social = auth_api::admin::run_on(&s.auth.app, &["key-add", "ci-bot", "reader", "social"]).await.unwrap();
    assert_eq!(code(&s.publish(Some(social.trim_end()), app("io.ci.app", "1.0.1", "App", "")).await), err(403, "forbidden"));
}
