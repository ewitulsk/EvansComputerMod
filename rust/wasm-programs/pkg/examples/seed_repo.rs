//! Publish the fixture packages the repository scenarios install
//! (`repo_open`, `repo_internet`, `repo_datacenter`, `repo_curated`), and put
//! all but one in the `stable` channel. Run by `scripts/repo-backend-local.sh`:
//!
//!   cargo run -p pkg --example seed_repo -- <origin> <publisher key> <admin key> <program.wasm>
//!
//! It is the game's own client code (`ecm_pkg`) on the host's network, so a
//! seeded repository has been published to exactly as `pkg publish` does it.

use ecm_pkg::account::{self, Account, MemorySecrets};
use ecm_pkg::client::{Call, Net, Progress, Reply};
use ecm_pkg::http::Url;
use ecm_pkg::package;

fn toml(id: &str, name: &str, rest: &str) -> Vec<u8> {
    format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\nsummary = \"A fixture for the repository scenarios\"\npublisher = \"ECM tests\"\n{rest}\n").into_bytes()
}

/// The local stack is plain http on this machine. The game's client refuses
/// to send a sign-in over plain http (and to sign in at all), so this tool
/// carries the calls itself, on the host's sockets: same requests, no TLS.
struct LocalNet;

impl LocalNet {
    fn send(base: &Url, request: Vec<u8>) -> Result<(u16, Vec<u8>), String> {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect((base.host.as_str(), base.port)).map_err(|e| format!("{}: {e}", base.origin()))?;
        s.write_all(&request).map_err(|e| e.to_string())?;
        let mut all = Vec::new();
        s.read_to_end(&mut all).map_err(|e| e.to_string())?;
        let cut = all.windows(4).position(|w| w == b"\r\n\r\n").ok_or("no http answer")?;
        let head = String::from_utf8_lossy(&all[..cut]).to_string();
        let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("no http status")?;
        Ok((status, all[cut + 4..].to_vec()))
    }
}

impl Net for LocalNet {
    fn get(&mut self, base: &Url, file: &str, _progress: Progress) -> Result<Vec<u8>, String> {
        let request = format!("GET {}{} HTTP/1.0\r\nHost: {}:{}\r\nConnection: close\r\n\r\n", base.path, file, base.host, base.port);
        match Self::send(base, request.into_bytes())? {
            (200, body) => Ok(body),
            (status, _) => Err(format!("{file}: http {status}")),
        }
    }

    fn call(&mut self, base: &Url, call: &Call) -> Result<Reply, String> {
        let mut head = format!("{} {} HTTP/1.0\r\nHost: {}:{}\r\nConnection: close\r\nContent-Length: {}\r\n", call.method, call.path, base.host, base.port, call.body.len());
        if !call.content_type.is_empty() {
            head.push_str(&format!("Content-Type: {}\r\n", call.content_type));
        }
        if let Some(t) = &call.token {
            head.push_str(&format!("Authorization: Bearer {t}\r\n"));
        }
        for (k, v) in &call.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let mut request = head.into_bytes();
        request.extend_from_slice(&call.body);
        let (status, body) = Self::send(base, request)?;
        Ok(Reply { status, body })
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [origin, publisher_key, admin_key, program] = args.as_slice() else {
        eprintln!("usage: seed_repo <origin> <publisher key> <admin key> <program.wasm>");
        std::process::exit(2);
    };
    let base = Url::parse(&format!("{}/r/community/", origin.trim_end_matches('/'))).expect("origin");
    let wasm = std::fs::read(program).unwrap_or_else(|e| panic!("cannot read {program}: {e}"));
    let mut net = LocalNet;
    let me = Account { origin: base.origin(), handle: "ecmadmin".into(), token: publisher_key.clone(), scopes: vec!["publish".into()] };
    // (files, promote to stable?)
    let fixtures: Vec<(Vec<(String, Vec<u8>)>, bool)> = vec![
        (
            vec![
                ("app.toml".into(), toml("com.ecmtest.hello", "Hello", "entry = \"main.lua\"")),
                ("main.lua".into(), b"print(\"hello from the repository\")\n".to_vec()),
            ],
            true,
        ),
        (
            vec![
                ("app.toml".into(), toml("com.ecmtest.geo", "Geometry", "kind = \"library\"\nprovides = [\"ecmgeo\"]")),
                ("python/ecmgeo.py".into(), b"def area(w, h):\n    return w * h\n".to_vec()),
                ("lua/ecmgeo.lua".into(), b"return { area = function(w, h) return w * h end }\n".to_vec()),
            ],
            true,
        ),
        (
            vec![
                ("app.toml".into(), toml("com.ecmtest.area", "Area", "entry = \"main.lua\"\ndepends = [\"com.ecmtest.geo >= 1.0.0\"]")),
                ("main.lua".into(), b"print(\"area is \" .. require(\"ecmgeo\").area(6, 7))\n".to_vec()),
            ],
            true,
        ),
        (
            vec![
                ("app.toml".into(), toml("com.ecmtest.shout", "Shout", "kind = \"program\"\nbin = [\"ecmshout.wasm\"]")),
                ("ecmshout.wasm".into(), wasm),
            ],
            true,
        ),
        (
            vec![
                ("app.toml".into(), toml("com.ecmtest.fresh", "Fresh", "entry = \"main.lua\"")),
                ("main.lua".into(), b"print(\"only in the community channel\")\n".to_vec()),
            ],
            false,
        ),
    ];
    let mut secrets = MemorySecrets::default();
    for (files, stable) in fixtures {
        let (p, bytes) = package::build(files).expect("fixture builds");
        let id = p.manifest.id.clone();
        match account::publish(&mut net, &mut secrets, &base, Some(&me), &bytes) {
            Ok(done) => println!("published {} {} ({})", done.id, done.version, done.channel),
            Err(e) if e.contains("already published") => println!("{id} 1.0.0 is already there"),
            Err(e) => panic!("publish {id}: {e}"),
        }
        if stable {
            let body = serde_json::json!({ "id": id, "version": "1.0.0", "channel": "stable" });
            let r = net.call(&base, &Call::json("POST", "/v1/admin/promote", &body).token(Some(admin_key))).unwrap_or_else(|e| panic!("promote {id}: {e}"));
            assert!(r.ok(), "promote {id}: {}", r.error());
            println!("  in stable");
        }
    }
    let ix = net.get(&Url::parse(&format!("{}/r/stable/", origin.trim_end_matches('/'))).unwrap(), "index.json", &mut |_, _| {}).expect("stable index");
    let ix = package::parse_index(&String::from_utf8_lossy(&ix)).expect("index parses");
    println!("stable: {} packages, key {}", ix.packages.len(), ix.publisher_key.unwrap_or_default());
}
