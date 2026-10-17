//! domain: your names at the registrar, your TLD and your number blocks at IANA
//! (docs/DNS.md).
//!
//! ```text
//! domain account create HANDLE PASSWORD     make an account (and log in)
//! domain login HANDLE PASSWORD              remember who you are (/etc/domain.auth)
//! domain check NAME.ecm                     is it free?
//! domain register NAME.ecm
//! domain set NAME.ecm www A 100.70.0.120 [TTL]
//! domain del NAME.ecm www A [DATA]
//! domain ns NAME.ecm ns1.NAME.ecm=10.200.0.53 [more]    delegate to your own named
//! domain ns NAME.ecm --hosted                          back to records at the registry
//! domain show NAME.ecm | domain list | domain delete NAME.ecm
//! domain tld apply base ns1.base=10.200.0.53 ns2.base=10.201.0.53
//! domain tld show base | domain tld list | domain tld revoke base
//! domain enum apply +1214555 cell-red.ecm [KEY]        a number block for a SIP domain
//! domain enum port +12145550142 cell-blue.ecm CODE KEY  repoint a number (port-in)
//! domain enum list
//! ```
//! Options: `--registrar URL` (default http://nic.ring/), `--iana URL` (http://iana.ring/),
//! `--handle H --password P` instead of the saved login.

use ecm_dns::registrar::url_encode;
use ecm_dns::sys;

const AUTH: &str = "/etc/domain.auth";

fn usage() {
    println!("Usage: domain account create HANDLE PASSWORD | login HANDLE PASSWORD");
    println!("       domain check|register|show|delete NAME.ecm");
    println!("       domain set NAME.ecm NAME TYPE DATA [TTL]     (A, CNAME, MX, TXT; TTL >= 60)");
    println!("       domain del NAME.ecm NAME TYPE [DATA]");
    println!("       domain ns NAME.ecm NS[=ADDRESS]... | --hosted");
    println!("       domain list");
    println!("       domain tld apply TLD NS[=ADDRESS]... | show TLD | list | revoke TLD");
    println!("       domain enum apply BLOCK SIP-DOMAIN [KEY] | port NUMBER SIP-DOMAIN CODE KEY | list");
    println!("Options: --registrar URL (http://nic.ring/), --iana URL (http://iana.ring/), --handle H --password P");
}

fn saved() -> (String, String) {
    let text = std::fs::read_to_string(AUTH).unwrap_or_default();
    let mut h = String::new();
    let mut p = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("handle ") {
            h = v.trim().to_string();
        }
        if let Some(v) = line.strip_prefix("password ") {
            p = v.trim().to_string();
        }
    }
    (h, p)
}

fn post(url: &str, fields: &[(&str, &str)]) -> i32 {
    let body: Vec<String> = fields.iter().map(|(k, v)| format!("{}={}", k, url_encode(v))).collect();
    let api = format!("{}/api", url.trim_end_matches('/'));
    match sys::http_post(&api, &body.join("&"), 15_000) {
        Ok(reply) => {
            let reply = reply.trim_end();
            if let Some(ok) = reply.strip_prefix("OK ") {
                println!("{ok}");
                0
            } else {
                eprintln!("domain: {}", reply.strip_prefix("ERR ").unwrap_or(reply));
                1
            }
        }
        Err(e) => {
            eprintln!("domain: {e}");
            1
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut registrar = "http://nic.ring/".to_string();
    let mut iana = "http://iana.ring/".to_string();
    let (mut handle, mut password) = saved();
    let mut i = 0;
    while i < args.len() {
        let take = |args: &mut Vec<String>, i: usize| -> String {
            let v = args.get(i + 1).cloned().unwrap_or_default();
            args.drain(i..(i + 2).min(args.len()));
            v
        };
        match args[i].as_str() {
            "--registrar" => registrar = take(&mut args, i),
            "--iana" => iana = take(&mut args, i),
            "--handle" => handle = take(&mut args, i),
            "--password" => password = take(&mut args, i),
            _ => i += 1,
        }
    }
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let auth = [("handle", handle.as_str()), ("password", password.as_str())];
    let code = match a.as_slice() {
        ["account", "create", h, p] => {
            let c = post(&registrar, &[("op", "account"), ("handle", h), ("password", p)]);
            if c == 0 && sys::write_file(AUTH, &format!("handle {h}\npassword {p}\n")) {
                println!("logged in as {h} (saved in {AUTH})");
            }
            c
        }
        ["login", h, p] => {
            let c = post(&registrar, &[("op", "login"), ("handle", h), ("password", p)]);
            if c == 0 {
                sys::write_file(AUTH, &format!("handle {h}\npassword {p}\n"));
            }
            c
        }
        ["check", d] => post(&registrar, &[("op", "check"), ("domain", d)]),
        ["register", d] => post(&registrar, &[("op", "register"), ("domain", d), auth[0], auth[1]]),
        ["show", d] => post(&registrar, &[("op", "show"), ("domain", d), auth[0], auth[1]]),
        ["delete", d] => post(&registrar, &[("op", "delete"), ("domain", d), auth[0], auth[1]]),
        ["list"] => post(&registrar, &[("op", "list"), auth[0], auth[1]]),
        ["set", d, name, t, rest @ ..] if !rest.is_empty() => {
            // The last word is a TTL when it is a number and there is more data before it
            // (an MX's preference is a number too: "10 mail").
            let (data, ttl) = match rest {
                [data @ .., ttl] if data.len() >= 1 && ttl.parse::<u32>().is_ok() && !(t.eq_ignore_ascii_case("MX") && data.len() == 1) => {
                    (data.join(" "), ttl.to_string())
                }
                _ => (rest.join(" "), String::new()),
            };
            post(&registrar, &[("op", "set"), ("domain", d), ("name", name), ("type", t), ("data", &data), ("ttl", &ttl), auth[0], auth[1]])
        }
        ["del", d, name, t, rest @ ..] => {
            let data = rest.join(" ");
            post(&registrar, &[("op", "del"), ("domain", d), ("name", name), ("type", t), ("data", &data), auth[0], auth[1]])
        }
        ["ns", d, "--hosted"] => post(&registrar, &[("op", "ns"), ("domain", d), ("ns", ""), auth[0], auth[1]]),
        ["ns", d, list @ ..] if !list.is_empty() => {
            let ns = list.join(",");
            post(&registrar, &[("op", "ns"), ("domain", d), ("ns", &ns), auth[0], auth[1]])
        }
        ["tld", "apply", t, list @ ..] if !list.is_empty() => {
            let ns = list.join(",");
            post(&iana, &[("op", "apply"), ("tld", t), ("ns", &ns), auth[0], auth[1]])
        }
        ["tld", "show", t] => post(&iana, &[("op", "show"), ("tld", t)]),
        ["tld", "list"] => post(&iana, &[("op", "list")]),
        ["tld", "revoke", t] => post(&iana, &[("op", "revoke"), ("tld", t), auth[0], auth[1]]),
        ["enum", "apply", b, d] => post(&iana, &[("op", "enum-apply"), ("block", b), ("domain", d)]),
        ["enum", "apply", b, d, k] => post(&iana, &[("op", "enum-apply"), ("block", b), ("domain", d), ("key", k)]),
        ["enum", "port", n, d, code, k] => post(&iana, &[("op", "enum-port"), ("number", n), ("domain", d), ("code", code), ("key", k)]),
        ["enum", "list"] => post(&iana, &[("op", "enum-list")]),
        _ => {
            usage();
            1
        }
    };
    std::process::exit(code);
}
