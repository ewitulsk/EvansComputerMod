//! Signing in to a repository that has accounts (the official one,
//! docs/REPO_API.md), and what a signed-in player can do there: publish,
//! yank, like and favorite. Anyone can download without any of this.
//!
//! A sign-in is a token for one repository origin. It is kept in the
//! computer's secret store when there is one (docs/REPO.md "Secret store"),
//! otherwise in `/etc/pkg/account`. It is only ever sent to the origin it was
//! made for, and only over `https://` or the server's own link.

use crate::client::{Call, Net, Reply};
use crate::http::Url;
use crate::package;
use serde_json::{json, Value};

/// A sign-in.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Account {
    /// `https://repo.example` (or `host://repo`).
    pub origin: String,
    pub handle: String,
    pub token: String,
    pub scopes: Vec<String>,
}

impl Account {
    pub fn to_json(&self) -> String {
        json!({ "origin": self.origin, "handle": self.handle, "token": self.token, "scopes": self.scopes }).to_string()
    }

    pub fn parse(text: &str) -> Option<Account> {
        let v: Value = serde_json::from_str(text).ok()?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let a = Account {
            origin: s("origin"),
            handle: s("handle"),
            token: s("token"),
            scopes: v.get("scopes").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        };
        (!a.origin.is_empty() && !a.token.is_empty()).then_some(a)
    }

    pub fn can(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

/// Where sign-ins and the author's signing key are kept.
pub trait Secrets {
    fn get(&self, name: &str) -> Option<String>;
    fn put(&mut self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&mut self, name: &str);
    /// The author's public key (hex), making the key if there is none.
    fn signing_public(&mut self) -> Result<String, String>;
    /// The author's signature (hex) over `message`.
    fn sign(&mut self, message: &[u8]) -> Result<String, String>;
    /// Ask the player to confirm publishing. `Ok` is yes.
    fn confirm(&mut self, text: &str) -> Result<(), String> {
        let _ = text;
        Ok(())
    }
    /// Where secrets are, for `pkg whoami`.
    fn describe(&self) -> String;
}

/// The name a sign-in is stored under.
pub fn secret_name(origin: &str) -> String {
    let host: String = origin.split("://").nth(1).unwrap_or(origin).chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    format!("pkg.account.{host}")
}

pub fn load(secrets: &dyn Secrets, base: &Url) -> Option<Account> {
    let a = Account::parse(&secrets.get(&secret_name(&base.origin()))?)?;
    // A stored sign-in is for one origin; never offer it to another.
    (a.origin == base.origin()).then_some(a)
}

pub fn save(secrets: &mut dyn Secrets, a: &Account) -> Result<(), String> {
    secrets.put(&secret_name(&a.origin), &a.to_json())
}

pub fn forget(secrets: &mut dyn Secrets, base: &Url) {
    secrets.delete(&secret_name(&base.origin()));
}

/// Secrets in memory (tests), signing with a fixed key.
#[derive(Default)]
pub struct MemorySecrets {
    pub values: std::collections::HashMap<String, String>,
    pub seed: Option<String>,
    /// What the player answers to a confirmation.
    pub decline: bool,
    pub asked: Vec<String>,
}

impl Secrets for MemorySecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
    fn put(&mut self, name: &str, value: &str) -> Result<(), String> {
        self.values.insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&mut self, name: &str) {
        self.values.remove(name);
    }
    fn signing_public(&mut self) -> Result<String, String> {
        let seed = self.seed.get_or_insert_with(|| package::hex(&[0x42; 32])).clone();
        crate::sign::public_of(&seed).ok_or_else(|| "bad key".into())
    }
    fn sign(&mut self, message: &[u8]) -> Result<String, String> {
        self.signing_public()?;
        Ok(crate::sign::sign(self.seed.as_deref().unwrap_or(""), message))
    }
    fn confirm(&mut self, text: &str) -> Result<(), String> {
        self.asked.push(text.to_string());
        if self.decline {
            Err("not confirmed".into())
        } else {
            Ok(())
        }
    }
    fn describe(&self) -> String {
        "memory".into()
    }
}

/// The computer's secret store, or files under `root` when it has none.
pub struct HostSecrets {
    pub root: String,
}

const SIGNING_NAME: &str = "pkg.signing";

impl HostSecrets {
    pub fn new(root: &str) -> HostSecrets {
        HostSecrets { root: root.trim_end_matches('/').to_string() }
    }

    fn has_store() -> bool {
        use ecm_host_abi::keystore::{self, Error};
        !matches!(keystore::get("pkg.probe"), Err(Error::NoStore))
    }

    fn file(&self) -> String {
        format!("{}{}", self.root, crate::paths::ACCOUNT_FILE)
    }

    /// The fallback file: one JSON object, name -> value.
    fn file_values(&self) -> serde_json::Map<String, Value> {
        std::fs::read_to_string(self.file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
    }

    fn write_file(&self, m: &serde_json::Map<String, Value>) -> Result<(), String> {
        let f = self.file();
        if let Some(d) = std::path::Path::new(&f).parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&f, Value::Object(m.clone()).to_string() + "\n").map_err(|e| format!("cannot write {}: {}", crate::paths::ACCOUNT_FILE, e))
    }

    fn file_seed(&self) -> Result<String, String> {
        let path = format!("{}{}", self.root, crate::paths::SIGNING_KEY);
        if let Ok(t) = std::fs::read_to_string(&path) {
            return Ok(t.trim().to_string());
        }
        let (_, secret) = crate::sign::generate();
        if let Some(d) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&path, &secret).map_err(|e| format!("cannot write {}: {}", crate::paths::SIGNING_KEY, e))?;
        Ok(secret)
    }
}

impl Secrets for HostSecrets {
    fn get(&self, name: &str) -> Option<String> {
        if HostSecrets::has_store() {
            return ecm_host_abi::keystore::get_string(name).ok();
        }
        self.file_values().get(name).and_then(|v| v.as_str()).map(str::to_string)
    }

    fn put(&mut self, name: &str, value: &str) -> Result<(), String> {
        if HostSecrets::has_store() {
            return ecm_host_abi::keystore::put(name, value.as_bytes()).map_err(|e| e.message().to_string());
        }
        let mut m = self.file_values();
        m.insert(name.into(), value.into());
        self.write_file(&m)
    }

    fn delete(&mut self, name: &str) {
        if HostSecrets::has_store() {
            let _ = ecm_host_abi::keystore::delete(name);
            return;
        }
        let mut m = self.file_values();
        if m.remove(name).is_some() {
            let _ = self.write_file(&m);
        }
    }

    fn signing_public(&mut self) -> Result<String, String> {
        if HostSecrets::has_store() {
            return ecm_host_abi::keystore::keygen(SIGNING_NAME).map(|k| package::hex(&k)).map_err(|e| e.message().to_string());
        }
        crate::sign::public_of(&self.file_seed()?).ok_or_else(|| format!("{} is not a signing key", crate::paths::SIGNING_KEY))
    }

    fn sign(&mut self, message: &[u8]) -> Result<String, String> {
        if HostSecrets::has_store() {
            ecm_host_abi::keystore::keygen(SIGNING_NAME).map_err(|e| e.message().to_string())?;
            return ecm_host_abi::keystore::sign(SIGNING_NAME, message).map(|s| package::hex(&s)).map_err(|e| e.message().to_string());
        }
        let seed = self.file_seed()?;
        crate::sign::public_of(&seed).ok_or_else(|| format!("{} is not a signing key", crate::paths::SIGNING_KEY))?;
        Ok(crate::sign::sign(&seed, message))
    }

    fn confirm(&mut self, text: &str) -> Result<(), String> {
        use ecm_host_abi::keystore::{self, Error};
        match keystore::confirm(text, 60_000) {
            Ok(()) => Ok(()),
            // No store: there is nobody to ask, and nothing more to protect than the file.
            Err(Error::NoStore) => Ok(()),
            Err(Error::Declined) => Err("not confirmed (click Allow in the chat within a minute)".into()),
            Err(e) => Err(e.message().to_string()),
        }
    }

    fn describe(&self) -> String {
        if HostSecrets::has_store() {
            "the computer's secret store".into()
        } else {
            format!("{} (this computer has no secret store)", crate::paths::ACCOUNT_FILE)
        }
    }
}

// ---------------------------------------------------------------- device sign-in

/// What `pkg login` shows: a short code to type on the repository's web page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevicePoll {
    Pending,
    Granted(Account),
    Denied,
    Expired,
}

fn expect(reply: Reply, what: &str) -> Result<Value, String> {
    if reply.ok() {
        Ok(reply.json())
    } else {
        Err(format!("{}: {}", what, reply.error()))
    }
}

/// Ask the repository for a sign-in code. `client_name` names this computer on the account page.
pub fn device_start(net: &mut dyn Net, base: &Url, client_name: &str, scopes: &[&str]) -> Result<DeviceStart, String> {
    if !base.private() {
        return Err(format!("{} is not encrypted; signing in needs an https:// repository", base.origin()));
    }
    let v = expect(net.call(base, &Call::json("POST", "/auth/v1/device/start", &json!({ "client_name": client_name, "scopes": scopes })))?, "cannot start signing in")?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let n = |k: &str, d: u64| v.get(k).and_then(|x| x.as_u64()).unwrap_or(d);
    let d = DeviceStart {
        device_code: s("device_code"),
        user_code: s("user_code"),
        verification_uri: s("verification_uri"),
        verification_uri_complete: s("verification_uri_complete"),
        interval: n("interval", 3).clamp(1, 30),
        expires_in: n("expires_in", 600),
    };
    if d.device_code.is_empty() || d.user_code.is_empty() {
        return Err("the repository's answer has no sign-in code".into());
    }
    Ok(d)
}

/// Has the player approved the code yet?
pub fn device_poll(net: &mut dyn Net, base: &Url, device_code: &str) -> Result<DevicePoll, String> {
    let r = net.call(base, &Call::json("POST", "/auth/v1/device/token", &json!({ "device_code": device_code })))?;
    match r.status {
        200 => {
            let v = r.json();
            let a = Account {
                origin: base.origin(),
                handle: v.get("handle").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                token: v.get("token").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                scopes: v.get("scopes").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            };
            if a.token.is_empty() {
                return Err("the repository's answer has no token".into());
            }
            Ok(DevicePoll::Granted(a))
        }
        428 => Ok(DevicePoll::Pending),
        403 => Ok(DevicePoll::Denied),
        410 => Ok(DevicePoll::Expired),
        _ => Err(format!("signing in failed: {}", r.error())),
    }
}

/// Who a token belongs to: checks an API key typed in with `pkg login --key`.
pub fn whoami(net: &mut dyn Net, base: &Url, token: &str) -> Result<Account, String> {
    if !base.private() {
        return Err(format!("{} is not encrypted; signing in needs an https:// repository", base.origin()));
    }
    let r = net.call(base, &Call::get("/auth/v1/whoami").token(Some(token)))?;
    if r.status == 401 {
        return Err("that key is not valid (or was revoked); make a new one on the account page".into());
    }
    let v = expect(r, "cannot check the sign-in")?;
    Ok(Account {
        origin: base.origin(),
        handle: v.get("handle").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        token: token.to_string(),
        scopes: v.get("scopes").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    })
}

/// Sign out: forget the sign-in. One made here with a code (`ecm_d_...`) is
/// also revoked at the repository (best effort). An API key is not: the player
/// made it on the account page, may use it elsewhere, and revokes it there.
pub fn logout(net: &mut dyn Net, secrets: &mut dyn Secrets, base: &Url) -> Option<Account> {
    let a = load(secrets, base)?;
    if a.token.starts_with("ecm_d_") {
        let _ = net.call(base, &Call { method: "POST".into(), path: "/auth/v1/token/revoke".into(), token: Some(a.token.clone()), ..Call::default() });
    }
    forget(secrets, base);
    Some(a)
}

// ---------------------------------------------------------------- the catalogue

/// A package as the repository lists it, with what the index does not carry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub kind: String,
    pub version: String,
    pub owner: String,
    pub channels: Vec<String>,
    pub likes: u64,
    pub downloads: u64,
    pub liked: bool,
    pub favorited: bool,
}

impl Summary {
    pub fn from_json(v: &Value) -> Option<Summary> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        let id = s("id");
        crate::manifest::valid_id(&id).then(|| Summary {
            id,
            name: s("name"),
            summary: s("summary"),
            kind: s("kind"),
            version: s("version"),
            owner: s("owner"),
            channels: v.get("channels").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            likes: n("likes"),
            downloads: n("downloads"),
            liked: b("liked"),
            favorited: b("favorited"),
        })
    }
}

fn summaries(v: &Value) -> Vec<Summary> {
    let list = v.get("packages").unwrap_or(v);
    list.as_array().map(|a| a.iter().filter_map(Summary::from_json).collect()).unwrap_or_default()
}

fn need(account: Option<&Account>, scope: &str, base: &Url) -> Result<String, String> {
    let a = account.ok_or_else(|| format!("sign in first: pkg login {}", base.origin()))?;
    if !a.can(scope) {
        return Err(format!("this sign-in may not {} (sign in again: pkg login)", if scope == "publish" { "publish" } else { "like or favorite" }));
    }
    Ok(a.token.clone())
}

/// One package's likes, downloads and whether the signed-in player liked or favorited it.
pub fn detail(net: &mut dyn Net, base: &Url, account: Option<&Account>, id: &str) -> Result<Summary, String> {
    let v = expect(net.call(base, &Call::get(&format!("/v1/packages/{id}")).token(account.map(|a| a.token.as_str())))?, id)?;
    Summary::from_json(&v).ok_or_else(|| format!("{}: bad answer from the repository", id))
}

/// The packages of a channel with their likes: `sort` is `likes`, `new`, `downloads` or `name`.
pub fn browse(net: &mut dyn Net, base: &Url, account: Option<&Account>, channel: &str, sort: &str) -> Result<Vec<Summary>, String> {
    let path = format!("/v1/packages?channel={channel}&sort={sort}");
    Ok(summaries(&expect(net.call(base, &Call::get(&path).token(account.map(|a| a.token.as_str())))?, "cannot list packages")?))
}

/// `/v1/me/favorites`, `/v1/me/packages` or `/v1/me/likes`.
pub fn mine(net: &mut dyn Net, base: &Url, account: Option<&Account>, what: &str) -> Result<Vec<Summary>, String> {
    let token = need(account, "social", base)?;
    Ok(summaries(&expect(net.call(base, &Call::get(&format!("/v1/me/{what}")).token(Some(&token)))?, "cannot read your list")?))
}

/// Like or favorite (`what`), or take it back (`on` false).
pub fn mark(net: &mut dyn Net, base: &Url, account: Option<&Account>, id: &str, what: &str, on: bool) -> Result<(), String> {
    let token = need(account, "social", base)?;
    let call = Call { method: if on { "PUT" } else { "DELETE" }.into(), path: format!("/v1/packages/{id}/{what}"), token: Some(token), ..Call::default() };
    let r = net.call(base, &call)?;
    if r.status == 404 {
        return Err(format!("{} is not in this repository", id));
    }
    expect(r, id).map(|_| ())
}

/// The repository's answer to a publish.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Published {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub channel: String,
}

/// Publish a package: checked here first, confirmed by the player, signed as
/// its author and uploaded.
pub fn publish(net: &mut dyn Net, secrets: &mut dyn Secrets, base: &Url, account: Option<&Account>, bytes: &[u8]) -> Result<Published, String> {
    let token = need(account, "publish", base)?;
    let p = package::open(bytes)?;
    let m = &p.manifest;
    let handle = account.map(|a| a.handle.as_str()).unwrap_or("");
    let what = if m.kind.unconfined() { format!("the {} {}", m.kind.name(), m.id) } else { m.id.clone() };
    secrets.confirm(&format!("Publish {} {} to {} as {}?", what, m.version, base.host, handle))?;
    let message = package::signed_message(&m.id, &m.version, &p.sha256);
    let key = secrets.signing_public()?;
    let signature = secrets.sign(&message)?;
    let call = Call {
        method: "PUT".into(),
        path: "/v1/publish".into(),
        token: Some(token),
        headers: vec![("X-Ecm-Author-Key".into(), key), ("X-Ecm-Author-Signature".into(), signature)],
        content_type: "application/octet-stream".into(),
        body: bytes.to_vec(),
    };
    let r = net.call(base, &call)?;
    let v = match r.status {
        409 => return Err(format!("{} {} is already published; raise the version in app.toml", m.id, m.version)),
        413 => return Err(format!("the repository says the package is too large: {}", r.error())),
        _ => expect(r, "the repository refused the package")?,
    };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    Ok(Published { id: s("id"), version: s("version"), kind: s("kind"), channel: s("channel") })
}

/// Take a published version out of the listings.
pub fn yank(net: &mut dyn Net, base: &Url, account: Option<&Account>, id: &str, version: &str) -> Result<(), String> {
    let token = need(account, "publish", base)?;
    let r = net.call(base, &Call::json("POST", &format!("/v1/packages/{id}/yank"), &json!({ "version": version })).token(Some(&token)))?;
    expect(r, &format!("cannot yank {id} {version}")).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MemoryNet;

    fn base() -> Url {
        Url::parse("https://repo.test/r/community/").unwrap()
    }

    fn reply(status: u16, body: Value) -> Reply {
        Reply { status, body: body.to_string().into_bytes() }
    }

    #[test]
    fn device_sign_in_and_storage() {
        let mut net = MemoryNet::default();
        net.api.insert(
            "POST https://repo.test/auth/v1/device/start".into(),
            reply(200, json!({"device_code":"dc","user_code":"ABCD-EFGH","verification_uri":"https://repo.test/device","verification_uri_complete":"https://repo.test/device?code=ABCD-EFGH","interval":3,"expires_in":600})),
        );
        let d = device_start(&mut net, &base(), "Evan's computer", &["social", "publish"]).unwrap();
        assert_eq!((d.user_code.as_str(), d.interval), ("ABCD-EFGH", 3));
        assert_eq!(net.calls[0].body, br#"{"client_name":"Evan's computer","scopes":["social","publish"]}"#);
        // Pending, then granted.
        net.api.insert("POST https://repo.test/auth/v1/device/token".into(), reply(428, json!({"error":"pending","message":"not approved yet"})));
        assert_eq!(device_poll(&mut net, &base(), "dc").unwrap(), DevicePoll::Pending);
        net.api.insert("POST https://repo.test/auth/v1/device/token".into(), reply(200, json!({"token":"ecm_d_x","handle":"evan","scopes":["social","publish"]})));
        let DevicePoll::Granted(a) = device_poll(&mut net, &base(), "dc").unwrap() else { panic!() };
        assert_eq!((a.origin.as_str(), a.handle.as_str(), a.can("publish"), a.can("admin")), ("https://repo.test", "evan", true, false));
        for (status, want) in [(403, DevicePoll::Denied), (410, DevicePoll::Expired)] {
            net.api.insert("POST https://repo.test/auth/v1/device/token".into(), reply(status, json!({})));
            assert_eq!(device_poll(&mut net, &base(), "dc").unwrap(), want);
        }

        let mut secrets = MemorySecrets::default();
        save(&mut secrets, &a).unwrap();
        assert_eq!(load(&secrets, &base()), Some(a.clone()));
        // Another origin never sees it, even under the same host name on plain http.
        assert_eq!(load(&secrets, &Url::parse("http://repo.test/").unwrap()), None);
        assert_eq!(load(&secrets, &Url::parse("https://other.test/").unwrap()), None);
        assert!(a.token.starts_with("ecm_d_"), "{}", a.token);
        assert_eq!(logout(&mut net, &mut secrets, &base()), Some(a.clone()));
        assert_eq!(net.calls.last().unwrap().path, "/auth/v1/token/revoke");
        assert_eq!(load(&secrets, &base()), None);
        // An API key is forgotten here but not revoked: it is the player's, from the account page.
        let key = Account { token: "ecm_k_made_on_the_account_page".into(), ..a };
        save(&mut secrets, &key).unwrap();
        let before = net.calls.len();
        assert_eq!(logout(&mut net, &mut secrets, &base()), Some(key));
        assert_eq!(net.calls.len(), before, "no call to the repository");
        assert_eq!(load(&secrets, &base()), None);
        // No signing in over plain http.
        assert!(device_start(&mut net, &Url::parse("http://10.0.0.1:8080/").unwrap(), "x", &[]).unwrap_err().contains("not encrypted"));
    }

    #[test]
    fn api_keys_likes_and_lists() {
        let mut net = MemoryNet::default();
        net.api.insert("GET https://repo.test/auth/v1/whoami".into(), reply(200, json!({"handle":"evan","email_verified":true,"scopes":["social"],"token_name":"k"})));
        let a = whoami(&mut net, &base(), "ecm_k_abc").unwrap();
        assert_eq!((a.handle.as_str(), a.token.as_str()), ("evan", "ecm_k_abc"));
        assert_eq!(net.calls[0].all_headers(), vec![("Authorization".to_string(), "Bearer ecm_k_abc".to_string())]);
        net.api.insert("GET https://repo.test/auth/v1/whoami".into(), reply(401, json!({"error":"unauthorized","message":"bad token"})));
        assert!(whoami(&mut net, &base(), "nope").unwrap_err().contains("not valid"));

        net.api.insert("PUT https://repo.test/v1/packages/dev.t.app/like".into(), Reply { status: 204, body: vec![] });
        net.api.insert("DELETE https://repo.test/v1/packages/dev.t.app/favorite".into(), Reply { status: 204, body: vec![] });
        mark(&mut net, &base(), Some(&a), "dev.t.app", "like", true).unwrap();
        mark(&mut net, &base(), Some(&a), "dev.t.app", "favorite", false).unwrap();
        assert!(mark(&mut net, &base(), Some(&a), "dev.t.nope", "like", true).unwrap_err().contains("not in this repository"));
        assert!(mark(&mut net, &base(), None, "dev.t.app", "like", true).unwrap_err().contains("sign in first: pkg login https://repo.test"));
        // This key may like but not publish.
        assert!(yank(&mut net, &base(), Some(&a), "dev.t.app", "1.0.0").unwrap_err().contains("may not publish"));

        let one = json!({"id":"dev.t.app","name":"App","summary":"s","kind":"app","version":"1.2.0","owner":"evan","channels":["community"],"likes":7,"downloads":40,"liked":true,"favorited":false});
        net.api.insert("GET https://repo.test/v1/me/favorites".into(), reply(200, json!([one.clone(), {"id":"not an id"}])));
        let f = mine(&mut net, &base(), Some(&a), "favorites").unwrap();
        assert_eq!((f.len(), f[0].likes, f[0].liked, f[0].channels.clone()), (1, 7, true, vec!["community".to_string()]));
        net.api.insert("GET https://repo.test/v1/packages?channel=community&sort=likes".into(), reply(200, json!({"total":1,"page":1,"per_page":50,"packages":[one.clone()]})));
        assert_eq!(browse(&mut net, &base(), None, "community", "likes").unwrap()[0].downloads, 40);
        net.api.insert("GET https://repo.test/v1/packages/dev.t.app".into(), reply(200, one));
        assert_eq!(detail(&mut net, &base(), None, "dev.t.app").unwrap().version, "1.2.0");
    }

    #[test]
    fn publishing_is_confirmed_signed_and_explained() {
        let (p, bytes) = crate::package::build(vec![
            ("app.toml".into(), b"id = \"dev.evan.tool\"\nname = \"Tool\"\nversion = \"1.0.0\"\nkind = \"program\"\nbin = [\"tool.wasm\"]\n".to_vec()),
            ("tool.wasm".into(), vec![0]),
        ])
        .unwrap();
        let a = Account { origin: "https://repo.test".into(), handle: "evan".into(), token: "ecm_d_x".into(), scopes: vec!["publish".into(), "social".into()] };
        let mut net = MemoryNet::default();
        let mut secrets = MemorySecrets::default();
        net.api.insert("PUT https://repo.test/v1/publish".into(), reply(201, json!({"id":"dev.evan.tool","version":"1.0.0","kind":"program","channel":"community","file":"dev.evan.tool-1.0.0.ecmpkg","sha256":p.sha256})));
        let done = publish(&mut net, &mut secrets, &base(), Some(&a), &bytes).unwrap();
        assert_eq!((done.id.as_str(), done.channel.as_str()), ("dev.evan.tool", "community"));
        assert_eq!(secrets.asked, vec!["Publish the program dev.evan.tool 1.0.0 to repo.test as evan?"]);
        // The upload carries the author's key and a signature that checks out.
        let call = net.calls.last().unwrap().clone();
        let h = |n: &str| call.headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap();
        assert!(crate::sign::verify(&h("X-Ecm-Author-Key"), &package::signed_message("dev.evan.tool", "1.0.0", &p.sha256), &h("X-Ecm-Author-Signature")));
        assert_eq!((call.body.len(), call.content_type.as_str()), (bytes.len(), "application/octet-stream"));

        // The player says no: nothing is sent.
        let sent = net.calls.len();
        secrets.decline = true;
        assert!(publish(&mut net, &mut secrets, &base(), Some(&a), &bytes).unwrap_err().contains("not confirmed"));
        assert_eq!(net.calls.len(), sent);
        secrets.decline = false;
        // A broken package never leaves the computer either.
        assert!(publish(&mut net, &mut secrets, &base(), Some(&a), b"junk").is_err());
        assert_eq!(net.calls.len(), sent);
        // The repository's refusals, in words.
        net.api.insert("PUT https://repo.test/v1/publish".into(), reply(409, json!({"error":"conflict","message":"exists"})));
        assert!(publish(&mut net, &mut secrets, &base(), Some(&a), &bytes).unwrap_err().contains("raise the version"));
        net.api.insert("PUT https://repo.test/v1/publish".into(), reply(403, json!({"error":"forbidden","message":"dev.evan.tool belongs to another account"})));
        assert!(publish(&mut net, &mut secrets, &base(), Some(&a), &bytes).unwrap_err().contains("belongs to another account"));
        assert!(publish(&mut net, &mut secrets, &base(), None, &bytes).unwrap_err().contains("sign in first"));
    }
}
